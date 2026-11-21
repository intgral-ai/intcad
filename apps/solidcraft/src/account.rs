//! Desktop account composition root (Task 11).
//!
//! The runtime owns the one credential session, the durable operation ledger, the conservative
//! budget and the monotonic clock, and installs the UI account port. It never opens a dialog or
//! the network on a render pass.
//!
//! Fail closed: while no documented `ProductSessionExchange` and no authenticated durable
//! settlement adapter exist, the desktop does **not** install production billing or commit
//! authority and does not turn a WorkOS provider token into an Intcad product session. Ordinary
//! CAD stays free and anonymous; controlled entries keep their existing default refusal. A secure
//! store is probed before any session is restored, and sign-out or account switch ends the old
//! session's authority immediately.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use solidcraft_account_native::{CredentialSession, FileOperationStore};
use solidcraft_engine::entitlement::{AccountService, AuthenticatedHostScope, BudgetTable, MonotonicClock, MonotonicInstant, Refusal, RefusalCode};
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::account::{AccountPort, AccountView, DeleteAccountGuide, DeviceCode, LifecycleEvent, OwnModelView, State};

/// Account facts read once before the window exists.
#[derive(Clone, Debug)]
pub struct AccountStartup {
    secure_store: bool,
    /// A durable sign-out marker is present, or its state could not be read. Either way startup
    /// refuses to restore or admit anything until a reviewed sign-in path clears it.
    logout_pending: bool,
    restored: Option<AuthenticatedHostScope>,
}

/// The host-owned account runtime.
///
/// `service` stays `None` until a documented and reviewed account service plus an authenticated
/// durable settlement adapter can be installed. The field is kept as the single composition seam
/// so no second, competing constructor can appear later.
pub struct AccountRuntime {
    credentials: CredentialSession,
    ledger: Option<FileOperationStore>,
    /// Measured safe-stop bounds for controlled entries. Retained unverified until a reviewed
    /// billing adapter and its evidence exist; nothing in this task makes a bound verified.
    #[allow(dead_code)]
    budget: BudgetTable,
    clock: Option<Box<dyn MonotonicClock + Send + Sync>>,
    service: Option<Box<dyn AccountService + Send + Sync>>,
}

impl AccountRuntime {
    /// The production desktop runtime: the operating-system credential store and the durable
    /// ledger under the settings folder. Both are optional; a missing one fails closed.
    pub fn desktop(settings: Option<&Path>) -> Self {
        let credentials = crate::keyring_store::account_credential_session();
        let ledger = settings.and_then(|directory| match FileOperationStore::open(directory.join("account")) {
            Ok(store) => Some(store),
            Err(error) => {
                solidcraft_engine::note!("account: the durable operation ledger could not be opened ({error})");
                None
            }
        });
        Self::new(credentials, ledger)
    }

