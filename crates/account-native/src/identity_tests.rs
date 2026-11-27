#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::http::Secret;
use crate::identity::*;
use crate::identity_journal::IdentityJournal;
use crate::identity_store::{IdentityStore, IdentityStoreError, StoredIdentity};
use crate::identity_transport::{IdentityTransport, RefreshResponse, RefreshTransportError};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
const WAIT: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Clock {
    wall: AtomicU64,
    millis: AtomicU64,
}

impl IdentityClock for Arc<Clock> {
    fn now_unix_seconds(&self) -> u64 {
        self.wall.load(Ordering::Relaxed)
    }

    fn monotonic_now(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::Relaxed))
    }
}

#[derive(Default)]
struct StoreState {
    record: Option<StoredIdentity>,
    unreadable: bool,
    fail_delete: bool,
    events: Vec<(&'static str, Option<u64>)>,
}

struct Store(Arc<Mutex<StoreState>>, PathBuf);

impl IdentityStore for Store {
    fn load(&self) -> Result<Option<StoredIdentity>, IdentityStoreError> {
        let state = self.0.lock().unwrap();
        if state.unreadable { Err(IdentityStoreError::Unavailable) } else { Ok(state.record.clone()) }
    }

    fn store_pending(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError> {
        let mut state = self.0.lock().unwrap();
        assert!(record.pending);
        assert!(state.record.as_ref().is_none_or(|old| old.token_generation != record.token_generation));
        state.events.push(("pending", None));
        state.record = Some(record.clone());
        Ok(())
    }

    fn activate(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError> {
        let mut state = self.0.lock().unwrap();
        assert!(state.record.as_ref().unwrap().pending);
        state.events.push(("active", None));
        let mut record = record.clone();
        record.pending = false;
        state.record = Some(record);
        Ok(())
    }

    fn delete(&mut self) -> Result<(), IdentityStoreError> {
        let mut state = self.0.lock().unwrap();
        let revoked = IdentityJournal::new(self.1.clone()).unwrap().read_revocation().unwrap();
        state.events.push(("delete", revoked.map(|record| record.login_generation)));
        if state.fail_delete {
            return Err(IdentityStoreError::Unavailable);
        }
        state.record = None;
        Ok(())
    }
}

type Reply = Result<RefreshResponse, RefreshTransportError>;
type Verify = Result<VerifiedSession, VerifyError>;
type SendHook = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Script {
    sends: AtomicU64,
    replies: Mutex<VecDeque<Reply>>,
    checks: Mutex<VecDeque<Verify>>,
    on_send: Mutex<Option<SendHook>>,
}

impl IdentityTransport for Arc<Script> {
    fn refresh(&self, client_id: &str, token: &Secret) -> Reply {
        assert_eq!(client_id, "client");
        assert_eq!(token.expose_secret(), "old-refresh");
        self.sends.fetch_add(1, Ordering::Relaxed);
        if let Some(hook) = self.on_send.lock().unwrap().take() {
            hook();
        }
        self.replies.lock().unwrap().pop_front().unwrap()
    }
}

impl SessionVerifier for Arc<Script> {
    fn verify(&self, _: &Secret) -> Verify {
        self.checks.lock().unwrap().pop_front().unwrap()
    }
}

struct Fixture {
    root: PathBuf,
    lifecycle: IdentityLifecycle,
    clock: Arc<Clock>,
    script: Arc<Script>,
    store: Arc<Mutex<StoreState>>,
}

impl Fixture {
    fn new() -> Self {
        let serial = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("intcad-identity-core-{}-{serial}", std::process::id()));
        let domain = IdentityDomain::new("test", "client", "https://product.example").unwrap();
        let clock = Arc::new(Clock::default());
        clock.wall.store(100, Ordering::Relaxed);
        let script = Arc::new(Script::default());
        let store = Arc::new(Mutex::new(StoreState::default()));
        let lifecycle = IdentityLifecycle::with_identity_store(
            domain,
            Box::new(Store(store.clone(), root.clone())),
            Box::new(script.clone()),
            Box::new(script.clone()),
            Box::new(clock.clone()),
            root.clone(),
        )
        .unwrap();
        Self { root, lifecycle, clock, script, store }
    }

    fn login(&self) -> Result<(), IdentityError> {
        self.lifecycle.adopt_login("provider", Secret::new("old-access"), Secret::new("old-refresh"), 100, 200, "binding")
    }

    fn due(&self) {
        self.clock.wall.store(170, Ordering::Relaxed);
        self.clock.millis.store(70_000, Ordering::Relaxed);
    }

    fn refreshes(&self) -> u64 {
        self.script.sends.load(Ordering::Relaxed)
    }

    fn reply(&self, reply: Reply) {
        self.script.replies.lock().unwrap().push_back(reply);
    }

    fn success(&self) {
        self.reply(Ok(RefreshResponse {
            access_token: Secret::new("new-access"),
            refresh_token: Secret::new("new-refresh"),
            provider_account_id: "provider".to_owned(),
            expires_in: Some(Duration::from_secs(120)),
        }));
    }

    fn verifies(&self, binding: Option<&str>) {
        self.script
            .checks
            .lock()
            .unwrap()
            .push_back(Ok(VerifiedSession { account_id: "verified".to_owned(), same_login_binding: binding.map(str::to_owned) }));
    }

    fn ensure(&self) -> Result<IdentityLease, IdentityError> {
        self.lifecycle.ensure(IdentityTrigger::Background, WAIT)
    }

    fn record(&self) -> StoredIdentity {
        self.store.lock().unwrap().record.clone().unwrap()
    }

    fn journal(&self) -> IdentityJournal {
        IdentityJournal::new(self.root.clone()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn an_active_identity_is_returned_without_a_refresh() {
    let f = Fixture::new();
    f.login().unwrap();
    let lease = f.ensure().unwrap();
    assert_eq!(lease.access_token().expose_secret(), "old-access");
    assert_eq!(lease.login_generation(), 1);
    assert_eq!(lease.token_generation(), 1);
    assert_eq!(lease.expires_at_unix_seconds(), 200);
    assert!(!format!("{lease:?}").contains("old-access"));
    assert_eq!(f.refreshes(), 0);
}

#[test]
fn an_expiring_identity_is_refreshed_once_and_activated_with_a_matching_binding() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    f.verifies(Some("binding"));
    let lease = f.ensure().unwrap();
    assert_eq!(lease.access_token().expose_secret(), "new-access");
    assert_eq!((lease.login_generation(), lease.token_generation(), lease.expires_at_unix_seconds()), (1, 2, 290));
    assert_eq!(f.ensure().unwrap().token_generation(), 2);
    assert_eq!(f.refreshes(), 1);
    assert_eq!(
        f.lifecycle.status(),
        IdentityStatus::Active { account_id: "verified".to_owned(), login_generation: 1, token_generation: 2, expires_at_unix: 290 }
    );
}

#[test]
fn a_refresh_without_a_matching_binding_stays_pending() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    f.verifies(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::VerificationPending);
    assert!(f.record().pending);
    assert_eq!(f.record().refresh_token, "new-refresh");
    f.verifies(Some("binding"));
    assert_eq!(f.ensure().unwrap().token_generation(), 2);
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn a_refresh_with_a_different_binding_is_session_changed() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    f.verifies(Some("different"));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SessionChanged);
    assert!(f.record().pending);
    assert_eq!(f.record().token_generation, 2);
}

#[test]
fn invalid_grant_signs_out_and_writes_the_barrier_before_delete() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.reply(Err(RefreshTransportError::Terminal));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SignedOut);
    assert_eq!(f.store.lock().unwrap().events.last(), Some(&("delete", Some(1))));
    assert!(f.journal().read_attempt().unwrap().is_none());
    assert_eq!(f.lifecycle.status(), IdentityStatus::SignedOut);
}

