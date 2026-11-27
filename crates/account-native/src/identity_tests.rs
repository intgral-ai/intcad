#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use crate::http::Secret;
use crate::identity::*;
use crate::identity_journal::{AttemptRecord, IdentityJournal};
use crate::identity_store::{IdentityStore, IdentityStoreError, StoredIdentity};
use crate::identity_transport::{IdentityTransport, RefreshResponse, RefreshTransportError};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
const WAIT: Duration = Duration::from_secs(15);
const RACE_WAIT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Clock {
    wall: AtomicU64,
    millis: AtomicU64,
    on_resume_before_lock: Mutex<Option<SendHook>>,
    on_background_before_lock: Mutex<Option<SendHook>>,
    on_local_refresh_contention: Mutex<Option<SendHook>>,
    on_attempt_write: Mutex<Option<SendHook>>,
    on_refresh_locked: Mutex<Option<SendHook>>,
}

impl IdentityClock for Arc<Clock> {
    fn now_unix_seconds(&self) -> u64 {
        self.wall.load(Ordering::Relaxed)
    }

    fn monotonic_now(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::Relaxed))
    }

    fn before_refresh_lock(&self, trigger: IdentityTrigger) {
        let hooks = match trigger {
            IdentityTrigger::Resume => &self.on_resume_before_lock,
            IdentityTrigger::Background => &self.on_background_before_lock,
            _ => return,
        };
        let hook = hooks.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }

    fn after_attempt_write(&self) {
        let hook = self.on_attempt_write.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }

    fn after_refresh_lock(&self) {
        if let Some(hook) = self.on_refresh_locked.lock().unwrap().take() {
            hook();
        }
    }

    fn on_local_refresh_contention(&self) {
        let hook = self.on_local_refresh_contention.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
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
    reads: usize,
    unreadable: bool,
    fail_delete: bool,
    on_delete: Option<SendHook>,
    events: Vec<(&'static str, Option<u64>)>,
}

struct Store(Arc<Mutex<StoreState>>, PathBuf);

impl IdentityStore for Store {
    fn load(&self) -> Result<Option<StoredIdentity>, IdentityStoreError> {
        let mut state = self.0.lock().unwrap();
        state.reads += 1;
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
        if let Some(hook) = state.on_delete.take() {
            hook();
        }
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
    on_verify: Mutex<Option<SendHook>>,
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
        let hook = self.on_verify.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
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
fn missing_barrier_blocks_surviving_credentials_and_explicit_login_repairs_it() {
    let f = Fixture::new();
    f.login().unwrap();
    let slot = f.root.join("identity/revoked.0");
    assert_eq!(f.journal().read_revocation().unwrap().unwrap().login_generation, 0);
    assert!(f.ensure().is_ok());
    std::fs::remove_file(&slot).unwrap();
    assert_eq!(f.lifecycle.status(), IdentityStatus::SignedOut);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SignedOut);
    f.login().unwrap();
    assert_eq!(f.ensure().unwrap().login_generation(), 2);
}

#[test]
fn torn_first_barrier_fails_closed_and_explicit_login_repairs_it() {
    let f = Fixture::new();
    f.login().unwrap();
    std::fs::write(f.root.join("identity/revoked.0"), b"torn").unwrap();
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Storage);
    assert_eq!(f.lifecycle.status(), IdentityStatus::Recovering);
    f.login().unwrap();
    assert_eq!(f.ensure().unwrap().login_generation(), 2);
}

#[test]
fn restarted_background_and_failed_resume_require_verification() {
    let f = Fixture::new();
    f.login().unwrap();
    let peer = f.restarted();
    assert_eq!(peer.status(), IdentityStatus::Recovering);
    for (trigger, wall) in [(IdentityTrigger::Background, 100), (IdentityTrigger::Resume, 101), (IdentityTrigger::Background, 103)] {
        f.clock.set_wall(wall);
        f.script.checks.lock().unwrap().push_back(Err(VerifyError::Transient));
        assert_eq!(peer.ensure(trigger, WAIT).unwrap_err(), IdentityError::Transient);
        assert!(f.script.checks.lock().unwrap().is_empty());
    }
    f.clock.set_wall(108);
    f.verifies(None);
    assert!(peer.ensure(IdentityTrigger::Background, WAIT).is_ok());
    assert!(peer.ensure(IdentityTrigger::Background, WAIT).is_ok());
    assert_eq!(f.refreshes(), 0);
}

#[test]
fn residual_busy_resume_discards_an_in_flight_background_verification() {
    let f = Fixture::new();
    f.login().unwrap();
    let peer = f.restarted();
    f.verifies(None);
    let (entered, release) = block_call(&f.script.on_verify);
    let background = peer.clone();
    let background = thread::spawn(move || background.ensure(IdentityTrigger::Background, RACE_WAIT));
    entered.recv_timeout(RACE_WAIT).unwrap();

    assert_eq!(peer.ensure(IdentityTrigger::Resume, Duration::ZERO).unwrap_err(), IdentityError::Busy);
    release.send(()).unwrap();
    let stale = background.join().unwrap();

    f.clock.set_wall(101);
    f.clock.set_monotonic(1);
    f.verifies(None);
    let lease = peer.ensure(IdentityTrigger::Background, WAIT).unwrap();
    assert!(f.script.checks.lock().unwrap().is_empty(), "Background must verify again before leasing");
    assert_eq!(lease.token_generation(), 1);
    assert_eq!(stale.unwrap_err(), IdentityError::Transient);
}

#[test]
fn rollback_inside_token_lifetime_cannot_lease() {
    let f = Fixture::new();
    f.login().unwrap();
    f.clock.set_wall(130);
    assert!(f.ensure().is_ok());
    f.clock.set_wall(120);
    f.verifies(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 0);
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
    assert_eq!(f.journal().read_current_login().unwrap().unwrap().login_generation, 1);
    assert_eq!(
        f.lifecycle.status(),
        IdentityStatus::Active { account_id: "verified".to_owned(), login_generation: 1, token_generation: 2, expires_at_unix: 290 }
    );
}

fn block_transport(script: &Script) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    block_call(&script.on_send)
}

