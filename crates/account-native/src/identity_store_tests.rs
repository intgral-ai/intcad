#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use solidcraft_engine::entitlement::{CredentialError, CredentialErrorCode, CredentialStore};

use crate::identity_store::{CredentialIdentityStore, IdentityStore, IdentityStoreError, StoredIdentity};

const COMMIT_KEY: &str = "identity/session/commit";
const ACTIVE_KEY: &str = "identity/session/active";
const OWNED_KEY: &str = "identity/session/owned";

fn access_key(generation: u64) -> String {
    format!("identity/session/{generation}/access")
}

fn refresh_key(generation: u64) -> String {
    format!("identity/session/{generation}/refresh")
}

fn meta_key(generation: u64) -> String {
    format!("identity/session/{generation}/meta")
}

fn record(login_generation: u64, token_generation: u64) -> StoredIdentity {
    StoredIdentity {
        version: 1,
        domain: "https://account.example.test".to_owned(),
        login_generation,
        token_generation,
        provider_account_id: "user_provider_0001".to_owned(),
        verified_account_id: None,
        access_token: format!("access_SECRET_{login_generation}_{token_generation}"),
        refresh_token: format!("refresh_SECRET_{login_generation}_{token_generation}"),
        issued_at_unix: 1_700_000_000,
        expires_at_unix: 1_700_000_300,
        session_binding: None,
        pending: true,
    }
}

fn activated(mut record: StoredIdentity) -> StoredIdentity {
    record.pending = false;
    record.verified_account_id = Some("acct_verified_0001".to_owned());
    record
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Get(String),
    Set(String),
    Delete(String),
}

impl Call {
    fn key(&self) -> &str {
        match self {
            Call::Get(key) | Call::Set(key) | Call::Delete(key) => key,
        }
    }
}

#[derive(Default)]
struct StoreState {
    entries: BTreeMap<String, String>,
    calls: Vec<Call>,
    fail_set: Option<String>,
    fail_delete: Option<String>,
}

/// A local recording fake of the engine credential store: no network, no OS keyring.
#[derive(Clone, Default)]
struct RecordingStore {
    inner: Arc<Mutex<StoreState>>,
}

impl RecordingStore {
    fn put(&self, key: &str, value: &str) {
        self.inner.lock().unwrap().entries.insert(key.to_owned(), value.to_owned());
    }

    fn remove(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap().entries.remove(key)
    }

    fn entry(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap().entries.get(key).cloned()
    }

    fn entries(&self) -> BTreeMap<String, String> {
        self.inner.lock().unwrap().entries.clone()
    }

    fn keys(&self) -> Vec<String> {
        self.inner.lock().unwrap().entries.keys().cloned().collect()
    }

    fn fail_set(&self, key: Option<&str>) {
        self.inner.lock().unwrap().fail_set = key.map(str::to_owned);
    }

    fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut self.inner.lock().unwrap().calls)
    }
}

impl CredentialStore for RecordingStore {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        let mut state = self.inner.lock().map_err(|_| unavailable())?;
        state.calls.push(Call::Get(key.to_owned()));
        Ok(state.entries.get(key).cloned())
    }

    fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
        let mut state = self.inner.lock().map_err(|_| unavailable())?;
        state.calls.push(Call::Set(key.to_owned()));
        if state.fail_set.as_deref() == Some(key) {
            return Err(unavailable());
        }
        state.entries.insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
        let mut state = self.inner.lock().map_err(|_| unavailable())?;
        state.calls.push(Call::Delete(key.to_owned()));
        if state.fail_delete.as_deref() == Some(key) {
            return Err(unavailable());
        }
        state.entries.remove(key);
        Ok(())
    }

    fn persistent(&self) -> bool {
        true
    }
}

fn unavailable() -> CredentialError {
    CredentialError { code: CredentialErrorCode::Unavailable, message: "unavailable".into() }
}

fn identity_store(fake: &RecordingStore) -> CredentialIdentityStore {
    CredentialIdentityStore::new(Box::new(fake.clone()))
}

