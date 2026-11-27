//! Platform credential storage and the explicit WorkOS device-code login.
//!
//! The desktop keeps one account session in the operating-system credential store (macOS
//! Keychain, Windows Credential Manager, Linux Secret Service). A headless process without a
//! secure store either keeps a short, explicit in-memory session or refuses to persist at all;
//! it never writes a credential to a file or the operation ledger.
//!
//! Secrets are handed to [`CredentialSession`] by the HTTP adapter and read back by the same
//! session; there is no second, no-op handoff. Credentials are redacted from `Debug`, `Display`,
//! and every error message this module produces. The explicit device-code login lives in
//! [`crate::device_code`].
//!
//! Provider and product credentials are deliberately separate. A completed WorkOS provider login
//! is stored under `provider/…` and is never returned as the `Authorization: Bearer` product
//! session. Only a typed [`ProductSession`] returned by a trusted [`ProductSessionExchange`] can
//! activate `auth/product_session`. The default [`BlockedProductExchange`] keeps the live path
//! absent; the reviewed server-verified scheme is
//! [`crate::ServerVerifiedProductSession`], which activates only after the configured product
//! service confirms the WorkOS token through the frozen status contract.

use sha2::{Digest, Sha256};
use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, Capability, CredentialError, CredentialErrorCode, CredentialStore, GrantId, HostSessionBinding,
    InstallationId, OperationId, Refusal, RefusalCode,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::http::{CredentialHandoff, Secret, WireCredentials};

/// The desktop credential-store service. Deliberately distinct from the agent chat's
/// `"SolidCraft agent chat"` service so account credentials never share an entry namespace.
pub const ACCOUNT_KEYRING_SERVICE: &str = "Intcad account";

const MAX_TRACKED_KEYS: usize = 4096;
const MAX_KEY_BYTES: usize = 200;
const MAX_INDEX_BYTES: usize = 64 * 1024;
pub(crate) const MAX_ACCOUNT_BYTES: usize = 128;
const MAX_IDENTIFIER_BYTES: usize = 80;
const MAX_SESSION_BINDING_BYTES: usize = 128;

const INDEX_KEY: &str = "credentials/index";
const ACCOUNT_KEY: &str = "account/id";
pub(crate) const PRODUCT_SESSION_KEY: &str = "auth/product_session";
const SESSION_BINDING_KEY: &str = "auth/session_binding";
const INSTALLATION_ID_KEY: &str = "install/id";
const INSTALLATION_CREDENTIAL_KEY: &str = "install/credential";
// Provider-side identity and token. These are never returned as an Intcad product bearer; they
// are input only to the explicit product-session exchange.
const PROVIDER_ACCOUNT_KEY: &str = "provider/account";
const PROVIDER_ACCESS_TOKEN_KEY: &str = "provider/access_token";
// Legacy key from the first Task 4 delivery. It is deleted during cleanup but never written again:
// the documented WorkOS flow has no reviewed refresh use yet.
const REFRESH_TOKEN_KEY: &str = "auth/refresh_token";
/// A durable sign-out intent. It is written before any credential is deleted and removed only
/// after the old authority is fully gone. A present or unreadable marker means startup must not
/// restore a session or admit controlled work.
const LOGOUT_KEY: &str = "auth/logout_pending";
/// The single durable activation commit. No `account/…`, `auth/product_session`, installation or
/// operation credential is read unless this marker holds a complete, matching identity. Staged
/// keys are never read directly. Writing this marker last is the documented linearization point;
/// a crash before it leaves only non-restorable staged/partial values.
const LOGIN_COMMIT_KEY: &str = "login/commit";
const LOGIN_STAGE_ACCOUNT_KEY: &str = "login/stage/account";
const LOGIN_STAGE_BINDING_KEY: &str = "login/stage/binding";
const LOGIN_STAGE_PRODUCT_KEY: &str = "login/stage/product_session";
const LOGIN_STAGE_PROVIDER_ACCOUNT_KEY: &str = "login/stage/provider_account";
const LOGIN_STAGE_PROVIDER_TOKEN_KEY: &str = "login/stage/provider_token";
const LOGIN_COMMIT_DONE: &str = "intcad-login/1";
const LOGIN_COMMIT_REVOKED: &str = "intcad-login/revoked";
const KEYRING_PROBE_KEY: &str = "intcad/keyring-probe";

// CredentialStore does not expose a namespace identifier, so serialize login replacement
// and bearer publication across all independently opened stores in this process.
static PRODUCT_AUTHORITY_WRITE: Mutex<()> = Mutex::new(());

fn lock_product_authority_write() -> Result<MutexGuard<'static, ()>, CredentialError> {
    PRODUCT_AUTHORITY_WRITE.lock().map_err(|_| unavailable())
}

pub(crate) fn unavailable() -> CredentialError {
    CredentialError { code: CredentialErrorCode::Unavailable, message: "the credential store is unavailable".into() }
}

fn no_persistent_credential() -> CredentialError {
    CredentialError {
        code: CredentialErrorCode::NoPersistentCredential,
        message: "no secure credential store is available; the session is not persisted".into(),
    }
}

pub(crate) fn auth_unavailable(message: &'static str) -> Refusal {
    Refusal::new(RefusalCode::AuthUnavailable, message)
}

fn handoff_refusal() -> Refusal {
    auth_unavailable("the credential store could not persist the credential")
}

/// A failed durable write locks the session unless the store is simply not persistent. A headless
/// caller keeps the stable `NoPersistentCredential` cause instead of a generic failure.
fn store_failure(state: &mut SessionState, error: CredentialError) -> CredentialError {
    if error.code != CredentialErrorCode::NoPersistentCredential {
        state.locked = true;
    }
    error
}

pub(crate) fn has_control(value: &str) -> bool {
    value.bytes().any(|byte| byte.is_ascii_control())
}

pub(crate) fn safe_token(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A headless store with no secure storage: every write is refused, so no plaintext is kept.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoPersistentStore;

impl CredentialStore for NoPersistentStore {
    fn get(&self, _key: &str) -> Result<Option<String>, CredentialError> {
        Ok(None)
    }

    fn set(&mut self, _key: &str, _secret: &str) -> Result<(), CredentialError> {
        Err(no_persistent_credential())
    }

    fn delete(&mut self, _key: &str) -> Result<(), CredentialError> {
        Ok(())
    }

    fn persistent(&self) -> bool {
        false
    }
}

/// A process-local session. It is the explicit headless fallback: tokens vanish when the
/// process exits (or [`MemorySession::clear`] runs) and are never written to disk.
#[derive(Clone, Default)]
pub struct MemorySession {
    entries: Arc<Mutex<BTreeMap<String, String>>>,
}

impl MemorySession {
    pub fn clear(&self) -> Result<(), CredentialError> {
        let mut entries = self.entries.lock().map_err(|_| unavailable())?;
        for value in entries.values_mut() {
            value.clear();
        }
        entries.clear();
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().map(|entries| entries.is_empty()).unwrap_or(false)
    }
}

impl fmt::Debug for MemorySession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("MemorySession").field("entries", &"<redacted>").finish()
    }
}

impl CredentialStore for MemorySession {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        Ok(self.entries.lock().map_err(|_| unavailable())?.get(key).cloned())
    }

    fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
        self.entries.lock().map_err(|_| unavailable())?.insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
        self.entries.lock().map_err(|_| unavailable())?.remove(key);
        Ok(())
    }

    fn persistent(&self) -> bool {
        false
    }
}

/// The operating-system credential store. One entry per key in [`ACCOUNT_KEYRING_SERVICE`].
#[derive(Clone)]
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }

    pub fn account() -> Self {
        Self::new(ACCOUNT_KEYRING_SERVICE)
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(&self.service, key).map_err(|_| unavailable())
    }
}

impl fmt::Debug for KeyringStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("KeyringStore").field("service", &self.service).field("credentials", &"<redacted>").finish()
    }
}

impl CredentialStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        get_keyring_value(&self.entry(key)?, key)
    }

    fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
        set_keyring_value(&self.entry(key)?, key, secret)
    }

    fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(unavailable()),
        }
    }

    /// A non-destructive probe: a missing entry means the backend is reachable, while a storage
    /// or platform failure means this process has no persistent secure store.
    fn persistent(&self) -> bool {
        match keyring::Entry::new(&self.service, KEYRING_PROBE_KEY) {
            Ok(entry) => matches!(
                entry.get_password(),
                Ok(_) | Err(keyring::Error::NoEntry) | Err(keyring::Error::BadEncoding(_)) | Err(keyring::Error::Ambiguous(_))
            ),
            Err(_) => false,
        }
    }
}