fn block_call(hook: &Mutex<Option<SendHook>>) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered, wait_for_send) = mpsc::channel();
    let (release, wait_for_release) = mpsc::channel();
    *hook.lock().unwrap() = Some(Box::new(move || {
        entered.send(()).unwrap();
        wait_for_release.recv_timeout(RACE_WAIT).unwrap();
    }));
    (wait_for_send, release)
}

#[cfg(unix)]
fn observe_background_contender(clock: &Clock) -> mpsc::Receiver<()> {
    let (entered, wait_for_entry) = mpsc::channel();
    *clock.on_background_before_lock.lock().unwrap() = Some(Box::new(move || entered.send(()).unwrap()));
    wait_for_entry
}

#[test]
fn a_concurrent_refresh_calls_the_transport_once() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    f.verifies(Some("binding"));
    let (sent, release) = block_transport(&f.script);

    let first = f.lifecycle.clone();
    let first = thread::spawn(move || first.ensure(IdentityTrigger::Background, RACE_WAIT));
    sent.recv_timeout(RACE_WAIT).unwrap();
    let (contended, wait_for_contention) = mpsc::channel();
    *f.clock.on_local_refresh_contention.lock().unwrap() = Some(Box::new(move || contended.send(()).unwrap()));
    let second = f.lifecycle.clone();
    let second = thread::spawn(move || second.ensure(IdentityTrigger::Background, RACE_WAIT));
    wait_for_contention.recv_timeout(RACE_WAIT).unwrap();
    release.send(()).unwrap();

    for call in [first, second] {
        let lease = call.join().unwrap().unwrap();
        assert_eq!((lease.login_generation(), lease.token_generation()), (1, 2));
        assert_eq!(lease.access_token().expose_secret(), "new-access");
    }
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn sign_out_during_a_refresh_discards_the_late_success() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    let (sent, release) = block_transport(&f.script);
    let refresh = f.lifecycle.clone();
    let refresh = thread::spawn(move || refresh.ensure(IdentityTrigger::Background, RACE_WAIT));
    sent.recv_timeout(RACE_WAIT).unwrap();

    let events_before = f.store.lock().unwrap().events.len();
    assert!(f.lifecycle.clone().sign_out().unwrap().credentials_cleared);
    release.send(()).unwrap();
    assert_eq!(refresh.join().unwrap().unwrap_err(), IdentityError::SignedOut);
    let store = f.store.lock().unwrap();
    assert!(store.record.is_none());
    assert_eq!(&store.events[events_before..], &[("delete", Some(1))]);
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn a_late_refresh_observes_peer_logout_while_cleanup_holds_the_barrier_lock() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    let (sent, release_response) = block_transport(&f.script);
    let refresh = f.lifecycle.clone();
    let refresh = thread::spawn(move || refresh.ensure(IdentityTrigger::Background, WAIT));
    sent.recv_timeout(RACE_WAIT).unwrap();

    let (deleting, wait_for_delete) = mpsc::channel();
    let (release_delete, wait_for_release) = mpsc::channel();
    f.store.lock().unwrap().on_delete = Some(Box::new(move || {
        deleting.send(()).unwrap();
        wait_for_release.recv_timeout(RACE_WAIT).unwrap();
    }));
    let peer = f.restarted(); // Separate lifecycle, with no shared local revocation flag.
    let logout = thread::spawn(move || peer.sign_out());
    wait_for_delete.recv_timeout(RACE_WAIT).unwrap();
    release_response.send(()).unwrap();
    let result = refresh.join().unwrap();
    release_delete.send(()).unwrap();
    assert!(logout.join().unwrap().unwrap().credentials_cleared);
    assert_eq!(result.unwrap_err(), IdentityError::SignedOut);
    assert!(f.store.lock().unwrap().record.is_none());
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn a_current_logout_is_resolved_without_a_store_read() {
    let f = Fixture::new();
    f.login().unwrap();
    f.lifecycle.sign_out().unwrap();
    let peer = f.restarted();
    let reads = {
        let mut store = f.store.lock().unwrap();
        store.unreadable = true;
        store.reads
    };
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::SignedOut);
    assert_eq!(peer.status(), IdentityStatus::SignedOut);
    assert_eq!(f.store.lock().unwrap().reads, reads);

    // A durable revocation is enough even when no current generation was published.
    std::fs::remove_file(f.root.join("identity/current.0")).unwrap();
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::SignedOut);
    assert_eq!(peer.status(), IdentityStatus::SignedOut);
    assert_eq!(f.store.lock().unwrap().reads, reads);
}