#[test]
fn pending_and_active_records_round_trip_without_mixing_generations() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    assert_eq!(store.load().unwrap(), None);

    // Hostile records are rejected before any write.
    let mut invalid = Vec::new();
    let mut bad = record(7, 3);
    bad.version = 2;
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.access_token = String::new();
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.access_token = "access\nSECRET".to_owned();
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.refresh_token = String::new();
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.login_generation = 0;
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.token_generation = 0;
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.expires_at_unix = bad.issued_at_unix;
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.expires_at_unix = bad.issued_at_unix - 1;
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.domain = String::new();
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.domain = "https://account.example.test\n".to_owned();
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.provider_account_id = String::new();
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.verified_account_id = Some("bad account".to_owned());
    invalid.push(bad);
    let mut bad = record(7, 3);
    bad.session_binding = Some("bad binding".to_owned());
    invalid.push(bad);
    for bad in invalid {
        assert_eq!(store.store_pending(&bad).unwrap_err(), IdentityStoreError::Corrupt);
    }
    assert!(fake.keys().is_empty(), "an invalid record must not write anything");

    let pending = record(7, 3);
    store.store_pending(&pending).unwrap();
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded, pending);
    assert!(loaded.pending);

    // Debug prints token presence only, never the token bytes.
    let debug = format!("{loaded:?}");
    assert!(!debug.contains(&pending.access_token));
    assert!(!debug.contains(&pending.refresh_token));
    assert!(!format!("{store:?}").contains("SECRET"));
    assert!(!format!("{:?}", IdentityStoreError::Corrupt).contains("SECRET"));

    // The meta value carries only the non-token fields.
    let meta = fake.entry(&meta_key(3)).unwrap();
    assert!(!meta.contains(&pending.access_token));
    assert!(!meta.contains(&pending.refresh_token));
    // No single keyring value holds both tokens.
    for value in fake.entries().values() {
        assert!(!(value.contains(&pending.access_token) && value.contains(&pending.refresh_token)));
    }

    let mut active = activated(pending.clone());
    active.session_binding = Some("binding_digest_0001".to_owned());
    store.activate(&active).unwrap();
    assert_eq!(store.load().unwrap().unwrap(), active);

    // A new token generation in the same login replaces the old one without mixing.
    let next = record(7, 4);
    store.store_pending(&next).unwrap();
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded, next);
    assert_eq!(loaded.token_generation, 4);
    assert!(loaded.pending, "the stale active marker must not activate the new generation");
    assert!(fake.entry(&access_key(3)).is_none());
    assert_eq!(fake.entry(&access_key(4)).unwrap(), next.access_token);
    assert_eq!(fake.entry(&refresh_key(4)).unwrap(), next.refresh_token);
}

#[test]
fn a_missing_commit_is_absent_and_a_missing_token_is_corrupt() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    assert_eq!(store.load().unwrap(), None);
    assert!(store.delete().is_ok(), "deleting an absent record succeeds");

    // Staged generation keys without a commit are never read.
    fake.put(&access_key(3), "access_SECRET_staged");
    fake.put(&refresh_key(3), "refresh_SECRET_staged");
    fake.put(&meta_key(3), "{}");
    assert_eq!(store.load().unwrap(), None);
    for key in [access_key(3), refresh_key(3), meta_key(3)] {
        fake.remove(&key);
    }

    let rec = record(7, 3);
    store.store_pending(&rec).unwrap();
    // Every generation key is required once the commit exists.
    for key in [access_key(3), refresh_key(3), meta_key(3)] {
        let value = fake.remove(&key).unwrap();
        assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt, "missing {key}");
        fake.put(&key, &value);
    }
    // An empty or control-bearing token is corrupt.
    let value = fake.remove(&access_key(3)).unwrap();
    fake.put(&access_key(3), "");
    assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt);
    fake.put(&access_key(3), "access\nSECRET");
    assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt);
    fake.put(&access_key(3), &value);
    // Invalid meta JSON, a wrong meta version and a generation mismatch are corrupt.
    let meta = fake.remove(&meta_key(3)).unwrap();
    fake.put(&meta_key(3), "not json");
    assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt);
    let mut parsed: serde_json::Value = serde_json::from_str(&meta).unwrap();
    parsed["version"] = serde_json::json!(2);
    fake.put(&meta_key(3), &parsed.to_string());
    assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt);
    let mut parsed: serde_json::Value = serde_json::from_str(&meta).unwrap();
    parsed["token_generation"] = serde_json::json!(9);
    fake.put(&meta_key(3), &parsed.to_string());
    assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt);
    // A token field smuggled into the meta is refused, not read.
    let mut parsed: serde_json::Value = serde_json::from_str(&meta).unwrap();
    parsed["access_token"] = serde_json::json!("access_SECRET_smuggled");
    fake.put(&meta_key(3), &parsed.to_string());
    assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt);
    fake.put(&meta_key(3), &meta);
    assert_eq!(store.load().unwrap().unwrap(), rec);
    // A malformed commit is corrupt, not absent.
    let commit = fake.remove(COMMIT_KEY).unwrap();
    for malformed in ["garbage", "intcad-identity/1/7/3", "intcad-identity/1/0/3/0", "intcad-identity/2/7/3/0"] {
        fake.put(COMMIT_KEY, malformed);
        assert_eq!(store.load().unwrap_err(), IdentityStoreError::Corrupt, "{malformed}");
    }
    fake.put(COMMIT_KEY, &commit);
    assert_eq!(store.load().unwrap().unwrap(), rec);
}

