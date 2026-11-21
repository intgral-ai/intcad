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
//! activate `auth/product_session`; the default [`BlockedProductExchange`] keeps the live exchange
//! path absent until a documented protocol is reviewed.

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

const INDEX_KEY: &str = "credentials/index";
const ACCOUNT_KEY: &str = "account/id";
pub(crate) const PRODUCT_SESSION_KEY: &str = "auth/product_session";
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
const KEYRING_PROBE_KEY: &str = "intcad/keyring-probe";

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
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(unavailable()),
        }
    }

    fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
        self.entry(key)?.set_password(secret).map_err(|_| unavailable())
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

/// A completed provider login. It proves only the WorkOS provider session; it is not an Intcad
/// product login and cannot be used as a product bearer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderLogin {
    pub provider_account_id: String,
}

/// The typed provider credentials offered to the explicit product-session exchange. The access
/// token is available to the trusted exchange implementation only; it is never returned by
/// [`WireCredentials`].
#[derive(Clone)]
pub struct ProviderSession {
    account_id: String,
    access_token: Secret,
}

impl ProviderSession {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn access_token(&self) -> &Secret {
        &self.access_token
    }
}

impl fmt::Debug for ProviderSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("ProviderSession").field("account_id", &self.account_id).field("access_token", &"<redacted>").finish()
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
/// reviewed exchange; this crate ships only [`BlockedProductExchange`] and scripted test doubles.
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
    /// Set after a failed delete or a failed durable write; every read and write then fails closed.
    locked: bool,
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
        }
    }

    /// Fail all credential and scope reads synchronously. This never takes the session lock, so a
    /// caller can revoke on the window thread while a worker holds the lock for keyring IO.
    /// The epoch advances first, so a reader already waiting on that lock cannot mistake a
    /// later barrier clear for permission to return an old credential.
    pub fn revoke_in_process(&self) {
        // The barrier closes first so a new reader cannot capture the new epoch and slip
        // through before the boolean is published; the epoch then invalidates readers that
        // passed the pre-check before this call.
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
        state.account.clone()
    }

    pub fn is_persistent(&self) -> bool {
        self.inner.lock().map(|state| state.store.persistent()).unwrap_or(false)
    }

    /// The host's provable account, installation and session binding, or `None`.
    ///
    /// The binding is an opaque digest of the active product session and the persisted account and
    /// installation. A provider-only login never produces a scope, and a missing persisted label
    /// fails closed. Logout, a product-session exchange and switching accounts each change (or
    /// remove) the binding, so a result admitted by an old session cannot commit into a new one.
    pub fn authenticated_scope(&self) -> Option<AuthenticatedHostScope> {
        let (mut state, epoch) = self.lock_if_authorized()?;
        if state.account.is_none() || state.installation.is_none() {
            adopt_persisted_labels(&mut state)?;
        }
        let (account, installation, _) = bound_scope(&state)?;
        let product = read_secret(&state, PRODUCT_SESSION_KEY)?;
        if !self.epoch_unchanged(epoch) {
            return None;
        }
        let binding = session_binding_digest(&account, &installation, product.expose_secret());
        Some(AuthenticatedHostScope::new(AccountId::new(account), InstallationId::new(installation), HostSessionBinding::new(binding)))
    }

    pub fn bind_scope(&self, scope: &AuthenticatedHostScope) -> Result<(), CredentialError> {
        if !scope.is_provable() {
            return Err(unavailable());
        }
        let account = scope.account().as_str();
        let installation = scope.installation().as_str();
        if !safe_token(account, MAX_ACCOUNT_BYTES) || !safe_token(installation, MAX_IDENTIFIER_BYTES) {
            return Err(unavailable());
        }
        let mut state = self.lock()?;
        if state.locked {
            return Err(unavailable());
        }
        let persisted_account = state.store.get(ACCOUNT_KEY).map_err(|_| unavailable())?;
        let same_bound = state.account.as_deref() == Some(account);
        let same_persisted = persisted_account.as_deref() == Some(account);
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
        Ok(())
    }

    /// Drop the old account's authority before writing the new account label. A failed delete
    /// locks the whole session rather than leaving old authority usable.
    pub fn switch_account(&self, account: &str) -> Result<(), CredentialError> {
        if !safe_token(account, MAX_ACCOUNT_BYTES) {
            return Err(unavailable());
        }
        let mut state = self.lock()?;
        if state.account.as_deref() == Some(account) && !state.locked {
            return Ok(());
        }
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
        let mut state = self.lock()?;
        clear_authority(&mut state)?;
        let result = write_provider_login(&mut state, provider_account, access_token);
        if result.is_err() {
            let _ = clear_authority(&mut state);
        }
        result?;
        Ok(ProviderLogin { provider_account_id: provider_account.to_owned() })
    }

    /// Activate an Intcad product session only from a typed, trusted exchange result. The provider
    /// token is input to the exchange, never the product bearer. Without a documented exchange the
    /// default [`BlockedProductExchange`] leaves product login inactive.
    pub fn activate_product_session(&self, exchange: &dyn ProductSessionExchange) -> Result<ProductLogin, CredentialError> {
        let provider = {
            let state = self.inner.lock().map_err(|_| unavailable())?;
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

        let mut state = self.lock()?;
        let current = read_provider_session(&state).ok_or_else(provider_required)?;
        if current.account_id != provider.account_id || current.access_token != provider.access_token {
            // A provider re-login or logout raced the exchange; fail closed and write nothing.
            return Err(unavailable());
        }
        clear_product_authority(&mut state)?;
        let result = write_product_login(&mut state, &account, product.product_session().expose_secret());
        if result.is_err() {
            let _ = clear_product_authority(&mut state);
        }
        result?;
        Ok(ProductLogin { account_id: account })
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
    Some(ProviderSession { account_id: account, access_token: Secret::new(access_token) })
}

fn provider_required() -> CredentialError {
    CredentialError { code: CredentialErrorCode::Unavailable, message: "a provider sign-in is required".into() }
}

fn product_exchange_error(error: ProductExchangeError) -> CredentialError {
    let message = match error {
        ProductExchangeError::NotConfigured => "no documented product-session exchange is configured",
        ProductExchangeError::Rejected => "the product-session exchange was rejected",
        ProductExchangeError::Unavailable => "the product-session exchange is unavailable",
    };
    CredentialError { code: CredentialErrorCode::Unavailable, message: message.into() }
}

/// Delete every credential and scope label, in authority order. The account/session credential is
/// removed first; only a fully successful clear unlocks the session.
fn clear_authority(state: &mut SessionState) -> Result<(), CredentialError> {
    state.locked = true;
    let result = clear_authority_inner(state);
    if result.is_ok() {
        state.locked = false;
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

fn authority_usable(state: &SessionState) -> bool {
    !state.locked && logout_tombstone(state) == Some(false)
}

/// Delete only the Intcad product authority; the provider login is kept for the exchange.
fn clear_product_authority(state: &mut SessionState) -> Result<(), CredentialError> {
    state.locked = true;
    let result = clear_product_authority_inner(state);
    if result.is_ok() {
        state.locked = false;
    }
    result
}

fn clear_product_authority_inner(state: &mut SessionState) -> Result<(), CredentialError> {
    for key in [PRODUCT_SESSION_KEY, REFRESH_TOKEN_KEY, INSTALLATION_CREDENTIAL_KEY] {
        state.store.delete(key).map_err(|_| unavailable())?;
    }
    let tracked = load_index(state)?;
    for key in tracked {
        state.store.delete(&key).map_err(|_| unavailable())?;
    }
    for key in [INSTALLATION_ID_KEY, ACCOUNT_KEY, INDEX_KEY] {
        state.store.delete(key).map_err(|_| unavailable())?;
    }
    // The old authority is fully gone; only now may the sign-out marker disappear. A failed
    // marker delete leaves it present, which is conservative: startup fails closed.
    state.store.delete(LOGOUT_KEY).map_err(|_| unavailable())?;
    state.index.clear();
    state.account = None;
    state.installation = None;
    Ok(())
}

fn clear_authority_inner(state: &mut SessionState) -> Result<(), CredentialError> {
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

/// Adopt the account and installation labels a previous process persisted under the active
/// product session. This is a restart path only: nothing is written and a provider-only session
/// has no product secret, so it cannot synthesize a scope.
fn adopt_persisted_labels(state: &mut SessionState) -> Option<()> {
    let account = state.store.get(ACCOUNT_KEY).ok().flatten()?;
    let installation = state.store.get(INSTALLATION_ID_KEY).ok().flatten()?;
    if !safe_token(&account, MAX_ACCOUNT_BYTES) || !safe_token(&installation, MAX_IDENTIFIER_BYTES) {
        return None;
    }
    read_secret(state, PRODUCT_SESSION_KEY)?;
    state.account = Some(account);
    state.installation = Some(installation);
    Some(())
}

fn bound_scope(state: &SessionState) -> Option<(String, String, String)> {
    if !authority_usable(state) {
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

/// An opaque binding for the exact active product session. The product secret is never logged:
/// the digest only distinguishes sessions that share an account and installation.
fn session_binding_digest(account: &str, installation: &str, product_session: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account.as_bytes());
    hasher.update([0]);
    hasher.update(installation.as_bytes());
    hasher.update([0]);
    hasher.update(product_session.as_bytes());
    let digest = hasher.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
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

    fn installation_credential(&self) -> Option<Secret> {
        let (state, epoch) = self.lock_if_authorized()?;
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
}