#[test]
fn a_newer_login_generation_can_still_reach_the_store_after_an_older_revocation() {
    let f = Fixture::new();
    f.login().unwrap();
    f.lifecycle.sign_out().unwrap();
    f.login().unwrap();
    let peer = f.restarted();
    let reads = {
        let mut store = f.store.lock().unwrap();
        store.unreadable = true;
        store.reads
    };
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Storage);
    assert_eq!(peer.status(), IdentityStatus::Recovering);
    {
        let mut store = f.store.lock().unwrap();
        assert_eq!(store.reads, reads + 2);
        store.unreadable = false;
    }
    f.verifies(None);
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap().login_generation(), 2);

    // Simulate an activated but unpublished store record alongside an unrevoked
    // journal generation; it must not become leaseable.
    f.store.lock().unwrap().record.as_mut().unwrap().login_generation = 3;
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Storage);
    assert_eq!(peer.status(), IdentityStatus::Recovering);
}

#[test]
fn a_torn_first_current_publication_can_be_repaired_by_a_new_login() {
    let f = Fixture::new();
    f.login().unwrap();
    let journal = f.journal();
    let slot = f.root.join("identity/current.0");
    std::fs::write(slot, b"intcad-identity-journal/1\n1\n").unwrap();
    assert!(matches!(journal.read_current_login(), Err(crate::identity_journal::JournalError::Corrupt)));

    let peer = f.restarted();
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Storage);
    assert_eq!(peer.status(), IdentityStatus::Recovering);

    f.restarted().adopt_login("provider", Secret::new("old-access"), Secret::new("old-refresh"), 100, 200, "binding").unwrap();
    assert_eq!(journal.read_current_login().unwrap().unwrap().login_generation, 2);
    f.verifies(None);
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap().login_generation(), 2);
    assert!(matches!(peer.status(), IdentityStatus::Active { login_generation: 2, .. }));
}

#[test]
fn an_activated_login_without_a_published_generation_is_not_leaseable() {
    let f = Fixture::new();
    f.login().unwrap();
    let target = f.root.join("identity/current.1");
    std::fs::create_dir(&target).unwrap(); // Block publication of login 2.
    assert_eq!(f.login().unwrap_err(), IdentityError::Storage);
    assert!(!f.record().pending);
    assert_eq!(f.record().login_generation, 2);
    assert_eq!(f.journal().read_current_login().unwrap().unwrap().login_generation, 1);

    let peer = f.restarted();
    let reads = f.store.lock().unwrap().reads;
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::SignedOut);
    assert_eq!(peer.status(), IdentityStatus::SignedOut);
    assert_eq!(f.store.lock().unwrap().reads, reads);

    std::fs::remove_dir(target).unwrap();
    f.login().unwrap();
    f.verifies(None);
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap().login_generation(), 3);
}

#[cfg(unix)]
mod process {
    use super::*;
    use serde_json::{Value, json};
    use std::fs::{self, File, OpenOptions, TryLockError};
    use std::io::ErrorKind;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    const POLL: Duration = Duration::from_millis(10);

    #[derive(Clone)]
    struct FileStore {
        path: PathBuf,
        unavailable: bool,
    }

    impl FileStore {
        fn mutation_lock(&self) -> File {
            let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(self.path.with_extension("lock")).unwrap();
            file.lock().unwrap();
            file
        }

        fn write(&self, record: &StoredIdentity, pending: bool) {
            let _lock = self.mutation_lock();
            let data = json!({
                "version": record.version,
                "domain": record.domain,
                "login_generation": record.login_generation,
                "token_generation": record.token_generation,
                "provider_account_id": record.provider_account_id,
                "verified_account_id": record.verified_account_id,
                "access_token": record.access_token,
                "refresh_token": record.refresh_token,
                "issued_at_unix": record.issued_at_unix,
                "expires_at_unix": record.expires_at_unix,
                "session_binding": record.session_binding,
                "pending": pending,
            });
            let temp = self.path.with_extension("tmp");
            fs::write(&temp, data.to_string()).unwrap();
            fs::rename(temp, &self.path).unwrap();
        }
    }

    fn field<T: serde::de::DeserializeOwned>(data: &Value, name: &str) -> T {
        serde_json::from_value(data[name].clone()).unwrap()
    }

    impl IdentityStore for FileStore {
        fn load(&self) -> Result<Option<StoredIdentity>, IdentityStoreError> {
            if self.unavailable {
                use std::io::Write as _;
                OpenOptions::new().append(true).create(true).open(self.path.with_extension("reads")).unwrap().write_all(b"1\n").unwrap();
                return Err(IdentityStoreError::Unavailable);
            }
            let data = match fs::read(&self.path) {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap(),
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
                Err(error) => panic!("read test store: {error}"),
            };
            Ok(Some(StoredIdentity {
                version: field(&data, "version"),
                domain: field(&data, "domain"),
                login_generation: field(&data, "login_generation"),
                token_generation: field(&data, "token_generation"),
                provider_account_id: field(&data, "provider_account_id"),
                verified_account_id: field(&data, "verified_account_id"),
                access_token: field(&data, "access_token"),
                refresh_token: field(&data, "refresh_token"),
                issued_at_unix: field(&data, "issued_at_unix"),
                expires_at_unix: field(&data, "expires_at_unix"),
                session_binding: field(&data, "session_binding"),
                pending: field(&data, "pending"),
            }))
        }

        fn store_pending(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError> {
            self.write(record, true);
            Ok(())
        }

        fn activate(&mut self, record: &StoredIdentity) -> Result<(), IdentityStoreError> {
            self.write(record, false);
            Ok(())
        }

        fn delete(&mut self) -> Result<(), IdentityStoreError> {
            let _lock = self.mutation_lock();
            match fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => panic!("delete test store: {error}"),
            }
            Ok(())
        }
    }

