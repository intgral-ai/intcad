//! Generation-keyed refreshable identity storage over the OS credential store.
//!
//! One [`StoredIdentity`] is a versioned record: domain, login/token generations, account ids,
//! the paired access/refresh tokens, expiry, session binding and a pending flag. Each token
//! lives in its own generation key (`identity/session/<token_generation>/access` and `…/refresh`)
//! so a large token still fits one Windows Credential Manager value (2560-byte cap); the `…/meta`
//! key holds only the non-token fields. No keyring value ever contains both tokens.
//!
//! [`CredentialIdentityStore::store_pending`] writes the new generation's keys and the
//! `identity/session/commit` marker last: until the commit lands, readers keep seeing the previous
//! committed generation, and a torn write is rolled back best-effort. `identity/session/owned`
//! lists every generation that may hold keys; it gains the new generation before any key is
//! written and is rewritten after a successful commit retires the other generations. `delete`
//! removes the commit first — a corrupt `owned` list cannot block revocation — then the active
//! marker, every known generation and `owned`, so a crash during cleanup leaves no readable
//! record. Every serialized value, including the meta, is bounded below the Windows Credential
//! Manager 2560-byte cap before it is written.
//!
//! `load` returns `None` without a commit (legacy access-only stores are never migrated), and
//! `Corrupt` when a commit lacks valid generation keys or meta. A record is active only when the
//! `identity/session/active` marker matches the commit and the meta says `pending=false`;
//! anything else loads as pending. Tokens are redacted from `Debug` and every error message.

use std::fmt;

use serde::{Deserialize, Serialize};
use solidcraft_engine::entitlement::CredentialStore;

use crate::credentials::{MAX_ACCOUNT_BYTES, has_control, safe_token};

const RECORD_VERSION: u32 = 1;
const COMMIT_KEY: &str = "identity/session/commit";
const ACTIVE_KEY: &str = "identity/session/active";
const OWNED_KEY: &str = "identity/session/owned";
const COMMIT_MAGIC: &str = "intcad-identity/1";
const ACTIVE_MAGIC: &str = "intcad-identity-active/1";
/// Windows Credential Manager caps one value at 2560 bytes. Each token stays well below the cap
/// in its own key; the meta, commit, active and owned values are far smaller.
const MAX_TOKEN_BYTES: usize = 2400;
/// The serialized meta must also fit one keyring value even if a future caller passes a large
/// domain, account id or binding; it is refused before any key is written.
const MAX_META_BYTES: usize = 2400;
const MAX_DOMAIN_BYTES: usize = 2048;
const MAX_BINDING_BYTES: usize = 128;
const MAX_OWNED_GENERATIONS: usize = 64;

/// One refreshable identity record. The lifecycle owns the domain comparison and the generation
/// sequencing; the store only validates and persists. `Debug` prints token presence, never bytes.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct StoredIdentity {
    pub version: u32,
    pub domain: String,
    pub login_generation: u64,
    pub token_generation: u64,
    pub provider_account_id: String,
    pub verified_account_id: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub session_binding: Option<String>,
    pub pending: bool,
}

impl fmt::Debug for StoredIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredIdentity")
            .field("version", &self.version)
            .field("domain", &self.domain)
            .field("login_generation", &self.login_generation)
            .field("token_generation", &self.token_generation)
            .field("provider_account_id", &self.provider_account_id)
            .field("verified_account_id", &self.verified_account_id)
            .field("access_token", &!self.access_token.is_empty())
            .field("refresh_token", &!self.refresh_token.is_empty())
            .field("issued_at_unix", &self.issued_at_unix)
            .field("expires_at_unix", &self.expires_at_unix)
            .field("session_binding", &self.session_binding)
            .field("pending", &self.pending)
            .finish()
    }
}

/// The identity store could not be used. Messages are static and never quote stored values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentityStoreError {
    /// The credential store could not be read or written.
    Unavailable,
    /// A committed record is incomplete or invalid; it must not be used.
    Corrupt,
}

impl fmt::Display for IdentityStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("identity store unavailable"),
            Self::Corrupt => formatter.write_str("identity store corrupt"),
        }
    }
}

impl std::error::Error for IdentityStoreError {}