#[test]
fn activation_requires_the_matching_pending_generation() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    // Nothing committed: nothing to activate.
    assert_eq!(store.activate(&activated(record(7, 3))).unwrap_err(), IdentityStoreError::Corrupt);

    store.store_pending(&record(7, 3)).unwrap();
    // Another token generation or another login cannot activate this commit.
    assert_eq!(store.activate(&activated(record(7, 4))).unwrap_err(), IdentityStoreError::Corrupt);
    assert_eq!(store.activate(&activated(record(8, 3))).unwrap_err(), IdentityStoreError::Corrupt);
    assert!(store.load().unwrap().unwrap().pending, "a refused activation leaves the record pending");
    // An invalid record is rejected before any write.
    let mut bad = activated(record(7, 3));
    bad.refresh_token = String::new();
    assert_eq!(store.activate(&bad).unwrap_err(), IdentityStoreError::Corrupt);

    let active = activated(record(7, 3));
    store.activate(&active).unwrap();
    let loaded = store.load().unwrap().unwrap();
    assert!(!loaded.pending);
    assert_eq!(loaded.verified_account_id.as_deref(), Some("acct_verified_0001"));
    assert_eq!(fake.entry(ACTIVE_KEY).unwrap(), "intcad-identity-active/1/7/3");

    // A torn activation (meta written, active marker failed) stays pending on load.
    let next = record(7, 4);
    store.store_pending(&next).unwrap();
    fake.fail_set(Some(ACTIVE_KEY));
    assert_eq!(store.activate(&activated(next.clone())).unwrap_err(), IdentityStoreError::Unavailable);
    fake.fail_set(None);
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.token_generation, 4);
    assert!(loaded.pending, "meta.pending=false without a matching active marker stays pending");
}

#[test]
fn old_generation_keys_are_removed_after_a_new_commit() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    store.store_pending(&record(7, 3)).unwrap();
    store.activate(&activated(record(7, 3))).unwrap();

    // A failed new-generation write must not hide the old readable record.
    fake.fail_set(Some(&meta_key(4)));
    assert_eq!(store.store_pending(&record(7, 4)).unwrap_err(), IdentityStoreError::Unavailable);
    fake.fail_set(None);
    assert_eq!(store.load().unwrap().unwrap(), activated(record(7, 3)));
    for key in [access_key(4), refresh_key(4), meta_key(4)] {
        assert!(fake.entry(&key).is_none(), "the torn generation is removed: {key}");
    }
    assert_eq!(fake.entry(OWNED_KEY).unwrap(), "3", "owned is restored after a failed write");

    // A failed commit write rolls back the same way.
    fake.fail_set(Some(COMMIT_KEY));
    assert_eq!(store.store_pending(&record(7, 5)).unwrap_err(), IdentityStoreError::Unavailable);
    fake.fail_set(None);
    assert_eq!(store.load().unwrap().unwrap(), activated(record(7, 3)));
    for key in [access_key(5), refresh_key(5), meta_key(5)] {
        assert!(fake.entry(&key).is_none());
    }
    assert_eq!(fake.entry(OWNED_KEY).unwrap(), "3");

    // A successful commit retires the old generation.
    store.store_pending(&record(7, 4)).unwrap();
    for key in [access_key(3), refresh_key(3), meta_key(3)] {
        assert!(fake.entry(&key).is_none(), "the old generation is removed: {key}");
    }
    assert_eq!(fake.entry(OWNED_KEY).unwrap(), "4");
    assert!(fake.entry(&access_key(4)).is_some());
    assert!(store.load().unwrap().unwrap().pending);
}

#[test]
fn an_orphan_generation_is_removed_by_the_next_commit_or_delete() {
    // An interrupted write left generation 9 keys listed in owned but never committed.
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    store.store_pending(&record(7, 3)).unwrap();
    fake.put(&access_key(9), "access_SECRET_orphan");
    fake.put(&refresh_key(9), "refresh_SECRET_orphan");
    fake.put(&meta_key(9), "{}");
    fake.put(OWNED_KEY, "3\n9");

    store.store_pending(&record(7, 4)).unwrap();
    for generation in [3, 9] {
        for key in [access_key(generation), refresh_key(generation), meta_key(generation)] {
            assert!(fake.entry(&key).is_none(), "orphan/old key removed: {key}");
        }
    }
    assert_eq!(fake.entry(OWNED_KEY).unwrap(), "4");
    assert_eq!(store.load().unwrap().unwrap().token_generation, 4);

    // Delete also removes an orphan generation listed in owned.
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    store.store_pending(&record(7, 3)).unwrap();
    fake.put(&access_key(9), "access_SECRET_orphan");
    fake.put(&refresh_key(9), "refresh_SECRET_orphan");
    fake.put(&meta_key(9), "{}");
    fake.put(OWNED_KEY, "3\n9");
    store.delete().unwrap();
    assert!(fake.keys().iter().all(|key| !key.starts_with("identity/")), "every identity key is gone");
}