    struct FileScript(PathBuf);

    impl IdentityTransport for FileScript {
        fn refresh(&self, client_id: &str, token: &Secret) -> Reply {
            assert_eq!(client_id, "client");
            let expected = if self.0.join("expect-rotated").exists() { "new-refresh" } else { "old-refresh" };
            assert!(token.expose_secret() == expected);
            let consumed = self.0.join("response.consumed");
            // The rename consumes the only response and tells the parent we entered transport.
            fs::rename(self.0.join("response.json"), &consumed).unwrap();
            let started = Instant::now();
            while !self.0.join("response.release").exists() {
                assert!(started.elapsed() < RACE_WAIT, "parent did not release the response");
                thread::sleep(POLL);
            }
            let data: Value = serde_json::from_slice(&fs::read(consumed).unwrap()).unwrap();
            Ok(RefreshResponse {
                access_token: Secret::new(field::<String>(&data, "access_token")),
                refresh_token: Secret::new(field::<String>(&data, "refresh_token")),
                provider_account_id: field(&data, "provider_account_id"),
                expires_in: Some(Duration::from_secs(field(&data, "expires_in"))),
            })
        }
    }

    impl SessionVerifier for FileScript {
        fn verify(&self, token: &Secret) -> Verify {
            assert!(token.expose_secret() == "new-access");
            if self.0.join("stop-before-verification").exists() {
                signal_and_pause(&self.0);
            }
            Ok(VerifiedSession { account_id: "verified".to_owned(), same_login_binding: Some("binding".to_owned()) })
        }
    }

    fn lifecycle(root: &Path, store: FileStore, clock: Arc<Clock>) -> IdentityLifecycle {
        IdentityLifecycle::with_identity_store(
            IdentityDomain::new("test", "client", "https://product.example").unwrap(),
            Box::new(store),
            Box::new(FileScript(root.to_owned())),
            Box::new(FileScript(root.to_owned())),
            Box::new(clock),
            root.to_owned(),
        )
        .unwrap()
    }

    struct Peer(Option<Child>, Instant);