pub(super) fn get_keyring_value(entry: &keyring::Entry, key: &str) -> Result<Option<String>, CredentialError> {
    let value = if key.starts_with("identity/") {
        entry.get_secret().and_then(|bytes| String::from_utf8(bytes).map_err(|error| keyring::Error::BadEncoding(error.into_bytes())))
    } else {
        entry.get_password()
    };
    match value {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}

pub(super) fn set_keyring_value(entry: &keyring::Entry, key: &str, secret: &str) -> Result<(), CredentialError> {
    // Windows password storage encodes UTF-16LE, doubling ASCII token sizes.
    if key.starts_with("identity/") { entry.set_secret(secret.as_bytes()) } else { entry.set_password(secret) }.map_err(|_| unavailable())
}

/// A completed provider login. It proves only the WorkOS provider session; it is not an Intcad
/// product login and cannot be used as a product bearer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderLogin {
    pub provider_account_id: String,
}

/// The refresh material captured from a completed device login. It is kept in memory on the
/// provider session for the renewal lifecycle; the current login paths do not persist it.
#[derive(Clone)]
pub struct ProviderRefresh {
    refresh_token: Secret,
    expires_at_unix_seconds: u64,
}

impl ProviderRefresh {
    pub fn refresh_token(&self) -> &Secret {
        &self.refresh_token
    }

    /// The access token's expiry as Unix seconds. It is a scheduling hint for renewal, not an
    /// authentication decision.
    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }
}

impl fmt::Debug for ProviderRefresh {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRefresh")
            .field("refresh_token", &"<redacted>")
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
            .finish()
    }
}

/// The typed provider credentials offered to the explicit product-session exchange. The access
/// token is available to the trusted exchange implementation only; it is never returned by
/// [`WireCredentials`].
#[derive(Clone)]
pub struct ProviderSession {
    account_id: String,
    access_token: Secret,
    refresh: Option<ProviderRefresh>,
}

impl ProviderSession {
    /// The legacy typed provider handoff. Stored provider logins from before refresh capture
    /// have no refresh token, so this constructor keeps them usable with `refresh() == None`.
    /// It carries no product authority and must be verified by a trusted exchange before
    /// anything is persisted.
    pub(crate) fn from_authorized(account_id: String, access_token: Secret) -> Self {
        Self { account_id, access_token, refresh: None }
    }

    /// The refreshable handoff from a device login that issued a refresh token and an access
    /// token expiry. Like [`Self::from_authorized`] it carries no product authority.
    pub(crate) fn from_authorized_refreshable(account_id: String, access_token: Secret, refresh_token: Secret, expires_at_unix_seconds: u64) -> Self {
        Self { account_id, access_token, refresh: Some(ProviderRefresh { refresh_token, expires_at_unix_seconds }) }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn access_token(&self) -> &Secret {
        &self.access_token
    }

    /// The captured refresh material, or `None` for a legacy login without a refresh token.
    pub fn refresh(&self) -> Option<&ProviderRefresh> {
        self.refresh.as_ref()
    }
}

impl fmt::Debug for ProviderSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSession")
            .field("account_id", &self.account_id)
            .field("access_token", &"<redacted>")
            .field("refresh", &self.refresh.is_some())
            .finish()
    }
}

/// A typed Intcad product session produced only by a trusted [`ProductSessionExchange`].
#[derive(Clone)]
pub struct ProductSession {
    account_id: String,
    product_session: Secret,
    expires_at_unix_seconds: Option<u64>,
}

impl ProductSession {
    /// The only public construction path. Call this only from a host-owned exchange that follows
    /// a documented, reviewed protocol; the value must never come from a provider token directly.
    pub fn from_trusted_exchange(account_id: impl Into<String>, product_session: Secret, expires_at_unix_seconds: Option<u64>) -> Self {
        Self { account_id: account_id.into(), product_session, expires_at_unix_seconds }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn product_session(&self) -> &Secret {
        &self.product_session
    }

    pub fn expires_at_unix_seconds(&self) -> Option<u64> {
        self.expires_at_unix_seconds
    }
}

impl fmt::Debug for ProductSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductSession")
            .field("account_id", &self.account_id)
            .field("product_session", &"<redacted>")
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
            .finish()
    }
}

/// Why a documented product-session exchange did not produce a product session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductExchangeError {
    /// No reviewed exchange is configured yet; the live endpoint is intentionally absent.
    NotConfigured,
    /// The exchange refused the provider session.
    Rejected,
    /// The exchange could not complete (transport or malformed result).
    Unavailable,
}

/// The narrow, typed product-session exchange seam. A real implementation must use a documented,
/// reviewed exchange; this crate ships only [`BlockedProductExchange`],
/// [`crate::ServerVerifiedProductSession`] and scripted test doubles.
pub trait ProductSessionExchange {
    fn exchange(&self, provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError>;
}

/// The default blocked path: no provider token ever becomes a product bearer.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockedProductExchange;

impl ProductSessionExchange for BlockedProductExchange {
    fn exchange(&self, _provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
        Err(ProductExchangeError::NotConfigured)
    }
}

/// An activated Intcad product account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductLogin {
    pub account_id: String,
}

/// The single credential owner shared by [`WireCredentials`] and [`CredentialHandoff`].
///
/// `account/…`, `auth/…`, `install/…`, and `scope/…` keys are deliberately separate. Per-operation
/// proofs and settlement credentials are scoped by the authenticated account and installation; an
/// initial lease proof is additionally keyed by the server-issued authorization id. Clones share
/// one store and one lock, so the HTTP adapter can hold both trait objects without a no-op handoff.
#[derive(Clone)]
pub struct CredentialSession {
    inner: Arc<Mutex<SessionState>>,
    // Separate from the keyring lock: desktop gates read this without doing credential I/O.
    refused_renewals: Arc<Mutex<BTreeSet<(AuthenticatedHostScope, OperationId)>>>,
    /// An in-process revocation barrier, set synchronously before any keyring cleanup starts.
    /// Reads consult it without taking the session lock, so they fail immediately even while a
    /// worker holds the lock across a slow keyring delete. Only a completed cleanup for the
    /// owning generation may clear it.
    revoked: Arc<AtomicBool>,
    /// Monotonic revocation epoch. A reader that captured it before waiting for the session lock
    /// must not return authority after it changes, even if a newer cleanup cleared the boolean.
    epoch: Arc<AtomicU64>,
}

struct SessionState {
    store: Box<dyn CredentialStore + Send>,
    index: BTreeSet<String>,
    /// The Intcad product account. `None` until a trusted product exchange has activated a session.
    account: Option<String>,
    installation: Option<String>,
    /// The WorkOS provider account. It never authorizes an Intcad call by itself.
    provider_account: Option<String>,
    /// Blocks credential reads and ordinary mutations during promotion or after a storage
    /// failure. Sign-out cleanup may still remove credentials but cannot lift a failure lock.
    /// Only a fresh verified login can replace a failed session: revoke the old commit first,
    /// then unlock only after the new commit succeeds.
    locked: bool,
}

/// A prepared login. Until the matching commit is written, even a fresh reader
/// in another process cannot use the staged bearer.
pub struct StagedLogin {
    account: String,
    binding: String,
    generation: u128,
    epoch: u64,
    owner: std::sync::Weak<Mutex<SessionState>>,
}

impl StagedLogin {
    pub fn account_id(&self) -> &str {
        &self.account
    }

    pub fn session_binding(&self) -> &str {
        &self.binding
    }
}

