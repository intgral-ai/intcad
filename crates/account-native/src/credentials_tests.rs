#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::default_constructed_unit_structs)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, Capability, CredentialError, CredentialErrorCode, CredentialStore, GrantId, HostSessionBinding,
    InstallationId, OperationId, RefusalCode,
};

use crate::credentials::{
    BlockedProductExchange, CredentialSession, MemorySession, NoPersistentStore, PRODUCT_SESSION_KEY, ProductExchangeError, ProductSession,
    ProductSessionExchange, ProviderSession,
};
use crate::http::{CredentialHandoff, WireCredentials};

#[test]
fn account_paths_refuse_unavailable_relative_and_non_directory_os_answers() {
    use std::path::PathBuf;

    use crate::AccountPaths;

    assert!(AccountPaths::from_os_base(None).is_none());
    assert!(AccountPaths::from_os_base(Some(PathBuf::from("."))).is_none());
    assert!(AccountPaths::from_os_base(Some(std::env::current_exe().unwrap())).is_none());
}

fn scope(account: &str, installation: &str) -> AuthenticatedHostScope {
    AuthenticatedHostScope::new(AccountId::new(account), InstallationId::new(installation), HostSessionBinding::new("host_session_binding"))
}

fn operation() -> OperationId {
    OperationId::parse("op_example_0001").unwrap()
}

fn grant() -> GrantId {
    GrantId::new("grant_example_0001")
}

struct FixedExchange<'a> {
    account_id: &'a str,
    product_session: &'a str,
    expires_at_unix_seconds: Option<u64>,
}

impl ProductSessionExchange for FixedExchange<'_> {
    fn exchange(&self, provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
        if provider.access_token().is_empty() {
            return Err(ProductExchangeError::Rejected);
        }
        Ok(ProductSession::from_trusted_exchange(self.account_id, crate::http::Secret::new(self.product_session), self.expires_at_unix_seconds))
    }
}

fn product_sign_in(session: &CredentialSession, account: &str, product_secret: &str) {
    session.complete_provider_login("user_provider_0001", "provider_access_SECRET").unwrap();
    session.activate_product_session(&FixedExchange { account_id: account, product_session: product_secret, expires_at_unix_seconds: None }).unwrap();
}

#[test]
fn provider_completion_alone_cannot_activate_or_report_a_product_session() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_old", "product_old_SECRET");
    assert!(session.product_session_active());
    store.take_calls();

    session.complete_provider_login("user_provider_0002", "provider_access_SECRET_2").unwrap();
    assert!(!session.product_session_active());
    assert!(WireCredentials::product_session(&session).is_none());
    assert!(!store.keys().iter().any(|key| key.contains("refresh")), "no refresh token may be persisted without a documented use");
    assert_eq!(session.activate_product_session(&BlockedProductExchange).unwrap_err().code, CredentialErrorCode::Unavailable);
    assert!(!session.product_session_active());
    let calls = store.take_calls();
    let revoked = calls.iter().position(|call| matches!(call, Call::Set(key) if key == "login/commit")).expect("the commit is revoked");
    let product_delete =
        calls.iter().position(|call| matches!(call, Call::Delete(key) if key == PRODUCT_SESSION_KEY)).expect("the old bearer is deleted");
    assert!(revoked < product_delete, "the durable commit is revoked before authority deletion: {calls:?}");
    assert_eq!(
        calls.iter().find(|call| matches!(call, Call::Delete(_))),
        Some(&Call::Delete(PRODUCT_SESSION_KEY.to_owned())),
        "the product bearer is the first credential removed"
    );
}

#[test]
fn only_a_typed_product_exchange_result_activates_a_product_session() {
    let session = CredentialSession::new(Box::new(RecordingStore::default()));
    session.complete_provider_login("user_provider_0001", "provider_access_SECRET").unwrap();
    assert!(!session.product_session_active());

    let login = session
        .activate_product_session(&FixedExchange { account_id: "acct_product", product_session: "product_SECRET", expires_at_unix_seconds: None })
        .unwrap();
    assert_eq!(login.account_id, "acct_product");
    assert!(session.product_session_active());
    let product = WireCredentials::product_session(&session).unwrap();
    assert_eq!(product.expose_secret(), "product_SECRET");
    assert_ne!(product.expose_secret(), "provider_access_SECRET");
}

#[test]
fn product_activation_requires_a_provider_login_and_rejects_expired_results() {
    let session = CredentialSession::new(Box::new(RecordingStore::default()));
    assert_eq!(
        session
            .activate_product_session(&FixedExchange { account_id: "acct", product_session: "product_SECRET", expires_at_unix_seconds: None })
            .unwrap_err()
            .code,
        CredentialErrorCode::Unavailable
    );

    session.complete_provider_login("user_provider_0001", "provider_access_SECRET").unwrap();
    assert_eq!(
        session
            .activate_product_session(&FixedExchange { account_id: "acct", product_session: "product_SECRET", expires_at_unix_seconds: Some(1) })
            .unwrap_err()
            .code,
        CredentialErrorCode::Unavailable
    );
    assert!(!session.product_session_active());
}