#[test]
fn a_corrupt_owned_list_cannot_block_commit_revocation() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    store.store_pending(&record(7, 3)).unwrap();
    store.activate(&activated(record(7, 3))).unwrap();
    // A corrupt owned list must not keep the old record readable: the commit is revoked first.
    fake.put(OWNED_KEY, "3\nnot-a-generation");
    assert_eq!(store.delete().unwrap_err(), IdentityStoreError::Unavailable);
    assert!(fake.entry(COMMIT_KEY).is_none(), "the commit is revoked even with a corrupt owned list");
    assert_eq!(store.load().unwrap(), None);
    // The current generation and the markers are gone even though owned was unreadable.
    for key in [access_key(3), refresh_key(3), meta_key(3)] {
        assert!(fake.entry(&key).is_none(), "the current generation is removed: {key}");
    }
    assert!(fake.entry(ACTIVE_KEY).is_none());
    // An unenumerable index must not turn into a later false cleanup success.
    assert!(fake.entry(OWNED_KEY).is_some());
    assert_eq!(store.delete().unwrap_err(), IdentityStoreError::Unavailable);
}

#[test]
fn oversized_meta_is_refused_before_any_write() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    // Field validation alone would accept these values, but the serialized meta would exceed
    // one keyring value; it is refused before any store call.
    let mut oversized = record(7, 3);
    oversized.domain = "d".repeat(2048);
    oversized.provider_account_id = "p".repeat(128);
    oversized.verified_account_id = Some("v".repeat(128));
    oversized.session_binding = Some("b".repeat(128));
    assert_eq!(store.store_pending(&oversized).unwrap_err(), IdentityStoreError::Corrupt);
    assert!(fake.take_calls().is_empty(), "no store call may happen before the meta bound");
    assert!(fake.keys().is_empty());

    // Activation applies the same bound before writing; the stored record stays unchanged.
    let stored = record(7, 3);
    store.store_pending(&stored).unwrap();
    fake.take_calls();
    let mut oversized = stored.clone();
    oversized.pending = false;
    oversized.domain = "d".repeat(2048);
    oversized.provider_account_id = "p".repeat(128);
    oversized.verified_account_id = Some("v".repeat(128));
    oversized.session_binding = Some("b".repeat(128));
    assert_eq!(store.activate(&oversized).unwrap_err(), IdentityStoreError::Corrupt);
    assert!(fake.take_calls().iter().all(|call| matches!(call, Call::Get(_))), "activation only reads before the meta bound");
    assert_eq!(store.load().unwrap().unwrap(), stored);
    for value in fake.entries().values() {
        assert!(value.len() < 2000);
    }
}

#[test]
fn a_large_access_token_stays_in_its_own_keyring_value() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    let mut rec = record(7, 3);
    rec.access_token = "A".repeat(1800);
    rec.refresh_token = "R".repeat(1800);
    store.store_pending(&rec).unwrap();

    // Windows Credential Manager caps a value at 2560 bytes; every value stays well below it.
    for (key, value) in fake.entries() {
        assert!(value.len() < 2000, "value for {key} is {} bytes, too large for one keyring entry", value.len());
        assert!(!(value.contains(&rec.access_token) && value.contains(&rec.refresh_token)), "two tokens share the value for {key}");
    }
    assert_eq!(fake.entry(&access_key(3)).unwrap(), rec.access_token);
    assert_eq!(fake.entry(&refresh_key(3)).unwrap(), rec.refresh_token);
    let meta = fake.entry(&meta_key(3)).unwrap();
    assert!(!meta.contains(&rec.access_token));
    assert!(!meta.contains(&rec.refresh_token));
    assert_eq!(store.load().unwrap().unwrap(), rec);
}