    /// A test-only runtime over a caller-supplied credential store.
    #[cfg(test)]
    pub fn with_store<S: solidcraft_engine::entitlement::CredentialStore + Send + 'static>(store: S) -> Self {
        Self::new(CredentialSession::new(Box::new(store)), None)
    }

    /// A test-only runtime over an already composed credential session.
    #[cfg(test)]
    pub fn with_session(credentials: CredentialSession) -> Self {
        Self::new(credentials, None)
    }

    fn new(credentials: CredentialSession, ledger: Option<FileOperationStore>) -> Self {
        Self { credentials, ledger, budget: BudgetTable::unverified_default(), clock: Some(Box::new(SystemMonotonicClock::new())), service: None }
    }

    /// The host's authenticated account, installation and session binding, or `None`.
    pub fn authenticated_scope(&self) -> Option<AuthenticatedHostScope> {
        self.credentials.authenticated_scope()
    }

    /// A session is restored only when the secure store is usable, a product session is active
    /// and the authenticated account, installation and session binding are provable.
    #[cfg(test)]
    pub fn restored_session(&self) -> Option<AuthenticatedHostScope> {
        self.restored_with_store(self.credentials.is_persistent())
    }

    /// Read the secure-store facts once, before the window exists, so `install` does no keyring
    /// work on the window's thread.
    pub fn startup(&self) -> AccountStartup {
        let secure_store = self.credentials.is_persistent();
        // An unreadable tombstone is treated as present: revocation/safety cannot be confirmed.
        let logout_pending = self.credentials.logout_pending().unwrap_or(true);
        let restored = if logout_pending { None } else { self.restored_with_store(secure_store) };
        AccountStartup { secure_store, logout_pending, restored }
    }

    fn restored_with_store(&self, secure_store: bool) -> Option<AuthenticatedHostScope> {
        let scope = self.authenticated_scope();
        crate::startup::can_restore_account_session(
            secure_store,
            self.credentials.product_session_active(),
            scope.as_ref().is_some_and(AuthenticatedHostScope::is_provable),
        )
        .then_some(scope)
        .flatten()
    }

    /// The current safe-stop bounds, still unverified in this task.
    #[allow(dead_code)]
    pub fn budget(&self) -> &BudgetTable {
        &self.budget
    }

    /// Take the installed account service and clock only when a provable session backs them.
    fn take_commit_authority(&mut self) -> Option<(AuthenticatedHostScope, Box<dyn AccountService + Send>, Box<dyn MonotonicClock + Send>)> {
        // Check the service first: with no reviewed adapter this task performs no credential
        // read while the window is being created.
        let service = self.service.take()?;
        let clock = self.clock.take()?;
        let scope = self.authenticated_scope()?;
        Some((scope, service, clock))
    }
}

fn account_unavailable(message: &'static str) -> Refusal {
    Refusal::new(RefusalCode::AuthUnavailable, message)
}

/// A monotonic clock for the runtime. It counts this process's elapsed time; suspend handling and
/// a wall-clock diagnostic are deliberately separate from admission until a reviewed adapter
/// installs real commit authority.
struct SystemMonotonicClock {
    start: Instant,
}

