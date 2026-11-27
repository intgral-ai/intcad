#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::http::Secret;
use crate::identity::*;
use crate::identity_journal::{AttemptRecord, IdentityJournal};
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

impl Clock {
    fn set_wall(&self, seconds: u64) {
        self.wall.store(seconds, Ordering::Relaxed);
    }

    fn set_monotonic(&self, seconds: u64) {
        self.millis.store(seconds * 1000, Ordering::Relaxed);
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
    replies: Mutex<VecDeque<(&'static str, Reply)>>,
    checks: Mutex<VecDeque<Verify>>,
    on_send: Mutex<Option<SendHook>>,
}

impl IdentityTransport for Arc<Script> {
    fn refresh(&self, client_id: &str, token: &Secret) -> Reply {
        assert_eq!(client_id, "client");
        self.sends.fetch_add(1, Ordering::Relaxed);
        if let Some(hook) = self.on_send.lock().unwrap().take() {
            hook();
        }
        let (expected_token, reply) = self.replies.lock().unwrap().pop_front().unwrap();
        assert_eq!(token.expose_secret(), expected_token);
        reply
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
        self.clock.set_wall(170);
        self.clock.set_monotonic(70);
    }

    fn restarted(&self) -> IdentityLifecycle {
        IdentityLifecycle::with_identity_store(
            IdentityDomain::new("test", "client", "https://product.example").unwrap(),
            Box::new(Store(self.store.clone(), self.root.clone())),
            Box::new(self.script.clone()),
            Box::new(self.script.clone()),
            Box::new(self.clock.clone()),
            self.root.clone(),
        )
        .unwrap()
    }

    fn refreshes(&self) -> u64 {
        self.script.sends.load(Ordering::Relaxed)
    }

    fn reply(&self, reply: Reply) {
        self.script.replies.lock().unwrap().push_back(("old-refresh", reply));
    }

    fn transient(&self, retry_after: Option<u64>) {
        self.reply(Err(RefreshTransportError::Transient { retry_after: retry_after.map(Duration::from_secs) }));
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

    fn attempt(&self, sends: u8, diagnostic_used: bool, until: u64, step: u8) -> AttemptRecord {
        let record = self.journal().read_attempt().unwrap().unwrap();
        assert_eq!((record.sends, record.diagnostic_used, record.cooldown_until_unix, record.cooldown_step), (sends, diagnostic_used, until, step));
        record
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
    f.login().unwrap(); // Retires login 1 in revoked.0, leaving revoked.1 as the next write target.
    f.due();
    f.reply(Err(RefreshTransportError::Terminal));
    let root = f.root.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        std::fs::create_dir(root.join("identity/revoked.1")).unwrap();
        // Reads still succeed; the directory only blocks the next durable write.
        assert_eq!(IdentityJournal::new(root).unwrap().read_revocation().unwrap().unwrap().login_generation, 1);
    }));
    let before = f.record();
    let events_before = f.store.lock().unwrap().events.len();
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Storage);
    assert_eq!(f.refreshes(), 1);
    assert_eq!(f.journal().read_revocation().unwrap().unwrap().login_generation, 1);
    assert_eq!(f.record(), before);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SignedOut);
    assert_eq!(f.store.lock().unwrap().events.len(), events_before);
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
    assert!(IdentityDomain::new("test", "bad\u{2003}client", "https://product.example").is_none());
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
fn sign_out_with_an_unreadable_current_login_does_not_revoke_only_the_observed_generation() {
    let f = Fixture::new();
    f.login().unwrap();
    assert_eq!(f.ensure().unwrap().login_generation(), 1);
    let other = IdentityLifecycle::with_identity_store(
        IdentityDomain::new("test", "client", "https://product.example").unwrap(),
        Box::new(Store(f.store.clone(), f.root.clone())),
        Box::new(f.script.clone()),
        Box::new(f.script.clone()),
        Box::new(f.clock.clone()),
        f.root.clone(),
    )
    .unwrap();
    other.adopt_login("provider", Secret::new("other-access"), Secret::new("other-refresh"), 100, 200, "binding").unwrap();
    let revocation_before = f.journal().read_revocation().unwrap();
    let events_before = f.store.lock().unwrap().events.len();
    f.store.lock().unwrap().unreadable = true;
    let outcome = f.lifecycle.sign_out();
    f.store.lock().unwrap().unreadable = false;
    assert_eq!(outcome, Err(IdentityError::Storage));
    assert_eq!(f.journal().read_revocation().unwrap(), revocation_before);
    assert_eq!(f.store.lock().unwrap().events.len(), events_before);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SignedOut);
    assert_eq!(other.ensure(IdentityTrigger::Background, WAIT).unwrap().login_generation(), 2);
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
fn resume_rechecks_an_unexpired_identity_without_granting_on_failure() {
    let f = Fixture::new();
    f.login().unwrap();
    f.script.checks.lock().unwrap().push_back(Err(VerifyError::Rejected));
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Rejected);
    f.verifies(None);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap().token_generation(), 1);
    f.due(); // Unexpired but already inside the proactive refresh lead.
    f.script.checks.lock().unwrap().push_back(Err(VerifyError::Transient));
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    assert!(f.script.checks.lock().unwrap().is_empty());
    assert_eq!(f.refreshes(), 0);
    assert!(f.journal().read_attempt().unwrap().is_none());
}