#[test]
fn a_legacy_provider_session_without_refresh_still_activates() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    // A stored provider login from before refresh capture has no refresh key at all.
    session.complete_provider_login("user_provider_0001", "provider_access_SECRET").unwrap();
    assert!(!store.keys().iter().any(|key| key.contains("refresh")));

    let legacy = ProviderSession::from_authorized("user_provider_0001".into(), crate::http::Secret::new("provider_access_SECRET"));
    assert!(legacy.refresh().is_none());
    assert!(format!("{legacy:?}").contains("refresh: false"));

    let login = session
        .activate_product_session(&FixedExchange {
            account_id: "acct_legacy",
            product_session: "product_legacy_SECRET",
            expires_at_unix_seconds: None,
        })
        .unwrap();
    assert_eq!(login.account_id, "acct_legacy");
    assert!(session.product_session_active());
    assert_eq!(WireCredentials::product_session(&session).unwrap().expose_secret(), "product_legacy_SECRET");
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Get(String),
    Set(String),
    Delete(String),
}

#[derive(Default)]
struct StoreState {
    entries: BTreeMap<String, String>,
    calls: Vec<Call>,
    fail_delete: Option<String>,
    fail_set: Option<String>,
    fail_all_sets: bool,
    fail_commit: bool,
    block_commit: Option<Receiver<()>>,
}

impl StoreState {
    fn is_commit_value(key: &str, secret: &str) -> bool {
        key == "login/commit" && secret.starts_with("intcad-login/1")
    }
}

#[derive(Clone, Default)]
struct RecordingStore {
    inner: Arc<Mutex<StoreState>>,
    commit_waiting: Arc<AtomicUsize>,
}

impl RecordingStore {
    fn set_fail_delete(&self, key: &str) {
        self.inner.lock().unwrap().fail_delete = Some(key.to_owned());
    }

    fn set_fail_all_sets(&self) {
        self.inner.lock().unwrap().fail_all_sets = true;
    }

    fn set_fail_set(&self, key: &str) {
        self.inner.lock().unwrap().fail_set = Some(key.to_owned());
    }

    fn put(&self, key: &str, value: &str) {
        self.inner.lock().unwrap().entries.insert(key.to_owned(), value.to_owned());
    }

    fn set_fail_commit(&self, fail: bool) {
        self.inner.lock().unwrap().fail_commit = fail;
    }

    fn block_next_commit(&self, gate: Receiver<()>) {
        self.inner.lock().unwrap().block_commit = Some(gate);
    }

    fn wait_until_commit_blocked(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.commit_waiting.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(self.commit_waiting.load(Ordering::SeqCst) > 0, "the commit write was never reached");
    }

    fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut self.inner.lock().unwrap().calls)
    }

    fn contains_key(&self, key: &str) -> bool {
        self.inner.lock().unwrap().entries.contains_key(key)
    }

    fn keys(&self) -> Vec<String> {
        self.inner.lock().unwrap().entries.keys().cloned().collect()
    }

    fn entry(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap().entries.get(key).cloned()
    }

    fn force_put(&self, key: &str, value: &str) {
        self.inner.lock().unwrap().entries.insert(key.to_owned(), value.to_owned());
    }
}

impl CredentialStore for RecordingStore {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        let mut state = self.inner.lock().map_err(|_| unavailable())?;
        state.calls.push(Call::Get(key.to_owned()));
        Ok(state.entries.get(key).cloned())
    }

    fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
        let gate = {
            let mut state = self.inner.lock().map_err(|_| unavailable())?;
            state.calls.push(Call::Set(key.to_owned()));
            if state.fail_all_sets || state.fail_set.as_deref() == Some(key) || (state.fail_commit && StoreState::is_commit_value(key, secret)) {
                return Err(unavailable());
            }
            if StoreState::is_commit_value(key, secret) { state.block_commit.take() } else { None }
        };
        if let Some(gate) = gate {
            self.commit_waiting.fetch_add(1, Ordering::SeqCst);
            let _ = gate.recv();
            self.commit_waiting.fetch_sub(1, Ordering::SeqCst);
        }
        let mut state = self.inner.lock().map_err(|_| unavailable())?;
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

#[test]
fn headless_without_secure_store_keeps_session_in_memory_only() {
    let mut store = NoPersistentStore::default();
    assert_eq!(store.set("session", "secret").unwrap_err().code, CredentialErrorCode::NoPersistentCredential);
    assert!(!store.persistent());
    assert_eq!(store.get("session").unwrap(), None);
    assert!(store.delete("session").is_ok());
}