/// The durable identity seam used by the renewal lifecycle.
pub(crate) trait IdentityStore: Send {
    fn load(&self) -> Result<Option<StoredIdentity>, IdentityStoreError>;
    fn store_pending(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError>;
    fn activate(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError>;
    fn delete(&mut self) -> Result<(), IdentityStoreError>;
}

/// The meta key payload: exactly the non-token record fields. Unknown fields (including any
/// smuggled token) are refused on read.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityMeta {
    version: u32,
    domain: String,
    login_generation: u64,
    token_generation: u64,
    provider_account_id: String,
    verified_account_id: Option<String>,
    issued_at_unix: u64,
    expires_at_unix: u64,
    session_binding: Option<String>,
    pending: bool,
}

impl IdentityMeta {
    fn from_record(record: &StoredIdentity) -> Self {
        Self {
            version: record.version,
            domain: record.domain.clone(),
            login_generation: record.login_generation,
            token_generation: record.token_generation,
            provider_account_id: record.provider_account_id.clone(),
            verified_account_id: record.verified_account_id.clone(),
            issued_at_unix: record.issued_at_unix,
            expires_at_unix: record.expires_at_unix,
            session_binding: record.session_binding.clone(),
            pending: record.pending,
        }
    }

    fn valid(&self) -> bool {
        self.version == RECORD_VERSION
            && valid_domain(&self.domain)
            && self.login_generation > 0
            && self.token_generation > 0
            && safe_token(&self.provider_account_id, MAX_ACCOUNT_BYTES)
            && self.verified_account_id.as_deref().is_none_or(|id| safe_token(id, MAX_ACCOUNT_BYTES))
            && self.expires_at_unix > self.issued_at_unix
            && self.session_binding.as_deref().is_none_or(|binding| safe_token(binding, MAX_BINDING_BYTES))
    }
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty() && domain.len() <= MAX_DOMAIN_BYTES && !has_control(domain) && !domain.bytes().any(|byte| byte.is_ascii_whitespace())
}

fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= MAX_TOKEN_BYTES && !has_control(token)
}

fn valid_record(record: &StoredIdentity) -> bool {
    IdentityMeta::from_record(record).valid() && valid_token(&record.access_token) && valid_token(&record.refresh_token)
}

struct Commit {
    login_generation: u64,
    token_generation: u64,
}

fn parse_generation(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Parse `intcad-identity/1/<login_generation>/<token_generation>/<previous_generation or 0>`.
fn parse_commit(value: &str) -> Option<Commit> {
    let rest = value.strip_prefix(COMMIT_MAGIC)?.strip_prefix('/')?;
    let mut parts = rest.split('/');
    let login_generation = parse_generation(parts.next()?)?;
    let token_generation = parse_generation(parts.next()?)?;
    let _previous_generation = parse_generation(parts.next()?)?;
    if parts.next().is_some() || login_generation == 0 || token_generation == 0 {
        return None;
    }
    Some(Commit { login_generation, token_generation })
}

/// Whether the active marker proves exactly this commit's login and token generation.
fn active_matches(value: &str, commit: &Commit) -> bool {
    let Some(rest) = value.strip_prefix(ACTIVE_MAGIC).and_then(|rest| rest.strip_prefix('/')) else {
        return false;
    };
    let Some((login, token)) = rest.split_once('/') else {
        return false;
    };
    parse_generation(login) == Some(commit.login_generation) && parse_generation(token) == Some(commit.token_generation)
}

/// Parse the newline-separated owned generations. Invalid content is refused, never guessed.
fn parse_owned(value: &str) -> Option<Vec<u64>> {
    let mut generations: Vec<u64> = Vec::new();
    for line in value.split('\n').filter(|line| !line.is_empty()) {
        let generation = parse_generation(line)?;
        if generation == 0 || generations.len() >= MAX_OWNED_GENERATIONS {
            return None;
        }
        if !generations.contains(&generation) {
            generations.push(generation);
        }
    }
    Some(generations)
}

/// The OS-credential-store implementation of [`IdentityStore`].
pub(crate) struct CredentialIdentityStore {
    store: Box<dyn CredentialStore + Send>,
}

impl fmt::Debug for CredentialIdentityStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("CredentialIdentityStore").field("credentials", &"<redacted>").finish()
    }
}

impl CredentialIdentityStore {
    pub(crate) fn new(store: Box<dyn CredentialStore + Send>) -> Self {
        Self { store }
    }