    impl Peer {
        fn spawn(root: &Path, mode: &str, store: &Path) -> Self {
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "identity_tests::child_process_helper", "--nocapture"])
                .env("INTCAD_IDENTITY_CHILD_ROOT", root)
                .env("INTCAD_IDENTITY_CHILD_MODE", mode)
                .env("INTCAD_IDENTITY_CHILD_STORE", store)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            Self(Some(child), Instant::now())
        }

        fn exited(&mut self) -> bool {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                return true;
            }
            assert!(self.1.elapsed() < RACE_WAIT, "identity child timed out");
            false
        }

        fn status(mut self) -> Value {
            while !self.exited() {
                thread::sleep(POLL);
            }
            let output = self.0.take().unwrap().wait_with_output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(output.status.success(), "identity child failed: {stdout}\n{}", String::from_utf8_lossy(&output.stderr));
            let mut lines = stdout.lines().filter_map(|line| serde_json::from_str(line).ok());
            let status = lines.next().unwrap_or_else(|| panic!("no JSON status in child output: {stdout}"));
            assert!(lines.next().is_none(), "more than one JSON status line: {stdout}");
            status
        }

        fn kill_at_signal(mut self, root: &Path) {
            while !root.join("kill-ready").exists() {
                assert!(!self.exited(), "child exited before the designated crash point");
                thread::sleep(POLL);
            }
            let mut child = self.0.take().unwrap();
            child.kill().unwrap();
            use std::os::unix::process::ExitStatusExt as _;
            assert!(child.wait().unwrap().signal().is_some());
        }
    }

    impl Drop for Peer {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    pub(super) fn child(root: PathBuf) {
        let mode = std::env::var("INTCAD_IDENTITY_CHILD_MODE").unwrap();
        assert!(matches!(mode.as_str(), "refresh" | "barrier" | "crash-locked" | "crash-pending"));
        let path = std::env::var_os("INTCAD_IDENTITY_CHILD_STORE").unwrap().into();
        let clock = Arc::new(Clock::default());
        clock.set_wall(170);
        if mode == "crash-locked" {
            let root = root.clone();
            *clock.on_refresh_locked.lock().unwrap() = Some(Box::new(move || signal_and_pause(&root)));
        }
        let lifecycle = lifecycle(&root, FileStore { path, unavailable: mode == "barrier" }, clock);
        let ensured = lifecycle.ensure(IdentityTrigger::Background, RACE_WAIT);
        let status = match (&ensured, mode.as_str()) {
            // A lease proves activation. A zero-budget status probe could race the parent.
            (Ok(lease), "refresh") => json!({"state": "Active", "token_generation": lease.token_generation()}),
            _ => json!({"state": format!("{:?}", lifecycle.status())}),
        };
        let ensure = match ensured {
            Ok(lease) => json!({"token_generation": lease.token_generation()}),
            Err(error) => json!(format!("{error:?}")),
        };
        println!("{}", json!({"status": status, "ensure": ensure}));
    }

    fn signal_and_pause(root: &Path) -> ! {
        fs::write(root.join("kill-ready"), []).unwrap();
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < RACE_WAIT, "parent did not kill child");
            thread::sleep(POLL);
        }
    }

    fn seed(f: &Fixture) -> (IdentityLifecycle, FileStore) {
        let store = FileStore { path: f.root.join("test-store.json"), unavailable: false };
        let parent = lifecycle(&f.root, store.clone(), f.clock.clone());
        parent.adopt_login("provider", Secret::new("old-access"), Secret::new("old-refresh"), 100, 200, "binding").unwrap();
        f.due();
        (parent, store)
    }

    pub(super) fn refresh() {
        let f = Fixture::new();
        let (parent, store) = seed(&f);
        let script = json!({"access_token": "new-access", "refresh_token": "new-refresh", "provider_account_id": "provider", "expires_in": 120});
        fs::write(f.root.join("response.json"), script.to_string()).unwrap();
        let mut child = Peer::spawn(&f.root, "refresh", &store.path);
        let consumed = f.root.join("response.consumed");
        while !consumed.exists() {
            assert!(!child.exited(), "identity child exited before it entered transport");
            thread::sleep(POLL);
        }

        // A separate process holds the actual domain file lock throughout the exchange.
        let lock = OpenOptions::new().read(true).write(true).open(f.root.join("identity/refresh.lock")).unwrap();
        assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
        let entered = observe_background_contender(&f.clock);
        let clock = f.clock.clone();
        let parent = thread::spawn(move || {
            let first = parent.ensure(IdentityTrigger::Background, RACE_WAIT);
            if matches!(first, Err(IdentityError::Transient)) {
                clock.set_wall(171);
                clock.set_monotonic(71);
                parent.ensure(IdentityTrigger::Background, RACE_WAIT)
            } else {
                first
            }
        });
        entered.recv_timeout(RACE_WAIT).unwrap();
        fs::write(f.root.join("response.release"), []).unwrap();

        let lease = parent.join().unwrap().unwrap();
        assert_eq!(lease.token_generation(), 2);
        assert_eq!(lease.access_token().expose_secret(), "new-access");
        assert_eq!(child.status(), json!({"status":{"state":"Active","token_generation":2},"ensure":{"token_generation":2}}));
        assert!(!f.root.join("response.json").exists());
        assert_eq!(fs::read_to_string(consumed).unwrap(), script.to_string());
        let record = store.load().unwrap().unwrap();
        assert!(!record.pending);
        assert_eq!((record.login_generation, record.token_generation), (1, 2));
        assert_eq!(record.refresh_token, "new-refresh");
    }

    pub(super) fn barrier() {
        let f = Fixture::new();
        let (parent, store) = seed(&f);
        assert_eq!(Peer::spawn(&f.root, "barrier", &store.path).status(), json!({"status":{"state":"Recovering"},"ensure":"Storage"}));
        let reads = store.path.with_extension("reads");
        assert_eq!(fs::read_to_string(&reads).unwrap().lines().count(), 2);
        fs::remove_file(&reads).unwrap();
        assert!(parent.sign_out().unwrap().credentials_cleared);
        assert_eq!(Peer::spawn(&f.root, "barrier", &store.path).status(), json!({"status":{"state":"SignedOut"},"ensure":"SignedOut"}));
        assert!(!reads.exists(), "a logged-out peer must not read credentials");
    }

    pub(super) fn killed(pending: bool) {
        let f = Fixture::new();
        let (parent, store) = seed(&f);
        let script = json!({"access_token": "new-access", "refresh_token": "new-refresh", "provider_account_id": "provider", "expires_in": 120});
        fs::write(f.root.join("response.json"), script.to_string()).unwrap();
        fs::write(f.root.join("response.release"), []).unwrap();
        let mode = if pending {
            fs::write(f.root.join("stop-before-verification"), []).unwrap();
            "crash-pending"
        } else {
            "crash-locked"
        };
        Peer::spawn(&f.root, mode, &store.path).kill_at_signal(&f.root);
        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.pending, pending);
        assert_eq!(saved.token_generation, if pending { 2 } else { 1 });
        assert_eq!(parent.ensure(IdentityTrigger::Background, Duration::ZERO).unwrap_err(), IdentityError::Transient);
        if pending {
            assert!(matches!(parent.status(), IdentityStatus::PendingVerification { .. }));
            fs::remove_file(f.root.join("stop-before-verification")).unwrap();
            fs::write(f.root.join("expect-rotated"), []).unwrap();
            fs::write(f.root.join("response.json"), script.to_string()).unwrap();
            f.clock.set_wall(saved.expires_at_unix);
        } else {
            assert!(!f.root.join("response.consumed").exists());
        }
        let lease = parent.ensure(IdentityTrigger::Background, WAIT).unwrap();
        assert_eq!(lease.token_generation(), if pending { 3 } else { 2 });
        assert!(!f.root.join("response.json").exists());
    }
}

#[cfg(unix)]
#[test]
fn child_process_helper() {
    let Some(root) = std::env::var_os("INTCAD_IDENTITY_CHILD_ROOT") else {
        return;
    };
    process::child(root.into());
}

#[cfg(unix)]
#[test]
fn a_second_process_shares_the_file_lock_and_reuses_the_new_record() {
    process::refresh();
}

#[cfg(unix)]
#[test]
fn a_second_process_sees_the_logout_barrier_without_keyring() {
    process::barrier();
}

#[cfg(unix)]
#[test]
fn process_death_after_refresh_lock_releases_it_without_a_stale_lease() {
    process::killed(false);
}