#[test]
fn memory_session_is_process_local_and_clears_on_sign_out() {
    let mut memory = MemorySession::default();
    assert!(!memory.persistent());
    memory.set("session", "secret").unwrap();
    assert_eq!(memory.get("session").unwrap().as_deref(), Some("secret"));
    memory.clear().unwrap();
    assert_eq!(memory.get("session").unwrap(), None);
    assert!(!format!("{memory:?}").contains("secret"));
}

#[test]
fn logout_and_switch_account_delete_old_credential_first() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_old", "product_old_SECRET");
    session.bind_scope(&scope("acct_old", "inst_old")).unwrap();
    let mut handoff = session.clone();
    CredentialHandoff::initial_lease_proof(&mut handoff, &grant(), Capability::Chat, crate::http::Secret::new("proof_old_SECRET")).unwrap();
    CredentialHandoff::execution_session_proof(&mut handoff, Capability::Chat, &operation(), crate::http::Secret::new("op_proof_secret")).unwrap();
    CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("settle_old_SECRET")).unwrap();
    CredentialHandoff::installation_credential(&mut handoff, "inst_old", crate::http::Secret::new("install_old_SECRET")).unwrap();
    assert_eq!(WireCredentials::initial_lease_proof(&session, &grant()).unwrap().expose_secret(), "proof_old_SECRET");
    assert_eq!(WireCredentials::execution_session_proof(&session, &operation()).unwrap().expose_secret(), "op_proof_secret");
    store.take_calls();

    session.switch_account("acct_new").unwrap();
    let calls = store.take_calls();
    let tombstone = calls.iter().position(|call| matches!(call, Call::Set(key) if key == "auth/logout_pending")).unwrap();
    let first_delete = calls.iter().position(|call| matches!(call, Call::Delete(_))).unwrap();
    let last_delete = calls.iter().rposition(|call| matches!(call, Call::Delete(_))).unwrap();
    let account_set = calls.iter().position(|call| matches!(call, Call::Set(key) if key == "account/id")).unwrap();
    assert!(tombstone < first_delete, "the durable sign-out marker is written before any delete: {calls:?}");
    assert!(last_delete < account_set, "old credentials must be deleted before the new account is written: {calls:?}");
    assert_eq!(calls.get(first_delete), Some(&Call::Delete(PRODUCT_SESSION_KEY.to_owned())), "the session credential is removed first");
    assert!(calls.iter().any(|call| matches!(call, Call::Delete(key) if key == "install/credential")));
    assert!(!store.contains_key(PRODUCT_SESSION_KEY));
    assert!(WireCredentials::product_session(&session).is_none());
    assert!(WireCredentials::initial_lease_proof(&session, &grant()).is_none());
    assert!(WireCredentials::execution_session_proof(&session, &operation()).is_none());
    assert!(WireCredentials::settlement_credential(&session, &operation()).is_none());
}

#[test]
fn account_switch_that_cannot_delete_old_credentials_fails_closed() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_old", "product_old_SECRET");
    session.bind_scope(&scope("acct_old", "inst_old")).unwrap();
    store.set_fail_delete(PRODUCT_SESSION_KEY);

    assert_eq!(session.switch_account("acct_new").unwrap_err().code, CredentialErrorCode::Unavailable);
    assert!(WireCredentials::product_session(&session).is_none());
    assert_eq!(session.bound_account().as_deref(), None);
    let mut handoff = session.clone();
    assert!(!CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("new_SECRET")).is_ok());

    // The failed delete leaves the old bearer in the store, but the durable sign-out marker
    // written before the first delete keeps a fresh process from restoring or handing it off.
    assert!(store.contains_key(PRODUCT_SESSION_KEY), "the failed delete deliberately leaves the old bearer");
    store.take_calls();
    let fresh = CredentialSession::new(Box::new(store.clone()));
    assert!(fresh.logout_pending().unwrap());
    assert!(WireCredentials::product_session(&fresh).is_none());
    assert!(fresh.authenticated_scope().is_none());
    assert_eq!(fresh.bound_account().as_deref(), None);
    let mut fresh_handoff = fresh.clone();
    assert!(!CredentialHandoff::settlement_credential(&mut fresh_handoff, &operation(), crate::http::Secret::new("new_SECRET")).is_ok());
    assert!(
        store.take_calls().iter().all(|call| !matches!(call, Call::Set(_) | Call::Delete(_))),
        "the fresh process must refuse the handoff without writing to the inert store"
    );
}

#[test]
fn a_revoked_session_refuses_a_late_provider_login_or_product_activation() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    store.take_calls();
    session.revoke_in_process();

    assert_eq!(
        session.complete_provider_login("user_provider_late", "provider_late_SECRET").unwrap_err().code,
        CredentialErrorCode::Unavailable,
        "a login that finishes after sign-out or switch must not write provider credentials"
    );
    assert_eq!(
        session
            .activate_product_session(&FixedExchange {
                account_id: "acct_late",
                product_session: "product_late_SECRET",
                expires_at_unix_seconds: None
            })
            .unwrap_err()
            .code,
        CredentialErrorCode::Unavailable,
        "a product activation that finishes after sign-out or switch must not write a product session"
    );
    assert!(store.take_calls().is_empty(), "a revoked session refuses before any store write");
    assert!(!session.product_session_active());
    assert!(session.provider_account().is_none());
}