    fn generation_key(generation: u64, suffix: &str) -> String {
        format!("identity/session/{generation}/{suffix}")
    }

    fn read_owned(&self) -> Result<Vec<u64>, IdentityStoreError> {
        match self.store.get(OWNED_KEY).map_err(|_| IdentityStoreError::Unavailable)? {
            Some(value) => parse_owned(&value).ok_or(IdentityStoreError::Corrupt),
            None => Ok(Vec::new()),
        }
    }

    fn write_owned(&mut self, generations: &[u64]) -> Result<(), IdentityStoreError> {
        let value = generations.iter().map(u64::to_string).collect::<Vec<_>>().join("\n");
        self.store.set(OWNED_KEY, &value).map_err(|_| IdentityStoreError::Unavailable)
    }

    fn delete_generation(&mut self, generation: u64) -> Result<(), IdentityStoreError> {
        for suffix in ["access", "refresh", "meta"] {
            self.store.delete(&Self::generation_key(generation, suffix)).map_err(|_| IdentityStoreError::Unavailable)?;
        }
        Ok(())
    }
}

impl IdentityStore for CredentialIdentityStore {
    fn load(&self) -> Result<Option<StoredIdentity>, IdentityStoreError> {
        let Some(commit_value) = self.store.get(COMMIT_KEY).map_err(|_| IdentityStoreError::Unavailable)? else {
            return Ok(None);
        };
        let commit = parse_commit(&commit_value).ok_or(IdentityStoreError::Corrupt)?;
        let generation = commit.token_generation;
        let access = self
            .store
            .get(&Self::generation_key(generation, "access"))
            .map_err(|_| IdentityStoreError::Unavailable)?
            .ok_or(IdentityStoreError::Corrupt)?;
        let refresh = self
            .store
            .get(&Self::generation_key(generation, "refresh"))
            .map_err(|_| IdentityStoreError::Unavailable)?
            .ok_or(IdentityStoreError::Corrupt)?;
        let meta_value = self
            .store
            .get(&Self::generation_key(generation, "meta"))
            .map_err(|_| IdentityStoreError::Unavailable)?
            .ok_or(IdentityStoreError::Corrupt)?;
        let meta: IdentityMeta = serde_json::from_str(&meta_value).map_err(|_| IdentityStoreError::Corrupt)?;
        if !meta.valid()
            || meta.login_generation != commit.login_generation
            || meta.token_generation != commit.token_generation
            || !valid_token(&access)
            || !valid_token(&refresh)
        {
            return Err(IdentityStoreError::Corrupt);
        }
        let active = self.store.get(ACTIVE_KEY).map_err(|_| IdentityStoreError::Unavailable)?;
        let activated = !meta.pending && active.as_deref().is_some_and(|value| active_matches(value, &commit));
        Ok(Some(StoredIdentity {
            version: meta.version,
            domain: meta.domain,
            login_generation: meta.login_generation,
            token_generation: meta.token_generation,
            provider_account_id: meta.provider_account_id,
            verified_account_id: meta.verified_account_id,
            access_token: access,
            refresh_token: refresh,
            issued_at_unix: meta.issued_at_unix,
            expires_at_unix: meta.expires_at_unix,
            session_binding: meta.session_binding,
            pending: !activated,
        }))
    }