impl SystemMonotonicClock {
    fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl MonotonicClock for SystemMonotonicClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_millis(self.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// The desktop implementation of the UI account seam.
///
/// No method opens the network or starts a provider flow. Sign-in is an explicit action and still
/// fails closed until a documented product-session exchange exists. The view is cached; the
/// render path never calls the port.
#[derive(Default)]
struct CleanupStatus {
    /// The latest revocation generation whose durable cleanup has not finished.
    pending: Option<u64>,
    /// The latest generation's cleanup error; the session stays fail-closed until a reviewed
    /// sign-in path can repair it.
    error: Option<Refusal>,
}

struct DesktopAccountPort {
    runtime: AccountRuntime,
    view: AccountView,
    lifecycle: Option<LifecycleEvent>,
    /// `true` until a reviewed sign-in path has cleared a durable logout marker. It keeps the
    /// failed-cleanup identity fail-closed across this process too.
    logout_pending: bool,
    secure_store: bool,
    generation: u64,
    cleanup: Arc<Mutex<CleanupStatus>>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl DesktopAccountPort {
    fn new(runtime: AccountRuntime, startup: &AccountStartup) -> Self {
        let view = current_view(startup);
        Self {
            runtime,
            view,
            lifecycle: None,
            logout_pending: startup.logout_pending,
            secure_store: startup.secure_store,
            generation: 0,
            cleanup: Arc::new(Mutex::new(CleanupStatus::default())),
            workers: Vec::new(),
        }
    }

    /// Refuse account work while a durable sign-out is unconfirmed, a cleanup is running or its
    /// latest generation failed. An unreadable status is itself a refusal.
    fn cleanup_confirmed(&self) -> Result<(), Refusal> {
        if self.logout_pending {
            return Err(account_unavailable("a previous sign-out is not yet confirmed"));
        }
        let status = self.cleanup.lock().map_err(|_| account_unavailable("the account cleanup state is unavailable"))?;
        if status.pending.is_some() {
            return Err(account_unavailable("the sign-out cleanup is still running"));
        }
        if let Some(error) = &status.error {
            return Err(error.clone());
        }
        if self.runtime.credentials.is_revoked() {
            return Err(account_unavailable("the account authority is revoked"));
        }
        Ok(())
    }

    /// Revoke authority in this process immediately and run the durable tombstone + delete on a
    /// worker. The worker's result is applied only for its own generation, so a late result from
    /// an older switch can never clear a newer cleanup state.
    fn revoke(&mut self, event: LifecycleEvent) -> Result<(), Refusal> {
        // Synchronous, lock-free in-process revocation: no reader can take the credential store
        // or derive a scope while the worker holds the session lock across keyring IO.
        self.runtime.credentials.revoke_in_process();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.lifecycle = Some(event);
        self.view = if self.secure_store { AccountView::signed_out() } else { AccountView::unavailable() };
        if let Ok(mut status) = self.cleanup.lock() {
            status.pending = Some(generation);
            status.error = None;
        } else {
            return Err(account_unavailable("the account cleanup state is unavailable"));
        }

        let credentials = self.runtime.credentials.clone();
        let cleanup = Arc::clone(&self.cleanup);
        let worker = std::thread::Builder::new().name("intcad-account-cleanup".into()).spawn(move || {
            let result = credentials.sign_out_keeping_revocation();
            if let Ok(mut status) = cleanup.lock()
                && status.pending == Some(generation)
            {
                status.pending = None;
                if result.is_ok() {
                    // Only the owning generation may lift the barrier.
                    credentials.end_revocation();
                    status.error = None;
                } else {
                    status.error = Some(account_unavailable("the account session cleanup did not complete"));
                }
            }
        });
        match worker {
            Ok(handle) => {
                self.workers.push(handle);
                Ok(())
            }
            // In-process authority and the event are already applied. The durable marker was
            // never written, so the status says cleanup never started (not that it is pending),
            // and this process stays fail-closed; cross-process suppression cannot be claimed.
            Err(_) => {
                if let Ok(mut status) = self.cleanup.lock() {
                    status.pending = None;
                    status.error = Some(account_unavailable("the sign-out cleanup could not start"));
                }
                Err(account_unavailable("the sign-out cleanup could not start"))
            }
        }
    }

    fn join_workers(&mut self) {
        let workers = std::mem::take(&mut self.workers);
        for worker in workers {
            let _ = worker.join();
        }
    }
}

fn current_view(startup: &AccountStartup) -> AccountView {
    if !startup.secure_store || startup.logout_pending {
        return AccountView::unavailable();
    }
    match startup.restored.as_ref() {
        Some(scope) => AccountView {
            signed_in: true,
            sign_in_visible: false,
            account_label: Some(scope.account().as_str().to_owned()),
            available: None,
            held: 0,
            settling: false,
            // An authenticated account still needs an online status and a model to say chat is
            // available; without a reviewed account service this task cannot prove either.
            chat_available: false,
            tool_blocked: Some(RefusalCode::AuthUnavailable),
            own_model_label: None,
        },
        None => AccountView::signed_out(),
    }
}

impl AccountPort for DesktopAccountPort {
    fn view(&self) -> AccountView {
        self.view.clone()
    }

    fn refresh_status(&mut self) -> Result<AccountView, Refusal> {
        self.cleanup_confirmed()?;
        if !self.secure_store {
            self.view = AccountView::unavailable();
            return Ok(self.view.clone());
        }
        if self.runtime.authenticated_scope().is_none() {
            self.view = AccountView::signed_out();
            return Ok(self.view.clone());
        }
        // A product session exists, but this task installs no authenticated account service or
        // settlement adapter. Never invent a quota or a successful online status: keep the cached
        // view and report the stable refusal.
        Err(account_unavailable("no authenticated account service is installed"))
    }

    fn sign_in(&mut self) -> Result<(), Refusal> {
        self.cleanup_confirmed()?;
        // A provider token is not a product session. Do not start device authorization from a
        // normal UI action while no reviewed ProductSessionExchange exists.
        Err(account_unavailable("no documented product-session exchange is configured"))
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        self.revoke(LifecycleEvent::SignedOut)
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        self.revoke(LifecycleEvent::AccountSwitched)
    }

    fn delete_account_guide(&self) -> DeleteAccountGuide {
        // No reviewed account-deletion page exists in this build; never send a user to a guessed
        // address. The UI omits the guide when the URL is empty.
        DeleteAccountGuide { help_url: "", steps: &[] }
    }

    fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        // Only an explicit terminal command may start a device flow; no desktop key or client id
        // is configured or documented for this build.
        Err(Refusal::new(RefusalCode::AuthRequired, "an explicit sign-in command is required"))
    }

    fn own_model_view(&self) -> Option<OwnModelView> {
        None
    }

    fn take_lifecycle_event(&mut self) -> Option<LifecycleEvent> {
        self.lifecycle.take()
    }

    fn shutdown(&mut self) {
        self.join_workers();
    }
}

/// Install the desktop account runtime into the app.
///
/// The UI port is always installed so the start page and title bar have one host-owned seam. The
/// durable ledgers are installed when they opened. Commit and billing authority are installed
/// only from the same runtime when a reviewed service exists; in this task it does not, so
/// controlled entries keep the existing default refusal.
pub fn install(app: &mut SolveApp, mut runtime: AccountRuntime, startup: AccountStartup) {
    if let Some(ledger) = runtime.ledger.take() {
        app.install_review_store(Box::new(ledger));
    }
    if let Some((scope, service, clock)) = runtime.take_commit_authority() {
        app.install_commit_authority(scope, service, clock);
        solidcraft_engine::note!("account: authenticated commit authority installed for the restored session");
    } else if startup.logout_pending {
        solidcraft_engine::note!("account: a previous sign-out is not confirmed; restore and controlled authority stay closed");
    } else if startup.restored.is_some() {
        solidcraft_engine::note!("account: session restored from the secure store; controlled authority stays closed");
    } else {
        solidcraft_engine::note!("account: no account session restored; CAD stays anonymous and offline");
    }
    app.account = State::with_port(Box::new(DesktopAccountPort::new(runtime, &startup)));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use serde_json::json;
    use solidcraft_account_native::{
        CredentialHandoff, CredentialSession, MemorySession, NoPersistentStore, ProductExchangeError, ProductSession, ProductSessionExchange,
        ProviderSession, Secret, WireCredentials,
    };
    use solidcraft_engine::entitlement::{
        Admission, BillingResult, Capability, CredentialError, CredentialErrorCode, CredentialStore, EntryCategory, Grant, OperationId, Refusal,
        RefusalCode,
    };
    use solidcraft_ui_egui::account::{AccountPort, Action};
    use solidcraft_ui_egui::control;

    use super::*;

    fn store_error() -> CredentialError {
        CredentialError { code: CredentialErrorCode::Unavailable, message: "inert test store failure".into() }
    }

    #[derive(Clone)]
    struct TestStore {
        entries: Arc<Mutex<BTreeMap<String, String>>>,
        fail_delete: Arc<AtomicBool>,
        fail_get: Arc<AtomicBool>,
        block_delete: Arc<Mutex<Option<Receiver<()>>>>,
    }

    impl Default for TestStore {
        fn default() -> Self {
            Self {
                entries: Arc::new(Mutex::new(BTreeMap::new())),
                fail_delete: Arc::new(AtomicBool::new(false)),
                fail_get: Arc::new(AtomicBool::new(false)),
                block_delete: Arc::new(Mutex::new(None)),
            }
        }
    }

    impl TestStore {
        /// Make the first delete wait until the test releases it, so the worker's off-thread
        /// order is observable.
        fn block_next_delete(&self, gate: Receiver<()>) {
            *self.block_delete.lock().unwrap() = Some(gate);
        }
    }

    impl CredentialStore for TestStore {
        fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
            if self.fail_get.load(Ordering::SeqCst) {
                return Err(store_error());
            }
            Ok(self.entries.lock().unwrap().get(key).cloned())
        }

        fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
            self.entries.lock().unwrap().insert(key.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
            let gate = self.block_delete.lock().unwrap().take();
            if let Some(gate) = gate {
                let _ = gate.recv();
            }
            if self.fail_delete.load(Ordering::SeqCst) {
                return Err(store_error());
            }
            self.entries.lock().unwrap().remove(key);
            Ok(())
        }

        fn persistent(&self) -> bool {
            true
        }
    }

    struct TestExchange;

    impl ProductSessionExchange for TestExchange {
        fn exchange(&self, _provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
            Ok(ProductSession::from_trusted_exchange("account_desktop", Secret::new("product_session_test_0000000000"), None))
        }
    }

    fn activate_session(store: &TestStore) -> CredentialSession {
        let mut session = CredentialSession::new(Box::new(store.clone()));
        session.complete_provider_login("provider_account", "provider_token").unwrap();
        session.activate_product_session(&TestExchange).unwrap();
        CredentialHandoff::installation_credential(&mut session, "installation_desktop", Secret::new("installation_credential")).unwrap();
        session
    }

    fn wait_for_cleanup(port: &DesktopAccountPort) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if port.cleanup.lock().unwrap().pending.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(port.cleanup.lock().unwrap().pending.is_none(), "cleanup worker did not finish");
    }