#[test]
fn a_product_exchange_that_revokes_the_session_before_activation_writes_nothing() {
    struct RevokingExchange(CredentialSession);
    impl ProductSessionExchange for RevokingExchange {
        fn exchange(&self, _provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
            self.0.revoke_in_process();
            Ok(ProductSession::from_trusted_exchange("acct_late", crate::http::Secret::new("product_late_SECRET"), None))
        }
    }

    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    session.complete_provider_login("user_provider_0001", "provider_access_SECRET").unwrap();
    store.take_calls();
    assert_eq!(
        session.activate_product_session(&RevokingExchange(session.clone())).unwrap_err().code,
        CredentialErrorCode::Unavailable,
        "the exchange must not resurrect product authority after a sign-out raced it"
    );
    assert!(store.take_calls().iter().all(|call| !matches!(call, Call::Set(_))), "no product credential may be written");
    assert!(WireCredentials::product_session(&session).is_none());
}

#[test]
fn a_revoked_session_refuses_handoff_writes_without_touching_the_store() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    session.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    store.take_calls();

    session.revoke_in_process();
    let mut handoff = session.clone();
    for refusal in [
        CredentialHandoff::initial_lease_proof(&mut handoff, &grant(), Capability::Chat, crate::http::Secret::new("proof_SECRET")),
        CredentialHandoff::execution_session_proof(&mut handoff, Capability::Chat, &operation(), crate::http::Secret::new("operation_SECRET")),
        CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("settlement_SECRET")),
        CredentialHandoff::installation_credential(&mut handoff, "inst_one", crate::http::Secret::new("install_SECRET")),
    ] {
        assert_eq!(refusal.unwrap_err().code, RefusalCode::AuthUnavailable);
    }
    assert!(store.take_calls().is_empty(), "a revoked session must refuse before any store write or read");
}

#[test]
fn a_failed_sign_out_delete_persists_a_tombstone_and_a_fresh_session_fails_closed() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_old", "product_old_SECRET");
    session.bind_scope(&scope("acct_old", "inst_old")).unwrap();
    assert!(session.authenticated_scope().is_some());
    store.set_fail_delete(PRODUCT_SESSION_KEY);

    assert_eq!(session.sign_out().unwrap_err().code, CredentialErrorCode::Unavailable);
    assert!(session.logout_pending().unwrap(), "the marker is durable before the failed delete");
    assert!(session.authenticated_scope().is_none(), "the old process refuses its old authority");
    assert!(WireCredentials::product_session(&session).is_none());

    // A fresh process over the same inert store sees the marker and restores nothing.
    let fresh = CredentialSession::new(Box::new(store.clone()));
    assert!(fresh.logout_pending().unwrap());
    assert!(fresh.authenticated_scope().is_none());
    assert!(!fresh.product_session_active());
}

#[test]
fn a_completed_sign_out_removes_the_tombstone_only_after_authority_is_gone() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_old", "product_old_SECRET");
    session.bind_scope(&scope("acct_old", "inst_old")).unwrap();
    session.sign_out().unwrap();
    assert!(!session.logout_pending().unwrap());
    assert!(!store.contains_key(PRODUCT_SESSION_KEY));
    assert!(session.authenticated_scope().is_none());
}

#[test]
fn initial_lease_proofs_and_operation_credentials_share_one_store_and_use_separate_keys() {
    let store = RecordingStore::default();
    let mut handoff = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&handoff, "acct_one", "product_one_SECRET");
    handoff.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    let wire = handoff.clone();
    let other_grant = GrantId::new("grant_example_0002");

    CredentialHandoff::initial_lease_proof(&mut handoff, &grant(), Capability::Chat, crate::http::Secret::new("initial_one")).unwrap();
    CredentialHandoff::initial_lease_proof(&mut handoff, &other_grant, Capability::Chat, crate::http::Secret::new("initial_two")).unwrap();
    CredentialHandoff::execution_session_proof(&mut handoff, Capability::Chat, &operation(), crate::http::Secret::new("operation_one")).unwrap();
    CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("settlement_one")).unwrap();

    assert_eq!(WireCredentials::initial_lease_proof(&wire, &grant()).unwrap().expose_secret(), "initial_one");
    assert_eq!(WireCredentials::initial_lease_proof(&wire, &other_grant).unwrap().expose_secret(), "initial_two");
    assert!(WireCredentials::initial_lease_proof(&wire, &GrantId::new("grant_unknown")).is_none());
    assert_eq!(WireCredentials::execution_session_proof(&wire, &operation()).unwrap().expose_secret(), "operation_one");
    assert_eq!(WireCredentials::settlement_credential(&wire, &operation()).unwrap().expose_secret(), "settlement_one");
    assert!(WireCredentials::execution_session_proof(&wire, &OperationId::parse("op_example_other").unwrap()).is_none());

    let other = wire.clone();
    other.bind_scope(&scope("acct_two", "inst_two")).unwrap();
    assert!(WireCredentials::initial_lease_proof(&other, &grant()).is_none());
    assert!(WireCredentials::execution_session_proof(&other, &operation()).is_none());
    assert!(WireCredentials::settlement_credential(&other, &operation()).is_none());
}