    fn store_pending(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError> {
        if !valid_record(record) {
            return Err(IdentityStoreError::Corrupt);
        }
        let meta = serde_json::to_string(&IdentityMeta::from_record(record)).map_err(|_| IdentityStoreError::Unavailable)?;
        if meta.len() > MAX_META_BYTES {
            return Err(IdentityStoreError::Corrupt);
        }
        let previous = match self.store.get(COMMIT_KEY).map_err(|_| IdentityStoreError::Unavailable)? {
            Some(value) => parse_commit(&value).map(|commit| commit.token_generation).unwrap_or(0),
            None => 0,
        };
        let old_owned = self.read_owned()?;
        let generation = record.token_generation;
        let mut owned = old_owned.clone();
        if !owned.contains(&generation) {
            owned.push(generation);
        }
        // Track the generation before writing any of its keys, so an interrupted write can
        // always be cleaned up by a later commit or delete.
        self.write_owned(&owned)?;
        let result = (|| {
            self.store.set(&Self::generation_key(generation, "access"), &record.access_token).map_err(|_| IdentityStoreError::Unavailable)?;
            self.store.set(&Self::generation_key(generation, "refresh"), &record.refresh_token).map_err(|_| IdentityStoreError::Unavailable)?;
            self.store.set(&Self::generation_key(generation, "meta"), &meta).map_err(|_| IdentityStoreError::Unavailable)?;
            // The commit is the linearization point: readers keep the previous generation until
            // this write lands.
            let commit = format!("{COMMIT_MAGIC}/{}/{}/{}", record.login_generation, generation, previous);
            self.store.set(COMMIT_KEY, &commit).map_err(|_| IdentityStoreError::Unavailable)
        })();
        if result.is_err() {
            // Keep the durable owned list unless every staged key is confirmed gone.
            if self.delete_generation(generation).is_ok() {
                let _ = self.write_owned(&old_owned);
            }
            return result;
        }
        // The new commit is durable. Retire every other generation best-effort; a generation
        // whose delete failed stays in owned so its keys are never untracked.
        let mut retained = vec![generation];
        for other in old_owned.iter().copied().filter(|other| *other != generation) {
            if self.delete_generation(other).is_err() {
                retained.push(other);
            }
        }
        let _ = self.write_owned(&retained);
        Ok(())
    }

    fn activate(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError> {
        if !valid_record(record) {
            return Err(IdentityStoreError::Corrupt);
        }
        let commit_value = self.store.get(COMMIT_KEY).map_err(|_| IdentityStoreError::Unavailable)?.ok_or(IdentityStoreError::Corrupt)?;
        let commit = parse_commit(&commit_value).ok_or(IdentityStoreError::Corrupt)?;
        if commit.login_generation != record.login_generation || commit.token_generation != record.token_generation {
            return Err(IdentityStoreError::Corrupt);
        }
        let mut meta = IdentityMeta::from_record(record);
        meta.pending = false;
        let meta = serde_json::to_string(&meta).map_err(|_| IdentityStoreError::Unavailable)?;
        if meta.len() > MAX_META_BYTES {
            return Err(IdentityStoreError::Corrupt);
        }
        self.store.set(&Self::generation_key(record.token_generation, "meta"), &meta).map_err(|_| IdentityStoreError::Unavailable)?;
        // The active marker lands after the meta: a crash between them loads as pending.
        let active = format!("{ACTIVE_MAGIC}/{}/{}", record.login_generation, record.token_generation);
        self.store.set(ACTIVE_KEY, &active).map_err(|_| IdentityStoreError::Unavailable)
    }

    fn delete(&mut self) -> Result<(), IdentityStoreError> {
        let commit = self.store.get(COMMIT_KEY).map_err(|_| IdentityStoreError::Unavailable)?;
        let current = commit.as_deref().and_then(parse_commit).map(|commit| commit.token_generation);
        // Revoke the commit first: this is the logout linearization point. A corrupt or unreadable
        // owned list must never leave the old record readable, and a crash during the remaining
        // deletes leaves no readable record.
        self.store.delete(COMMIT_KEY).map_err(|_| IdentityStoreError::Unavailable)?;
        let mut result = Ok(());
        // Read owned best-effort only after revocation. A corrupt list cannot block cleanup of the
        // current generation, but unknown orphan keys may remain, so the cleanup result stays
        // `Unavailable` even when the commit and the current generation are gone. Keep
        // an unenumerable list rather than reporting false success on the next cleanup.
        let mut generations = match self.read_owned() {
            Ok(generations) => generations,
            Err(_) => {
                result = Err(IdentityStoreError::Unavailable);
                Vec::new()
            }
        };
        if self.store.delete(ACTIVE_KEY).is_err() {
            result = Err(IdentityStoreError::Unavailable);
        }
        if let Some(generation) = current
            && !generations.contains(&generation)
        {
            generations.push(generation);
        }
        let mut retained = Vec::new();
        for generation in generations {
            if self.delete_generation(generation).is_err() {
                retained.push(generation);
                result = Err(IdentityStoreError::Unavailable);
            }
        }
        if retained.is_empty() && result.is_ok() && self.store.delete(OWNED_KEY).is_err() {
            result = Err(IdentityStoreError::Unavailable);
        }
        result
    }
}
