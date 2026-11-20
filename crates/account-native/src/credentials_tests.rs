#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::default_constructed_unit_structs)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, Capability, CredentialError, CredentialErrorCode, CredentialStore, GrantId, HostSessionBinding,
    InstallationId, OperationId, RefusalCode,
};

use crate::credentials::{CredentialSession, MemorySession, NoPersistentStore, PRODUCT_SESSION_KEY};
use crate::http::{CredentialHandoff, WireCredentials};

fn scope(account: &str, installation: &str) -> AuthenticatedHostScope {
    AuthenticatedHostScope::new(AccountId::new(account), InstallationId::new(installation), HostSessionBinding::new("host_session_binding"))
}

fn operation() -> OperationId {
    OperationId::parse("op_example_0001").unwrap()
}

fn grant() -> GrantId {
    GrantId::new("grant_example_0001")
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
    fail_all_sets: bool,
}

#[derive(Clone, Default)]
struct RecordingStore {
    inner: Arc<Mutex<StoreState>>,
}

impl RecordingStore {
    fn set_fail_delete(&self, key: &str) {
        self.inner.lock().unwrap().fail_delete = Some(key.to_owned());
    }

    fn set_fail_all_sets(&self) {
        self.inner.lock().unwrap().fail_all_sets = true;
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
        if state.fail_all_sets {
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
    session.complete_device_login("acct_old", "access_old_SECRET", Some("refresh_old_SECRET")).unwrap();
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
    let first_set = calls.iter().position(|call| matches!(call, Call::Set(_))).unwrap();
    let last_delete = calls.iter().rposition(|call| matches!(call, Call::Delete(_))).unwrap();
    assert!(last_delete < first_set, "old credentials must be deleted before the new account is written: {calls:?}");
    assert_eq!(calls.first(), Some(&Call::Delete(PRODUCT_SESSION_KEY.to_owned())), "the session credential is removed first");
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
    session.complete_device_login("acct_old", "access_old_SECRET", None).unwrap();
    session.bind_scope(&scope("acct_old", "inst_old")).unwrap();
    store.set_fail_delete(PRODUCT_SESSION_KEY);

    assert_eq!(session.switch_account("acct_new").unwrap_err().code, CredentialErrorCode::Unavailable);
    assert!(WireCredentials::product_session(&session).is_none());
    assert_eq!(session.bound_account().as_deref(), None);
    let mut handoff = session.clone();
    assert!(!CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("new_SECRET")).is_ok());
}

#[test]
fn initial_lease_proofs_and_operation_credentials_share_one_store_and_use_separate_keys() {
    let store = RecordingStore::default();
    let mut handoff = CredentialSession::new(Box::new(store.clone()));
    handoff.complete_device_login("acct_one", "access_one", None).unwrap();
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
    session.complete_device_login("acct_one", "access_one", None).unwrap();
    session.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    assert_eq!(
        CredentialHandoff::installation_credential(&mut session, "inst_two", crate::http::Secret::new("wrong")).unwrap_err().code,
        RefusalCode::AuthUnavailable
    );
    assert!(WireCredentials::installation_credential(&session).is_none());

    let headless = CredentialSession::new(Box::new(NoPersistentStore::default()));
    assert_eq!(headless.complete_device_login("acct_one", "plaintext_SECRET", None).unwrap_err().code, CredentialErrorCode::NoPersistentCredential);
    assert!(WireCredentials::product_session(&headless).is_none());
}

#[test]
fn token_never_in_debug_display_logs_or_error_messages() {
    let store = RecordingStore::default();
    let session = CredentialSession::new(Box::new(store.clone()));
    session.complete_device_login("acct_one", "access_SECRET_alpha", Some("refresh_SECRET_beta")).unwrap();
    session.bind_scope(&scope("acct_one", "inst_one")).unwrap();
    let mut handoff = session.clone();
    CredentialHandoff::initial_lease_proof(&mut handoff, &grant(), Capability::Chat, crate::http::Secret::new("proof_SECRET_gamma")).unwrap();
    CredentialHandoff::execution_session_proof(&mut handoff, Capability::Chat, &operation(), crate::http::Secret::new("operation_SECRET_delta"))
        .unwrap();
    CredentialHandoff::settlement_credential(&mut handoff, &operation(), crate::http::Secret::new("settlement_SECRET_epsilon")).unwrap();
    CredentialHandoff::installation_credential(&mut handoff, "inst_one", crate::http::Secret::new("install_SECRET_zeta")).unwrap();

    let rendered = [format!("{session:?}"), format!("{session}"), format!("{handoff:?}")].join("\n");
    for secret in [
        "access_SECRET_alpha",
        "refresh_SECRET_beta",
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
    first.complete_device_login("acct_one", "access_one", None).unwrap();
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