#[test]
fn installation_credential_rejects_a_different_installation_and_never_persists_in_headless_no_store() {
    let store = RecordingStore::default();
    let mut session = CredentialSession::new(Box::new(store));
    session.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    assert_eq!(
        CredentialHandoff::installation_credential(&mut session, "inst_two", crate::http::Secret::new("wrong")).unwrap_err().code,
        RefusalCode::AuthUnavailable
    );
    assert!(WireCredentials::installation_credential(&session).is_none());

    let headless = CredentialSession::new(Box::new(NoPersistentStore::default()));
    assert_eq!(
        headless.complete_provider_login("user_provider_0001", "provider_plaintext_SECRET").unwrap_err().code,
        CredentialErrorCode::NoPersistentCredential
    );
    assert!(WireCredentials::product_session(&headless).is_none());
}

#[test]
fn token_never_in_debug_display_logs_or_error_messages() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_SECRET_alpha");
    session.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    let mut handoff = session.clone();
    CredentialHandoff::initial_lease_proof(&mut handoff, &grant(), Capability::Chat, crate::http::Secret::new("proof_SECRET_gamma")).unwrap();
    CredentialHandoff::execution_session_proof(&mut handoff, Capability::Chat, &operation(), crate::http::Secret::new("operation_SECRET_delta"))
        .unwrap();
    CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("settlement_SECRET_epsilon")).unwrap();
    CredentialHandoff::installation_credential(&mut handoff, "inst_one", crate::http::Secret::new("install_SECRET_zeta")).unwrap();

    let rendered = [format!("{session:?}"), format!("{session}"), format!("{handoff:?}")].join("\n");
    for secret in [
        "product_SECRET_alpha",
        "provider_access_SECRET",
        "proof_SECRET_gamma",
        "operation_SECRET_delta",
        "settlement_SECRET_epsilon",
        "install_SECRET_zeta",
    ] {
        assert!(!rendered.contains(secret), "{secret} leaked through Debug or Display");
    }

    store.set_fail_all_sets();
    let error = CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("error_SECRET_eta")).unwrap_err();
    assert!(!error.message.contains("error_SECRET_eta"));
}

#[test]
fn a_restarted_session_preserves_and_then_clears_every_tracked_credential() {
    let store = RecordingStore::default();
    let first = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&first, "acct_one", "product_one_SECRET");
    first.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    let mut first_handoff = first.clone();
    CredentialHandoff::settlement_credential(&mut first_handoff, &operation(), crate::http::Secret::new("settlement_one")).unwrap();

    let second = CredentialSession::new(Box::new(store.clone()));
    second.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    assert_eq!(
        WireCredentials::settlement_credential(&second, &operation()).unwrap().expose_secret(),
        "settlement_one",
        "a restart must keep the settlement credential for recovery"
    );
    let mut second_handoff = second.clone();
    CredentialHandoff::initial_lease_proof(&mut second_handoff, &grant(), Capability::Chat, crate::http::Secret::new("initial_one")).unwrap();
    assert!(store.keys().iter().any(|key| key.starts_with("scope/")));

    second.sign_out().unwrap();
    assert!(store.keys().iter().all(|key| !key.starts_with("scope/")), "sign-out must delete every tracked scope key");
    assert!(!store.contains_key(PRODUCT_SESSION_KEY));
    assert!(WireCredentials::product_session(&second).is_none());
    assert!(WireCredentials::settlement_credential(&second, &operation()).is_none());
    assert!(WireCredentials::initial_lease_proof(&second, &grant()).is_none());
}

#[test]
fn credential_session_is_send_and_sync_for_desktop_composition() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CredentialSession>();
}

#[test]
fn failed_provider_cleanup_keeps_the_durable_sign_out_marker() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    store.set_fail_delete("provider/access_token");
    assert!(session.sign_out().is_err());
    let reopened = CredentialSession::new(Box::new(store));
    assert!(reopened.logout_pending().unwrap());
    assert!(!reopened.product_session_active());
}

const FIRST_INSTALL_ID: &str = "inst_product_0001";
const FIRST_INSTALL_CREDENTIAL: &str = "installation_credential_000000000000000000000000";
const INSTALL_ID_KEY: &str = "install/id";
const INSTALL_CREDENTIAL_KEY: &str = "install/credential";

