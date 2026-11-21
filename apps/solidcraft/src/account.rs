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
use std::time::{Instant, SystemTime};

use solidcraft_account_native::{CredentialSession, FileOperationStore};
use solidcraft_engine::entitlement::{AccountService, AuthenticatedHostScope, BudgetTable, MonotonicClock, MonotonicInstant, Refusal, RefusalCode};
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::account::{AccountPort, AccountView, DeleteAccountGuide, DeviceCode, LifecycleEvent, OwnModelView, State};

/// Account facts read once before the window exists.
#[derive(Clone, Debug)]
pub struct AccountStartup {
    secure_store: bool,
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
        AccountStartup { secure_store, restored: self.restored_with_store(secure_store) }
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

    /// Clear the in-memory session and deny every later credential read. A failed delete leaves
    /// the session locked instead of keeping the old authority usable.
    fn sign_out(&mut self) -> Result<(), Refusal> {
        self.credentials.sign_out().map_err(|_| account_unavailable("the account session could not be cleared"))
    }

    /// Sign out the old account before any new account work can begin.
    fn switch_account(&mut self) -> Result<(), Refusal> {
        self.credentials.sign_out().map_err(|_| account_unavailable("the old account session could not be cleared"))
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
struct DesktopAccountPort {
    runtime: AccountRuntime,
    view: AccountView,
    lifecycle: Option<LifecycleEvent>,
}

impl DesktopAccountPort {
    fn new(runtime: AccountRuntime, startup: &AccountStartup) -> Self {
        let view = current_view(startup);
        Self { runtime, view, lifecycle: None }
    }
}

fn current_view(startup: &AccountStartup) -> AccountView {
    if !startup.secure_store {
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
        if !self.runtime.credentials.is_persistent() {
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
        // A provider token is not a product session. Do not start device authorization from a
        // normal UI action while no reviewed ProductSessionExchange exists.
        Err(account_unavailable("no documented product-session exchange is configured"))
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        // Raise the event before the durable delete: even a failed delete leaves the credential
        // session locked, and the app must close the old authority now rather than after one more
        // request. A storage refusal is still returned to the panel.
        self.lifecycle = Some(LifecycleEvent::SignedOut);
        self.view = if self.runtime.credentials.is_persistent() { AccountView::signed_out() } else { AccountView::unavailable() };
        self.runtime.sign_out()
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        self.lifecycle = Some(LifecycleEvent::AccountSwitched);
        self.view = if self.runtime.credentials.is_persistent() { AccountView::signed_out() } else { AccountView::unavailable() };
        self.runtime.switch_account()
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
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use solidcraft_account_native::{
        CredentialHandoff, CredentialSession, NoPersistentStore, ProductExchangeError, ProductSession, ProductSessionExchange, ProviderSession,
        Secret,
    };
    use solidcraft_engine::entitlement::{
        Admission, BillingResult, Capability, CredentialError, CredentialStore, EntryCategory, Grant, OperationId, Refusal, RefusalCode,
    };
    use solidcraft_ui_egui::account::{AccountPort, Action};
    use solidcraft_ui_egui::control;

    use super::*;

    #[derive(Clone, Default)]
    struct TestStore {
        entries: Arc<Mutex<BTreeMap<String, String>>>,
    }

    impl CredentialStore for TestStore {
        fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
            Ok(self.entries.lock().unwrap().get(key).cloned())
        }

        fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
            self.entries.lock().unwrap().insert(key.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
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
        session.installation_credential("installation_desktop", Secret::new("installation_credential")).unwrap();
        session
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
            assert!(port.runtime.authenticated_scope().is_none(), "the old session binding is gone");
            assert!(!port.view().signed_in);
            assert_eq!(port.take_lifecycle_event(), Some(expected));
            assert_eq!(port.take_lifecycle_event(), None, "one sign-out raises one event");
        }
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