    fn active_session() -> CredentialSession {
        activate_session(&TestStore::default())
    }

    fn installed_session() -> CredentialSession {
        CredentialSession::new(Box::new(TestStore::default()))
    }

    #[test]
    fn startup_without_secure_store_does_not_restore_session() {
        let runtime = AccountRuntime::with_store(NoPersistentStore);
        assert!(runtime.restored_session().is_none());
    }

    #[test]
    fn restore_requires_an_active_product_session_and_a_provable_scope() {
        let no_product = AccountRuntime::with_session(installed_session());
        assert!(no_product.restored_session().is_none(), "a provider-only or empty session never restores");

        let active = AccountRuntime::with_session(active_session());
        assert!(active.restored_session().is_some(), "the persisted product session and installation restore one scope");
        assert!(active.authenticated_scope().is_some_and(|scope| scope.is_provable()));
    }

    #[test]
    fn a_fresh_process_restores_the_persisted_scope_from_the_secure_store() {
        let store = TestStore::default();
        let previous = activate_session(&store);
        let previous_binding = previous.authenticated_scope().unwrap().session_binding().clone();

        // A new process has empty in-memory labels. Only the secure store proves the account,
        // installation and the exact product session; the binding must match the old one.
        let restored = AccountRuntime::with_session(CredentialSession::new(Box::new(store)));
        let scope = restored.restored_session().expect("a persisted product session is restorable");
        assert_eq!(scope.account().as_str(), "account_desktop");
        assert_eq!(scope.installation().as_str(), "installation_desktop");
        assert_eq!(scope.session_binding(), &previous_binding);
    }