#[test]
fn a_barrier_write_failure_keeps_the_record() {
    let f = Fixture::new();
    f.login().unwrap();
    // A symlink is an invalid journal slot on Unix; on every platform a directory
    // in the target slot cannot be opened for writing as a file.
    std::fs::create_dir(f.root.join("identity/revoked.0")).unwrap();
    assert_eq!(f.lifecycle.sign_out().unwrap_err(), IdentityError::Storage);
    assert_eq!(f.record().token_generation, 1);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SignedOut);
    assert_eq!(f.store.lock().unwrap().events.last(), Some(&("active", None)));
}

#[test]
fn a_delete_failure_after_the_barrier_is_still_signed_out() {
    let f = Fixture::new();
    f.login().unwrap();
    f.store.lock().unwrap().fail_delete = true;
    assert!(!f.lifecycle.sign_out().unwrap().credentials_cleared);
    assert_eq!(f.store.lock().unwrap().events.last(), Some(&("delete", Some(1))));
    assert_eq!(f.record().login_generation, 1);
    assert_eq!(f.lifecycle.status(), IdentityStatus::SignedOut);
}

#[test]
fn protocol_and_transient_failures_keep_the_record_and_grant_nothing() {
    for (transport, expected) in [
        (RefreshTransportError::Protocol, IdentityError::Protocol),
        (RefreshTransportError::Transient { retry_after: None }, IdentityError::Transient),
    ] {
        let f = Fixture::new();
        f.login().unwrap();
        f.due();
        f.reply(Err(transport));
        assert_eq!(f.ensure().unwrap_err(), expected);
        assert_eq!(f.record().token_generation, 1);
        let attempt = f.journal().read_attempt().unwrap().unwrap();
        assert_eq!(attempt.token_generation, 1);
        assert_eq!(attempt.cooldown_until_unix, 175);
        assert!(f.ensure().is_err());
        assert_eq!(f.refreshes(), 1);
    }
}

#[test]
fn status_reports_pending_and_signed_out() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    f.verifies(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::VerificationPending);
    assert_eq!(f.lifecycle.status(), IdentityStatus::PendingVerification { account_id: "provider".to_owned() });
    assert!(f.lifecycle.sign_out().unwrap().credentials_cleared);
    assert_eq!(f.lifecycle.status(), IdentityStatus::SignedOut);
}