fn publish_first_installation(session: &CredentialSession, expected: &str) -> Result<(), solidcraft_engine::entitlement::Refusal> {
    let mut handoff = session.clone();
    CredentialHandoff::first_installation_credential(
        &mut handoff,
        &crate::http::Secret::new(expected),
        FIRST_INSTALL_ID,
        crate::http::Secret::new(FIRST_INSTALL_CREDENTIAL),
    )
}

#[test]
fn first_installation_publishes_the_credential_before_the_id_and_both_read_after_restart() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    assert!(!WireCredentials::installation_bound(&session));
    assert!(session.authenticated_scope().is_none(), "the account alone is not an installation binding");
    store.take_calls();

    publish_first_installation(&session, "product_one_SECRET").unwrap();
    let calls = store.take_calls();
    let credential_set = calls.iter().position(|call| matches!(call, Call::Set(key) if key == INSTALL_CREDENTIAL_KEY)).unwrap();
    let id_set = calls.iter().position(|call| matches!(call, Call::Set(key) if key == INSTALL_ID_KEY)).unwrap();
    assert!(credential_set < id_set, "the credential is durable before the id marks the pair published: {calls:?}");
    assert!(session.authenticated_scope().is_some());
    assert_eq!(session.authenticated_scope().unwrap().installation().as_str(), FIRST_INSTALL_ID);
    assert_eq!(WireCredentials::installation_credential(&session).unwrap().expose_secret(), FIRST_INSTALL_CREDENTIAL);

    let restarted = CredentialSession::new(Box::new(store.clone()));
    assert!(WireCredentials::installation_bound(&restarted), "the persisted identity suppresses a replacement bootstrap");
    let scope = restarted.authenticated_scope().expect("the durable pair restores one provable scope");
    assert_eq!(scope.account().as_str(), "acct_one");
    assert_eq!(scope.installation().as_str(), FIRST_INSTALL_ID);
    assert_eq!(
        WireCredentials::installation_credential(&restarted).unwrap().expose_secret(),
        FIRST_INSTALL_CREDENTIAL,
        "the credential and id are read as one binding after restart"
    );
}

#[test]
fn first_installation_requires_the_exact_active_product_session_and_an_unrevoked_epoch() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    store.take_calls();

    assert_eq!(
        publish_first_installation(&session, "product_other_SECRET").unwrap_err().code,
        RefusalCode::AuthUnavailable,
        "a token that is no longer the active product session cannot publish an installation"
    );
    let calls = store.take_calls();
    assert!(calls.iter().all(|call| matches!(call, Call::Get(_))), "a mismatched token performs no store write: {calls:?}");

    let control = CredentialSession::new(Box::new(RecordingStore::default()));
    product_sign_in(&control, "acct_one", "product_one_SECRET");
    publish_first_installation(&control, "product_one_SECRET").unwrap();
    assert!(WireCredentials::installation_bound(&control), "the exact active token publishes the pair");

    session.revoke_in_process();
    assert_eq!(publish_first_installation(&session, "product_one_SECRET").unwrap_err().code, RefusalCode::AuthUnavailable);
    assert!(store.take_calls().is_empty(), "a revoked epoch writes nothing");
    assert!(!WireCredentials::installation_bound(&session));
}

#[test]
fn a_failed_credential_write_publishes_no_id_and_a_restart_fails_closed() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    store.set_fail_set(INSTALL_CREDENTIAL_KEY);

    assert_eq!(publish_first_installation(&session, "product_one_SECRET").unwrap_err().code, RefusalCode::AuthUnavailable);
    let calls = store.take_calls();
    assert!(
        calls.iter().any(|call| matches!(call, Call::Set(key) if key == INSTALL_CREDENTIAL_KEY)),
        "the credential write was attempted and failed: {calls:?}"
    );
    assert!(!store.contains_key(INSTALL_ID_KEY), "a failed credential write must not publish the identity");
    assert!(!store.contains_key(INSTALL_CREDENTIAL_KEY));
    assert!(!WireCredentials::installation_bound(&session));
    assert!(session.authenticated_scope().is_none(), "the failed write locks this process");

    let restarted = CredentialSession::new(Box::new(store));
    assert!(!WireCredentials::installation_bound(&restarted));
    assert!(restarted.authenticated_scope().is_none(), "a partial failure restores no scope");
}