    #[test]
    fn an_active_memory_session_is_never_restored_across_processes() {
        let memory = MemorySession::default();
        let mut session = CredentialSession::new(Box::new(memory.clone()));
        session.complete_provider_login("provider_account", "provider_token").unwrap();
        session.activate_product_session(&TestExchange).unwrap();
        CredentialHandoff::installation_credential(&mut session, "installation_memory", Secret::new("installation_credential")).unwrap();
        assert!(session.authenticated_scope().is_some(), "the in-memory session is usable now");

        let restarted = AccountRuntime::with_session(CredentialSession::new(Box::new(memory)));
        let startup = restarted.startup();
        assert!(!startup.secure_store, "memory is not a secure store");
        assert!(startup.restored.is_none(), "an in-memory session must not survive a restart");
        assert!(restarted.restored_session().is_none());
    }

    #[test]
    fn failed_delete_keeps_a_durable_tombstone_and_restart_fails_closed() {
        let store = TestStore::default();
        let session = activate_session(&store);
        assert!(session.authenticated_scope().is_some());
        store.fail_delete.store(true, Ordering::SeqCst);
        assert_eq!(session.sign_out().unwrap_err().code, CredentialErrorCode::Unavailable);
        assert!(session.logout_pending().unwrap(), "the tombstone was written before the failed delete");

        let restarted = AccountRuntime::with_session(CredentialSession::new(Box::new(store.clone())));
        let startup = restarted.startup();
        assert!(startup.logout_pending, "startup sees the unconfirmed sign-out");
        assert!(startup.restored.is_none());
        assert!(restarted.restored_session().is_none());

        // Once the inert store is healthy, the marker still suppresses restore until a reviewed
        // sign-in path clears it.
        store.fail_delete.store(false, Ordering::SeqCst);
        assert!(AccountRuntime::with_session(CredentialSession::new(Box::new(store))).restored_session().is_none());
    }