#[test]
fn a_new_login_removes_the_previous_generation_before_writing() {
    let f = Fixture::new();
    f.login().unwrap();
    f.login().unwrap();
    let record = f.record();
    assert_eq!((record.login_generation, record.token_generation), (2, 1));
    let state = f.store.lock().unwrap();
    assert_eq!(&state.events[3..], &[("delete", Some(1)), ("pending", None), ("active", None)]);
}

#[test]
fn domains_with_different_environments_or_origins_have_different_keys() {
    let key = |environment, origin| IdentityDomain::new(environment, "client", origin).unwrap().storage_key().to_owned();
    let base = key("test", "https://product.example");
    assert_eq!(base.len(), 64);
    assert!(base.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    assert_ne!(base, key("prod", "https://product.example"));
    assert_ne!(base, key("test", "https://other.example"));
    assert!(IdentityDomain::new("test", "bad client", "https://product.example").is_none());
    assert!(IdentityDomain::new("test", "client", "http://product.example").is_none());
}

#[test]
fn an_unavailable_store_never_creates_a_memory_session() {
    let f = Fixture::new();
    f.store.lock().unwrap().unreadable = true;
    assert_eq!(f.login().unwrap_err(), IdentityError::Storage);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Storage);
    assert_eq!(f.lifecycle.status(), IdentityStatus::Recovering);
    assert_eq!(f.refreshes(), 0);
    assert!(std::fs::read_dir(f.root.join("identity")).unwrap().all(|entry| { entry.unwrap().file_name().to_string_lossy().ends_with(".lock") }));
    assert!(f.store.lock().unwrap().events.is_empty());
}

#[test]
fn late_success_after_sign_out_does_not_write() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    let lifecycle = f.lifecycle.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        assert!(lifecycle.sign_out().unwrap().credentials_cleared);
    }));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SignedOut);
    assert!(f.store.lock().unwrap().record.is_none());
    assert_eq!(f.store.lock().unwrap().events.len(), 4);
}

#[test]
fn a_stale_refresh_cannot_replace_an_explicit_new_login() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    let lifecycle = f.lifecycle.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        lifecycle.adopt_login("second-provider", Secret::new("other-access"), Secret::new("other-refresh"), 170, 290, "other-binding").unwrap();
    }));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SessionChanged);
    let active = f.ensure().unwrap();
    assert_eq!(active.login_generation(), 2);
    assert_eq!(active.access_token().expose_secret(), "other-access");
    assert_eq!(f.store.lock().unwrap().events.len(), 6);
}

#[test]
fn a_separate_lifecycle_observes_the_durable_barrier_after_delete_failure() {
    let f = Fixture::new();
    f.login().unwrap();
    f.store.lock().unwrap().fail_delete = true;
    assert!(!f.lifecycle.sign_out().unwrap().credentials_cleared);
    let other = IdentityLifecycle::with_identity_store(
        IdentityDomain::new("test", "client", "https://product.example").unwrap(),
        Box::new(Store(f.store.clone(), f.root.clone())),
        Box::new(f.script.clone()),
        Box::new(f.script.clone()),
        Box::new(f.clock.clone()),
        f.root.clone(),
    )
    .unwrap();
    assert_eq!(other.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::SignedOut);
    assert_eq!(other.status(), IdentityStatus::SignedOut);
    assert_eq!(f.refreshes(), 0);
}

#[test]
fn a_contended_refresh_file_lock_obeys_zero_wait_without_sending() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    let lock = std::fs::File::create(f.root.join("identity/refresh.lock")).unwrap();
    lock.try_lock().unwrap();
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Background, Duration::ZERO).unwrap_err(), IdentityError::Busy);
    assert_eq!(f.refreshes(), 0);
    assert!(f.journal().read_attempt().unwrap().is_none());
    assert!(f.lifecycle.sign_out().unwrap().credentials_cleared);
}

#[test]
fn resume_checks_the_unchanged_bearer_without_requiring_a_new_binding() {
    let f = Fixture::new();
    f.login().unwrap();
    f.script.checks.lock().unwrap().push_back(Err(VerifyError::Rejected));
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Rejected);
    f.verifies(None);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap().token_generation(), 1);
    assert_eq!(f.refreshes(), 0);
}

#[test]
fn a_clock_before_issuance_cannot_yield_the_stored_lease() {
    let f = Fixture::new();
    f.login().unwrap();
    f.clock.wall.store(99, Ordering::Relaxed);
    f.reply(Err(RefreshTransportError::Transient { retry_after: None }));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn a_rotated_pair_with_too_little_time_to_use_is_saved_but_not_published() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.reply(Ok(RefreshResponse {
        access_token: Secret::new("new-access"),
        refresh_token: Secret::new("new-refresh"),
        provider_account_id: "provider".to_owned(),
        expires_in: Some(Duration::from_secs(3)),
    }));
    f.verifies(Some("binding"));
    f.verifies(Some("binding"));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert!(f.record().pending);
    assert_eq!(f.record().refresh_token, "new-refresh");
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 1);
}
