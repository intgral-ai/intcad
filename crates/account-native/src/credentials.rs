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

use sha2::{Digest, Sha256};
use solidcraft_engine::entitlement::{
    AuthenticatedHostScope, Capability, CredentialError, CredentialErrorCode, CredentialStore, GrantId, OperationId, Refusal, RefusalCode,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

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
const REFRESH_TOKEN_KEY: &str = "auth/refresh_token";
const INSTALLATION_ID_KEY: &str = "install/id";
const INSTALLATION_CREDENTIAL_KEY: &str = "install/credential";
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

/// The single credential owner shared by [`WireCredentials`] and [`CredentialHandoff`].
///
/// `account/…`, `auth/…`, `install/…`, and `scope/…` keys are deliberately separate. Per-operation
/// proofs and settlement credentials are scoped by the authenticated account and installation; an
/// initial lease proof is additionally keyed by the server-issued authorization id. Clones share
/// one store and one lock, so the HTTP adapter can hold both trait objects without a no-op handoff.
#[derive(Clone)]
pub struct CredentialSession {
    inner: Arc<Mutex<SessionState>>,
}

struct SessionState {
    store: Box<dyn CredentialStore + Send>,
    index: BTreeSet<String>,
    account: Option<String>,
    installation: Option<String>,
    /// Set after a failed delete or a failed durable write; every read and write then fails closed.
    locked: bool,
}

impl CredentialSession {
    pub fn new(store: Box<dyn CredentialStore + Send>) -> Self {
        Self { inner: Arc::new(Mutex::new(SessionState { store, index: BTreeSet::new(), account: None, installation: None, locked: false })) }
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
        self.inner.lock().ok().filter(|state| !state.locked).and_then(|state| state.account.clone())
    }

    pub fn is_persistent(&self) -> bool {
        self.inner.lock().map(|state| state.store.persistent()).unwrap_or(false)
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
        clear_authority(&mut state)?;
        state.account = Some(account.to_owned());
        if let Err(error) = state.store.set(ACCOUNT_KEY, account) {
            return Err(store_failure(&mut state, error));
        }
        Ok(())
    }

    pub fn sign_out(&self) -> Result<(), CredentialError> {
        let mut state = self.lock()?;
        clear_authority(&mut state)
    }

    /// Persist a completed device login. Old authority is deleted first; a failed write clears
    /// whatever this call wrote and fails closed.
    pub fn complete_device_login(&self, account: &str, access_token: &str, refresh_token: Option<&str>) -> Result<(), CredentialError> {
        if !safe_token(account, MAX_ACCOUNT_BYTES)
            || access_token.is_empty()
            || has_control(access_token)
            || refresh_token.is_some_and(|token| token.is_empty() || has_control(token))
        {
            return Err(unavailable());
        }
        let mut state = self.lock()?;
        clear_authority(&mut state)?;
        state.account = Some(account.to_owned());
        let result = write_login(&mut state, account, access_token, refresh_token);
        if result.is_err() {
            let _ = clear_authority(&mut state);
        }
        result
    }

    fn lock(&self) -> Result<MutexGuard<'_, SessionState>, CredentialError> {
        self.inner.lock().map_err(|_| unavailable())
    }
}

impl fmt::Debug for CredentialSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (account, installation, locked) = match self.inner.lock() {
            Ok(state) => (state.account.clone(), state.installation.clone(), state.locked),
            Err(_) => (None, None, true),
        };
        formatter
            .debug_struct("CredentialSession")
            .field("account", &account)
            .field("installation", &installation)
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

