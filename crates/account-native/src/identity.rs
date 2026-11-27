//! Native identity rotation. Refresh locks span the provider exchange; the barrier lock
//! serializes changes to the journal and credential store, but never spans online verification.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError as MutexTryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use solidcraft_engine::entitlement::CredentialStore;

use crate::credentials::{MAX_ACCOUNT_BYTES, safe_token};
use crate::http::{Secret, WireConnection};
use crate::identity_journal::{AttemptRecord, IdentityJournal, JournalError, RevocationRecord};
use crate::identity_store::{CredentialIdentityStore, IdentityStore, IdentityStoreError, StoredIdentity};
use crate::identity_transport::{IdentityTransport, RefreshResponse, RefreshTransportError, UreqIdentityTransport, UreqRefreshHttp};
use crate::token_expiry::access_token_expiry_unix;

const REFRESH_LEAD_MAX: Duration = Duration::from_secs(60);
const REFRESH_LEAD_DIVISOR: u64 = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
// The batch/replay state machine is implemented in the next task.
#[allow(dead_code)]
const BATCH_BUDGET: Duration = Duration::from_secs(20);
#[allow(dead_code)]
const MAX_SENDS: u8 = 3;
#[allow(dead_code)]
const REPLAY_WINDOW: Duration = Duration::from_secs(30);
const COOLDOWN_BASE: Duration = Duration::from_secs(5);
const COOLDOWN_MAX: Duration = Duration::from_secs(60);
const LOCK_POLL: Duration = Duration::from_millis(5);
const MUTATION_WAIT: Duration = Duration::from_secs(1);
const RECORD_VERSION: u32 = 1;
// Match the pinned credential store's bounds before deleting any existing login.
const MAX_TOKEN_BYTES: usize = 2400;
const MAX_BINDING_BYTES: usize = 128;

/// Isolation key for one environment, public OAuth client, and product origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityDomain {
    client_id: String,
    product_origin: String,
    storage_key: String,
}

impl IdentityDomain {
    pub fn new(environment: &str, client_id: &str, product_origin: &str) -> Option<Self> {
        if !valid_text(environment, 64)
            || !valid_text(client_id, 256)
            || client_id.bytes().any(|byte| byte.is_ascii_whitespace())
            || !valid_text(product_origin, 512)
            || !product_origin.starts_with("https://")
        {
            return None;
        }
        let mut hash = Sha256::new();
        hash.update(environment);
        hash.update(b"\0");
        hash.update(client_id);
        hash.update(b"\0");
        hash.update(product_origin);
        Some(Self { client_id: client_id.to_owned(), product_origin: product_origin.to_owned(), storage_key: format!("{:x}", hash.finalize()) })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn storage_key(&self) -> &str {
        &self.storage_key
    }
}

fn valid_text(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}

pub trait IdentityClock: Send + Sync {
    fn now_unix_seconds(&self) -> u64;
    fn monotonic_now(&self) -> Duration;
}

pub struct SystemIdentityClock {
    origin: Instant,
}

impl SystemIdentityClock {
    pub fn new() -> Self {
        Self { origin: Instant::now() }
    }
}

impl Default for SystemIdentityClock {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentityClock for SystemIdentityClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    fn monotonic_now(&self) -> Duration {
        self.origin.elapsed()
    }
}

pub trait SessionVerifier: Send + Sync {
    fn verify(&self, access: &Secret) -> Result<VerifiedSession, VerifyError>;
}

pub struct VerifiedSession {
    pub account_id: String,
    pub same_login_binding: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    Transient,
    Rejected,
    Protocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityTrigger {
    Background,
    UserRetry,
    Resume,
}

pub struct IdentityLease {
    access_token: Secret,
    login_generation: u64,
    token_generation: u64,
    expires_at_unix: u64,
}

impl IdentityLease {
    pub fn access_token(&self) -> &Secret {
        &self.access_token
    }

    pub fn login_generation(&self) -> u64 {
        self.login_generation
    }

