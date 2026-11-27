//! Native identity rotation. Refresh locks span the provider exchange; the barrier lock
//! serializes credential changes and logout, but never spans online verification.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError as MutexTryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use solidcraft_engine::entitlement::CredentialStore;

use crate::credentials::{CredentialLogoutFence, CredentialSession, MAX_ACCOUNT_BYTES, StagedLogin, safe_token, unavailable};
use crate::http::{Secret, WireConnection};
use crate::identity_journal::{AttemptRecord, CurrentLoginRecord, IdentityJournal, JournalError, RevocationRecord, StatusAttempt};
use crate::identity_store::{CredentialIdentityStore, IdentityStore, IdentityStoreError, StoredIdentity};
use crate::identity_transport::{IdentityTransport, RefreshResponse, RefreshTransportError, UreqIdentityTransport, UreqRefreshHttp};
use crate::token_expiry::access_token_expiry_unix;

const REFRESH_LEAD_MAX: Duration = Duration::from_secs(60);
const REFRESH_LEAD_DIVISOR: u64 = 3;
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const BATCH_BUDGET: Duration = Duration::from_secs(20);
const MAX_SENDS: u8 = 3;
const REPLAY_WINDOW: Duration = Duration::from_secs(30);
const COOLDOWN_BASE: Duration = Duration::from_secs(5);
const MAX_COOLDOWN_STEP: u8 = 3;
// Reserved values in the durable attempt format; neither permits automatic replay.
const PROTOCOL_BLOCKED: u8 = u8::MAX;
// The protocol result came from a reservation that had already used the final diagnostic.
const PROTOCOL_EXHAUSTED: u8 = u8::MAX - 1;
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
            || client_id.chars().any(char::is_whitespace)
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

    #[cfg(test)]
    fn before_refresh_lock(&self, _: IdentityTrigger) {}

    #[cfg(test)]
    fn on_local_refresh_contention(&self) {}

    #[cfg(test)]
    fn after_attempt_write(&self) {}

    #[cfg(test)]
    fn after_refresh_lock(&self) {}
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
    /// Implementations must bound the complete request to five seconds.
    fn verify(&self, access: &Secret) -> Result<VerifiedSession, VerifyError>;
}

pub struct VerifiedSession {
    pub account_id: String,
    pub same_login_binding: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    Transient,
    RetryAfter(Duration),
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
    session_binding: Option<String>,
    verified_account_id: Option<String>,
    login_generation: u64,
    token_generation: u64,
    expires_at_unix: u64,
}

impl IdentityLease {
    pub fn access_token(&self) -> &Secret {
        &self.access_token
    }