    #[test]
    fn an_unreadable_sign_out_marker_fails_startup_closed() {
        let store = TestStore::default();
        let session = activate_session(&store);
        assert!(session.authenticated_scope().is_some());
        store.fail_get.store(true, Ordering::SeqCst);
        let runtime = AccountRuntime::with_session(CredentialSession::new(Box::new(store)));
        let startup = runtime.startup();
        assert!(startup.logout_pending, "an unreadable safety marker is treated as unsafe");
        assert!(startup.restored.is_none());
    }

    #[test]
    fn a_persisted_provider_login_alone_never_restores_a_scope() {
        let store = TestStore::default();
        let session = CredentialSession::new(Box::new(store.clone()));
        session.complete_provider_login("provider_account", "provider_token").unwrap();
        let runtime = AccountRuntime::with_session(CredentialSession::new(Box::new(store)));
        assert!(runtime.restored_session().is_none());
    }

    #[test]
    fn sign_out_and_switch_clear_credentials_and_raise_one_lifecycle_event() {
        for (action, expected) in [(Action::SignOut, LifecycleEvent::SignedOut), (Action::SwitchAccount, LifecycleEvent::AccountSwitched)] {
            let runtime = AccountRuntime::with_session(active_session());
            let startup = runtime.startup();
            let mut port = DesktopAccountPort::new(runtime, &startup);
            assert!(port.view().signed_in);
            if action == Action::SignOut {
                port.sign_out().unwrap();
            } else {
                port.switch_account().unwrap();
            }
            wait_for_cleanup(&port);
            assert!(port.runtime.authenticated_scope().is_none(), "the old session binding is gone");
            assert!(!port.view().signed_in);
            assert_eq!(port.take_lifecycle_event(), Some(expected));
            assert_eq!(port.take_lifecycle_event(), None, "one sign-out raises one event");
        }
    }

    #[test]
    fn sign_out_revokes_authority_immediately_and_cleanup_runs_off_thread() {
        let store = TestStore::default();
        let (gate_tx, gate_rx) = channel();
        let runtime = AccountRuntime::with_session(activate_session(&store));
        store.block_next_delete(gate_rx);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        port.sign_out().unwrap();
        assert_eq!(port.take_lifecycle_event(), Some(LifecycleEvent::SignedOut));
        assert!(!port.view().signed_in, "authority is revoked before the worker finishes");
        assert!(port.cleanup.lock().unwrap().pending.is_some(), "the worker is waiting on the inert delete");
        // The lock-free barrier is already set, so no reader can recover the old bearer or derive
        // its scope even though the worker is blocked before its first delete.
        assert!(port.runtime.credentials.is_revoked());
        assert!(WireCredentials::product_session(&port.runtime.credentials).is_none());
        assert!(WireCredentials::installation_credential(&port.runtime.credentials).is_none());
        assert!(port.runtime.authenticated_scope().is_none());

        gate_tx.send(()).unwrap();
        wait_for_cleanup(&port);
        assert!(port.cleanup.lock().unwrap().error.is_none(), "cleanup completed");
        assert!(!port.runtime.credentials.is_revoked(), "the owning generation lifted the barrier");
        port.shutdown();
        assert!(!port.logout_pending, "the startup gate was already clear");
    }