#[test]
fn a_legacy_access_only_store_is_absent_and_not_migrated() {
    let fake = RecordingStore::default();
    let store = identity_store(&fake);
    // A pre-renewal store: provider login, product session and the legacy login commit.
    fake.put("provider/account", "user_provider_0001");
    fake.put("provider/access_token", "provider_access_SECRET");
    fake.put("auth/product_session", "product_SECRET");
    fake.put("account/id", "acct_legacy");
    fake.put("login/commit", "intcad-login/1/0/acct_legacy");
    fake.take_calls();

    assert_eq!(store.load().unwrap(), None);
    // Loading reads only identity keys and never writes or migrates anything.
    let calls = fake.take_calls();
    assert!(calls.iter().all(|call| matches!(call, Call::Get(_))));
    assert!(calls.iter().all(|call| call.key().starts_with("identity/session/")));
    assert_eq!(fake.entry("provider/access_token").unwrap(), "provider_access_SECRET");
    assert_eq!(fake.entry("login/commit").unwrap(), "intcad-login/1/0/acct_legacy");
    assert!(fake.keys().iter().all(|key| !key.starts_with("identity/")));
}

#[test]
fn legacy_provider_keys_are_not_touched() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    let legacy = [
        ("provider/account", "user_provider_0001"),
        ("provider/access_token", "provider_access_SECRET"),
        ("auth/product_session", "product_SECRET"),
        ("account/id", "acct_legacy"),
        ("install/id", "install_0001"),
        ("login/commit", "intcad-login/1/0/acct_legacy"),
    ];
    for (key, value) in legacy {
        fake.put(key, value);
    }

    store.store_pending(&record(7, 3)).unwrap();
    store.activate(&activated(record(7, 3))).unwrap();
    store.store_pending(&record(7, 4)).unwrap();
    store.delete().unwrap();

    for (key, value) in legacy {
        assert_eq!(fake.entry(key).as_deref(), Some(value), "legacy key {key} must survive untouched");
    }
    // Every store call targeted an identity key; nothing else was read, written or deleted.
    assert!(fake.take_calls().iter().all(|call| call.key().starts_with("identity/session/")));
    assert!(fake.keys().iter().all(|key| !key.starts_with("identity/")));
}
#[test]
fn failed_staging_rollback_remains_owned_until_cleanup_can_delete_it() {
    let fake = RecordingStore::default();
    let mut store = CredentialIdentityStore::new(Box::new(fake.clone()));
    let old = record(7, 3);
    store.store_pending(&old).unwrap();
    let access = access_key(4);
    fake.fail_set(Some(&meta_key(4)));
    fake.inner.lock().unwrap().fail_delete = Some(access.clone());
    assert!(store.store_pending(&record(7, 4)).is_err());
    assert!(fake.inner.lock().unwrap().entries.contains_key(&access));
    fake.inner.lock().unwrap().fail_delete = None;
    store.delete().unwrap();
    assert!(fake.inner.lock().unwrap().entries.is_empty());
}

#[test]
fn failed_logout_cleanup_keeps_owned_index_for_retry() {
    let fake = RecordingStore::default();
    let mut store = CredentialIdentityStore::new(Box::new(fake.clone()));
    store.store_pending(&record(7, 3)).unwrap();
    fake.inner.lock().unwrap().fail_delete = Some(access_key(3));
    assert!(store.delete().is_err());
    assert!(fake.inner.lock().unwrap().entries.contains_key(OWNED_KEY));
    assert!(store.load().unwrap().is_none());
    fake.inner.lock().unwrap().fail_delete = None;
    store.delete().unwrap();
    assert!(fake.inner.lock().unwrap().entries.is_empty());
}

#[test]
fn residual_logout_tracks_a_missing_owned_commit_generation_before_cleanup() {
    let fake = RecordingStore::default();
    let mut store = identity_store(&fake);
    store.store_pending(&record(7, 3)).unwrap();
    fake.put(OWNED_KEY, "2");
    fake.inner.lock().unwrap().fail_delete = Some(access_key(3));
    fake.take_calls();

    assert_eq!(store.delete(), Err(IdentityStoreError::Unavailable));
    assert!(store.load().unwrap().is_none());
    assert_eq!(fake.entry(OWNED_KEY).as_deref(), Some("3"));
    let calls = fake.take_calls();
    let revoke = calls.iter().position(|call| *call == Call::Delete(COMMIT_KEY.to_owned())).unwrap();
    let retain = calls.iter().position(|call| *call == Call::Set(OWNED_KEY.to_owned())).unwrap();
    let delete = calls.iter().position(|call| *call == Call::Delete(access_key(3))).unwrap();
    assert!(revoke < retain && retain < delete);

    fake.inner.lock().unwrap().fail_delete = None;
    identity_store(&fake).delete().unwrap();
    assert!(fake.keys().is_empty());
}