    pub fn matches_binding(&self, account: &str, binding: &str) -> bool {
        self.verified_account_id.as_deref() == Some(account) && self.session_binding.as_deref() == Some(binding)
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
            VerifyError::Transient | VerifyError::RetryAfter(_) => Self::Transient,
            VerifyError::Rejected => Self::Rejected,
            VerifyError::Protocol => Self::Protocol,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignOutOutcome {
    pub credentials_cleared: bool,
}

/// Proof of the exact durable logout barrier published for one login.
pub struct IdentityLogoutFence {
    domain: String,
    current: Option<CurrentLoginRecord>,
    generation: u64,
    binding: Option<String>,
    id: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityStatus {
    SignedOut,
    Recovering,
    Active { account_id: String, login_generation: u64, token_generation: u64, expires_at_unix: u64 },
    PendingVerification { account_id: String },
}

#[derive(Clone, Copy)]
struct AttemptStart {
    attempt_id: u64,
    login_generation: u64,
    token_generation: u64,
    started: Duration,
}

impl AttemptStart {
    fn matches(&self, attempt: &AttemptRecord) -> bool {
        self.attempt_id == attempt.attempt_id
            && self.login_generation == attempt.login_generation
            && self.token_generation == attempt.token_generation
    }

    fn fits_request(&self, now: Duration) -> bool {
        now.checked_sub(self.started).is_some_and(|elapsed| elapsed.saturating_add(REQUEST_TIMEOUT) <= BATCH_BUDGET && fits_replay_window(elapsed))
    }
}

pub(super) fn fits_replay_window(elapsed: Duration) -> bool {
    elapsed.saturating_add(REQUEST_TIMEOUT) <= REPLAY_WINDOW
}

struct Snapshot {
    record: StoredIdentity,
    attempt: Option<AttemptRecord>,
}

#[derive(Default)]
struct Readiness {
    epoch: u64,
    // Both generations matter: an explicit login starts token generations over at 1.
    generation: Option<(u64, u64)>,
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
    readiness: Arc<Mutex<Readiness>>,
    last_now_unix: Arc<AtomicU64>,
    refresh_preserves_binding: bool,
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
        Self::new_with_transport(
            domain,
            credentials,
            connection,
            Box::new(UreqIdentityTransport::new(UreqRefreshHttp::new())),
            verifier,
            clock,
            journal_root,
        )
    }

    /// Compose the same durable lifecycle with an injected refresh transport.
    pub fn new_with_transport(
        domain: IdentityDomain,
        credentials: Box<dyn CredentialStore + Send>,
        connection: WireConnection,
        transport: Box<dyn IdentityTransport>,
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
        let store = CredentialIdentityStore::new(credentials, domain.storage_key());
        Self::with_identity_store(domain, Box::new(store), transport, verifier, clock, journal_root)
    }

    pub(crate) fn with_identity_store(
        domain: IdentityDomain,
        store: Box<dyn IdentityStore>,
        transport: Box<dyn IdentityTransport>,
        verifier: Box<dyn SessionVerifier>,
        clock: Box<dyn IdentityClock>,
        journal_root: PathBuf,
    ) -> Result<Self, IdentityError> {
        let lock_dir = journal_root.join("identity").join(domain.storage_key());
        let journal = Arc::new(IdentityJournal::new(journal_root, domain.storage_key())?);
        Ok(Self {
            domain,
            store: Arc::new(Mutex::new(store)),
            transport: Arc::from(transport),
            verifier: Arc::from(verifier),
            clock: Arc::from(clock),
            journal,
            lock_dir,
            attempt_start: Arc::new(Mutex::new(None)),
            refresh: Arc::new(Mutex::new(())),
            revoked: Arc::new(AtomicBool::new(false)),
            last_login_generation: Arc::new(AtomicU64::new(0)),
            readiness: Arc::new(Mutex::new(Readiness::default())),
            last_now_unix: Arc::new(AtomicU64::new(0)),
            refresh_preserves_binding: false,
        })
    }

    /// Opt into the WorkOS refresh guarantee for this domain before using or cloning
    /// the lifecycle. This preserves only local correlation; product requests must
    /// still be verified by the server against its own session identity.
    pub fn set_refresh_preserves_binding(&mut self, enabled: bool) {
        self.refresh_preserves_binding = enabled;
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
        self.adopt_login_inner(provider_account_id, None, access_token, refresh_token, issued_at_unix, expires_at_unix, session_binding, || true)
    }

    /// Adopt an explicit login with the product account that the server already verified.
    /// The refresh-preserves-binding policy checks future status responses against this account.
    #[allow(clippy::too_many_arguments)]
    pub fn adopt_verified_login(
        &self,
        provider_account_id: &str,
        verified_account_id: &str,
        access_token: Secret,
        refresh_token: Secret,
        issued_at_unix: u64,
        expires_at_unix: u64,
        session_binding: &str,
    ) -> Result<(), IdentityError> {
        self.adopt_verified_login_if_current(
            provider_account_id,
            verified_account_id,
            access_token,
            refresh_token,
            issued_at_unix,
            expires_at_unix,
            session_binding,
            || true,
        )
    }

    /// Check the host's staged login under the identity mutation barrier, before replacing any pair.
    #[allow(clippy::too_many_arguments)]
    pub fn adopt_verified_login_if_current(
        &self,
        provider_account_id: &str,
        verified_account_id: &str,
        access_token: Secret,
        refresh_token: Secret,
        issued_at_unix: u64,
        expires_at_unix: u64,
        session_binding: &str,
        current: impl FnOnce() -> bool,
    ) -> Result<(), IdentityError> {
        self.adopt_login_inner(
            provider_account_id,
            Some(verified_account_id),
            access_token,
            refresh_token,
            issued_at_unix,
            expires_at_unix,
            session_binding,
            current,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn adopt_login_inner(
        &self,
        provider_account_id: &str,
        verified_account_id: Option<&str>,
        access_token: Secret,
        refresh_token: Secret,
        issued_at_unix: u64,
        expires_at_unix: u64,
        session_binding: &str,
        staged_is_current: impl FnOnce() -> bool,
    ) -> Result<(), IdentityError> {
        if !safe_token(provider_account_id, MAX_ACCOUNT_BYTES)
            || verified_account_id.is_some_and(|account| !safe_token(account, MAX_ACCOUNT_BYTES))
            || !safe_token(session_binding, MAX_BINDING_BYTES)
            || !valid_text(access_token.expose_secret(), MAX_TOKEN_BYTES)
            || !valid_text(refresh_token.expose_secret(), MAX_TOKEN_BYTES)
            || expires_at_unix <= issued_at_unix
        {
            return Err(IdentityError::Protocol);
        }
        let _barrier = self.barrier_lock(&LockWait::new(MUTATION_WAIT))?;
        let epoch = self.readiness_epoch()?;
        let mut store = self.store()?;
        let old = self.load(store.as_ref())?;
        let revoked = match self.read_revocation() {
            Ok(record) => record.unwrap_or(0),
            Err(IdentityError::Storage) if matches!(self.journal.read_revocation(), Err(JournalError::Corrupt)) => 0,
            Err(error) => return Err(error),
        };
        // Explicit login can replace a torn publication. The store and revocation
        // still bound the next generation, and peers still reject a corrupt journal.
        let current = match self.journal.read_current_login() {
            Ok(record) => self.current_login_generation(record)?,
            Err(JournalError::Corrupt) => 0,
            Err(error) => return Err(error.into()),
        };
        let retiring = current.max(old.as_ref().map_or(0, |record| record.login_generation));
        let high_water = self.last_login_generation.load(Ordering::Acquire).max(revoked).max(retiring);
        let login_generation = high_water.checked_add(1).ok_or(IdentityError::Storage)?;
        if !staged_is_current() {
            return Err(IdentityError::SessionChanged);
        }
        // Retire the old login before reusing its generation-1 key names. The tombstone
        // also prevents generation reuse by another process if delete or adoption is interrupted.
        self.write_revocation(retiring.max(revoked))?;
        store.delete()?;
        let record = StoredIdentity {
            version: RECORD_VERSION,
            domain: self.domain.storage_key().to_owned(),
            login_generation,
            token_generation: 1,
            provider_account_id: provider_account_id.to_owned(),
            verified_account_id: verified_account_id.map(str::to_owned),
            access_token: access_token.expose_secret().to_owned(),
            refresh_token: refresh_token.expose_secret().to_owned(),
            issued_at_unix,
            expires_at_unix,
            session_binding: Some(session_binding.to_owned()),
            pending: true,
        };
        self.observe(login_generation);
        store.store_pending(&record)?;
        // Clear the old attempt before publishing a leaseable record.
        self.clear_attempt()?;
        store.activate(&record)?;
        self.journal.write_current_login(&CurrentLoginRecord {
            version: RECORD_VERSION,
            domain: record.domain,
            login_generation,
            session_binding: record.session_binding,
        })?;
        // Adoption can finish after Resume, but must not restore pre-Resume readiness.
        let _ = self.set_verified(epoch, (login_generation, 1));
        self.revoked.store(false, Ordering::Release);
        Ok(())
    }

    pub fn ensure(&self, trigger: IdentityTrigger, wait: Duration) -> Result<IdentityLease, IdentityError> {
        self.ensure_with_recovery(trigger, wait, || {})
    }

    /// Notify a host when there is no cached lease and recovery may do network work.
    /// The callback must not call back into this lifecycle.
    pub fn ensure_with_recovery(&self, trigger: IdentityTrigger, wait: Duration, recovering: impl FnOnce()) -> Result<IdentityLease, IdentityError> {
        self.check_local_barrier()?;
        if trigger == IdentityTrigger::Resume {
            self.invalidate_readiness()?;
        }
        let wait = LockWait::new(wait);
        {
            let _barrier = self.barrier_lock(&wait)?;
            if trigger == IdentityTrigger::Resume {
                // Sleep may not be included in the monotonic clock on this platform.
                *self.attempt_start.lock().map_err(|_| IdentityError::Storage)? = None;
            }
            let snapshot = self.snapshot(self.store()?.as_ref())?;
            if trigger != IdentityTrigger::Resume && self.ready(&snapshot) {
                return self.lease(snapshot.record);
            }
        }

        recovering();
        #[cfg(test)]
        self.clock.before_refresh_lock(trigger);
        let _local = self.lock_refresh(&wait)?;
        let _file = lock_file(&self.lock_dir.join("refresh.lock"), &wait)?;
        #[cfg(test)]
        self.clock.after_refresh_lock();
        let snapshot = {
            let _barrier = self.barrier_lock(&wait)?;
            let snapshot = self.snapshot(self.store()?.as_ref())?;
            if trigger != IdentityTrigger::Resume && self.ready(&snapshot) {
                return self.lease(snapshot.record);
            }
            snapshot
        };
        if snapshot.attempt.is_none()
            && !snapshot.record.pending
            && self.unexpired(&snapshot.record)
            && (trigger == IdentityTrigger::Resume || self.fresh(&snapshot.record) || !self.clock_is_safe(&snapshot.record))
            && let Some(lease) = self.verify_active(&snapshot, &wait)?
        {
            return Ok(lease);
        }
        if snapshot.record.pending && self.unexpired(&snapshot.record) && snapshot.attempt.is_none() {
            return self.verify_pending(&snapshot, &wait);
        }
        self.refresh(snapshot, trigger, &wait)
    }

    /// An explicit status read bypasses cached readiness but obeys the background retry budget.
    pub fn recheck_status(&self, wait: Duration) -> Result<IdentityLease, IdentityError> {
        self.invalidate_readiness()?;
        self.ensure(IdentityTrigger::UserRetry, wait)
    }

    /// A non-network readiness check for renewal of an already admitted operation.
    pub fn verified_generation_for(&self, account: &str, binding: &str) -> Option<(u64, u64)> {
        let _barrier = self.barrier_lock(&LockWait::new(Duration::ZERO)).ok()?;
        let snapshot = self.snapshot(self.store().ok()?.as_ref()).ok()?;
        (snapshot.record.verified_account_id.as_deref() == Some(account)
            && snapshot.record.session_binding.as_deref() == Some(binding)
            && self.ready(&snapshot))
        .then(|| Self::generations(&snapshot.record))
    }

    pub fn sign_out(&self) -> Result<SignOutOutcome, IdentityError> {
        let wait = LockWait::new(MUTATION_WAIT);
        let generation = self.publish_logout(&wait)?;
        Ok(self.cleanup_revoked(generation, &wait))
    }

    /// Durably retire the identity without deleting its keyring record.
    pub fn mark_logout_pending(&self) -> Result<IdentityLogoutFence, IdentityError> {
        self.revoked.store(true, Ordering::Release);
        self.clear_readiness();
        let _barrier = self.barrier_lock(&LockWait::new(MUTATION_WAIT))?;
        self.mark_logout_pending_locked()
    }

    /// Publish both barriers while excluding cooperating login writers. Both
    /// writes are attempted even when one fails. The caller receives no cleanup
    /// handle unless both writes complete.
    pub fn mark_logout_pending_with_credentials(
        &self,
        credentials: &CredentialSession,
    ) -> Result<(IdentityLogoutFence, CredentialLogoutFence), IdentityError> {
        self.revoked.store(true, Ordering::Release);
        self.clear_readiness();
        let wait = LockWait::new(MUTATION_WAIT);
        let _barrier = match self.barrier_lock(&wait) {
            Ok(barrier) => barrier,
            Err(error) => {
                let _ = credentials.mark_logout_pending();
                return Err(error);
            }
        };
        let (identity, product) = credentials.fence_logout_with(|| self.mark_logout_pending_locked()).map_err(|_| IdentityError::Storage)?;
        Ok((identity?, product.map_err(|_| IdentityError::Storage)?))
    }

    fn mark_logout_pending_locked(&self) -> Result<IdentityLogoutFence, IdentityError> {
        self.revoked.store(true, Ordering::Release);
        self.clear_readiness();
        let current = self.journal.read_current_login()?;
        let current_generation = self.current_login_generation(current.clone())?;
        let mut generation = current_generation.max(self.last_login_generation.load(Ordering::Acquire));
        let mut binding = current.as_ref().and_then(|current| current.session_binding.clone());
        // The journal fences a known login even when the keyring is locked. A readable
        // orphan from an interrupted adoption must also be included in the fence.
        if let Ok(Some(record)) = self.store().and_then(|store| self.load(store.as_ref()))
            && record.login_generation >= generation
        {
            generation = record.login_generation;
            binding = record.session_binding;
        }
        generation = generation.max(self.read_revocation()?.unwrap_or(0));
        let mut id = [0; 16];
        rustls::crypto::ring::default_provider().secure_random.fill(&mut id).map_err(|_| IdentityError::Storage)?;
        self.write_revocation_with_fence(generation, Some(id))?;
        self.write_revocation_with_fence(generation, Some(id))?;
        Ok(IdentityLogoutFence { domain: self.domain.storage_key().to_owned(), current, generation, binding, id })
    }

    pub fn sign_out_fenced(&self, fence: IdentityLogoutFence) -> Result<SignOutOutcome, IdentityError> {
        let _barrier = self.barrier_lock(&LockWait::new(MUTATION_WAIT))?;
        let mut store = self.store()?;
        self.check_logout_fence(&fence, store.as_ref())?;
        Ok(SignOutOutcome { credentials_cleared: self.delete_fenced_identity(store.as_mut()) })
    }

    /// Delete the product credentials first, keeping their logout marker until
    /// identity deletion succeeds. Both ownership checks are held across deletion.
    pub fn sign_out_fenced_with_credentials(
        &self,
        fence: IdentityLogoutFence,
        credentials: &CredentialSession,
        product: CredentialLogoutFence,
    ) -> Result<(), IdentityError> {
        let _barrier = self.barrier_lock(&LockWait::new(MUTATION_WAIT))?;
        let mut store = self.store()?;
        self.check_logout_fence(&fence, store.as_ref())?;
        credentials
            .sign_out_fenced_with(product, || self.delete_fenced_identity(store.as_mut()).then_some(()).ok_or_else(unavailable))
            .map_err(|_| IdentityError::Storage)
    }

    fn check_logout_fence(&self, fence: &IdentityLogoutFence, store: &dyn IdentityStore) -> Result<(), IdentityError> {
        if fence.domain != self.domain.storage_key()
            || self.journal.read_current_login()? != fence.current
            || self.journal.read_revocation()?.is_none_or(|record| {
                record.domain != fence.domain || record.login_generation != fence.generation || record.fence_id != Some(fence.id)
            })
            || self.load(store)?.is_some_and(|record| record.login_generation > fence.generation || record.session_binding != fence.binding)
        {
            return Err(IdentityError::SessionChanged);
        }
        Ok(())
    }

    fn delete_fenced_identity(&self, store: &mut dyn IdentityStore) -> bool {
        let deleted = store.delete().is_ok();
        let _ = self.clear_attempt();
        deleted
    }

    fn publish_logout(&self, wait: &LockWait) -> Result<u64, IdentityError> {
        self.revoked.store(true, Ordering::Release);
        self.clear_readiness();
        let _barrier = self.barrier_lock(wait)?;
        self.publish_logout_locked()
    }

    fn publish_logout_locked(&self) -> Result<u64, IdentityError> {
        // An explicit login may have committed while this sign-out waited.
        self.revoked.store(true, Ordering::Release);
        self.clear_readiness();
        // Do not wait for keyring reads when the journal already proves the generation.
        let mut generation = self.read_current_login()?.max(self.last_login_generation.load(Ordering::Acquire));
        if generation == 0 {
            generation = self.load(self.store()?.as_ref())?.map_or(0, |record| record.login_generation);
        }
        self.publish_logout_barrier(generation)?;
        Ok(generation)
    }

    /// Abort the product stage before retiring its identity. Hold the identity barrier first
    /// (the same order as publishing a lease) and keep the product write fence through cleanup.
    pub fn abort_staged_login(&self, credentials: &CredentialSession, staged: StagedLogin) -> Result<(), IdentityError> {
        let _barrier = self.barrier_lock(&LockWait::new(MUTATION_WAIT))?;
        credentials
            .abort_staged_login_with(staged, || {
                let cleanup = self.publish_logout_locked().and_then(|generation| self.cleanup_revoked_locked(generation));
                if !cleanup.unwrap_or(false) {
                    return Err(unavailable());
                }
                Ok(())
            })
            .map_err(|_| IdentityError::Storage)
    }

    /// Run a short local commit decision if its previously authorized login still
    /// owns the barrier. The callback must not do network I/O or call back into
    /// this lifecycle. This checks revocation, not operation authorization.
    pub fn with_barrier_lock<T>(&self, login_generation: u64, wait: Duration, commit: impl FnOnce() -> T) -> Result<T, IdentityError> {
        let _barrier = self.barrier_lock(&LockWait::new(wait))?;
        self.check_local_barrier()?;
        let current = self.read_current_login()?;
        if current == 0 || self.read_revocation()?.is_none_or(|revoked| current <= revoked) {
            return Err(IdentityError::SignedOut);
        }
        if current != login_generation {
            return Err(IdentityError::SessionChanged);
        }
        Ok(commit())
    }

    /// Publish a verified lease into a host credential store only while both of its
    /// generations are current. The callback must not call back into the lifecycle.
    pub fn with_current_lease<T>(&self, lease: &IdentityLease, wait: Duration, publish: impl FnOnce() -> T) -> Result<T, IdentityError> {
        let _barrier = self.barrier_lock(&LockWait::new(wait))?;
        let snapshot = self.snapshot(self.store()?.as_ref())?;
        if Self::generations(&snapshot.record) != (lease.login_generation, lease.token_generation) {
            return Err(IdentityError::SessionChanged);
        }
        if !self.ready(&snapshot) {
            return Err(IdentityError::Transient);
        }
        Ok(publish())
    }

    pub fn status(&self) -> IdentityStatus {
        let status = (|| {
            self.check_local_barrier()?;
            let _barrier = self.barrier_lock(&LockWait::new(Duration::ZERO))?;
            let store = self.store()?;
            let record = self.current_record(store.as_ref())?;
            self.check_local_barrier()?;
            let clock_safe = self.clock_is_safe(&record);
            let account_id = record.verified_account_id.unwrap_or(record.provider_account_id);
            if record.pending {
                Ok(IdentityStatus::PendingVerification { account_id })
            } else if !clock_safe || !self.has_verified_generation((record.login_generation, record.token_generation)) {
                Ok(IdentityStatus::Recovering)
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

    fn refresh(&self, old: Snapshot, trigger: IdentityTrigger, wait: &LockWait) -> Result<IdentityLease, IdentityError> {
        wait.require_request()?;
        let ((attempt, batch_start), issued_at_unix) = {
            let _barrier = self.barrier_lock(wait)?;
            let current = self.snapshot(self.store()?.as_ref())?;
            Self::check_unchanged(&old, &current)?;
            let now = self.clock.now_unix_seconds();
            (self.mark_send(current, trigger, now)?, now)
        };
        self.check_local_barrier()?;
        let refresh_token = Secret::new(old.record.refresh_token.clone());
        wait.require_request()?;
        if !batch_start.fits_request(self.clock.monotonic_now()) {
            // The durable reservation is consumed even when the journal sync used up the window.
            return Err(IdentityError::Transient);
        }
        let response = self
            .transport
            .refresh(self.domain.client_id(), &refresh_token)
            .map(|response| Self::rotated_record(old.record.clone(), response, issued_at_unix));
        self.check_local_barrier()?;
        // Cleanup can hold the barrier lock after the peer publishes revocation.
        // Check before waiting; the snapshot under the lock still gates all writes.
        let revoked = self.read_revocation()?.ok_or(IdentityError::SignedOut)?;
        if revoked >= attempt.login_generation {
            return Err(if self.read_current_login()? > revoked { IdentityError::SessionChanged } else { IdentityError::SignedOut });
        }
        let attempt =
            if matches!(response, Err(RefreshTransportError::Protocol) | Ok(Err(_))) { self.mark_protocol_blocked(attempt)? } else { attempt };
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
                    self.clear_readiness();
                    self.publish_logout_barrier(generation)?;
                    drop(store);
                    drop(_barrier);
                    self.cleanup_revoked(generation, wait);
                    return Err(IdentityError::SignedOut);
                }
                Err(RefreshTransportError::Transient { retry_after }) => {
                    self.cool_down(attempt, retry_after, batch_start)?;
                    return Err(IdentityError::Transient);
                }
                Err(RefreshTransportError::Protocol) => return Err(IdentityError::Protocol),
                Ok(Err(error)) => return Err(error),
                Ok(Ok(record)) => {
                    store.store_pending(&record)?;
                    self.clear_attempt()?;
                    Snapshot { record, attempt: None }
                }
            }
        };
        self.verify_pending(&pending, wait)
    }

    // Runs under the barrier lock while the caller also holds the refresh locks.
    fn mark_send(&self, current: Snapshot, trigger: IdentityTrigger, now: u64) -> Result<(AttemptRecord, AttemptStart), IdentityError> {
        let monotonic = self.clock.monotonic_now();
        let mut local = self.attempt_start.lock().map_err(|_| IdentityError::Storage)?;
        let mut replay_start = None;
        let fresh_batch = current.attempt.is_none();
        let mut attempt = if let Some(mut attempt) = current.attempt {
            if now < attempt.cooldown_until_unix {
                return Err(IdentityError::Transient);
            }
            if trigger == IdentityTrigger::UserRetry {
                if attempt.cooldown_step != PROTOCOL_BLOCKED && attempt.diagnostic_used && attempt.sends >= MAX_SENDS {
                    return Err(IdentityError::Transient);
                }
                // Reserve the last slot for this one explicit send; never open a replay window.
                attempt.sends = MAX_SENDS - 1;
                attempt.diagnostic_used = true;
                attempt.first_possible_send_unix = now;
                if attempt.cooldown_step == PROTOCOL_BLOCKED {
                    attempt.cooldown_step = 1;
                }
            } else {
                if matches!(attempt.cooldown_step, PROTOCOL_BLOCKED | PROTOCOL_EXHAUSTED) {
                    return Err(IdentityError::Protocol);
                }
                if attempt.diagnostic_used || attempt.sends >= MAX_SENDS {
                    return Err(IdentityError::Transient);
                }
                // Resume cannot trust a start even if a background call installed it
                // between the early invalidation and acquiring the refresh locks.
                if trigger == IdentityTrigger::Background {
                    if local
                        .filter(|start| start.matches(&attempt))
                        .and_then(|start| monotonic.checked_sub(start.started))
                        .is_some_and(|elapsed| elapsed >= BATCH_BUDGET && fits_replay_window(elapsed))
                    {
                        return Err(IdentityError::Transient);
                    }
                    replay_start = local.filter(|start| {
                        start.matches(&attempt) && monotonic.checked_sub(start.started).is_some_and(|elapsed| elapsed < BATCH_BUDGET)
                    });
                }
                attempt.diagnostic_used = replay_start.is_none();
            }
            attempt
        } else {
            AttemptRecord {
                version: RECORD_VERSION,
                domain: current.record.domain,
                login_generation: current.record.login_generation,
                token_generation: current.record.token_generation,
                attempt_id: 1,
                first_possible_send_unix: now,
                sends: 0,
                cooldown_until_unix: 0,
                cooldown_step: 0,
                diagnostic_used: false,
            }
        };
        attempt.sends += 1;
        // If the process dies mid-exchange, leave a cooldown even without a response.
        attempt.cooldown_until_unix = now.saturating_add(COOLDOWN_BASE.as_secs());
        self.check_local_barrier()?;
        self.journal.write_attempt(&attempt)?;
        #[cfg(test)]
        self.clock.after_attempt_write();
        let start = replay_start.unwrap_or(AttemptStart {
            attempt_id: attempt.attempt_id,
            login_generation: attempt.login_generation,
            token_generation: attempt.token_generation,
            started: monotonic,
        });
        if fresh_batch {
            *local = Some(start);
        } else if attempt.diagnostic_used {
            *local = None;
        }
        Ok((attempt, start))
    }

    fn rotated_record(mut old: StoredIdentity, response: RefreshResponse, issued_at_unix: u64) -> Result<StoredIdentity, IdentityError> {
        if response.provider_account_id != old.provider_account_id {
            return Err(IdentityError::SessionChanged);
        }
        let access = response.access_token.expose_secret();
        let refresh = response.refresh_token.expose_secret();
        if !valid_text(access, MAX_TOKEN_BYTES) || !valid_text(refresh, MAX_TOKEN_BYTES) {
            return Err(IdentityError::Protocol);
        }
        // Do not discard a replacement refresh token just because the valid access
        // lifetime elapsed in transit. Pending verification checks freshness separately.
        let expires_at_unix = match response.expires_in {
            Some(lifetime) => issued_at_unix.checked_add(lifetime.as_secs()),
            None => access_token_expiry_unix(access),
        }
        .filter(|&expiry| expiry > issued_at_unix)
        .ok_or(IdentityError::Protocol)?;
        old.token_generation = old.token_generation.checked_add(1).ok_or(IdentityError::Storage)?;
        old.access_token = access.to_owned();
        old.refresh_token = refresh.to_owned();
        old.issued_at_unix = issued_at_unix;
        old.expires_at_unix = expires_at_unix;
        old.pending = true;
        Ok(old)
    }

    fn verify_active(&self, snapshot: &Snapshot, wait: &LockWait) -> Result<Option<IdentityLease>, IdentityError> {
        let epoch = self.invalidate_readiness()?;
        let verification = self.verify_status(snapshot, wait)?;
        let _barrier = self.barrier_lock(wait)?;
        let current = self.snapshot(self.store()?.as_ref())?;
        Self::check_unchanged(snapshot, &current)?;
        self.check_readiness_epoch(epoch)?;
        // This is the unchanged, previously bound bearer; a new binding is not required.
        let verified = verification?;
        if current.record.verified_account_id.as_ref().is_some_and(|account| account != &verified.account_id)
            || verified.same_login_binding.as_ref().is_some_and(|binding| current.record.session_binding.as_ref() != Some(binding))
        {
            return Err(IdentityError::SessionChanged);
        }
        if !self.clock_is_safe(&current.record) {
            return Err(IdentityError::Transient);
        }
        self.set_verified(epoch, Self::generations(&current.record))?;
        if self.fresh(&current.record) { self.lease(current.record).map(Some) } else { Ok(None) }
    }

    fn verify_pending(&self, snapshot: &Snapshot, wait: &LockWait) -> Result<IdentityLease, IdentityError> {
        if !self.unexpired(&snapshot.record) {
            return Err(IdentityError::Transient);
        }
        self.check_local_barrier()?;
        let epoch = self.readiness_epoch()?;
        let verified = self.verify_status(snapshot, wait)?;
        let _barrier = self.barrier_lock(wait)?;
        let mut store = self.store()?;
        let current = self.snapshot(store.as_ref())?;
        Self::check_unchanged(snapshot, &current)?;
        self.check_readiness_epoch(epoch)?;
        let verified = verified?;
        self.check_pending_verification(&current.record, &verified)?;
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
        self.set_verified(epoch, Self::generations(&active))?;
        self.lease(active)
    }

    fn check_pending_verification(&self, record: &StoredIdentity, verified: &VerifiedSession) -> Result<(), IdentityError> {
        let binding = match verified.same_login_binding.as_deref() {
            Some(binding) => binding,
            // Generations above 1 were produced only by rotating the stored refresh
            // token. This remains true of a pending pair recovered after a crash.
            None if self.refresh_preserves_binding && record.token_generation > 1 && record.verified_account_id.is_some() => {
                record.session_binding.as_deref().ok_or(IdentityError::VerificationPending)?
            }
            None => return Err(IdentityError::VerificationPending),
        };
        if record.session_binding.as_deref() != Some(binding)
            || record.verified_account_id.as_ref().is_some_and(|account| account != &verified.account_id)
        {
            return Err(IdentityError::SessionChanged);
        }
        if !safe_token(&verified.account_id, MAX_ACCOUNT_BYTES) {
            return Err(IdentityError::Protocol);
        }
        Ok(())
    }

    fn ready(&self, snapshot: &Snapshot) -> bool {
        let clock_safe = self.clock_is_safe(&snapshot.record);
        clock_safe
            && snapshot.attempt.is_none()
            && !snapshot.record.pending
            && self.fresh(&snapshot.record)
            && self.has_verified_generation(Self::generations(&snapshot.record))
    }

    fn has_verified_generation(&self, generations: (u64, u64)) -> bool {
        self.readiness.lock().is_ok_and(|readiness| readiness.generation == Some(generations))
    }

    /// Independent domain-wide read-only budget, persisted before transport so
    /// restarting (or changing triggers) cannot reset an in-progress batch.
    fn verify_status(&self, snapshot: &Snapshot, wait: &LockWait) -> Result<Result<VerifiedSession, VerifyError>, IdentityError> {
        wait.require_request()?;
        let (mut attempt, remaining, reserved_at) = {
            let _barrier = self.barrier_lock(wait)?;
            let current = self.snapshot(self.store()?.as_ref())?;
            Self::check_unchanged(snapshot, &current)?;
            let now = self.clock.now_unix_seconds();
            let mut attempt = self.journal.read_status_attempt()?.unwrap_or(StatusAttempt {
                domain: self.domain.storage_key().to_owned(),
                started: now,
                sends: 0,
                next: now,
                cooldown_step: 0,
            });
            if attempt.domain != self.domain.storage_key() {
                return Err(IdentityError::Storage);
            }
            if now < attempt.next {
                return Err(IdentityError::Transient);
            }
            let elapsed = now.saturating_sub(attempt.started);
            if attempt.sends >= MAX_SENDS || elapsed.saturating_add(REQUEST_TIMEOUT.as_secs()) > BATCH_BUDGET.as_secs() {
                attempt.started = now;
                attempt.sends = 0;
            }
            attempt.sends += 1;
            if attempt.sends >= MAX_SENDS {
                // If this process dies in transport, the next batch must still cool
                // down. Reserve through the request timeout; completion replaces it.
                attempt.next = now.saturating_add(REQUEST_TIMEOUT.as_secs()).saturating_add(batch_cooldown(attempt.cooldown_step));
            }
            let remaining = BATCH_BUDGET.saturating_sub(Duration::from_secs(now.saturating_sub(attempt.started)));
            let reserved_at = self.clock.monotonic_now();
            self.journal.write_status_attempt(&attempt)?;
            (attempt, remaining, reserved_at)
        };
        wait.require_request()?;
        self.check_local_barrier()?;
        if self.clock.now_unix_seconds().saturating_sub(attempt.started).saturating_add(REQUEST_TIMEOUT.as_secs()) > BATCH_BUDGET.as_secs()
            || self.clock.monotonic_now().checked_sub(reserved_at).is_none_or(|elapsed| elapsed.saturating_add(REQUEST_TIMEOUT) > remaining)
        {
            return Err(IdentityError::Transient);
        }
        let result = self.verifier.verify(&Secret::new(snapshot.record.access_token.clone()));
        let now = self.clock.now_unix_seconds();
        let failed =
            result.as_ref().map_or(true, |verified| snapshot.record.pending && self.check_pending_verification(&snapshot.record, verified).is_err());
        let retry_after = if let Err(VerifyError::RetryAfter(delay)) = result { delay.as_secs() } else { 0 };
        let retry = retry_delay(attempt.sends).max(retry_after);
        // The refresh locks already serialize all status reservations and completions.
        let exhausted = attempt.sends >= MAX_SENDS
            || now.saturating_sub(attempt.started).saturating_add(retry.saturating_add(REQUEST_TIMEOUT.as_secs())) > BATCH_BUDGET.as_secs();
        // Back off from completion, not reservation. Even a successful last send
        // leaves the independent inter-batch throttle in place.
        if failed || exhausted {
            let delay = if exhausted {
                let delay = batch_cooldown(attempt.cooldown_step).max(retry_after);
                attempt.cooldown_step = (attempt.cooldown_step + 1).min(MAX_COOLDOWN_STEP);
                attempt.sends = MAX_SENDS;
                delay
            } else {
                retry
            };
            attempt.next = now.saturating_add(delay);
            let _barrier = self.barrier_lock(wait)?;
            let current = self.snapshot(self.store()?.as_ref())?;
            Self::check_unchanged(snapshot, &current)?;
            self.journal.write_status_attempt(&attempt)?;
        }
        Ok(result)
    }

    fn generations(record: &StoredIdentity) -> (u64, u64) {
        (record.login_generation, record.token_generation)
    }

    fn readiness_epoch(&self) -> Result<u64, IdentityError> {
        Ok(self.readiness.lock().map_err(|_| IdentityError::Storage)?.epoch)
    }

    fn check_readiness_epoch(&self, epoch: u64) -> Result<(), IdentityError> {
        if self.readiness_epoch()? == epoch { Ok(()) } else { Err(IdentityError::Transient) }
    }

    fn invalidate_readiness(&self) -> Result<u64, IdentityError> {
        let mut readiness = self.readiness.lock().map_err(|_| IdentityError::Storage)?;
        readiness.epoch = readiness.epoch.checked_add(1).ok_or(IdentityError::Storage)?;
        readiness.generation = None;
        Ok(readiness.epoch)
    }

    fn set_verified(&self, epoch: u64, generations: (u64, u64)) -> Result<(), IdentityError> {
        let mut readiness = self.readiness.lock().map_err(|_| IdentityError::Storage)?;
        if readiness.epoch != epoch {
            return Err(IdentityError::Transient);
        }
        readiness.generation = Some(generations);
        Ok(())
    }

    fn clear_readiness(&self) {
        if let Ok(mut readiness) = self.readiness.lock() {
            readiness.generation = None;
        }
    }

    fn clock_is_safe(&self, record: &StoredIdentity) -> bool {
        let now = self.clock.now_unix_seconds();
        let previous = self.last_now_unix.fetch_max(now, Ordering::AcqRel);
        if now < previous || now < record.issued_at_unix {
            self.clear_readiness();
            if let Ok(mut start) = self.attempt_start.lock() {
                *start = None;
            }
            return false;
        }
        true
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
        if record.pending || !self.clock_is_safe(&record) || !self.fresh(&record) || !self.has_verified_generation(Self::generations(&record)) {
            return Err(IdentityError::Transient);
        }
        Ok(IdentityLease {
            access_token: Secret::new(record.access_token),
            session_binding: record.session_binding,
            verified_account_id: record.verified_account_id,
            login_generation: record.login_generation,
            token_generation: record.token_generation,
            expires_at_unix: record.expires_at_unix,
        })
    }

    fn snapshot(&self, store: &dyn IdentityStore) -> Result<Snapshot, IdentityError> {
        let record = self.current_record(store)?;
        let attempt = self.journal.read_attempt()?;
        if attempt.as_ref().is_some_and(|attempt| attempt.version != RECORD_VERSION) {
            return Err(IdentityError::Storage);
        }
        let attempt = match attempt {
            Some(attempt)
                if attempt.domain != record.domain
                    || attempt.login_generation != record.login_generation
                    || attempt.token_generation != record.token_generation =>
            {
                self.clear_attempt()?;
                None
            }
            attempt => attempt,
        };
        Ok(Snapshot { record, attempt })
    }

    fn check_unchanged(before: &Snapshot, after: &Snapshot) -> Result<(), IdentityError> {
        if before.record.login_generation != after.record.login_generation {
            return Err(IdentityError::SessionChanged);
        }
        if before.record != after.record || before.attempt != after.attempt {
            return Err(IdentityError::Transient);
        }
        Ok(())
    }

    fn cool_down(&self, mut attempt: AttemptRecord, retry_after: Option<Duration>, start: AttemptStart) -> Result<(), IdentityError> {
        let step = attempt.cooldown_step.min(MAX_COOLDOWN_STEP);
        let retry_after = retry_after.map_or(0, |delay| delay.as_secs());
        let intra_batch = retry_delay(attempt.sends).max(retry_after);
        let delay = if !attempt.diagnostic_used
            && attempt.sends < MAX_SENDS
            && start.fits_request(self.clock.monotonic_now().saturating_add(Duration::from_secs(intra_batch)))
        {
            intra_batch
        } else {
            attempt.cooldown_step = (step + 1).min(MAX_COOLDOWN_STEP);
            batch_cooldown(step).max(retry_after)
        };
        attempt.cooldown_until_unix = self.clock.now_unix_seconds().saturating_add(delay);
        self.check_local_barrier()?;
        self.journal.write_attempt(&attempt)?;
        Ok(())
    }

    fn mark_protocol_blocked(&self, mut attempt: AttemptRecord) -> Result<AttemptRecord, IdentityError> {
        // Refresh locks exclude other senders. Serialize only with tombstone writes, not
        // the barrier lock, which a host commit or keyring cleanup can hold past our budget.
        let _attempt = self.attempt_lock()?;
        self.check_local_barrier()?;
        if self.journal.read_attempt()?.as_ref() != Some(&attempt) {
            return Err(IdentityError::Transient);
        }
        let delay = batch_cooldown(attempt.cooldown_step);
        attempt.cooldown_step = if attempt.diagnostic_used && attempt.sends >= MAX_SENDS { PROTOCOL_EXHAUSTED } else { PROTOCOL_BLOCKED };
        attempt.sends = MAX_SENDS;
        attempt.diagnostic_used = true;
        attempt.cooldown_until_unix = self.clock.now_unix_seconds().saturating_add(delay);
        self.journal.write_attempt(&attempt)?;
        Ok(attempt)
    }

    fn clear_attempt(&self) -> Result<(), IdentityError> {
        *self.attempt_start.lock().map_err(|_| IdentityError::Storage)? = None;
        let _attempt = self.attempt_lock()?;
        self.journal.clear_attempt()?;
        Ok(())
    }

    fn attempt_lock(&self) -> Result<FileLock, IdentityError> {
        lock_file(&self.lock_dir.join("attempt.lock"), &LockWait::new(MUTATION_WAIT))
    }

    fn cleanup_revoked(&self, generation: u64, wait: &LockWait) -> SignOutOutcome {
        // Revocation is already visible. Re-enter only for conditional cleanup; a
        // concurrent explicit login must never have its new credentials deleted.
        let cleanup = (|| {
            let _barrier = self.barrier_lock(wait)?;
            self.cleanup_revoked_locked(generation)
        })();
        SignOutOutcome { credentials_cleared: cleanup.unwrap_or(false) }
    }

    fn cleanup_revoked_locked(&self, generation: u64) -> Result<bool, IdentityError> {
        let mut store = self.store()?;
        if self.read_revocation()?.is_none_or(|revoked| revoked < generation) {
            return Err(IdentityError::Storage);
        }
        if let Some(record) = self.load(store.as_ref())?.filter(|record| record.login_generation > generation) {
            if self.read_current_login()? > generation {
                // Do not remove a newer, fully published login.
                return Ok(true);
            }
            // An interrupted adoption left an owned pair, not a published login.
            self.publish_logout_barrier(record.login_generation)?;
        }
        let deleted = store.delete().is_ok();
        let _ = self.clear_attempt();
        Ok(deleted)
    }

    fn current_record(&self, store: &dyn IdentityStore) -> Result<StoredIdentity, IdentityError> {
        self.check_local_barrier()?;
        let revoked = self.read_revocation()?;
        let current = self.read_current_login()?;
        if revoked.is_some_and(|revoked| current <= revoked) || (current != 0 && revoked.is_none()) {
            return Err(IdentityError::SignedOut);
        }
        let record = self.load(store)?.ok_or(IdentityError::SignedOut)?;
        // Credentials may have been activated just before an interrupted publication.
        if record.login_generation != current {
            return Err(IdentityError::Storage);
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

    fn read_revocation(&self) -> Result<Option<u64>, IdentityError> {
        self.journal.read_revocation()?.map(|record| self.revocation_generation(record)).transpose()
    }

    fn revocation_generation(&self, record: RevocationRecord) -> Result<u64, IdentityError> {
        if record.version != RECORD_VERSION || record.domain != self.domain.storage_key() {
            return Err(IdentityError::Storage);
        }
        Ok(record.login_generation)
    }

    fn read_current_login(&self) -> Result<u64, IdentityError> {
        self.current_login_generation(self.journal.read_current_login()?)
    }

    fn current_login_generation(&self, record: Option<CurrentLoginRecord>) -> Result<u64, IdentityError> {
        let Some(record) = record else {
            return Ok(0);
        };
        if record.version != RECORD_VERSION || record.domain != self.domain.storage_key() || record.login_generation == 0 {
            return Err(IdentityError::Storage);
        }
        Ok(record.login_generation)
    }

    fn write_revocation(&self, login_generation: u64) -> Result<(), IdentityError> {
        self.write_revocation_with_fence(login_generation, None)
    }

    fn write_revocation_with_fence(&self, login_generation: u64, fence_id: Option<[u8; 16]>) -> Result<(), IdentityError> {
        self.journal.write_revocation(&RevocationRecord {
            version: RECORD_VERSION,
            domain: self.domain.storage_key().to_owned(),
            login_generation,
            revoked_at_unix: self.clock.now_unix_seconds(),
            fence_id,
        })?;
        Ok(())
    }

    fn publish_logout_barrier(&self, generation: u64) -> Result<(), IdentityError> {
        let generation = generation.max(self.read_revocation()?.unwrap_or(0));
        // Once logout is confirmed, neither slot alone may resurrect credentials.
        self.write_revocation(generation)?;
        self.write_revocation(generation)
    }

    fn barrier_lock(&self, wait: &LockWait) -> Result<FileLock, IdentityError> {
        lock_file(&self.lock_dir.join("barrier.lock"), wait)
    }

    fn lock_refresh(&self, wait: &LockWait) -> Result<MutexGuard<'_, ()>, IdentityError> {
        loop {
            match self.refresh.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(MutexTryLockError::WouldBlock) => {
                    #[cfg(test)]
                    self.clock.on_local_refresh_contention();
                    wait.poll()?;
                }
                Err(MutexTryLockError::Poisoned(_)) => return Err(IdentityError::Busy),
            }
        }
    }
}

/// All lock polls in one call share a wait budget. Use elapsed time (not a wall-clock deadline)
/// and try once even with zero budget. The injected clock governs identity policy, not sleeps.
struct LockWait {
    started: Instant,
    budget: Duration,
    lock_budget: Duration,
}

impl LockWait {
    fn new(budget: Duration) -> Self {
        Self { started: Instant::now(), budget, lock_budget: budget.min(MUTATION_WAIT) }
    }

    fn require_request(&self) -> Result<(), IdentityError> {
        if self.budget.saturating_sub(self.started.elapsed()) < REQUEST_TIMEOUT { Err(IdentityError::Transient) } else { Ok(()) }
    }

    fn poll(&self) -> Result<(), IdentityError> {
        let remaining = self.lock_budget.saturating_sub(self.started.elapsed());
        if remaining.is_zero() {
            return Err(IdentityError::Busy);
        }
        std::thread::sleep(remaining.min(LOCK_POLL));
        if self.started.elapsed() >= self.lock_budget { Err(IdentityError::Busy) } else { Ok(()) }
    }
}

fn batch_cooldown(step: u8) -> u64 {
    [5, 10, 20, 60][usize::from(step.min(MAX_COOLDOWN_STEP))]
}

fn retry_delay(sends: u8) -> u64 {
    // The durable schedule has second precision. Add up to one second of
    // per-process jitter in production, keeping scripted-clock tests exact.
    let jitter = if cfg!(test) { 0 } else { u64::from(std::process::id() % 2) };
    u64::from(sends.clamp(1, 2)) + jitter
}

struct FileLock(File);

impl Drop for FileLock {
    fn drop(&mut self) {
        // On Unix, a concurrent fork can keep the open file description alive until exec.
        // Release the lock at the end of this critical section, not at the last close.
        let _ = self.0.unlock();
    }
}

fn lock_file(path: &Path, wait: &LockWait) -> Result<FileLock, IdentityError> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(|_| IdentityError::Busy)?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(FileLock(file)),
            Err(TryLockError::WouldBlock) => wait.poll()?,
            Err(TryLockError::Error(_)) => return Err(IdentityError::Busy),
        }
    }
}