    pub fn token_generation(&self) -> u64 {
        self.token_generation
    }

    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix
    }
}

impl fmt::Debug for IdentityLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityLease")
            .field("access_token_present", &!self.access_token.is_empty())
            .field("login_generation", &self.login_generation)
            .field("token_generation", &self.token_generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityError {
    NotConfigured,
    SignedOut,
    Transient,
    VerificationPending,
    SessionChanged,
    Rejected,
    Protocol,
    Storage,
    Busy,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotConfigured => "identity is not configured",
            Self::SignedOut => "identity is signed out",
            Self::Transient => "identity service is temporarily unavailable",
            Self::VerificationPending => "identity verification is pending",
            Self::SessionChanged => "identity session changed",
            Self::Rejected => "identity verification was rejected",
            Self::Protocol => "identity service response is invalid",
            Self::Storage => "identity storage could not be used",
            Self::Busy => "identity lifecycle is busy",
        })
    }
}

impl std::error::Error for IdentityError {}

impl From<JournalError> for IdentityError {
    fn from(_: JournalError) -> Self {
        Self::Storage
    }
}

impl From<IdentityStoreError> for IdentityError {
    fn from(_: IdentityStoreError) -> Self {
        Self::Storage
    }
}

impl From<VerifyError> for IdentityError {
    fn from(error: VerifyError) -> Self {
        match error {
            VerifyError::Transient => Self::Transient,
            VerifyError::Rejected => Self::Rejected,
            VerifyError::Protocol => Self::Protocol,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignOutOutcome {
    pub credentials_cleared: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityStatus {
    SignedOut,
    Recovering,
    Active { account_id: String, login_generation: u64, token_generation: u64, expires_at_unix: u64 },
    PendingVerification { account_id: String },
}

struct AttemptStart {
    attempt_id: u64,
    started: Duration,
}

struct Snapshot {
    record: StoredIdentity,
    attempt: Option<AttemptRecord>,
}

#[derive(Clone)]
pub struct IdentityLifecycle {
    domain: IdentityDomain,
    store: Arc<Mutex<Box<dyn IdentityStore>>>,
    transport: Arc<dyn IdentityTransport>,
    verifier: Arc<dyn SessionVerifier>,
    clock: Arc<dyn IdentityClock>,
    journal: Arc<IdentityJournal>,
    lock_dir: PathBuf,
    attempt_start: Arc<Mutex<Option<AttemptStart>>>,
    refresh: Arc<Mutex<()>>,
    revoked: Arc<AtomicBool>,
    last_login_generation: Arc<AtomicU64>,
}

impl IdentityLifecycle {
    /// The caller supplies a private, absolute journal root and a credential namespace for
    /// this domain. No credential or lock failure falls back to a memory-only identity.
    pub fn new(
        domain: IdentityDomain,
        credentials: Box<dyn CredentialStore + Send>,
        connection: WireConnection,
        verifier: Box<dyn SessionVerifier>,
        clock: Box<dyn IdentityClock>,
        journal_root: PathBuf,
    ) -> Result<Self, IdentityError> {
        let origin = WireConnection::product_origin(&domain.product_origin, connection.client_version()).map_err(|_| IdentityError::NotConfigured)?;
        let connection_origin =
            WireConnection::product_origin(connection.base_url(), connection.client_version()).map_err(|_| IdentityError::NotConfigured)?;
        if origin.base_url() != connection_origin.base_url() {
            return Err(IdentityError::NotConfigured);
        }
        Self::with_identity_store(
            domain,
            Box::new(CredentialIdentityStore::new(credentials)),
            Box::new(UreqIdentityTransport::new(UreqRefreshHttp::new())),
            verifier,
            clock,
            journal_root,
        )
    }

    pub(crate) fn with_identity_store(
        domain: IdentityDomain,
        store: Box<dyn IdentityStore>,
        transport: Box<dyn IdentityTransport>,
        verifier: Box<dyn SessionVerifier>,
        clock: Box<dyn IdentityClock>,
        journal_root: PathBuf,
    ) -> Result<Self, IdentityError> {
        let lock_dir = journal_root.join("identity");
        Ok(Self {
            domain,
            store: Arc::new(Mutex::new(store)),
            transport: Arc::from(transport),
            verifier: Arc::from(verifier),
            clock: Arc::from(clock),
            journal: Arc::new(IdentityJournal::new(journal_root)?),
            lock_dir,
            attempt_start: Arc::new(Mutex::new(None)),
            refresh: Arc::new(Mutex::new(())),
            revoked: Arc::new(AtomicBool::new(false)),
            last_login_generation: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Adopt credentials and a binding already verified by the explicit login flow.
    pub fn adopt_login(
        &self,
        provider_account_id: &str,
        access_token: Secret,
        refresh_token: Secret,
        issued_at_unix: u64,
        expires_at_unix: u64,
        session_binding: &str,
    ) -> Result<(), IdentityError> {
        if !safe_token(provider_account_id, MAX_ACCOUNT_BYTES)
            || !safe_token(session_binding, MAX_BINDING_BYTES)
            || !valid_text(access_token.expose_secret(), MAX_TOKEN_BYTES)
            || !valid_text(refresh_token.expose_secret(), MAX_TOKEN_BYTES)
            || expires_at_unix <= issued_at_unix
        {
            return Err(IdentityError::Protocol);
        }
        let _barrier = self.barrier_lock(&LockWait::new(MUTATION_WAIT))?;
        let mut store = self.store()?;
        let old = self.load(store.as_ref())?;
        let revoked = self.read_revocation()?;
        let high_water = self.last_login_generation.load(Ordering::Acquire).max(revoked);
        let login_generation = high_water.checked_add(1).ok_or(IdentityError::Storage)?;
        // Retire the old login before reusing its generation-1 key names. The tombstone
        // also prevents generation reuse by another process if delete or adoption is interrupted.
        if let Some(old) = old
            && old.login_generation > revoked
        {
            self.write_revocation(old.login_generation)?;
        }
        store.delete()?;
        let record = StoredIdentity {
            version: RECORD_VERSION,
            domain: self.domain.storage_key().to_owned(),
            login_generation,
            token_generation: 1,
            provider_account_id: provider_account_id.to_owned(),
            verified_account_id: None,
            access_token: access_token.expose_secret().to_owned(),
            refresh_token: refresh_token.expose_secret().to_owned(),
            issued_at_unix,
            expires_at_unix,
            session_binding: Some(session_binding.to_owned()),
            pending: true,
        };
        store.store_pending(&record)?;
        self.observe(login_generation);
        // Clear the old attempt before publishing a leaseable record.
        self.clear_attempt()?;
        store.activate(&record)?;
        self.revoked.store(false, Ordering::Release);
        Ok(())
    }

    pub fn ensure(&self, trigger: IdentityTrigger, wait: Duration) -> Result<IdentityLease, IdentityError> {
        self.check_local_barrier()?;
        let wait = LockWait::new(wait);
        {
            let _barrier = self.barrier_lock(&wait)?;
            let snapshot = self.snapshot(self.store()?.as_ref())?;
            if trigger != IdentityTrigger::Resume && self.ready(&snapshot) {
                return self.lease(snapshot.record);
            }
        }

        let _local = lock_refresh(&self.refresh, &wait)?;
        let _file = lock_file(&self.lock_dir.join("refresh.lock"), &wait)?;
        let snapshot = {
            let _barrier = self.barrier_lock(&wait)?;
            let snapshot = self.snapshot(self.store()?.as_ref())?;
            if self.ready(&snapshot) && trigger != IdentityTrigger::Resume {
                return self.lease(snapshot.record);
            }
            snapshot
        };
        if self.ready(&snapshot) {
            return self.verify_active(snapshot, &wait);
        }
        if snapshot.record.pending && self.unexpired(&snapshot.record) && snapshot.attempt.is_none() {
            return self.verify_pending(&snapshot, &wait);
        }
        self.refresh(snapshot, &wait)
    }

    pub fn sign_out(&self) -> Result<SignOutOutcome, IdentityError> {
        self.revoked.store(true, Ordering::Release);
        let wait = LockWait::new(MUTATION_WAIT);
        let generation = {
            let _barrier = self.barrier_lock(&wait)?;
            // An explicit login may have committed while this sign-out waited.
            self.revoked.store(true, Ordering::Release);
            let store = self.store()?;
            let observed = self.last_login_generation.load(Ordering::Acquire);
            let generation = match self.load(store.as_ref()) {
                Ok(record) => record.map_or(observed, |record| record.login_generation),
                Err(_) if observed != 0 => observed,
                Err(error) => return Err(error),
            };
            if generation == 0 {
                return Ok(SignOutOutcome { credentials_cleared: true });
            }
            if self.read_revocation()? < generation {
                self.write_revocation(generation)?;
            }
            generation
        };
        Ok(self.cleanup_revoked(generation, &wait))
    }

    pub fn status(&self) -> IdentityStatus {
        let status = (|| {
            self.check_local_barrier()?;
            let _barrier = self.barrier_lock(&LockWait::new(Duration::ZERO))?;
            let store = self.store()?;
            let record = self.current_record(store.as_ref())?;
            self.check_local_barrier()?;
            let account_id = record.verified_account_id.unwrap_or(record.provider_account_id);
            if record.pending {
                Ok(IdentityStatus::PendingVerification { account_id })
            } else {
                Ok(IdentityStatus::Active {
                    account_id,
                    login_generation: record.login_generation,
                    token_generation: record.token_generation,
                    expires_at_unix: record.expires_at_unix,
                })
            }
        })();
        match status {
            Ok(status) => status,
            Err(IdentityError::SignedOut) => IdentityStatus::SignedOut,
            Err(_) => IdentityStatus::Recovering,
        }
    }

    fn refresh(&self, old: Snapshot, wait: &LockWait) -> Result<IdentityLease, IdentityError> {
        let (attempt, issued_at_unix) = {
            let _barrier = self.barrier_lock(wait)?;
            let current = self.snapshot(self.store()?.as_ref())?;
            Self::check_unchanged(&old, &current)?;
            let now = self.clock.now_unix_seconds();
            let started = self.clock.monotonic_now();
            let mut local = self.attempt_start.lock().map_err(|_| IdentityError::Storage)?;
            let attempt_id = if let Some(previous) = current.attempt {
                if now < previous.cooldown_until_unix
                    || local
                        .as_ref()
                        .is_some_and(|start| start.attempt_id == previous.attempt_id && started.saturating_sub(start.started) < COOLDOWN_BASE)
                {
                    return Err(IdentityError::Transient);
                }
                previous.attempt_id.checked_add(1).ok_or(IdentityError::Storage)?
            } else {
                1
            };
            let attempt = AttemptRecord {
                version: RECORD_VERSION,
                domain: self.domain.storage_key().to_owned(),
                login_generation: old.record.login_generation,
                token_generation: old.record.token_generation,
                attempt_id,
                first_possible_send_unix: now,
                sends: 1,
                cooldown_until_unix: now.saturating_add(COOLDOWN_BASE.as_secs()),
                cooldown_step: 0,
                diagnostic_used: false,
            };
            self.check_local_barrier()?;
            self.journal.write_attempt(&attempt)?;
            *local = Some(AttemptStart { attempt_id, started });
            (attempt, now)
        };
        self.check_local_barrier()?;
        let response = self.transport.refresh(self.domain.client_id(), &Secret::new(old.record.refresh_token.clone()));
        let pending = {
            let _barrier = self.barrier_lock(wait)?;
            let mut store = self.store()?;
            let current = self.snapshot(store.as_ref())?;
            Self::check_unchanged(&Snapshot { record: old.record, attempt: Some(attempt.clone()) }, &current)?;
            self.check_local_barrier()?;
            match response {
                Err(RefreshTransportError::Terminal) => {
                    let generation = attempt.login_generation;
                    self.revoked.store(true, Ordering::Release);
                    self.write_revocation(generation)?;
                    drop(store);
                    drop(_barrier);
                    self.cleanup_revoked(generation, wait);
                    return Err(IdentityError::SignedOut);
                }
                Err(RefreshTransportError::Transient { retry_after }) => {
                    self.cool_down(attempt, retry_after)?;
                    return Err(IdentityError::Transient);
                }
                Err(RefreshTransportError::Protocol) => {
                    self.cool_down(attempt, None)?;
                    return Err(IdentityError::Protocol);
                }
                Ok(response) => match Self::rotated_record(current.record, response, issued_at_unix, self.clock.now_unix_seconds()) {
                    Ok(record) => {
                        store.store_pending(&record)?;
                        self.clear_attempt()?;
                        Snapshot { record, attempt: None }
                    }
                    Err(error) => {
                        // Identity mismatch is not a retryable response; don't write it.
                        if error == IdentityError::Protocol {
                            self.cool_down(attempt, None)?;
                        }
                        return Err(error);
                    }
                },
            }
        };
        self.verify_pending(&pending, wait)
    }

    fn rotated_record(mut old: StoredIdentity, response: RefreshResponse, issued_at_unix: u64, now: u64) -> Result<StoredIdentity, IdentityError> {
        if response.provider_account_id != old.provider_account_id {
            return Err(IdentityError::SessionChanged);
        }
        let access = response.access_token.expose_secret();
        let refresh = response.refresh_token.expose_secret();
        if !valid_text(access, MAX_TOKEN_BYTES) || !valid_text(refresh, MAX_TOKEN_BYTES) {
            return Err(IdentityError::Protocol);
        }
        let expires_at_unix = match response.expires_in {
            Some(lifetime) => issued_at_unix.checked_add(lifetime.as_secs()),
            None => access_token_expiry_unix(access),
        }
        .filter(|&expiry| expiry > now && expiry > issued_at_unix)
        .ok_or(IdentityError::Protocol)?;
        old.token_generation = old.token_generation.checked_add(1).ok_or(IdentityError::Storage)?;
        old.access_token = access.to_owned();
        old.refresh_token = refresh.to_owned();
        old.issued_at_unix = issued_at_unix;
        old.expires_at_unix = expires_at_unix;
        old.pending = true;
        Ok(old)
    }

    fn verify_active(&self, snapshot: Snapshot, wait: &LockWait) -> Result<IdentityLease, IdentityError> {
        let verification = self.verifier.verify(&Secret::new(snapshot.record.access_token.clone()));
        let _barrier = self.barrier_lock(wait)?;
        let current = self.snapshot(self.store()?.as_ref())?;
        Self::check_unchanged(&snapshot, &current)?;
        // This is the unchanged, previously bound bearer; a new binding is not required.
        verification?;
        self.lease(current.record)
    }

    fn verify_pending(&self, snapshot: &Snapshot, wait: &LockWait) -> Result<IdentityLease, IdentityError> {
        if !self.unexpired(&snapshot.record) {
            return Err(IdentityError::Transient);
        }
        self.check_local_barrier()?;
        let verified = self.verifier.verify(&Secret::new(snapshot.record.access_token.clone()));
        let _barrier = self.barrier_lock(wait)?;
        let mut store = self.store()?;
        let current = self.snapshot(store.as_ref())?;
        Self::check_unchanged(snapshot, &current)?;
        let verified = verified?;
        let binding = verified.same_login_binding.as_deref().ok_or(IdentityError::VerificationPending)?;
        if current.record.session_binding.as_deref() != Some(binding)
            || current.record.verified_account_id.as_ref().is_some_and(|account| account != &verified.account_id)
        {
            return Err(IdentityError::SessionChanged);
        }
        if !safe_token(&verified.account_id, MAX_ACCOUNT_BYTES) {
            return Err(IdentityError::Protocol);
        }
        // Preserve a short-lived rotated pair without publishing an active record
        // that would immediately send another refresh on the next ensure.
        if !self.fresh(&current.record) {
            return Err(IdentityError::Transient);
        }
        let mut active = current.record;
        active.verified_account_id = Some(verified.account_id);
        self.check_local_barrier()?;
        store.activate(&active)?;
        active.pending = false;
        self.lease(active)
    }

    fn ready(&self, snapshot: &Snapshot) -> bool {
        snapshot.attempt.is_none() && !snapshot.record.pending && self.fresh(&snapshot.record)
    }

    fn fresh(&self, record: &StoredIdentity) -> bool {
        let now = self.clock.now_unix_seconds();
        let ttl = record.expires_at_unix.saturating_sub(record.issued_at_unix);
        let lead = (ttl / REFRESH_LEAD_DIVISOR).min(REFRESH_LEAD_MAX.as_secs());
        let remaining = record.expires_at_unix.saturating_sub(now);
        now >= record.issued_at_unix && remaining > lead && remaining >= REQUEST_TIMEOUT.as_secs()
    }

    fn unexpired(&self, record: &StoredIdentity) -> bool {
        (record.issued_at_unix..record.expires_at_unix).contains(&self.clock.now_unix_seconds())
    }

    fn lease(&self, record: StoredIdentity) -> Result<IdentityLease, IdentityError> {
        self.check_local_barrier()?;
        if record.pending || !self.fresh(&record) {
            return Err(IdentityError::Transient);
        }
        Ok(IdentityLease {
            access_token: Secret::new(record.access_token),
            login_generation: record.login_generation,
            token_generation: record.token_generation,
            expires_at_unix: record.expires_at_unix,
        })
    }

    fn snapshot(&self, store: &dyn IdentityStore) -> Result<Snapshot, IdentityError> {
        let record = self.current_record(store)?;
        let attempt = self.journal.read_attempt()?;
        if attempt.as_ref().is_some_and(|attempt| attempt.version != RECORD_VERSION || attempt.domain != record.domain) {
            return Err(IdentityError::Storage);
        }
        let attempt =
            attempt.filter(|attempt| attempt.login_generation == record.login_generation && attempt.token_generation == record.token_generation);
        Ok(Snapshot { record, attempt })
    }

    fn check_unchanged(before: &Snapshot, after: &Snapshot) -> Result<(), IdentityError> {
        if before.record.login_generation != after.record.login_generation {
            return Err(IdentityError::SessionChanged);
        }
        if before.record != after.record
            || before.attempt.as_ref().map(|attempt| attempt.attempt_id) != after.attempt.as_ref().map(|attempt| attempt.attempt_id)
        {
            return Err(IdentityError::Transient);
        }
        Ok(())
    }

    fn cool_down(&self, mut attempt: AttemptRecord, retry_after: Option<Duration>) -> Result<(), IdentityError> {
        let delay = COOLDOWN_BASE.min(COOLDOWN_MAX).max(retry_after.unwrap_or_default());
        attempt.cooldown_until_unix = self.clock.now_unix_seconds().saturating_add(delay.as_secs());
        attempt.cooldown_step = 1;
        self.check_local_barrier()?;
        self.journal.write_attempt(&attempt)?;
        Ok(())
    }

    fn clear_attempt(&self) -> Result<(), IdentityError> {
        self.journal.clear_attempt()?;
        *self.attempt_start.lock().map_err(|_| IdentityError::Storage)? = None;
        Ok(())
    }

    fn cleanup_revoked(&self, generation: u64, wait: &LockWait) -> SignOutOutcome {
        // Revocation is already visible. Re-enter only for conditional cleanup; a
        // concurrent explicit login must never have its new credentials deleted.
        let cleanup = (|| {
            let _barrier = self.barrier_lock(wait)?;
            let mut store = self.store()?;
            if self.read_revocation()? < generation {
                return Err(IdentityError::Storage);
            }
            if self.load(store.as_ref())?.is_some_and(|record| record.login_generation != generation) {
                return Ok(true);
            }
            let deleted = store.delete().is_ok();
            let _ = self.clear_attempt();
            Ok(deleted)
        })();
        SignOutOutcome { credentials_cleared: cleanup.unwrap_or(false) }
    }

    fn current_record(&self, store: &dyn IdentityStore) -> Result<StoredIdentity, IdentityError> {
        self.check_local_barrier()?;
        let record = self.load(store)?;
        let revoked = self.read_revocation()?;
        let record = record.ok_or(IdentityError::SignedOut)?;
        if record.login_generation <= revoked {
            return Err(IdentityError::SignedOut);
        }
        Ok(record)
    }

    fn load(&self, store: &dyn IdentityStore) -> Result<Option<StoredIdentity>, IdentityError> {
        let record = store.load()?;
        if let Some(record) = &record {
            if record.domain != self.domain.storage_key() || record.version != RECORD_VERSION || record.login_generation == 0 {
                return Err(IdentityError::Storage);
            }
            self.observe(record.login_generation);
        }
        Ok(record)
    }

    fn observe(&self, generation: u64) {
        self.last_login_generation.fetch_max(generation, Ordering::AcqRel);
    }

    fn store(&self) -> Result<MutexGuard<'_, Box<dyn IdentityStore>>, IdentityError> {
        self.store.lock().map_err(|_| IdentityError::Storage)
    }

    fn check_local_barrier(&self) -> Result<(), IdentityError> {
        if self.revoked.load(Ordering::Acquire) { Err(IdentityError::SignedOut) } else { Ok(()) }
    }

    fn read_revocation(&self) -> Result<u64, IdentityError> {
        let Some(record) = self.journal.read_revocation()? else {
            return Ok(0);
        };
        if record.version != RECORD_VERSION || record.domain != self.domain.storage_key() || record.login_generation == 0 {
            return Err(IdentityError::Storage);
        }
        Ok(record.login_generation)
    }

    fn write_revocation(&self, login_generation: u64) -> Result<(), IdentityError> {
        self.journal.write_revocation(&RevocationRecord {
            version: RECORD_VERSION,
            domain: self.domain.storage_key().to_owned(),
            login_generation,
            revoked_at_unix: self.clock.now_unix_seconds(),
        })?;
        Ok(())
    }

    fn barrier_lock(&self, wait: &LockWait) -> Result<File, IdentityError> {
        lock_file(&self.lock_dir.join("barrier.lock"), wait)
    }
}

/// All lock polls in one call share a wait budget. Use elapsed time (not a wall-clock deadline)
/// and try once even with zero budget. The injected clock governs identity policy, not sleeps.
struct LockWait {
    started: Instant,
    budget: Duration,
}

impl LockWait {
    fn new(budget: Duration) -> Self {
        Self { started: Instant::now(), budget }
    }

    fn poll(&self) -> Result<(), IdentityError> {
        let remaining = self.budget.saturating_sub(self.started.elapsed());
        if remaining.is_zero() {
            return Err(IdentityError::Busy);
        }
        std::thread::sleep(remaining.min(LOCK_POLL));
        if self.started.elapsed() >= self.budget { Err(IdentityError::Busy) } else { Ok(()) }
    }
}

fn lock_refresh<'a>(mutex: &'a Mutex<()>, wait: &LockWait) -> Result<MutexGuard<'a, ()>, IdentityError> {
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(MutexTryLockError::WouldBlock) => wait.poll()?,
            Err(MutexTryLockError::Poisoned(_)) => return Err(IdentityError::Busy),
        }
    }
}

fn lock_file(path: &Path, wait: &LockWait) -> Result<File, IdentityError> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(|_| IdentityError::Busy)?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => wait.poll()?,
            Err(TryLockError::Error(_)) => return Err(IdentityError::Busy),
        }
    }
}