#[cfg(unix)]
#[test]
fn process_death_after_pending_commit_recovers_with_the_rotated_refresh() {
    process::killed(true);
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
    f.clock.set_wall(171);
    f.clock.set_monotonic(71);
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
    f.login().unwrap(); // Retires login 1 in revoked.1, leaving revoked.0 as the next write target.
    f.due();
    f.reply(Err(RefreshTransportError::Terminal));
    let root = f.root.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        std::fs::remove_file(root.join("identity/revoked.0")).unwrap();
        std::fs::create_dir(root.join("identity/revoked.0")).unwrap();
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
fn losing_either_revocation_slot_after_logout_cannot_resurrect_failed_cleanup() {
    for slot in ["revoked.0", "revoked.1"] {
        let f = Fixture::new();
        f.login().unwrap();
        f.store.lock().unwrap().fail_delete = true;
        assert!(!f.lifecycle.sign_out().unwrap().credentials_cleared);
        std::fs::remove_file(f.root.join("identity").join(slot)).unwrap();
        let peer = f.restarted();
        assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::SignedOut);
        assert_eq!(peer.status(), IdentityStatus::SignedOut);
    }
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
        assert_eq!(attempt.cooldown_until_unix, if expected == IdentityError::Protocol { 175 } else { 171 });
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
fn sign_out_revokes_the_published_generation_even_if_the_keyring_is_locked() {
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
    let events_before = f.store.lock().unwrap().events.len();
    f.store.lock().unwrap().unreadable = true;
    let outcome = f.lifecycle.sign_out();
    f.store.lock().unwrap().unreadable = false;
    assert_eq!(outcome, Ok(SignOutOutcome { credentials_cleared: false }));
    assert_eq!(f.journal().read_revocation().unwrap().unwrap().login_generation, 2);
    assert_eq!(f.store.lock().unwrap().events.len(), events_before);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::SignedOut);
    assert_eq!(other.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::SignedOut);
}

#[test]
fn commit_holding_the_barrier_precedes_logout_and_later_commits_are_rejected() {
    let f = Fixture::new();
    f.login().unwrap();
    let generation = f.ensure().unwrap().login_generation();
    let (entered, wait_for_commit) = mpsc::channel();
    let (release, wait_for_release) = mpsc::channel();
    let host = f.restarted();
    let commit = thread::spawn(move || {
        host.with_barrier_lock(generation, WAIT, || {
            entered.send(()).unwrap();
            wait_for_release.recv_timeout(RACE_WAIT).unwrap();
            "committed"
        })
    });
    wait_for_commit.recv_timeout(RACE_WAIT).unwrap();
    let peer = f.restarted();
    // This is another process's barrier ordering, not a shared local flag.
    assert_eq!(peer.with_barrier_lock(generation, Duration::ZERO, || ()), Err(IdentityError::Busy));
    release.send(()).unwrap();
    assert_eq!(commit.join().unwrap().unwrap(), "committed");
    peer.sign_out().unwrap();
    assert_eq!(f.restarted().with_barrier_lock(generation, WAIT, || panic!("revoked commit ran")), Err::<(), _>(IdentityError::SignedOut));
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
    f.clock.set_wall(101);
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
    f.attempt(1, false, 100, 0);
    f.clock.set_wall(170);
    f.clock.set_monotonic(31); // A wall jump cannot restart the original replay clock.
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    f.attempt(2, true, 175, 1);
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
        f.attempt(3, true, now + 5, u8::MAX);
        assert_eq!(f.record().expires_at_unix, 200);
    }
}

#[test]
fn insufficient_caller_budget_never_sends() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    for budget in [Duration::ZERO, Duration::from_millis(4999)] {
        assert_eq!(f.lifecycle.ensure(IdentityTrigger::Background, budget).unwrap_err(), IdentityError::Transient);
    }
    assert_eq!(f.refreshes(), 0);
    assert!(f.journal().read_attempt().unwrap().is_none());
}

#[test]
fn first_reservation_must_leave_time_for_a_complete_request() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    let clock = f.clock.clone();
    *f.clock.on_attempt_write.lock().unwrap() = Some(Box::new(move || clock.set_monotonic(86)));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 0);
    assert_eq!(f.journal().read_attempt().unwrap().unwrap().sends, 1);
}

#[test]
fn protocol_failure_needs_explicit_retry_even_in_a_peer() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.reply(Err(RefreshTransportError::Protocol));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Protocol);
    f.clock.set_wall(175);
    f.clock.set_monotonic(75);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Protocol);
    assert_eq!(f.restarted().ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Protocol);
    assert_eq!(f.refreshes(), 1);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    assert_eq!(f.record().token_generation, 1);
}

#[test]
fn residual_protocol_failure_is_persisted_while_the_barrier_is_held() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.reply(Err(RefreshTransportError::Protocol));
    let (sent, release) = block_transport(&f.script);
    let background = f.lifecycle.clone();
    let background = thread::spawn(move || background.ensure(IdentityTrigger::Background, RACE_WAIT));
    sent.recv_timeout(RACE_WAIT).unwrap();
    let peer = f.restarted();
    let (result, marker) = peer
        .with_barrier_lock(1, WAIT, || {
            release.send(()).unwrap();
            let result = background.join().unwrap();
            (result, f.journal().read_attempt().unwrap().unwrap())
        })
        .unwrap();
    assert!(matches!(result, Err(IdentityError::Busy | IdentityError::Transient)));
    assert_eq!((marker.sends, marker.diagnostic_used, marker.cooldown_until_unix), (3, true, 175));

    f.clock.set_wall(175);
    f.clock.set_monotonic(75);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Protocol);
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn status_attempt_is_throttled_across_resume_background_and_restart() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.success();
    f.verifies(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::VerificationPending);
    f.verifies(Some("binding"));
    let peer = f.restarted();
    assert_eq!(peer.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.script.checks.lock().unwrap().len(), 1);
    f.clock.set_wall(171);
    f.clock.set_monotonic(71);
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap().token_generation(), 2);
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn status_retry_after_does_not_consume_workos_sends() {
    let f = Fixture::new();
    f.login().unwrap();
    f.script.checks.lock().unwrap().push_back(Err(VerifyError::RetryAfter(Duration::from_secs(90))));
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    f.due();
    f.success();
    f.verifies(Some("binding"));
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert!(f.record().pending);
    assert_eq!(f.refreshes(), 1);
    assert!(f.journal().read_attempt().unwrap().is_none());
    f.clock.set_wall(190);
    assert_eq!(f.ensure().unwrap().token_generation(), 2);
    assert_eq!(f.refreshes(), 1);
}