fn write_login(state: &mut SessionState, account: &str, access_token: &str, refresh_token: Option<&str>) -> Result<(), CredentialError> {
    if let Err(error) = state.store.set(ACCOUNT_KEY, account) {
        return Err(store_failure(state, error));
    }
    set_secret(state, PRODUCT_SESSION_KEY, access_token)?;
    if let Some(refresh_token) = refresh_token {
        set_secret(state, REFRESH_TOKEN_KEY, refresh_token)?;
    }
    Ok(())
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

fn clear_authority_inner(state: &mut SessionState) -> Result<(), CredentialError> {
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
    state.index.clear();
    state.account = None;
    state.installation = None;
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

fn bound_scope(state: &SessionState) -> Option<(String, String, String)> {
    if state.locked {
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
        let state = self.inner.lock().ok()?;
        if state.locked {
            return None;
        }
        let account = state.account.as_deref()?;
        if !safe_token(account, MAX_ACCOUNT_BYTES) {
            return None;
        }
        if state.store.get(ACCOUNT_KEY).ok().flatten().as_deref() != Some(account) {
            return None;
        }
        read_secret(&state, PRODUCT_SESSION_KEY)
    }

    fn installation_credential(&self) -> Option<Secret> {
        let state = self.inner.lock().ok()?;
        if state.locked {
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
        read_secret(&state, INSTALLATION_CREDENTIAL_KEY)
    }

    fn settlement_credential(&self, operation: &OperationId) -> Option<Secret> {
        let state = self.inner.lock().ok()?;
        let (_, _, digest) = bound_scope(&state)?;
        if !valid_operation(operation) {
            return None;
        }
        read_secret(&state, &operation_key(&digest, operation, "settlement"))
    }

    fn execution_session_proof(&self, operation: &OperationId) -> Option<Secret> {
        let state = self.inner.lock().ok()?;
        let (_, _, digest) = bound_scope(&state)?;
        if !valid_operation(operation) {
            return None;
        }
        read_secret(&state, &operation_key(&digest, operation, "proof"))
    }

    fn initial_lease_proof(&self, authorization: &GrantId) -> Option<Secret> {
        let state = self.inner.lock().ok()?;
        let (_, _, digest) = bound_scope(&state)?;
        if !safe_token(authorization.as_str(), MAX_IDENTIFIER_BYTES) {
            return None;
        }
        read_secret(&state, &lease_key(&digest, authorization))
    }
}

impl CredentialHandoff for CredentialSession {
    fn initial_lease_proof(&mut self, authorization: &GrantId, _capability: Capability, proof: Secret) -> Result<(), Refusal> {
        let mut state = self.inner.lock().map_err(|_| handoff_refusal())?;
        let (_, _, digest) = bound_scope(&state).ok_or_else(handoff_refusal)?;
        if !safe_token(authorization.as_str(), MAX_IDENTIFIER_BYTES) {
            return Err(handoff_refusal());
        }
        set_secret(&mut state, &lease_key(&digest, authorization), proof.expose_secret()).map_err(|_| handoff_refusal())
    }

    fn execution_session_proof(&mut self, _capability: Capability, operation: &OperationId, proof: Secret) -> Result<(), Refusal> {
        let mut state = self.inner.lock().map_err(|_| handoff_refusal())?;
        let (_, _, digest) = bound_scope(&state).ok_or_else(handoff_refusal)?;
        if !valid_operation(operation) {
            return Err(handoff_refusal());
        }
        set_secret(&mut state, &operation_key(&digest, operation, "proof"), proof.expose_secret()).map_err(|_| handoff_refusal())
    }

    fn settlement_credential(&mut self, operation: &OperationId, credential: Secret) -> Result<(), Refusal> {
        let mut state = self.inner.lock().map_err(|_| handoff_refusal())?;
        let (_, _, digest) = bound_scope(&state).ok_or_else(handoff_refusal)?;
        if !valid_operation(operation) {
            return Err(handoff_refusal());
        }
        set_secret(&mut state, &operation_key(&digest, operation, "settlement"), credential.expose_secret()).map_err(|_| handoff_refusal())
    }

    fn installation_credential(&mut self, installation_id: &str, credential: Secret) -> Result<(), Refusal> {
        let mut state = self.inner.lock().map_err(|_| handoff_refusal())?;
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
        set_secret(&mut state, INSTALLATION_CREDENTIAL_KEY, credential.expose_secret()).map_err(|_| handoff_refusal())
    }
}