#[test]
fn a_wall_clock_jump_does_not_extend_a_lease() {
    let f = Fixture::new();
    f.login().unwrap();
    let lease = f.ensure().unwrap();
    assert_eq!(lease.expires_at_unix_seconds(), 200);
    f.clock.set_wall(99);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 1);
    f.attempt(1, false, 104, 1);
    f.clock.set_wall(170);
    f.clock.set_monotonic(31); // A wall jump cannot restart the original replay clock.
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    f.attempt(2, true, 180, 2);
    assert_eq!(lease.expires_at_unix_seconds(), 200);
}

#[test]
fn an_expired_lease_is_not_returned() {
    for now in [200, 201] {
        let f = Fixture::new();
        f.login().unwrap();
        f.clock.set_wall(now);
        f.reply(Err(RefreshTransportError::Protocol));
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Protocol);
        assert_eq!(f.refreshes(), 1);
        f.attempt(1, false, now + 5, 1);
        assert_eq!(f.record().expires_at_unix, 200);
    }
}

#[test]
fn the_third_send_inside_the_batch_budget_is_the_last() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    for (sends, wall, mono, until) in [(1, 170, 70, 175), (2, 175, 75, 185), (3, 185, 85, 205)] {
        f.clock.set_wall(wall);
        f.clock.set_monotonic(mono);
        let root = f.root.clone();
        *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
            let marker = IdentityJournal::new(root).unwrap().read_attempt().unwrap().unwrap();
            assert_eq!((marker.sends, marker.diagnostic_used, marker.first_possible_send_unix), (sends, false, 170));
        }));
        f.transient(None);
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), u64::from(sends));
        f.attempt(sends, false, until, sends);
    }
    f.clock.set_wall(205); // Cooldown expired, but only 16 monotonic seconds have elapsed.
    f.clock.set_monotonic(86);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 3);
    f.attempt(3, false, 205, 3);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 4);
    assert_eq!(f.attempt(3, true, 245, 4).first_possible_send_unix, 205);
    f.clock.set_wall(245);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 4);
    f.attempt(3, true, 245, 4);
}

#[test]
fn cooldown_blocks_automatic_retry_and_honours_retry_after() {
    for retry_after in [2, 90] {
        let f = Fixture::new();
        f.login().unwrap();
        f.due();
        f.transient(Some(retry_after));
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        f.attempt(1, false, 170 + retry_after, 1);
        f.clock.set_monotonic(71);
        f.clock.set_wall(169 + retry_after);
        f.reply(Err(RefreshTransportError::Protocol));
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), 1);
        f.attempt(1, false, 170 + retry_after, 1);
        f.clock.set_wall(170 + retry_after); // Only wall time decides when cooldown ends.
        let before = f.record();
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Protocol);
        assert_eq!(f.refreshes(), 2);
        f.attempt(2, false, 180 + retry_after, 2);
        assert_eq!(f.record(), before);
    }
}

#[test]
fn a_same_process_replay_is_allowed_only_inside_the_thirty_second_window() {
    // The 20s batch is tighter than the provider's 30s window.
    for (elapsed, diagnostic) in [(19, false), (20, true), (26, true), (31, true)] {
        let f = Fixture::new();
        f.login().unwrap();
        f.due();
        f.transient(None);
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        let first = f.attempt(1, false, 175, 1);
        f.clock.set_wall(175);
        f.clock.set_monotonic(70 + elapsed);
        f.transient(None);
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), 2);
        let second = f.attempt(2, diagnostic, 185, 2);
        assert_eq!(second.attempt_id, first.attempt_id);
        assert_eq!(second.first_possible_send_unix, 170);
    }
}