#[test]
fn residual_status_backoff_starts_at_failure_even_after_a_full_timeout() {
    let f = Fixture::new();
    f.login().unwrap();
    let peer = f.restarted();
    for (start, finish, delay) in [(0, 5, 1), (6, 11, 2), (13, 18, 5)] {
        f.clock.set_wall(100 + start);
        f.clock.set_monotonic(start);
        f.script.checks.lock().unwrap().push_back(Err(VerifyError::Transient));
        let clock = f.clock.clone();
        *f.script.on_verify.lock().unwrap() = Some(Box::new(move || {
            clock.set_wall(100 + finish);
            clock.set_monotonic(finish);
        }));
        assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Transient);
        assert!(f.script.checks.lock().unwrap().is_empty(), "the next request is allowed when its cooldown expires");
        f.script.checks.lock().unwrap().push_back(Err(VerifyError::Transient));
        for time in [finish, finish + delay - 1] {
            f.clock.set_wall(100 + time);
            f.clock.set_monotonic(time);
            assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Transient);
            assert_eq!(f.script.checks.lock().unwrap().len(), 1, "status retried before its failure-based cooldown");
        }
        f.script.checks.lock().unwrap().clear();
    }
    f.clock.set_wall(123);
    f.clock.set_monotonic(23);
    f.verifies(None);
    assert_eq!(peer.ensure(IdentityTrigger::Background, WAIT).unwrap().token_generation(), 1);
}

#[test]
fn the_third_send_inside_the_batch_budget_is_the_last() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    for (sends, wall, mono, until, step) in [(1, 170, 70, 171, 0), (2, 171, 71, 173, 0), (3, 173, 73, 178, 1)] {
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
        f.attempt(sends, false, until, step);
        if sends < 3 {
            assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
            assert_eq!(f.refreshes(), u64::from(sends));
        }
    }
    f.clock.set_wall(205); // Cooldown expired, but only 16 monotonic seconds have elapsed.
    f.clock.set_monotonic(86);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 3);
    f.attempt(3, false, 178, 1);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 4);
    assert_eq!(f.attempt(3, true, 215, 2).first_possible_send_unix, 205);
    f.clock.set_wall(245);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 4);
    f.attempt(3, true, 215, 2);
}

#[test]
fn cooldown_blocks_automatic_retry_and_honours_retry_after() {
    for retry_after in [2, 90] {
        let f = Fixture::new();
        f.login().unwrap();
        f.due();
        f.transient(Some(retry_after));
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        let step = u8::from(retry_after == 90);
        f.attempt(1, false, 170 + retry_after, step);
        f.clock.set_monotonic(71);
        f.clock.set_wall(169 + retry_after);
        f.reply(Err(RefreshTransportError::Protocol));
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), 1);
        f.attempt(1, false, 170 + retry_after, step);
        f.clock.set_wall(170 + retry_after); // Only wall time decides when cooldown ends.
        let before = f.record();
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Protocol);
        assert_eq!(f.refreshes(), 2);
        f.attempt(3, true, 175 + retry_after + u64::from(step) * 5, u8::MAX);
        assert_eq!(f.record(), before);
    }
}

#[test]
fn a_same_process_replay_is_allowed_only_inside_the_thirty_second_window() {
    // The 20s batch is tighter than the provider's 30s window.
    for (elapsed, sends, reserved, diagnostic, until, step) in
        [(15, 2, 2, false, 180, 1), (19, 1, 2, false, 180, 0), (20, 1, 1, false, 171, 0), (26, 2, 2, true, 180, 1), (31, 2, 2, true, 180, 1)]
    {
        let f = Fixture::new();
        f.login().unwrap();
        f.due();
        f.transient(None);
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        let first = f.attempt(1, false, 171, 0);
        f.clock.set_wall(175);
        f.clock.set_monotonic(70 + elapsed);
        f.transient(None);
        assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), sends);
        let second = f.attempt(reserved, diagnostic, until, step);
        assert_eq!(second.attempt_id, first.attempt_id);
        assert_eq!(second.first_possible_send_unix, 170);
    }
}

#[test]
fn the_provider_replay_window_includes_exactly_one_request_at_its_boundary() {
    assert!(fits_replay_window(Duration::from_secs(25)));
    assert!(!fits_replay_window(Duration::from_millis(25_001)));
}