impl CredentialSession {
    pub fn new(store: Box<dyn CredentialStore + Send>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionState {
                store,
                index: BTreeSet::new(),
                account: None,
                installation: None,
                provider_account: None,
                locked: false,
            })),
            revoked: Arc::new(AtomicBool::new(false)),
            epoch: Arc::new(AtomicU64::new(0)),
            refused_renewals: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    pub fn renewal_refusal(&self, scope: &AuthenticatedHostScope, operation: &OperationId) -> Option<RefusalCode> {
        match self.refused_renewals.lock() {
            Ok(refusals) => refusals.contains(&(scope.clone(), operation.clone())).then_some(RefusalCode::AuthRequired),
            Err(_) => Some(RefusalCode::AuthUnavailable),
        }
    }

    /// Fail all credential, scope and handoff operations synchronously. This never takes the
    /// session lock, so a caller can revoke on the window thread while a worker holds the lock
    /// for keyring IO. The barrier closes before the epoch advances, so no reader can load the
    /// new epoch while the boolean is still open; the new epoch then invalidates readers and
    /// writers that captured an older one before waiting on the session lock.
    pub fn revoke_in_process(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Re-enable reads after the owning cleanup generation completed successfully.
    pub fn end_revocation(&self) {
        self.revoked.store(false, Ordering::SeqCst);
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::SeqCst)
    }

    /// The monotonic revocation generation. It advances before any sign-out or switch cleanup
    /// starts, so a caller comparing it before and after a network read can reject a result that
    /// was admitted by the retired session.
    pub fn authority_generation(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// The common read gate: check the lock-free barrier first, then recheck it and the epoch
    /// after the session lock is acquired. The returned epoch must also be checked after any
    /// store IO before returning a credential.
    fn lock_if_authorized(&self) -> Option<(MutexGuard<'_, SessionState>, u64)> {
        if self.revoked.load(Ordering::SeqCst) {
            return None;
        }
        let epoch = self.epoch.load(Ordering::SeqCst);
        let state = self.inner.lock().ok()?;
        if !self.epoch_unchanged(epoch) || !authority_usable(&state) {
            return None;
        }
        Some((state, epoch))
    }

    fn epoch_unchanged(&self, epoch: u64) -> bool {
        !self.revoked.load(Ordering::SeqCst) && self.epoch.load(Ordering::SeqCst) == epoch
    }

    /// Gate a host handoff write. The session lock serializes the write against cleanup, and the
    /// epoch recheck makes a revoke that happened while waiting fail the write. Cleanup needs the
    /// same lock, so a write that raced a revoke is deleted before cleanup reports success.
    fn lock_for_handoff(&self) -> Result<(MutexGuard<'_, SessionState>, u64), Refusal> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(handoff_refusal());
        }
        let epoch = self.epoch.load(Ordering::SeqCst);
        let state = self.lock().map_err(|_| handoff_refusal())?;
        if !self.epoch_unchanged(epoch) || !authority_usable(&state) {
            return Err(handoff_refusal());
        }
        Ok((state, epoch))
    }

    pub fn with_keyring() -> Self {
        Self::new(Box::new(KeyringStore::account()))
    }

    pub fn with_memory() -> Self {
        Self::new(Box::new(MemorySession::default()))
    }

    pub fn without_persistence() -> Self {
        Self::new(Box::new(NoPersistentStore))
    }

    pub fn bound_account(&self) -> Option<String> {
        let (state, _epoch) = self.lock_if_authorized()?;
        activation_committed(&state).then_some(state.account.clone()).flatten()
    }

    /// The account label of a persisted, active product session for display only. Unlike
    /// [`Self::bound_account`] this does not require an installation binding; it never proves
    /// `AuthenticatedHostScope` or authorizes controlled work. It still requires the durable
    /// activation commit, so a staged or partial write is never shown.
    pub fn display_account(&self) -> Option<String> {
        let (state, epoch) = self.lock_if_authorized()?;
        let account = committed_account(&state)?;
        self.epoch_unchanged(epoch).then_some(account)
    }

    pub fn is_persistent(&self) -> bool {
        self.inner.lock().map(|state| state.store.persistent()).unwrap_or(false)
    }

    /// The host's provable account, installation and session binding, or `None`.
    ///
    /// The binding is an opaque local identifier minted at explicit login, not a bearer or a
    /// server-verified claim. Rotating the bearer does not change it.
    pub fn authenticated_scope(&self) -> Option<AuthenticatedHostScope> {
        let (mut state, epoch) = self.lock_if_authorized()?;
        let scope = current_scope(&mut state)?;
        self.epoch_unchanged(epoch).then_some(scope)
    }

    /// Publish an already verified replacement bearer without changing the login's identity.
    ///
    /// For a shared keyring, all desktop/CLI writers in other processes must also honor the
    /// per-user `account-keyring.lock`; the credential-store interface has no cross-process CAS.
    pub fn update_product_session(&self, expected_scope: &AuthenticatedHostScope, token: &Secret) -> Result<(), CredentialError> {
        if token.is_empty() || has_control(token.expose_secret()) {
            return Err(unavailable());
        }
        let generation = self.authority_generation();
        let _write = lock_product_authority_write()?;
        let (mut state, epoch) = self.lock_for_handoff().map_err(|_| unavailable())?;
        if generation != epoch {
            return Err(unavailable());
        }
        let commit = read_secret(&state, LOGIN_COMMIT_KEY).ok_or_else(unavailable)?;
        if current_scope(&mut state).as_ref() != Some(expected_scope)
            || read_secret(&state, LOGIN_COMMIT_KEY).as_ref() != Some(&commit)
            || read_session_binding(&state).as_deref() != Some(expected_scope.session_binding().as_str())
            || !authority_usable(&state)
            || !self.epoch_unchanged(epoch)
        {
            return Err(unavailable());
        }
        state.store.set(PRODUCT_SESSION_KEY, token.expose_secret()).map_err(|error| store_failure(&mut state, error))?;
        if !authority_usable(&state) || !self.epoch_unchanged(epoch) {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(crate) fn bound_product_session(&self, scope: &AuthenticatedHostScope, generation: u64) -> Option<Secret> {
        let (mut state, epoch) = self.lock_if_authorized()?;
        if epoch != generation || current_scope(&mut state).as_ref() != Some(scope) {
            return None;
        }
        let bearer = read_secret(&state, PRODUCT_SESSION_KEY)?;
        self.epoch_unchanged(epoch).then_some(bearer)
    }

    /// Persist a response's settlement credential only into the session that sent admission.
    pub(crate) fn persist_bound_settlement(
        &self,
        expected: &AuthenticatedHostScope,
        generation: u64,
        operation: &OperationId,
        credential: &Secret,
    ) -> Result<(), Refusal> {
        let (mut state, epoch) = self.lock_for_handoff()?;
        let scope = current_scope(&mut state).ok_or_else(handoff_refusal)?;
        if epoch != generation || &scope != expected || !valid_operation(operation) || !self.epoch_unchanged(epoch) {
            return Err(handoff_refusal());
        }
        let digest = scope_digest(scope.account().as_str(), scope.installation().as_str());
        set_secret(&mut state, &operation_key(&digest, operation, "settlement"), credential.expose_secret()).map_err(|_| handoff_refusal())?;
        if !self.epoch_unchanged(epoch) {
            return Err(handoff_refusal());
        }
        Ok(())
    }

    pub fn bind_scope(&self, scope: &AuthenticatedHostScope) -> Result<(), CredentialError> {
        if !scope.is_provable() {
            return Err(unavailable());
        }
        let account = scope.account().as_str();
        let installation = scope.installation().as_str();
        let binding = scope.session_binding().as_str();
        if !safe_token(account, MAX_ACCOUNT_BYTES)
            || !safe_token(installation, MAX_IDENTIFIER_BYTES)
            || !safe_token(binding, MAX_SESSION_BINDING_BYTES)
        {
            return Err(unavailable());
        }
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        if state.locked {
            return Err(unavailable());
        }
        let persisted_binding = state.store.get(SESSION_BINDING_KEY).map_err(|_| unavailable())?;
        if persisted_binding.as_deref().is_some_and(|persisted| persisted != binding) {
            return Err(unavailable());
        }
        if persisted_binding.is_none()
            && state.store.get(LOGIN_COMMIT_KEY).map_err(|_| unavailable())?.is_some_and(|marker| marker.starts_with("intcad-login/1/"))
        {
            // Only explicit login can replace a pre-binding activation.
            return Err(unavailable());
        }
        let persisted_account = state.store.get(ACCOUNT_KEY).map_err(|_| unavailable())?;
        let same_bound = state.account.as_deref() == Some(account);
        let same_persisted = persisted_account.as_deref() == Some(account);
        if persisted_binding.is_some() && !same_persisted {
            return Err(unavailable());
        }
        if !(same_bound && same_persisted) {
            // A missing bound account is a restart when the persisted label still matches; that
            // adopt path must not delete the settlement credentials needed for recovery.
            if !(state.account.is_none() && same_persisted) {
                clear_authority(&mut state)?;
            }
            state.account = Some(account.to_owned());
            if !same_persisted && let Err(error) = state.store.set(ACCOUNT_KEY, account) {
                return Err(store_failure(&mut state, error));
            }
        }
        if state.installation.as_deref() != Some(installation) {
            set_installation(&mut state, installation)?;
        }
        if persisted_binding.is_none() {
            write_binding(&mut state, SESSION_BINDING_KEY, binding)?;
        }
        Ok(())
    }

    /// Drop the old account's authority before writing the new account label. A failed delete
    /// locks the whole session rather than leaving old authority usable.
    pub fn switch_account(&self, account: &str) -> Result<(), CredentialError> {
        if !safe_token(account, MAX_ACCOUNT_BYTES) {
            return Err(unavailable());
        }
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        if state.locked {
            return Err(unavailable());
        }
        if state.account.as_deref() == Some(account) {
            return Ok(());
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
        // A switch replaces all old authority. Persist the same sign-out tombstone first so a
        // failed delete cannot leave the old bearer restorable by a later process.
        mark_logout(&mut state)?;
        clear_authority(&mut state)?;
        state.account = Some(account.to_owned());
        if let Err(error) = state.store.set(ACCOUNT_KEY, account) {
            return Err(store_failure(&mut state, error));
        }
        Ok(())
    }

    /// Write the durable tombstone without deleting credentials. Desktop sign-out instead
    /// writes it as part of background cleanup; accepting the request does not write it.
    pub fn mark_logout_pending(&self) -> Result<(), CredentialError> {
        let mut state = self.lock()?;
        mark_logout(&mut state)
    }

    /// Mark a durable sign-out before deleting anything. The marker is intentionally written
    /// first: if a delete fails, the next process sees it and refuses to restore the old bearer.
    /// A non-persistent headless session has no cross-process bearer to suppress, so its marker
    /// write is allowed to fail with `NoPersistentCredential` while it still clears memory.
    pub fn sign_out(&self) -> Result<(), CredentialError> {
        self.revoke_in_process();
        let result = self.clear_with_tombstone();
        if result.is_ok() {
            self.end_revocation();
        }
        result
    }

    /// The host worker variant: keep the in-process barrier set after a successful delete. The
    /// host clears it only after confirming this cleanup belongs to the latest revocation
    /// generation.
    pub fn sign_out_keeping_revocation(&self) -> Result<(), CredentialError> {
        self.revoke_in_process();
        self.clear_with_tombstone()
    }

    fn clear_with_tombstone(&self) -> Result<(), CredentialError> {
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        mark_logout(&mut state)?;
        clear_authority(&mut state)
    }

    /// Whether a previous sign-out marker is present. `Err` means safety cannot be confirmed and
    /// callers must fail closed.
    pub fn logout_pending(&self) -> Result<bool, CredentialError> {
        let state = self.lock()?;
        match state.store.get(LOGOUT_KEY) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(_) => Err(unavailable()),
        }
    }

    /// The WorkOS provider account, if a provider login completed. This is not an Intcad account.
    pub fn provider_account(&self) -> Option<String> {
        self.inner.lock().ok().filter(|state| !state.locked).and_then(|state| state.provider_account.clone())
    }

    /// Whether a trusted product exchange has activated an Intcad product session.
    pub fn product_session_active(&self) -> bool {
        WireCredentials::product_session(self).is_some()
    }

    /// Persist a completed WorkOS provider login. Old authority (product and provider) is deleted
    /// first; this never activates an Intcad product session and never persists a refresh token.
    pub fn complete_provider_login(&self, provider_account: &str, access_token: &str) -> Result<ProviderLogin, CredentialError> {
        if !safe_token(provider_account, MAX_ACCOUNT_BYTES) || access_token.is_empty() || has_control(access_token) {
            return Err(unavailable());
        }
        // A sign-out or account switch advances the epoch and sets the barrier before any store
        // IO. A worker that reaches completion afterwards must not reconstruct provider authority
        // that the cleanup just removed. Check before the lock and again while holding it.
        let epoch = self.epoch.load(Ordering::SeqCst);
        if self.revoked.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        if !self.epoch_unchanged(epoch) || state.locked {
            return Err(unavailable());
        }
        clear_authority(&mut state)?;
        let result = write_provider_login(&mut state, provider_account, access_token);
        if result.is_err() {
            let _ = clear_authority(&mut state);
        }
        result?;
        Ok(ProviderLogin { provider_account_id: provider_account.to_owned() })
    }

    /// Persist a provider login together with the product session a trusted exchange derived from
    /// it. This is the explicit flow for a live device-code login: verification happens off-thread
    /// before any credential is written, so a canceled or obsolete attempt leaves the store as it
    /// was. The caller must have verified the provider session with the configured product origin;
    /// this method does not exchange, choose an origin or add execution authority.
    /// Stage, then promote a verified provider/product login. `generation` is the caller's
    /// attempt id and `current` reports whether that attempt still owns the sign-in. Every value
    /// is written before the single commit marker; readers ignore all of them until that marker
    /// is present. Once the old commit is revoked, a crash, cancellation before the new marker,
    /// or failure staging the new session cannot leave a restorable half-session. A new verified
    /// attempt may retry after failure; it must start by revoking the old commit again. The caller
    /// must hold its promotion lock so cancel/sign-out cannot change the generation during writes.
    pub fn persist_verified_login(
        &self,
        provider: &ProviderSession,
        product: &ProductSession,
        generation: u64,
        current: &dyn Fn() -> bool,
    ) -> Result<ProductLogin, CredentialError> {
        self.persist_verified_login_with_installation(provider, product, None, generation, current)
    }

    /// Publish the reviewed installation pair before exposing the new login's commit marker.
    pub fn persist_verified_login_with_installation(
        &self,
        provider: &ProviderSession,
        product: &ProductSession,
        installation: Option<crate::http::InstallationBootstrap>,
        generation: u64,
        current: &dyn Fn() -> bool,
    ) -> Result<ProductLogin, CredentialError> {
        let staged = self.stage_verified_login_with_installation(provider, product, installation, generation, current)?;
        let result = self.commit_staged_login(&staged, current);
        if result.is_err() {
            let _ = self.abort_staged_login(staged);
        }
        result
    }

    pub fn stage_verified_login_with_installation(
        &self,
        provider: &ProviderSession,
        product: &ProductSession,
        installation: Option<crate::http::InstallationBootstrap>,
        generation: u64,
        current: &dyn Fn() -> bool,
    ) -> Result<StagedLogin, CredentialError> {
        if !safe_token(&provider.account_id, MAX_ACCOUNT_BYTES)
            || provider.access_token.is_empty()
            || has_control(provider.access_token.expose_secret())
        {
            return Err(unavailable());
        }
        let account = product.account_id().to_owned();
        if !safe_token(&account, MAX_ACCOUNT_BYTES) || product.product_session().is_empty() || has_control(product.product_session().expose_secret())
        {
            return Err(unavailable());
        }
        if let Some(expires_at) = product.expires_at_unix_seconds() {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
            if expires_at <= now {
                return Err(unavailable());
            }
        }

        let generation = u128::from(generation);
        // Attempt counters are local to a session. Independent hosts can stage
        // the same account with the same counter, so mint a fresh owner token.
        let mut nonce = [0u8; 16];
        rustls::crypto::ring::default_provider().secure_random.fill(&mut nonce).map_err(|_| unavailable())?;
        let binding = session_binding_digest(
            &account,
            installation.as_ref().map_or("", |installation| installation.installation_id()),
            provider.access_token.expose_secret(),
            u128::from_le_bytes(nonce),
        );
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        if !current() {
            return Err(attempt_superseded());
        }
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        state.locked = true;
        let result = (|| {
            clear_authority_inner(&mut state)?;
            stage_provider(&mut state, &provider.account_id, provider.access_token.expose_secret())?;
            stage_product(&mut state, &account, product.product_session().expose_secret())?;
            write_binding(&mut state, LOGIN_STAGE_BINDING_KEY, &binding)?;
            write_provider_login(&mut state, &provider.account_id, provider.access_token.expose_secret())?;
            write_product_login(&mut state, &account, product.product_session().expose_secret())?;
            if let Some(installation) = installation {
                let id = installation.installation_id().to_owned();
                let credential = installation.into_credential();
                // The bootstrap decoder checked this opaque value. Write it under this
                // transaction's lock; the ordinary handoff rejects all writes while locked.
                state.store.set(INSTALLATION_CREDENTIAL_KEY, credential.expose_secret()).map_err(|error| store_failure(&mut state, error))?;
                state.store.set(INSTALLATION_ID_KEY, &id).map_err(|error| store_failure(&mut state, error))?;
                state.installation = Some(id);
            }
            write_binding(&mut state, SESSION_BINDING_KEY, &binding)?;
            if !current() {
                return Err(attempt_superseded());
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(StagedLogin { account, binding, generation, epoch, owner: Arc::downgrade(&self.inner) }),
            Err(error) => {
                // This process stays closed even if the initial revocation write itself failed.
                state.locked = true;
                Err(error)
            }
        }
    }

    fn check_staged(&self, state: &SessionState, staged: &StagedLogin, aborting: bool) -> Result<(), CredentialError> {
        let marker = state.store.get(LOGIN_COMMIT_KEY)?;
        let ambiguous_commit = aborting && marker.as_deref() == Some(&format!("{LOGIN_COMMIT_DONE}/{}/{}", staged.generation, staged.account));
        if !staged.owner.ptr_eq(&Arc::downgrade(&self.inner))
            || staged.epoch != self.authority_generation()
            || !state.locked
            || (marker.as_deref() != Some(LOGIN_COMMIT_REVOKED) && !ambiguous_commit)
            || state.store.get(LOGIN_STAGE_BINDING_KEY)?.as_deref() != Some(&staged.binding)
        {
            return Err(attempt_superseded());
        }
        Ok(())
    }

    pub fn commit_staged_login(&self, staged: &StagedLogin, current: &dyn Fn() -> bool) -> Result<ProductLogin, CredentialError> {
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        self.check_staged(&state, staged, false)?;
        if !current() || self.is_revoked() {
            return Err(attempt_superseded());
        }
        commit_marker(&mut state, &staged.account, staged.generation)?;
        // Cleanup here is best-effort: a crash or failed delete after commit does
        // not invalidate a fully committed login.
        discard_stage(&mut state);
        state.locked = false;
        Ok(ProductLogin { account_id: staged.account.clone() })
    }

    pub fn abort_staged_login(&self, staged: StagedLogin) -> Result<(), CredentialError> {
        self.abort_staged_login_with(staged, || Ok(()))
    }

    pub(crate) fn abort_staged_login_with(
        &self,
        staged: StagedLogin,
        after_abort: impl FnOnce() -> Result<(), CredentialError>,
    ) -> Result<(), CredentialError> {
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        self.check_staged(&state, &staged, true)?;
        let result = (|| {
            mark_logout(&mut state)?;
            delete_login_credentials(&mut state)?;
            after_abort()?;
            state.store.delete(LOGOUT_KEY).map_err(|_| unavailable())
        })();
        if result.is_err() {
            self.revoke_in_process();
        }
        result
    }

    /// Confirm that the same staged login still owns the credential store.
    pub fn staged_login_is_current(&self, staged: &StagedLogin) -> bool {
        (|| {
            let _write = lock_product_authority_write()?;
            let state = self.lock()?;
            self.check_staged(&state, staged, false)
        })()
        .is_ok()
    }

    /// Detect a crash between credential staging and final commit. Residual stage
    /// keys after a successful commit need not retire the valid login.
    pub fn interrupted_login(&self) -> Result<bool, CredentialError> {
        let state = self.lock()?;
        Ok(state.store.get(LOGIN_STAGE_BINDING_KEY)?.is_some()
            && !state.store.get(LOGIN_COMMIT_KEY)?.is_some_and(|marker| marker.starts_with(&format!("{LOGIN_COMMIT_DONE}/"))))
    }

    /// Activate an Intcad product session only from a typed, trusted exchange result. The provider
    /// token is input to the exchange, never the product bearer. Without a documented exchange the
    /// default [`BlockedProductExchange`] leaves product login inactive.
    ///
    /// This is the older test/composition seam: it writes the same durable commit marker but has
    /// no desktop attempt generation or promotion gate. The shipped desktop uses
    /// [`Self::persist_verified_login`]; future production callers must use that staged path.
    pub fn activate_product_session(&self, exchange: &dyn ProductSessionExchange) -> Result<ProductLogin, CredentialError> {
        let epoch = self.epoch.load(Ordering::SeqCst);
        if self.revoked.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        let provider = {
            let state = self.inner.lock().map_err(|_| unavailable())?;
            if !self.epoch_unchanged(epoch) || !authority_usable(&state) {
                return Err(unavailable());
            }
            read_provider_session(&state).ok_or_else(provider_required)?
        };
        let product = exchange.exchange(&provider).map_err(product_exchange_error)?;
        let account = product.account_id().to_owned();
        if !safe_token(&account, MAX_ACCOUNT_BYTES) || product.product_session().is_empty() || has_control(product.product_session().expose_secret())
        {
            return Err(unavailable());
        }
        if let Some(expires_at) = product.expires_at_unix_seconds() {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
            if expires_at <= now {
                return Err(unavailable());
            }
        }

        let mut nonce = [0; 16];
        rustls::crypto::ring::default_provider().secure_random.fill(&mut nonce).map_err(|_| unavailable())?;
        let attempt = u128::from_be_bytes(nonce);
        let _write = lock_product_authority_write()?;
        let mut state = self.lock()?;
        // The exchange can run for a whole network round trip. A sign-out or account switch that
        // happened meanwhile must win: the barrier is checked before the provider recheck so no
        // product credential is written for a session that is being retired.
        if !self.epoch_unchanged(epoch) || !authority_usable(&state) {
            return Err(unavailable());
        }
        let current = read_provider_session(&state).ok_or_else(provider_required)?;
        if current.account_id != provider.account_id || current.access_token != provider.access_token {
            // A provider re-login or logout raced the exchange; fail closed and write nothing.
            return Err(unavailable());
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
        // The legacy seam has no caller generation or bootstrap pair. Mint a fresh attempt
        // even if the exact same provider and product credentials are reused after restart.
        let binding = session_binding_digest(&account, "", provider.access_token.expose_secret(), attempt);
        state.locked = true;
        let result = (|| {
            clear_product_authority_inner(&mut state)?;
            stage_product(&mut state, &account, product.product_session().expose_secret())?;
            write_binding(&mut state, LOGIN_STAGE_BINDING_KEY, &binding)?;
            write_product_login(&mut state, &account, product.product_session().expose_secret())?;
            write_binding(&mut state, SESSION_BINDING_KEY, &binding)?;
            commit_marker(&mut state, &account, attempt)?;
            discard_stage(&mut state);
            Ok(())
        })();
        match result {
            Ok(()) => {
                state.locked = false;
                Ok(ProductLogin { account_id: account })
            }
            Err(error) => {
                state.locked = true;
                Err(error)
            }
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, SessionState>, CredentialError> {
        self.inner.lock().map_err(|_| unavailable())
    }
}

impl fmt::Debug for CredentialSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (account, installation, provider_account, locked) = match self.inner.lock() {
            Ok(state) => (state.account.clone(), state.installation.clone(), state.provider_account.clone(), state.locked),
            Err(_) => (None, None, None, true),
        };
        formatter
            .debug_struct("CredentialSession")
            .field("account", &account)
            .field("installation", &installation)
            .field("provider_account", &provider_account)
            .field("locked", &locked)
            .field("credentials", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for CredentialSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialSession(<redacted>)")
    }
}

fn write_provider_login(state: &mut SessionState, account: &str, access_token: &str) -> Result<(), CredentialError> {
    if let Err(error) = state.store.set(PROVIDER_ACCOUNT_KEY, account) {
        return Err(store_failure(state, error));
    }
    if let Err(error) = state.store.set(PROVIDER_ACCESS_TOKEN_KEY, access_token) {
        return Err(store_failure(state, error));
    }
    state.provider_account = Some(account.to_owned());
    Ok(())
}

fn write_product_login(state: &mut SessionState, account: &str, product_session: &str) -> Result<(), CredentialError> {
    if let Err(error) = state.store.set(ACCOUNT_KEY, account) {
        return Err(store_failure(state, error));
    }
    if let Err(error) = state.store.set(PRODUCT_SESSION_KEY, product_session) {
        return Err(store_failure(state, error));
    }
    state.account = Some(account.to_owned());
    Ok(())
}

fn read_provider_session(state: &SessionState) -> Option<ProviderSession> {
    if state.locked {
        return None;
    }
    let account = state.store.get(PROVIDER_ACCOUNT_KEY).ok().flatten()?;
    let access_token = state.store.get(PROVIDER_ACCESS_TOKEN_KEY).ok().flatten()?;
    if !safe_token(&account, MAX_ACCOUNT_BYTES) || access_token.is_empty() || has_control(&access_token) {
        return None;
    }
    // Stored provider logins have no refresh key; the legacy constructor restores without one.
    Some(ProviderSession::from_authorized(account, Secret::new(access_token)))
}

fn provider_required() -> CredentialError {
    CredentialError { code: CredentialErrorCode::Unavailable, message: "a provider sign-in is required".into() }
}

fn attempt_superseded() -> CredentialError {
    CredentialError { code: CredentialErrorCode::Unavailable, message: "the sign-in attempt is no longer current".into() }
}

fn product_exchange_error(error: ProductExchangeError) -> CredentialError {
    let message = match error {
        ProductExchangeError::NotConfigured => "no documented product-session exchange is configured",
        ProductExchangeError::Rejected => "the product-session exchange was rejected",
        ProductExchangeError::Unavailable => "the product-session exchange is unavailable",
    };
    CredentialError { code: CredentialErrorCode::Unavailable, message: message.into() }
}

/// Delete every credential and scope label, in authority order. A failed clear locks the
/// session; successful cleanup preserves any pre-existing failure lock.
fn clear_authority(state: &mut SessionState) -> Result<(), CredentialError> {
    let was_locked = state.locked;
    state.locked = true;
    let result = clear_authority_inner(state);
    if result.is_ok() {
        state.locked = was_locked;
    }
    result
}

/// Persist the sign-out intent before any delete. An unavailable persistent store locks the
/// session; no cross-process suppression can be claimed in that state.
fn mark_logout(state: &mut SessionState) -> Result<(), CredentialError> {
    match state.store.set(LOGOUT_KEY, "sign_out") {
        Ok(()) => Ok(()),
        Err(error) if error.code == CredentialErrorCode::NoPersistentCredential => Ok(()),
        Err(error) => Err(store_failure(state, error)),
    }
}

fn logout_tombstone(state: &SessionState) -> Option<bool> {
    match state.store.get(LOGOUT_KEY) {
        Ok(Some(_)) => Some(true),
        Ok(None) => Some(false),
        Err(_) => None,
    }
}

/// The account an activation commit proves, when the marker, account label, product secret and
/// logout state are all present and consistent. No staged key is ever read here.
fn committed_account(state: &SessionState) -> Option<String> {
    if state.locked {
        return None;
    }
    let marker = state.store.get(LOGIN_COMMIT_KEY).ok().flatten()?;
    let rest = marker.strip_prefix(LOGIN_COMMIT_DONE)?.strip_prefix('/')?;
    let (generation, account) = rest.split_once('/')?;
    if !generation.bytes().all(|byte| byte.is_ascii_digit()) || !safe_token(account, MAX_ACCOUNT_BYTES) {
        return None;
    }
    let persisted = state.store.get(ACCOUNT_KEY).ok().flatten()?;
    if persisted != account {
        return None;
    }
    read_secret(state, PRODUCT_SESSION_KEY)?;
    read_session_binding(state)?;
    Some(account.to_owned())
}

fn activation_committed(state: &SessionState) -> bool {
    authority_usable(state) && committed_account(state).is_some()
}

/// The durable linearization point: this single-key write makes all staged/authoritative values
/// readable at once. Callers write every value before it and never read them before it.
fn commit_marker(state: &mut SessionState, account: &str, generation: u128) -> Result<(), CredentialError> {
    state.store.set(LOGIN_COMMIT_KEY, &format!("{LOGIN_COMMIT_DONE}/{generation}/{account}")).map_err(|error| store_failure(state, error))
}

fn stage_provider(state: &mut SessionState, account: &str, access_token: &str) -> Result<(), CredentialError> {
    if let Err(error) = state.store.set(LOGIN_STAGE_PROVIDER_ACCOUNT_KEY, account) {
        return Err(store_failure(state, error));
    }
    if let Err(error) = state.store.set(LOGIN_STAGE_PROVIDER_TOKEN_KEY, access_token) {
        return Err(store_failure(state, error));
    }
    Ok(())
}

fn stage_product(state: &mut SessionState, account: &str, product_session: &str) -> Result<(), CredentialError> {
    if let Err(error) = state.store.set(LOGIN_STAGE_ACCOUNT_KEY, account) {
        return Err(store_failure(state, error));
    }
    if let Err(error) = state.store.set(LOGIN_STAGE_PRODUCT_KEY, product_session) {
        return Err(store_failure(state, error));
    }
    Ok(())
}

const LOGIN_STAGE_KEYS: [&str; 5] =
    [LOGIN_STAGE_ACCOUNT_KEY, LOGIN_STAGE_BINDING_KEY, LOGIN_STAGE_PRODUCT_KEY, LOGIN_STAGE_PROVIDER_ACCOUNT_KEY, LOGIN_STAGE_PROVIDER_TOKEN_KEY];

fn discard_stage(state: &mut SessionState) {
    for key in LOGIN_STAGE_KEYS {
        let _ = state.store.delete(key);
    }
}

fn authority_usable(state: &SessionState) -> bool {
    !state.locked && logout_tombstone(state) == Some(false)
}

fn clear_product_authority_inner(state: &mut SessionState) -> Result<(), CredentialError> {
    // Revoke the commit first: a crash during the deletes leaves every product reader refusing
    // the old values, rather than restoring a half-cleared session. The product bearer is the
    // first delete so the highest-authority credential is gone before anything else.
    if let Err(error) = state.store.set(LOGIN_COMMIT_KEY, LOGIN_COMMIT_REVOKED) {
        return Err(store_failure(state, error));
    }
    for key in [PRODUCT_SESSION_KEY, SESSION_BINDING_KEY, REFRESH_TOKEN_KEY, INSTALLATION_CREDENTIAL_KEY] {
        state.store.delete(key).map_err(|_| unavailable())?;
    }
    for key in LOGIN_STAGE_KEYS {
        state.store.delete(key).map_err(|_| unavailable())?;
    }
    let tracked = load_index(state)?;
    for key in tracked {
        state.store.delete(&key).map_err(|_| unavailable())?;
    }
    for key in [INSTALLATION_ID_KEY, ACCOUNT_KEY, INDEX_KEY] {
        state.store.delete(key).map_err(|_| unavailable())?;
    }
    state.index.clear();
    state.account = None;
    state.installation = None;
    Ok(())
}

fn clear_authority_inner(state: &mut SessionState) -> Result<(), CredentialError> {
    delete_login_credentials(state)?;
    // The previous provider and product authority must both be gone before clearing recovery.
    state.store.delete(LOGOUT_KEY).map_err(|_| unavailable())?;
    Ok(())
}

fn delete_login_credentials(state: &mut SessionState) -> Result<(), CredentialError> {
    clear_product_authority_inner(state)?;
    for key in [PROVIDER_ACCESS_TOKEN_KEY, PROVIDER_ACCOUNT_KEY] {
        state.store.delete(key).map_err(|_| unavailable())?;
    }
    state.provider_account = None;
    Ok(())
}

fn set_installation(state: &mut SessionState, installation: &str) -> Result<(), CredentialError> {
    if state.locked {
        return Err(unavailable());
    }
    let persisted = state.store.get(INSTALLATION_ID_KEY).map_err(|_| unavailable())?;
    let previous = state.installation.clone();
    match previous.as_deref() {
        Some(current) if current == installation => {
            if persisted.as_deref() != Some(installation) {
                state.installation = Some(installation.to_owned());
                if let Err(error) = state.store.set(INSTALLATION_ID_KEY, installation) {
                    return Err(store_failure(state, error));
                }
            }
            return Ok(());
        }
        Some(current) => {
            if let Err(error) = state.store.delete(INSTALLATION_CREDENTIAL_KEY) {
                return Err(store_failure(state, error));
            }
            clear_dynamic_scope(state, current)?;
        }
        None => {
            if persisted.as_deref() != Some(installation) {
                if let Err(error) = state.store.delete(INSTALLATION_CREDENTIAL_KEY) {
                    return Err(store_failure(state, error));
                }
                clear_dynamic_prefix(state, "scope/")?;
            }
        }
    }
    state.installation = Some(installation.to_owned());
    if persisted.as_deref() != Some(installation)
        && let Err(error) = state.store.set(INSTALLATION_ID_KEY, installation)
    {
        return Err(store_failure(state, error));
    }
    Ok(())
}

fn clear_dynamic_scope(state: &mut SessionState, previous_installation: &str) -> Result<(), CredentialError> {
    let Some(account) = state.account.clone() else {
        return clear_dynamic_prefix(state, "scope/");
    };
    let prefix = format!("scope/{}/", scope_digest(&account, previous_installation));
    clear_dynamic_prefix(state, &prefix)
}

fn clear_dynamic_prefix(state: &mut SessionState, prefix: &str) -> Result<(), CredentialError> {
    let tracked = load_index(state)?;
    let mut removed = false;
    for key in tracked {
        if key.starts_with(prefix) {
            if let Err(error) = state.store.delete(&key) {
                return Err(store_failure(state, error));
            }
            state.index.remove(&key);
            removed = true;
        }
    }
    if removed {
        write_index(state)?;
    }
    Ok(())
}

fn load_index(state: &mut SessionState) -> Result<BTreeSet<String>, CredentialError> {
    let mut keys = state.index.clone();
    if let Some(raw) = state.store.get(INDEX_KEY).map_err(|_| unavailable())? {
        if raw.len() > MAX_INDEX_BYTES {
            return Err(unavailable());
        }
        for line in raw.split('\n').filter(|line| !line.is_empty()) {
            if !valid_stored_key(line) {
                return Err(unavailable());
            }
            keys.insert(line.to_owned());
        }
    }
    if keys.len() > MAX_TRACKED_KEYS {
        return Err(unavailable());
    }
    // Keep the loaded keys so the next index write does not silently forget a credential left by
    // an earlier process.
    state.index = keys.clone();
    Ok(keys)
}

fn valid_stored_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_BYTES
        && !has_control(key)
        && (key.starts_with("scope/") || key.starts_with("auth/") || key.starts_with("install/") || key.starts_with("account/"))
}

fn remember_key(state: &mut SessionState, key: &str) -> Result<(), CredentialError> {
    if !valid_stored_key(key) || key.len() > MAX_KEY_BYTES {
        return Err(unavailable());
    }
    if !load_index(state)?.contains(key) {
        state.index.insert(key.to_owned());
        if state.index.len() > MAX_TRACKED_KEYS {
            state.index.remove(key);
            return Err(unavailable());
        }
        if let Err(error) = write_index(state) {
            state.index.remove(key);
            return Err(error);
        }
    }
    Ok(())
}

fn write_index(state: &mut SessionState) -> Result<(), CredentialError> {
    let joined = state.index.iter().cloned().collect::<Vec<_>>().join("\n");
    let result = if joined.is_empty() { state.store.delete(INDEX_KEY) } else { state.store.set(INDEX_KEY, &joined) };
    result.map_err(|error| store_failure(state, error))
}

fn set_secret(state: &mut SessionState, key: &str, secret: &str) -> Result<(), CredentialError> {
    if state.locked || secret.is_empty() || has_control(secret) {
        return Err(unavailable());
    }
    remember_key(state, key)?;
    if let Err(error) = state.store.set(key, secret) {
        return Err(store_failure(state, error));
    }
    Ok(())
}

fn read_secret(state: &SessionState, key: &str) -> Option<Secret> {
    match state.store.get(key) {
        Ok(Some(value)) if !value.is_empty() && !has_control(&value) => Some(Secret::new(value)),
        _ => None,
    }
}

/// Adopt the account label a previous process persisted under the active product session. This
/// does not require an installation identity, so a restart can complete a missing first install.
fn adopt_persisted_account(state: &mut SessionState) -> Option<()> {
    state.account = Some(committed_account(state)?);
    Some(())
}

/// Best-effort removal of a first-install pair that a revocation raced. The caller returns a
/// refusal either way; a failed delete locks the session so no half-binding becomes readable.
fn remove_first_installation(state: &mut SessionState) {
    if state.store.delete(INSTALLATION_ID_KEY).is_err() {
        state.locked = true;
    }
    if state.store.delete(INSTALLATION_CREDENTIAL_KEY).is_err() {
        state.locked = true;
    }
    state.installation = None;
}

/// Adopt the account and installation labels a previous process persisted under the active
/// product session. This is a restart path only: nothing is written and a provider-only session
/// has no product secret, so it cannot synthesize a scope.
fn adopt_persisted_labels(state: &mut SessionState) -> Option<()> {
    let account = committed_account(state)?;
    let installation = state.store.get(INSTALLATION_ID_KEY).ok().flatten()?;
    if !safe_token(&installation, MAX_IDENTIFIER_BYTES) {
        return None;
    }
    state.account = Some(account);
    state.installation = Some(installation);
    Some(())
}

fn bound_scope(state: &SessionState) -> Option<(String, String, String)> {
    if !activation_committed(state) {
        return None;
    }
    let account = state.account.clone()?;
    let installation = state.installation.clone()?;
    if !safe_token(&account, MAX_ACCOUNT_BYTES) || !safe_token(&installation, MAX_IDENTIFIER_BYTES) {
        return None;
    }
    if state.store.get(ACCOUNT_KEY).ok().flatten().as_deref() != Some(account.as_str()) {
        return None;
    }
    if state.store.get(INSTALLATION_ID_KEY).ok().flatten().as_deref() != Some(installation.as_str()) {
        return None;
    }
    let digest = scope_digest(&account, &installation);
    Some((account, installation, digest))
}

fn current_scope(state: &mut SessionState) -> Option<AuthenticatedHostScope> {
    if state.account.is_none() || state.installation.is_none() {
        adopt_persisted_labels(state)?;
    }
    let (account, installation, _) = bound_scope(state)?;
    let binding = read_session_binding(state)?;
    Some(AuthenticatedHostScope::new(AccountId::new(account), InstallationId::new(installation), HostSessionBinding::new(binding)))
}

fn read_session_binding(state: &SessionState) -> Option<String> {
    state.store.get(SESSION_BINDING_KEY).ok().flatten().filter(|binding| safe_token(binding, MAX_SESSION_BINDING_BYTES))
}

fn write_binding(state: &mut SessionState, key: &str, binding: &str) -> Result<(), CredentialError> {
    if !safe_token(binding, MAX_SESSION_BINDING_BYTES) {
        return Err(unavailable());
    }
    state.store.set(key, binding).map_err(|error| store_failure(state, error))
}

fn session_binding_digest(account: &str, installation: &str, login_secret: &str, generation: u128) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account.as_bytes());
    hasher.update([0]);
    hasher.update(installation.as_bytes());
    hasher.update([0]);
    hasher.update(login_secret.as_bytes());
    hasher.update([0]);
    hasher.update(generation.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn scope_digest(account: &str, installation: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account.as_bytes());
    hasher.update([0]);
    hasher.update(installation.as_bytes());
    let digest = hasher.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

fn operation_key(digest: &str, operation: &OperationId, suffix: &str) -> String {
    format!("scope/{digest}/operation/{}/{suffix}", operation.as_str())
}

fn lease_key(digest: &str, authorization: &GrantId) -> String {
    format!("scope/{digest}/lease/{}/proof", authorization.as_str())
}

fn valid_operation(operation: &OperationId) -> bool {
    safe_token(operation.as_str(), 72)
}

impl WireCredentials for CredentialSession {
    fn product_session(&self) -> Option<Secret> {
        let (state, epoch) = self.lock_if_authorized()?;
        if !activation_committed(&state) {
            return None;
        }
        let account = state.account.as_deref()?;
        if !safe_token(account, MAX_ACCOUNT_BYTES) {
            return None;
        }
        if state.store.get(ACCOUNT_KEY).ok().flatten().as_deref() != Some(account) {
            return None;
        }
        let secret = read_secret(&state, PRODUCT_SESSION_KEY);
        self.epoch_unchanged(epoch).then_some(secret).flatten()
    }

    fn installation_bound(&self) -> bool {
        let Some((mut state, epoch)) = self.lock_if_authorized() else {
            return false;
        };
        if state.account.is_none() || state.installation.is_none() {
            let Some(()) = adopt_persisted_labels(&mut state) else {
                return false;
            };
        }
        let Some(installation) = state.installation.as_deref() else {
            return false;
        };
        let persisted = state.store.get(INSTALLATION_ID_KEY).ok().flatten();
        persisted.as_deref() == Some(installation) && self.epoch_unchanged(epoch)
    }

    fn installation_credential(&self) -> Option<Secret> {
        let (mut state, epoch) = self.lock_if_authorized()?;
        if state.account.is_none() || state.installation.is_none() {
            adopt_persisted_labels(&mut state)?;
        }
        if !activation_committed(&state) {
            return None;
        }
        let account = state.account.as_deref()?;
        let installation = state.installation.as_deref()?;
        if !safe_token(account, MAX_ACCOUNT_BYTES) || !safe_token(installation, MAX_IDENTIFIER_BYTES) {
            return None;
        }
        if state.store.get(ACCOUNT_KEY).ok().flatten().as_deref() != Some(account) {
            return None;
        }
        if state.store.get(INSTALLATION_ID_KEY).ok().flatten().as_deref() != Some(installation) {
            return None;
        }
        let secret = read_secret(&state, INSTALLATION_CREDENTIAL_KEY);
        self.epoch_unchanged(epoch).then_some(secret).flatten()
    }

    fn settlement_credential(&self, operation: &OperationId) -> Option<Secret> {
        let (state, epoch) = self.lock_if_authorized()?;
        let (_, _, digest) = bound_scope(&state)?;
        if !valid_operation(operation) {
            return None;
        }
        let secret = read_secret(&state, &operation_key(&digest, operation, "settlement"));
        self.epoch_unchanged(epoch).then_some(secret).flatten()
    }

    fn execution_session_proof(&self, operation: &OperationId) -> Option<Secret> {
        let (state, epoch) = self.lock_if_authorized()?;
        let (_, _, digest) = bound_scope(&state)?;
        if !valid_operation(operation) {
            return None;
        }
        let secret = read_secret(&state, &operation_key(&digest, operation, "proof"));
        self.epoch_unchanged(epoch).then_some(secret).flatten()
    }

    fn initial_lease_proof(&self, authorization: &GrantId) -> Option<Secret> {
        let (state, epoch) = self.lock_if_authorized()?;
        let (_, _, digest) = bound_scope(&state)?;
        if !safe_token(authorization.as_str(), MAX_IDENTIFIER_BYTES) {
            return None;
        }
        let secret = read_secret(&state, &lease_key(&digest, authorization));
        self.epoch_unchanged(epoch).then_some(secret).flatten()
    }
}

impl CredentialHandoff for CredentialSession {
    fn renewal_unauthorized(&mut self, scope: &AuthenticatedHostScope, operation: &OperationId) {
        if let Ok(mut refusals) = self.refused_renewals.lock() {
            refusals.insert((scope.clone(), operation.clone()));
        }
    }

    fn initial_lease_proof(&mut self, authorization: &GrantId, _capability: Capability, proof: Secret) -> Result<(), Refusal> {
        let (mut state, epoch) = self.lock_for_handoff()?;
        let (_, _, digest) = bound_scope(&state).ok_or_else(handoff_refusal)?;
        if !safe_token(authorization.as_str(), MAX_IDENTIFIER_BYTES) {
            return Err(handoff_refusal());
        }
        let result = set_secret(&mut state, &lease_key(&digest, authorization), proof.expose_secret()).map_err(|_| handoff_refusal());
        if result.is_ok() && !self.epoch_unchanged(epoch) {
            return Err(handoff_refusal());
        }
        result
    }

    fn execution_session_proof(&mut self, _capability: Capability, operation: &OperationId, proof: Secret) -> Result<(), Refusal> {
        let (mut state, epoch) = self.lock_for_handoff()?;
        let (_, _, digest) = bound_scope(&state).ok_or_else(handoff_refusal)?;
        if !valid_operation(operation) {
            return Err(handoff_refusal());
        }
        let result = set_secret(&mut state, &operation_key(&digest, operation, "proof"), proof.expose_secret()).map_err(|_| handoff_refusal());
        if result.is_ok() && !self.epoch_unchanged(epoch) {
            return Err(handoff_refusal());
        }
        result
    }

    fn settlement_credential(&mut self, operation: &OperationId, credential: Secret) -> Result<(), Refusal> {
        let (mut state, epoch) = self.lock_for_handoff()?;
        let (_, _, digest) = bound_scope(&state).ok_or_else(handoff_refusal)?;
        if !valid_operation(operation) {
            return Err(handoff_refusal());
        }
        let result =
            set_secret(&mut state, &operation_key(&digest, operation, "settlement"), credential.expose_secret()).map_err(|_| handoff_refusal());
        if result.is_ok() && !self.epoch_unchanged(epoch) {
            return Err(handoff_refusal());
        }
        result
    }

    fn installation_credential(&mut self, installation_id: &str, credential: Secret) -> Result<(), Refusal> {
        let (mut state, epoch) = self.lock_for_handoff()?;
        if state.locked || state.account.is_none() {
            return Err(handoff_refusal());
        }
        if !safe_token(installation_id, MAX_IDENTIFIER_BYTES) {
            return Err(handoff_refusal());
        }
        let persisted = state.store.get(INSTALLATION_ID_KEY).map_err(|_| handoff_refusal())?;
        match state.installation.as_deref() {
            Some(current) if current != installation_id => return Err(handoff_refusal()),
            Some(_) => {}
            None => {
                if persisted.as_deref() != Some(installation_id) {
                    clear_dynamic_prefix(&mut state, "scope/").map_err(|_| handoff_refusal())?;
                }
                state.installation = Some(installation_id.to_owned());
                if persisted.as_deref() != Some(installation_id)
                    && let Err(error) = state.store.set(INSTALLATION_ID_KEY, installation_id)
                {
                    let _ = store_failure(&mut state, error);
                    return Err(handoff_refusal());
                }
            }
        }
        if let Err(error) = state.store.delete(INSTALLATION_CREDENTIAL_KEY) {
            let _ = store_failure(&mut state, error);
            return Err(handoff_refusal());
        }
        let result = set_secret(&mut state, INSTALLATION_CREDENTIAL_KEY, credential.expose_secret()).map_err(|_| handoff_refusal());
        if result.is_ok() && !self.epoch_unchanged(epoch) {
            return Err(handoff_refusal());
        }
        result
    }

    fn first_installation_credential(&mut self, expected_product_session: &Secret, installation_id: &str, credential: Secret) -> Result<(), Refusal> {
        let (mut state, epoch) = self.lock_for_handoff()?;
        adopt_persisted_account(&mut state).ok_or_else(handoff_refusal)?;
        let account = state.account.as_deref().ok_or_else(handoff_refusal)?.to_owned();
        if state.store.get(ACCOUNT_KEY).map_err(|_| handoff_refusal())?.as_deref() != Some(account.as_str()) {
            return Err(handoff_refusal());
        }
        let active_product = read_secret(&state, PRODUCT_SESSION_KEY).ok_or_else(handoff_refusal)?;
        if &active_product != expected_product_session {
            return Err(handoff_refusal());
        }
        if !safe_token(installation_id, MAX_IDENTIFIER_BYTES) {
            return Err(handoff_refusal());
        }
        if state.installation.is_some() || state.store.get(INSTALLATION_ID_KEY).map_err(|_| handoff_refusal())?.is_some() {
            return Err(handoff_refusal());
        }
        // Publish the credential first and the identity last. The id is the marker that makes
        // the pair readable, so a crash between the two writes leaves an unreadable orphan
        // instead of an identity without its credential. Any scope keys left without an
        // identity are unreachable too; drop them before the new binding is published.
        clear_dynamic_prefix(&mut state, "scope/").map_err(|_| handoff_refusal())?;
        if let Err(error) = state.store.delete(INSTALLATION_CREDENTIAL_KEY) {
            let _ = store_failure(&mut state, error);
            return Err(handoff_refusal());
        }
        if set_secret(&mut state, INSTALLATION_CREDENTIAL_KEY, credential.expose_secret()).is_err() {
            return Err(handoff_refusal());
        }
        if !self.epoch_unchanged(epoch) {
            remove_first_installation(&mut state);
            return Err(handoff_refusal());
        }
        if let Err(error) = state.store.set(INSTALLATION_ID_KEY, installation_id) {
            let _ = state.store.delete(INSTALLATION_CREDENTIAL_KEY);
            let _ = store_failure(&mut state, error);
            return Err(handoff_refusal());
        }
        state.installation = Some(installation_id.to_owned());
        if !self.epoch_unchanged(epoch) {
            remove_first_installation(&mut state);
            return Err(handoff_refusal());
        }
        Ok(())
    }
}