#[test]
fn a_failed_id_write_leaves_no_readable_binding_and_suppresses_authority() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    store.set_fail_set(INSTALL_ID_KEY);

    assert_eq!(publish_first_installation(&session, "product_one_SECRET").unwrap_err().code, RefusalCode::AuthUnavailable);
    let calls = store.take_calls();
    assert!(
        calls.iter().any(|call| matches!(call, Call::Set(key) if key == INSTALL_CREDENTIAL_KEY)),
        "the credential was written before the id attempt: {calls:?}"
    );
    assert!(calls.iter().any(|call| matches!(call, Call::Set(key) if key == INSTALL_ID_KEY)), "the id write was attempted and failed: {calls:?}");
    assert!(!store.contains_key(INSTALL_ID_KEY), "the identity is the last publication marker and must not be half-written");
    assert!(!store.contains_key(INSTALL_CREDENTIAL_KEY), "the unpublished credential is removed");
    assert!(!WireCredentials::installation_bound(&session));
    assert!(session.authenticated_scope().is_none());

    let restarted = CredentialSession::new(Box::new(store));
    assert!(!WireCredentials::installation_bound(&restarted));
    assert!(WireCredentials::installation_credential(&restarted).is_none());
    assert!(restarted.authenticated_scope().is_none());
}

#[test]
fn a_credential_orphaned_before_the_id_crash_marker_is_unreadable_and_a_retry_publishes_the_pair() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    store.take_calls();
    // Simulate a process that wrote the credential and died before the id write completed.
    store.put(INSTALL_CREDENTIAL_KEY, FIRST_INSTALL_CREDENTIAL);

    let restarted = CredentialSession::new(Box::new(store.clone()));
    assert!(!WireCredentials::installation_bound(&restarted), "an orphan credential is not an identity");
    assert!(WireCredentials::installation_credential(&restarted).is_none(), "an orphan credential is not readable");
    assert!(restarted.authenticated_scope().is_none(), "an orphan credential restores no scope");

    publish_first_installation(&restarted, "product_one_SECRET").unwrap();
    assert!(store.contains_key(INSTALL_ID_KEY), "the retry publishes the missing marker");
    assert_eq!(WireCredentials::installation_credential(&restarted).unwrap().expose_secret(), FIRST_INSTALL_CREDENTIAL);
    assert!(restarted.authenticated_scope().is_some());
}

#[test]
fn a_second_first_installation_is_refused_without_touching_the_store() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_one", "product_one_SECRET");
    publish_first_installation(&session, "product_one_SECRET").unwrap();
    store.take_calls();

    assert_eq!(publish_first_installation(&session, "product_one_SECRET").unwrap_err().code, RefusalCode::AuthUnavailable);
    let calls = store.take_calls();
    assert!(calls.iter().all(|call| matches!(call, Call::Get(_))), "an existing installation is never replaced by a new bootstrap: {calls:?}");
    assert_eq!(WireCredentials::installation_credential(&session).unwrap().expose_secret(), FIRST_INSTALL_CREDENTIAL);
}

#[test]
fn a_failed_commit_marker_write_leaves_no_restorable_session() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    let provider = ProviderSession::from_authorized("user_provider_0001".into(), crate::http::Secret::new("provider_access_SECRET"));
    let product = ProductSession::from_trusted_exchange("acct_partial", crate::http::Secret::new("product_partial_SECRET"), None);
    store.set_fail_commit(true);
    assert_eq!(session.persist_verified_login(&provider, &product, 2, &|| true).unwrap_err().code, CredentialErrorCode::Unavailable);
    assert!(store.contains_key(PRODUCT_SESSION_KEY), "the test writes the raw value before the failed marker");
    let fresh = CredentialSession::new(Box::new(store));
    assert!(!fresh.product_session_active(), "an uncommitted partial write is restorable");
    assert!(fresh.display_account().is_none());
    assert!(fresh.bound_account().is_none());
    assert!(fresh.authenticated_scope().is_none());
}

#[test]
fn only_a_fresh_verified_login_can_reopen_after_failed_commit() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    let provider = ProviderSession::from_authorized("user_provider_0001".into(), crate::http::Secret::new("provider_access_SECRET"));
    let partial = ProductSession::from_trusted_exchange("acct_partial", crate::http::Secret::new("product_partial_SECRET"), None);
    store.set_fail_commit(true);
    assert!(session.persist_verified_login(&provider, &partial, 1, &|| true).is_err());
    store.set_fail_commit(false);
    assert!(session.provider_account().is_none());
    assert!(!session.product_session_active());
    assert!(session.display_account().is_none());
    assert!(session.authenticated_scope().is_none());
    store.take_calls();
    assert!(session.bind_scope(&scope("acct_partial", "inst_old")).is_err());
    assert!(session.switch_account("acct_other").is_err());
    assert!(session.complete_provider_login("user_other", "provider_other_SECRET").is_err());
    assert!(store.take_calls().is_empty(), "normal mutations must refuse without touching storage");
    let retry = ProductSession::from_trusted_exchange("acct_retry", crate::http::Secret::new("product_retry_SECRET"), None);
    let (tx, rx) = channel();
    store.block_next_commit(rx);
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || worker_session.persist_verified_login(&provider, &retry, 2, &|| true));
    store.wait_until_commit_blocked();
    assert_eq!(store.take_calls().first(), Some(&Call::Set("login/commit".into())));
    assert_eq!(store.entry("login/commit").as_deref(), Some("intcad-login/revoked"));
    let fresh = CredentialSession::new(Box::new(store.clone()));
    assert!(fresh.display_account().is_none(), "a verified retry remains invisible before its new commit");
    tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap().account_id, "acct_retry");
    assert_eq!(session.display_account().as_deref(), Some("acct_retry"));
    assert_eq!(fresh.display_account().as_deref(), Some("acct_retry"));
    assert!(session.authenticated_scope().is_none(), "identity is not installation proof");
}