#[test]
fn a_restarted_attempt_gets_one_diagnostic_not_a_new_batch() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let first = f.attempt(1, false, 171, 0);
    let other = f.restarted();
    f.clock.set_wall(175);
    let root = f.root.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        let marker = IdentityJournal::new(root).unwrap().read_attempt().unwrap().unwrap();
        assert_eq!((marker.sends, marker.diagnostic_used), (2, true));
    }));
    f.transient(None);
    assert_eq!(other.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Transient);
    let diagnostic = f.attempt(2, true, 180, 1);
    assert_eq!(diagnostic.attempt_id, first.attempt_id);
    assert_eq!(diagnostic.first_possible_send_unix, 170);
    assert_eq!(f.refreshes(), 2);
    f.clock.set_wall(185);
    f.transient(None);
    for lifecycle in [f.restarted(), other, f.lifecycle.clone()] {
        assert_eq!(lifecycle.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), 2);
        f.attempt(2, true, 180, 1);
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
    let first = f.attempt(1, false, 171, 0); // Fresh UserRetry starts a normal batch.
    f.clock.set_wall(175);
    f.clock.set_monotonic(101);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    f.attempt(2, true, 180, 1);
    f.clock.set_wall(179);
    f.transient(None);
    assert_eq!(user(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    f.attempt(2, true, 180, 1);
    f.clock.set_wall(185);
    assert_eq!(user(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 3);
    let last = f.attempt(3, true, 195, 2);
    assert_eq!(last.attempt_id, first.attempt_id);
    assert_eq!(last.first_possible_send_unix, 185);
    f.clock.set_wall(205);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(user(), IdentityError::Transient);
    assert_eq!(f.restarted().ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 3);
    assert_eq!(f.attempt(3, true, 195, 2), last);
}

#[test]
fn a_user_retry_inside_the_replay_window_still_consumes_the_one_shot() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    f.attempt(1, false, 171, 0);

    f.clock.set_wall(175);
    f.clock.set_monotonic(75);
    let root = f.root.clone();
    *f.script.on_send.lock().unwrap() = Some(Box::new(move || {
        let attempt = IdentityJournal::new(root).unwrap().read_attempt().unwrap().unwrap();
        assert_eq!((attempt.sends, attempt.diagnostic_used, attempt.first_possible_send_unix), (3, true, 175));
    }));
    f.transient(None);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::UserRetry, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    let attempt = f.attempt(3, true, 180, 1);

    f.clock.set_wall(185);
    f.clock.set_monotonic(85);
    for trigger in [IdentityTrigger::Background, IdentityTrigger::UserRetry, IdentityTrigger::Resume] {
        assert_eq!(f.lifecycle.ensure(trigger, WAIT).unwrap_err(), IdentityError::Transient);
        assert_eq!(f.refreshes(), 2);
        assert_eq!(f.attempt(3, true, 180, 1), attempt);
    }
}

#[test]
fn resume_cannot_replay_a_background_attempt_started_before_it_acquires_the_refresh_lock() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(Some(0));
    f.transient(None);
    let lifecycle = f.lifecycle.clone();
    let root = f.root.clone();
    let clock = f.clock.clone();
    *f.clock.on_resume_before_lock.lock().unwrap() = Some(Box::new(move || {
        assert_eq!(lifecycle.ensure(IdentityTrigger::Background, WAIT).unwrap_err(), IdentityError::Transient);
        let attempt = IdentityJournal::new(root).unwrap().read_attempt().unwrap().unwrap();
        assert_eq!((attempt.sends, attempt.diagnostic_used, attempt.cooldown_until_unix), (1, false, 171));
        clock.set_wall(171);
    }));
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    f.attempt(2, true, 176, 1);
    f.clock.set_wall(176);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
}

#[test]
fn a_replay_reservation_crossing_the_batch_deadline_is_consumed_without_sending() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let first = f.attempt(1, false, 171, 0);

    f.clock.set_wall(175);
    f.clock.millis.store(89_900, Ordering::Relaxed);
    let clock = f.clock.clone();
    let root = f.root.clone();
    *f.clock.on_attempt_write.lock().unwrap() = Some(Box::new(move || {
        let attempt = IdentityJournal::new(root).unwrap().read_attempt().unwrap().unwrap();
        assert_eq!((attempt.sends, attempt.diagnostic_used), (2, false));
        clock.set_monotonic(90);
    }));
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 1);
    let reserved = f.attempt(2, false, 180, 0);
    assert_eq!(reserved.attempt_id, first.attempt_id);
    assert_eq!(reserved.first_possible_send_unix, 170);
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
        f.attempt(1, false, 171, 0);
    }
}

#[test]
fn resume_treats_the_window_as_unprovable_and_does_not_reset_the_batch() {
    let f = Fixture::new();
    f.login().unwrap();
    f.due();
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let first = f.attempt(1, false, 171, 0);
    f.clock.set_wall(170);
    f.clock.set_monotonic(74);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 1);
    assert_eq!(f.attempt(1, false, 171, 0), first);
    f.clock.set_wall(175);
    f.transient(None);
    assert_eq!(f.ensure().unwrap_err(), IdentityError::Transient);
    let last = f.attempt(2, true, 180, 1);
    assert_eq!(last.first_possible_send_unix, 170);
    assert_eq!(last.attempt_id, first.attempt_id);
    f.clock.set_wall(185);
    f.transient(None);
    assert_eq!(f.lifecycle.ensure(IdentityTrigger::Resume, WAIT).unwrap_err(), IdentityError::Transient);
    assert_eq!(f.refreshes(), 2);
    assert_eq!(f.attempt(2, true, 180, 1), last);
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