    #[test]
    fn sign_in_never_turns_a_provider_login_into_a_product_session() {
        let runtime = AccountRuntime::with_session(installed_session());
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        assert_eq!(port.sign_in().unwrap_err().code, RefusalCode::AuthUnavailable);
        assert!(!port.runtime.credentials.product_session_active());
        assert!(!port.view().signed_in);
        assert_eq!(port.begin_device_code().unwrap_err().code, RefusalCode::AuthRequired);
    }

    #[test]
    fn installing_the_runtime_leaves_controlled_authority_closed_without_a_service() {
        let runtime = AccountRuntime::with_session(active_session());
        let startup = runtime.startup();
        let mut app = solidcraft_ui_egui::SolveApp::new(solidcraft_engine::Session::default(), solidcraft_ui_egui::Services::default());
        install(&mut app, runtime, startup);
        assert!(app.account.is_installed(), "the desktop port is installed");
        assert!(app.account.view().signed_in, "the restored product session is shown");

        let (gui, _) = solidcraft_ui_egui::ControlRequest::trusted_gui("engine.execute", json!({"command": "document.inspect"}));
        assert!(matches!(control::admit_control_request(&gui, &BudgetTable::unverified_default(), &mut NoBilling), Admission::Allowed));
    }

    struct NoBilling;

    impl solidcraft_engine::entitlement::BillingPort for NoBilling {
        fn admit(&mut self, _operation: &OperationId, _capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
            Err(Refusal::new(RefusalCode::AuthUnavailable, "no billing authority"))
        }

        fn commit(&mut self, _operation: &OperationId, _result: BillingResult) -> Result<BillingResult, Refusal> {
            Err(Refusal::new(RefusalCode::AuthUnavailable, "no billing authority"))
        }

        fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
            Ok(())
        }
    }

    #[test]
    fn app_starts_without_account_and_cad_still_works() {
        let app = solidcraft_ui_egui::SolveApp::new(solidcraft_engine::Session::default(), solidcraft_ui_egui::Services::default());
        assert!(!app.account.is_installed());
        assert!(!app.account.view().sign_in_visible);
        let (gui, _) = solidcraft_ui_egui::ControlRequest::trusted_gui("engine.execute", json!({"command": "document.inspect"}));
        assert!(matches!(control::admit_control_request(&gui, &BudgetTable::unverified_default(), &mut NoBilling), Admission::Allowed));
    }

    #[test]
    fn desktop_control_is_direct_and_requires_the_unverified_bound() {
        let (request, _) = solidcraft_ui_egui::ControlRequest::trusted_control("engine.execute", json!({"command": "document.inspect"}));
        assert_eq!(request.entry_category(), EntryCategory::DirectControl);
        assert!(matches!(
            control::admit_control_request(&request, &BudgetTable::unverified_default(), &mut NoBilling),
            Admission::Refused(Refusal { code: RefusalCode::UnsafeToStart, .. })
        ));
    }

    #[test]
    fn a_window_port_refresh_is_local_and_never_claims_a_quota() {
        let runtime = AccountRuntime::with_session(active_session());
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        assert_eq!(port.view().available, None);
        assert!(port.refresh_status().is_err(), "no service may invent an online snapshot");
        assert_eq!(port.view().available, None);
    }
}