#[test]
fn successful_cleanup_does_not_reopen_a_failed_session() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    let provider = ProviderSession::from_authorized("user_provider_0001".into(), crate::http::Secret::new("provider_access_SECRET"));
    let product = ProductSession::from_trusted_exchange("acct_partial", crate::http::Secret::new("product_partial_SECRET"), None);
    store.set_fail_commit(true);
    assert!(session.persist_verified_login(&provider, &product, 1, &|| true).is_err());
    store.set_fail_commit(false);
    session.sign_out().unwrap();
    assert!(!store.contains_key(PRODUCT_SESSION_KEY));
    assert!(session.complete_provider_login("user_other", "provider_other_SECRET").is_err());
}

#[test]
fn a_blocked_commit_is_not_visible_until_the_marker_lands() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    let (gate_tx, gate_rx) = channel();
    store.block_next_commit(gate_rx);
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        let provider = ProviderSession::from_authorized("user_provider_0001".into(), crate::http::Secret::new("provider_access_SECRET"));
        let product = ProductSession::from_trusted_exchange("acct_blocked", crate::http::Secret::new("product_blocked_SECRET"), None);
        worker_session.persist_verified_login(&provider, &product, 3, &|| true)
    });
    store.wait_until_commit_blocked();
    let fresh = CredentialSession::new(Box::new(store.clone()));
    assert!(!fresh.product_session_active(), "the staged/partial activation was read before the commit");
    assert!(fresh.display_account().is_none());
    assert!(fresh.authenticated_scope().is_none());
    gate_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_ok());
    assert!(session.product_session_active(), "the commit was not visible after the marker");
}

#[test]
fn a_superseded_generation_never_writes_the_commit_marker() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    let calls = Arc::new(AtomicUsize::new(0));
    let current = Arc::clone(&calls);
    let provider = ProviderSession::from_authorized("user_provider_0001".into(), crate::http::Secret::new("provider_access_SECRET"));
    let product = ProductSession::from_trusted_exchange("acct_superseded", crate::http::Secret::new("product_superseded_SECRET"), None);
    let result = session.persist_verified_login(&provider, &product, 4, &move || current.fetch_add(1, Ordering::SeqCst) == 0);
    assert_eq!(result.unwrap_err().code, CredentialErrorCode::Unavailable);
    let committed = store
        .take_calls()
        .iter()
        .filter_map(|call| match call {
            Call::Set(key) => store.entry(key).filter(|value| value.starts_with("intcad-login/1")),
            _ => None,
        })
        .count();
    assert_eq!(committed, 0, "a superseded generation wrote a commit marker");
    assert!(!session.product_session_active());
    let fresh = CredentialSession::new(Box::new(store));
    assert!(!fresh.product_session_active());
    assert!(fresh.display_account().is_none());
}

#[test]
fn a_durable_tombstone_wins_a_race_with_a_blocked_commit() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    let (gate_tx, gate_rx) = channel();
    store.block_next_commit(gate_rx);
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        let provider = ProviderSession::from_authorized("user_provider_0001".into(), crate::http::Secret::new("provider_access_SECRET"));
        let product = ProductSession::from_trusted_exchange("acct_race", crate::http::Secret::new("product_race_SECRET"), None);
        worker_session.persist_verified_login(&provider, &product, 5, &|| true)
    });
    store.wait_until_commit_blocked();
    // A separate durable writer represents the tombstone that must survive a crash before the
    // commit finishes. The desktop's own promotion gate makes this an accepted sign-out only
    // before promotion, never during it.
    CredentialSession::new(Box::new(store.clone())).mark_logout_pending().unwrap();
    gate_tx.send(()).unwrap();
    let result = worker.join().unwrap();
    assert!(result.is_ok(), "the write itself completed");
    let fresh = CredentialSession::new(Box::new(store));
    assert!(fresh.logout_pending().unwrap());
    assert!(!fresh.product_session_active(), "the tombstone did not suppress the raced commit");
    assert!(fresh.display_account().is_none());
}

#[test]
fn display_identity_requires_the_same_durable_commit_as_product_authority() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    product_sign_in(&session, "acct_display", "product_display_SECRET");
    assert_eq!(session.display_account().as_deref(), Some("acct_display"));
    // Revoking the commit alone makes every reader, including the display-only identity, refuse.
    store.force_put("login/commit", "intcad-login/revoked");
    let fresh = CredentialSession::new(Box::new(store));
    assert!(fresh.display_account().is_none());
    assert!(!fresh.product_session_active());
}