#[test]
fn a_restarted_attempt_gets_one_diagnostic_not_a_new_batch() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let first = f.attempt(1, false, 175, 1);
    let other = f.restarted();
    f.clock.set_wall(175);
    let root = f.root.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        let marker = IdentityJournal::new(root).unwrap().read_attempt().unwrap().unwrap();
        assert_eq!((marker.sends, marker.diagnostic_used), (2, true));
    }));
    f.transient(None);
    assert_eq!(other.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Transient);
    let diagnostic = f.attempt(2, true, 185, 2);
    assert_eq!(diagnostic.attempt_id, first.attempt_id);
    assert_eq!(diagnostic.first_possible_send_unix, 170);
    assert_eq!(f.refreshes(), 2);
    f.clock.set_wall(185);
    f.transient(None);
    for lifecycle in [f.restarted(), other, f.lifecycle.clone()] {
        assert_eq!(lifecycle.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), 2);
        f.attempt(2, true, 185, 2);
    }
}

#[test]
fn a_user_retry_gets_one_new_diagnostic_after_cooldown() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    let user = || f.lifecycle.ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err();
    f.transient(None);
    assert_eq!(user(), IdentityError::Transient);
    let first = f.attempt(1, false, 175, 1); // Fresh UserRetry starts a normal batch.
    f.clock.set_wall(175);
    f.clock.set_monotonic(101);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    f.attempt(2, true, 185, 2);
    f.clock.set_wall(184);
    f.transient(None);
    assert_eq!(user(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    f.attempt(2, true, 185, 2);
    f.clock.set_wall(185);
    assert_eq!(user(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 3);
    let last = f.attempt(3, true, 205, 3);
    assert_eq!(last.attempt_id, first.attempt_id);
    assert_eq!(last.first_possible_send_unix, 185);
    f.clock.set_wall(205);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(user(), IdentityError::Transient);
    assert_eq!(f.restarted().ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 3);
    assert_eq!(f.attempt(3, true, 205, 3), last);
}

#[test]
fn a_stale_attempt_marker_for_another_generation_is_ignored() {
    for mismatch in ["domain", "login", "token"] {
        let f = Fixture::new();
        f.login().unwrap();
        let record = f.record();
        let mut marker = AttemptRecord {
            version: 1,
            domain: record.domain,
            login_generation: 1,
            token_generation: 1,
            attempt_id: 99,
            first_possible_send_unix: 100,
            sends: 3,
            diagnostic_used: true,
            cooldown_until_unix: 999,
            cooldown_step: 4,
        };
        match mismatch {
            "domain" => marker.domain = "another-domain".to_owned(),
            "login" => marker.login_generation = 2,
            _ => marker.token_generation = 2,
        }
        f.journal().write_attempt(&marker).unwrap();
        assert_eq!(f.ensure().unwrap().token_generation(), 1);
        assert_eq!(f.refreshes(), 0);
        assert!(f.journal().read_attempt().unwrap().is_none());
        f.due();
        f.transient(None);
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), 1);
        f.attempt(1, false, 175, 1);
    }
}

#[test]
fn resume_treats_the_window_as_unprovable_and_does_not_reset_the_batch() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let first = f.attempt(1, false, 175, 1);
    f.clock.set_wall(174);
    f.clock.set_monotonic(74);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 1);
    assert_eq!(f.attempt(1, false, 175, 1), first);
    f.clock.set_wall(175);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let last = f.attempt(2, true, 185, 2);
    assert_eq!(last.first_possible_send_unix, 170);
    assert_eq!(last.attempt_id, first.attempt_id);
    f.clock.set_wall(185);
    f.transient(None);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    assert_eq!(f.attempt(2, true, 185, 2), last);
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

#[test]
fn a_pair_that_expires_during_exchange_is_saved_and_used_for_recovery() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.reply(Ok(RefreshResponse {
        access_token: Secret::new("expired-access"),
        refresh_token: Secret::new("replacement-refresh"),
        provider_account_id: "provider".to_owned(),
        expires_in: Some(Duration::from_secs(3)),
    }));
    let clock = f.clock.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        clock.wall.store(174, Ordering::Relaxed);
        clock.millis.store(74_000, Ordering::Relaxed);
    }));
    // No verifier replies are queued for the already-expired bearer.
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let record = f.record();
    assert!(record.pending);
    assert_eq!(record.expires_at_unix, 173);
    assert_eq!(record.access_token, "expired-access");
    assert_eq!(record.refresh_token, "replacement-refresh");
    f.script.replies.lock().unwrap().push_back((
        "replacement-refresh",
        Ok(RefreshResponse {
            access_token: Secret::new("recovered-access"),
            refresh_token: Secret::new("recovered-refresh"),
            provider_account_id: "provider".to_owned(),
            expires_in: Some(Duration::from_secs(120)),
        }),
    ));
    f.verifies(Some("binding"));
    let lease = f.ensure().unwrap();
    assert_eq!(lease.access_token().expose_secret(), "recovered-access");
    assert_eq!((lease.login_generation(), lease.token_generation()), (1, 3));
    assert_eq!(f.refreshes(), 2);
}
