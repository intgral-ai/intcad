//! Desktop account composition root (Task 11).
//!
//! The runtime owns the one credential session, the durable operation ledger, the conservative
//! budget and the monotonic clock, and installs the UI account port. It never opens a dialog or
//! the network on a render pass.
//!
//! Fail closed: the desktop wires an explicit sign-in action to the existing WorkOS device-code
//! flow and the reviewed `ServerVerifiedProductSession`, but this build ships no reviewed WorkOS
//! client id or product service origin. `DesktopAccountConfig` is the only source of those
//! values; without it the action refuses before any request. Even a verified session installs no
//! production billing or commit authority and never promotes a provider token implicitly.
//! Ordinary CAD stays free and anonymous; controlled entries keep their existing default refusal.
//! A secure store is probed before any session is restored, and sign-out, account switch or
//! cancellation ends the current attempt's authority immediately.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use solidcraft_account_native::{
    CredentialSession, DeviceCodeFlow, DeviceCodeMode, DeviceCodePoll, DeviceCodeTransport, FileOperationStore, ProductExchangeError,
    ProductSessionExchange, ServerVerifiedProductSession, UreqDeviceCodeTransport, UreqTransport, WireConnection, WorkOsPublicClient,
};
use solidcraft_engine::entitlement::{
    AccountService, AuthenticatedHostScope, BudgetTable, CredentialErrorCode, MonotonicClock, MonotonicInstant, Refusal, RefusalCode,
};
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::account::{
    AccountPort, AccountView, DeleteAccountGuide, DeviceCode, LifecycleEvent, OwnModelView, SignInState, State, Timestamp,
};

/// Account facts read once before the window exists.
#[derive(Clone, Debug)]
pub struct AccountStartup {
    secure_store: bool,
    /// A durable sign-out marker is present, or its state could not be read. Either way startup
    /// refuses to restore or admit anything until a reviewed sign-in path clears it.
    logout_pending: bool,
    restored: Option<AuthenticatedHostScope>,
    /// A persisted product session's account label, shown even when this build cannot prove the
    /// installation scope that controlled work needs.
    active_account: Option<String>,
}

/// The reviewed values a desktop provider sign-in needs. Neither value comes from an environment
/// heuristic, a token field or a server response: the WorkOS public client id and the bare HTTPS
/// product origin are supplied by explicit configuration, and malformed values refuse to load.
#[derive(Clone)]
pub struct DesktopAccountConfig {
    client: WorkOsPublicClient,
    product: WireConnection,
}

impl DesktopAccountConfig {
    /// The only constructor: both values must be explicit. No reviewed production values ship in
    /// this build, so the composition root passes `None`; tests and a reviewed deployment can use
    /// this seam without inventing an environment fallback.
    #[allow(dead_code)]
    pub fn new(workos_public_client_id: &str, product_https_origin: &str) -> Option<Self> {
        let client = WorkOsPublicClient::new(workos_public_client_id)?;
        let product = WireConnection::new(product_https_origin, env!("CARGO_PKG_VERSION")).ok()?;
        Some(Self { client, product })
    }
}

/// The browser seam. The production launcher starts the system browser only for the exact
/// provider-issued HTTPS verification address of the current attempt.
pub trait BrowserLauncher: Send + Sync {
    fn open(&self, url: &str) -> Result<(), Refusal>;
}

struct SystemBrowserLauncher;

impl BrowserLauncher for SystemBrowserLauncher {
    fn open(&self, url: &str) -> Result<(), Refusal> {
        if !trusted_browser_url(url) {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the sign-in page is not a trusted HTTPS address"));
        }
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = std::process::Command::new("rundll32.exe");
            command.args(["url.dll,FileProtocolHandler", url]);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = std::process::Command::new("open");
            command.arg(url);
            command
        };
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let mut command = {
            let mut command = std::process::Command::new("xdg-open");
            command.arg(url);
            command
        };
        command.spawn().map_err(|_| Refusal::new(RefusalCode::AuthUnavailable, "the system browser could not be started"))?;
        Ok(())
    }
}

/// Only the provider's own HTTPS verification page may reach the system browser. This is an
/// address check, not a substitute for the provider's trust: the string is taken from the
/// current device-code prompt, never from a caller.
fn trusted_browser_url(url: &str) -> bool {
    let rest = url.strip_prefix("https://").unwrap_or_default();
    if rest.is_empty() || url.len() > 2048 || url.bytes().any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\') {
        return false;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    !authority.is_empty() && !authority.contains('@')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoginPhase {
    Idle,
    Starting,
    Waiting,
    Done,
    Expired,
    Failed(RefusalCode),
}

struct LoginStatus {
    attempt: u64,
    phase: LoginPhase,
    prompt: Option<DeviceCode>,
    account: Option<String>,
}

impl Default for LoginStatus {
    fn default() -> Self {
        Self { attempt: 0, phase: LoginPhase::Idle, prompt: None, account: None }
    }
}

struct LoginTask {
    attempt: u64,
    client: WorkOsPublicClient,
    connection: WireConnection,
    credentials: CredentialSession,
    status: Arc<Mutex<LoginStatus>>,
    persist_gate: Arc<Mutex<()>>,
}

type LoginRunner = Arc<dyn Fn(LoginTask) -> Result<std::thread::JoinHandle<()>, Refusal> + Send + Sync>;

fn production_login_runner() -> LoginRunner {
    Arc::new(|task| {
        std::thread::Builder::new()
            .name("intcad-account-login".into())
            .spawn(move || {
                let exchange = ServerVerifiedProductSession::new(UreqTransport::new(), task.connection.clone());
                run_login(task, UreqDeviceCodeTransport::new(), &exchange, &|duration| std::thread::sleep(duration));
            })
            .map_err(|_| Refusal::new(RefusalCode::AuthUnavailable, "the sign-in worker could not start"))
    })
}

/// Poll one explicit device-code attempt off the window thread. Network work never holds the
/// attempt fence; only the final credential write does, together with cancellation.
fn run_login<T: DeviceCodeTransport>(task: LoginTask, transport: T, exchange: &dyn ProductSessionExchange, wait: &dyn Fn(Duration)) {
    let LoginTask { attempt, client, connection: _, credentials, status, persist_gate } = task;
    let mut flow = DeviceCodeFlow::new(client, transport, credentials.clone(), DeviceCodeMode::Explicit);
    let prompt = match flow.start() {
        Ok(prompt) => prompt,
        Err(error) => {
            finish(&status, attempt, LoginPhase::Failed(error.code), None);
            return;
        }
    };
    let expires_at = Timestamp(unix_seconds().saturating_add(prompt.expires_in().as_secs()));
    let code = DeviceCode { user_code: prompt.user_code().to_owned(), verification_uri: prompt.verification_uri().to_owned(), expires_at };
    if !publish(&status, attempt, LoginPhase::Waiting, Some(code)) {
        return;
    }

    let deadline = Instant::now().checked_add(prompt.expires_in());
    let mut interval = prompt.interval();
    loop {
        if !current(&status, attempt) {
            return;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            finish(&status, attempt, LoginPhase::Expired, None);
            return;
        }
        wait(interval.min(Duration::from_secs(5)));
        if !current(&status, attempt) {
            return;
        }
        let poll = match flow.poll() {
            Ok(poll) => poll,
            Err(error) => {
                finish(&status, attempt, LoginPhase::Failed(error.code), None);
                return;
            }
        };
        match poll {
            DeviceCodePoll::Pending { interval: returned } => interval = returned,
            DeviceCodePoll::SlowDown { interval: returned } => interval = returned,
            DeviceCodePoll::Authorized => break,
            DeviceCodePoll::Denied => {
                finish(&status, attempt, LoginPhase::Failed(RefusalCode::AuthRequired), None);
                return;
            }
            DeviceCodePoll::Expired => {
                finish(&status, attempt, LoginPhase::Expired, None);
                return;
            }
            DeviceCodePoll::Failed(_) => {
                finish(&status, attempt, LoginPhase::Failed(RefusalCode::AuthUnavailable), None);
                return;
            }
        }
    }

    if !current(&status, attempt) {
        return;
    }
    let provider = match flow.authorized_provider() {
        Ok(provider) => provider,
        Err(error) => {
            finish(&status, attempt, LoginPhase::Failed(error.code), None);
            return;
        }
    };
    let product = match exchange.exchange(&provider) {
        Ok(product) => product,
        Err(ProductExchangeError::Rejected) => {
            finish(&status, attempt, LoginPhase::Failed(RefusalCode::AuthRequired), None);
            return;
        }
        Err(ProductExchangeError::NotConfigured | ProductExchangeError::Unavailable) => {
            finish(&status, attempt, LoginPhase::Failed(RefusalCode::AuthUnavailable), None);
            return;
        }
    };
    if !current(&status, attempt) {
        return;
    }
    // The exchange above is network work. The write and a cancel or sign-out serialize here, so
    // a stale attempt either sees the new generation or is deleted by the new cleanup.
    let _gate = match persist_gate.lock() {
        Ok(gate) => gate,
        Err(_) => return,
    };
    if !current(&status, attempt) {
        return;
    }
    match credentials.persist_verified_login(&provider, &product) {
        Ok(login) => finish(&status, attempt, LoginPhase::Done, Some(login.account_id)),
        Err(error) => {
            let code = match error.code {
                CredentialErrorCode::NoPersistentCredential => RefusalCode::AuthRequired,
                _ => RefusalCode::AuthUnavailable,
            };
            finish(&status, attempt, LoginPhase::Failed(code), None);
        }
    }
}

fn current(status: &Arc<Mutex<LoginStatus>>, attempt: u64) -> bool {
    status.lock().map(|status| status.attempt == attempt).unwrap_or(false)
}

fn publish(status: &Arc<Mutex<LoginStatus>>, attempt: u64, phase: LoginPhase, prompt: Option<DeviceCode>) -> bool {
    let Ok(mut status) = status.lock() else { return false };
    if status.attempt != attempt {
        return false;
    }
    status.phase = phase;
    status.prompt = prompt;
    true
}

fn finish(status: &Arc<Mutex<LoginStatus>>, attempt: u64, phase: LoginPhase, account: Option<String>) {
    let Ok(mut status) = status.lock() else { return };
    if status.attempt != attempt {
        return;
    }
    status.phase = phase;
    status.prompt = None;
    status.account = account;
}

fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0)
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
    config: Option<DesktopAccountConfig>,
}

impl AccountRuntime {
    /// The production desktop runtime: the operating-system credential store and the durable
    /// ledger under the settings folder. Both are optional; a missing one fails closed.
    pub fn desktop(settings: Option<&Path>) -> Self {
        Self::desktop_with_config(settings, None)
    }

    /// The production desktop runtime with explicit trusted account configuration. A missing
    /// configuration keeps the same anonymous CAD and leaves the explicit sign-in refused.
    pub fn desktop_with_config(settings: Option<&Path>, config: Option<DesktopAccountConfig>) -> Self {
        let credentials = crate::keyring_store::account_credential_session();
        let ledger = settings.and_then(|directory| match FileOperationStore::open(directory.join("account")) {
            Ok(store) => Some(store),
            Err(error) => {
                solidcraft_engine::note!("account: the durable operation ledger could not be opened ({error})");
                None
            }
        });
        Self::new(credentials, ledger, config)
    }

    /// A test-only runtime over a caller-supplied credential store.
    #[cfg(test)]
    pub fn with_store<S: solidcraft_engine::entitlement::CredentialStore + Send + 'static>(store: S) -> Self {
        Self::new(CredentialSession::new(Box::new(store)), None, None)
    }

    /// A test-only runtime over an already composed credential session.
    #[cfg(test)]
    pub fn with_session(credentials: CredentialSession) -> Self {
        Self::new(credentials, None, None)
    }

    /// A test-only runtime with the explicit trusted configuration and a composed session.
    #[cfg(test)]
    pub fn with_session_config(credentials: CredentialSession, config: DesktopAccountConfig) -> Self {
        Self::new(credentials, None, Some(config))
    }

    fn new(credentials: CredentialSession, ledger: Option<FileOperationStore>, config: Option<DesktopAccountConfig>) -> Self {
        Self {
            credentials,
            ledger,
            budget: BudgetTable::unverified_default(),
            clock: Some(Box::new(SystemMonotonicClock::new())),
            service: None,
            config,
        }
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
        let active_account =
            if secure_store && !logout_pending && self.credentials.product_session_active() { self.credentials.bound_account() } else { None };
        AccountStartup { secure_store, logout_pending, restored, active_account }
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
    login_attempt: u64,
    login_status: Arc<Mutex<LoginStatus>>,
    persist_gate: Arc<Mutex<()>>,
    login_worker: Option<std::thread::JoinHandle<()>>,
    browser: Arc<dyn BrowserLauncher>,
    runner: LoginRunner,
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
            login_attempt: 0,
            login_status: Arc::new(Mutex::new(LoginStatus::default())),
            persist_gate: Arc::new(Mutex::new(())),
            login_worker: None,
            browser: Arc::new(SystemBrowserLauncher),
            runner: production_login_runner(),
        }
    }

    #[cfg(test)]
    fn with_seams(mut self, browser: Arc<dyn BrowserLauncher>, runner: LoginRunner) -> Self {
        self.browser = browser;
        self.runner = runner;
        self
    }

    fn login_pending(&self) -> bool {
        self.view.sign_in_state.pending()
    }

    fn invalidate_login(&mut self) {
        // Serialize with the final credential write so a revoked or canceled attempt cannot
        // persist after this generation change.
        let _gate = self.persist_gate.lock();
        self.login_attempt = self.login_attempt.wrapping_add(1);
        if let Ok(mut status) = self.login_status.lock() {
            *status = LoginStatus::default();
            status.attempt = self.login_attempt;
        }
        self.view.sign_in_state = SignInState::Idle;
        self.view.sign_in_prompt = None;
        self.view.sign_in_error = None;
    }

    fn poll_login(&mut self) -> Option<AccountView> {
        let status = self.login_status.lock().ok()?;
        if status.attempt != self.login_attempt {
            return None;
        }
        match status.phase {
            LoginPhase::Idle => return None,
            LoginPhase::Starting => {
                self.view.sign_in_state = SignInState::Starting;
                self.view.sign_in_prompt = None;
                self.view.sign_in_error = None;
            }
            LoginPhase::Waiting => {
                self.view.sign_in_state = SignInState::Waiting;
                self.view.sign_in_prompt = status.prompt.clone();
                self.view.sign_in_error = None;
            }
            LoginPhase::Done => {
                let account = status.account.clone()?;
                self.logout_pending = false;
                // The product status verified the session, but this build installs no account
                // service or model authority, so Chat and controlled entries keep their existing
                // refusal until a reviewed adapter exists.
                self.view = signed_in_view(account);
            }
            LoginPhase::Expired => {
                self.view = AccountView { sign_in_state: SignInState::Expired, ..AccountView::signed_out() };
            }
            LoginPhase::Failed(code) => {
                self.view = AccountView { sign_in_state: SignInState::Failed, sign_in_error: Some(code), ..AccountView::signed_out() };
            }
        }
        Some(self.view.clone())
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
        self.invalidate_login();
        // Publish the new generation and set the barrier in the same short lock. A finishing
        // older worker takes that lock before clearing anything, so it can never clear a newer
        // barrier between generation publication and its own stale-result check. No keyring IO
        // happens while this lock is held.
        let mut status = match self.cleanup.lock() {
            Ok(status) => status,
            Err(_) => {
                self.runtime.credentials.revoke_in_process();
                self.lifecycle = Some(event);
                self.view = if self.secure_store { AccountView::signed_out() } else { AccountView::unavailable() };
                return Err(account_unavailable("the account cleanup state is unavailable"));
            }
        };
        self.runtime.credentials.revoke_in_process();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        status.pending = Some(generation);
        status.error = None;
        drop(status);
        self.lifecycle = Some(event);
        self.view = if self.secure_store { AccountView::signed_out() } else { AccountView::unavailable() };

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
        self.invalidate_login();
        if let Some(worker) = self.login_worker.take() {
            let _ = worker.join();
        }
        let workers = std::mem::take(&mut self.workers);
        for worker in workers {
            let _ = worker.join();
        }
    }
}

fn signed_in_view(account: String) -> AccountView {
    AccountView {
        signed_in: true,
        sign_in_visible: false,
        sign_in_state: SignInState::Idle,
        sign_in_prompt: None,
        sign_in_error: None,
        account_label: Some(account),
        available: None,
        held: 0,
        settling: false,
        // An authenticated account still needs an online status and a model to say chat is
        // available; without a reviewed account service this task cannot prove either.
        chat_available: false,
        tool_blocked: Some(RefusalCode::AuthUnavailable),
        own_model_label: None,
    }
}

fn current_view(startup: &AccountStartup) -> AccountView {
    if !startup.secure_store || startup.logout_pending {
        return AccountView::unavailable();
    }
    if let Some(scope) = startup.restored.as_ref() {
        return signed_in_view(scope.account().as_str().to_owned());
    }
    match startup.active_account.clone() {
        Some(account) => signed_in_view(account),
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
        if self.login_pending() {
            return Err(account_unavailable("a sign-in is already in progress"));
        }
        if !self.secure_store {
            return Err(account_unavailable("no secure credential store is available"));
        }
        if let Ok(status) = self.cleanup.lock() {
            if status.pending.is_some() {
                return Err(account_unavailable("the sign-out cleanup is still running"));
            }
            if let Some(error) = &status.error {
                return Err(error.clone());
            }
        } else {
            return Err(account_unavailable("the account cleanup state is unavailable"));
        }
        if self.runtime.credentials.is_revoked() {
            return Err(account_unavailable("the account authority is revoked"));
        }
        let config = self.runtime.config.clone().ok_or_else(|| account_unavailable("no configured product-session verification origin"))?;

        let _gate = self.persist_gate.lock();
        self.login_attempt = self.login_attempt.wrapping_add(1);
        let attempt = self.login_attempt;
        match self.login_status.lock() {
            Ok(mut status) => {
                *status = LoginStatus { attempt, phase: LoginPhase::Starting, prompt: None, account: None };
            }
            Err(_) => return Err(account_unavailable("the sign-in state is unavailable")),
        }
        self.view = AccountView { sign_in_state: SignInState::Starting, ..AccountView::signed_out() };
        drop(_gate);

        if let Some(worker) = self.login_worker.take() {
            let _ = worker.join();
        }
        let task = LoginTask {
            attempt,
            client: config.client,
            connection: config.product,
            credentials: self.runtime.credentials.clone(),
            status: Arc::clone(&self.login_status),
            persist_gate: Arc::clone(&self.persist_gate),
        };
        self.login_worker = Some((self.runner)(task)?);
        Ok(())
    }

    fn open_sign_in_browser(&mut self) -> Result<(), Refusal> {
        if self.view.sign_in_state != SignInState::Waiting {
            return Err(Refusal::new(RefusalCode::AuthRequired, "no sign-in is waiting for a browser"));
        }
        let url = self
            .view
            .sign_in_prompt
            .as_ref()
            .map(|prompt| prompt.verification_uri.as_str())
            .ok_or_else(|| Refusal::new(RefusalCode::AuthRequired, "no sign-in verification address is available"))?;
        if !trusted_browser_url(url) {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the sign-in page is not a trusted HTTPS address"));
        }
        self.browser.open(url)
    }

    fn cancel_sign_in(&mut self) {
        self.invalidate_login();
        self.view = if self.secure_store { AccountView::signed_out() } else { AccountView::unavailable() };
    }

    fn poll(&mut self) -> Option<AccountView> {
        self.poll_login()
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

    use std::collections::{BTreeMap, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use serde_json::json;
    use solidcraft_account_native::{
        CredentialHandoff, CredentialSession, DeviceCodeTransport, MemorySession, NoPersistentStore, ProductExchangeError, ProductSession,
        ProductSessionExchange, ProviderSession, Secret, WireCredentials,
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
        block_delete: Arc<Mutex<VecDeque<Receiver<()>>>>,
        block_get: Arc<Mutex<Option<Receiver<()>>>>,
        delete_waiting: Arc<AtomicUsize>,
        get_waiting: Arc<AtomicUsize>,
    }

    impl Default for TestStore {
        fn default() -> Self {
            Self {
                entries: Arc::new(Mutex::new(BTreeMap::new())),
                fail_delete: Arc::new(AtomicBool::new(false)),
                fail_get: Arc::new(AtomicBool::new(false)),
                block_delete: Arc::new(Mutex::new(VecDeque::new())),
                block_get: Arc::new(Mutex::new(None)),
                delete_waiting: Arc::new(AtomicUsize::new(0)),
                get_waiting: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl TestStore {
        /// Make the next delete wait until the test releases it, so overlapping workers are
        /// observable.
        fn block_next_delete(&self, gate: Receiver<()>) {
            self.block_delete.lock().unwrap().push_back(gate);
        }

        /// Make the next read wait while this store's session lock is held.
        fn block_next_get(&self, gate: Receiver<()>) {
            *self.block_get.lock().unwrap() = Some(gate);
        }
    }

    impl CredentialStore for TestStore {
        fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
            let gate = self.block_get.lock().unwrap().take();
            if let Some(gate) = gate {
                self.get_waiting.fetch_add(1, Ordering::SeqCst);
                let _ = gate.recv();
            }
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
            let gate = self.block_delete.lock().unwrap().pop_front();
            if let Some(gate) = gate {
                self.delete_waiting.fetch_add(1, Ordering::SeqCst);
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

    fn wait_for_counter(counter: &AtomicUsize, target: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if counter.load(Ordering::SeqCst) >= target {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(counter.load(Ordering::SeqCst) >= target, "expected {target} waiting operation(s)");
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
    fn an_older_cleanup_cannot_clear_the_newer_generation_barrier() {
        let store = TestStore::default();
        let (gate_one_tx, gate_one_rx) = channel();
        let (gate_two_tx, gate_two_rx) = channel();
        let runtime = AccountRuntime::with_session(activate_session(&store));
        store.block_next_delete(gate_one_rx);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        port.sign_out().unwrap();
        wait_for_counter(&store.delete_waiting, 1);
        store.block_next_delete(gate_two_rx);
        port.sign_out().unwrap();
        assert_eq!(port.cleanup.lock().unwrap().pending, Some(2));
        assert!(port.runtime.credentials.is_revoked());

        // Let the older worker finish. It must not clear the newer generation's barrier while
        // the newer worker waits before its first delete.
        gate_one_tx.send(()).unwrap();
        wait_for_counter(&store.delete_waiting, 2);
        assert!(port.runtime.credentials.is_revoked(), "the older cleanup cleared a newer barrier");
        assert_eq!(port.cleanup.lock().unwrap().pending, Some(2));

        gate_two_tx.send(()).unwrap();
        wait_for_cleanup(&port);
        assert!(!port.runtime.credentials.is_revoked(), "the owning generation lifts the barrier");
        port.shutdown();
    }

    #[test]
    fn a_reader_that_passed_the_pre_lock_check_cannot_return_authority_after_revocation() {
        let store = TestStore::default();
        let session = activate_session(&store);
        let (gate_tx, gate_rx) = channel();
        store.block_next_get(gate_rx);
        let reader_session = session.clone();
        let reader = std::thread::spawn(move || reader_session.authenticated_scope());

        wait_for_counter(&store.get_waiting, 1);
        // The reader passed the lock-free pre-check, holds the session lock and is blocked in
        // store IO. Revoke and then clear the boolean as a completed cleanup would.
        session.revoke_in_process();
        assert!(session.is_revoked());
        session.end_revocation();
        assert!(!session.is_revoked());
        gate_tx.send(()).unwrap();
        assert!(reader.join().unwrap().is_none(), "the epoch change must still invalidate the waiting reader");
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

    #[derive(Default)]
    struct GateState {
        started: bool,
        gate: Option<Receiver<()>>,
    }

    #[derive(Clone, Default)]
    struct ScriptedDeviceTransport {
        state: Arc<Mutex<GateState>>,
        blocked: bool,
        token_error: Option<&'static str>,
    }

    impl ScriptedDeviceTransport {
        fn blocked(gate: Receiver<()>) -> Self {
            Self { state: Arc::new(Mutex::new(GateState { gate: Some(gate), ..GateState::default() })), blocked: true, token_error: None }
        }

        fn expired() -> Self {
            Self { token_error: Some("expired_token"), ..Self::default() }
        }
    }

    impl DeviceCodeTransport for ScriptedDeviceTransport {
        fn post(&self, _url: &str, _content_type: &str, _body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError> {
            let mut state = self.state.lock().map_err(|_| store_error())?;
            if !state.started {
                state.started = true;
                return Ok((
                    200,
                    json!({
                        "device_code": "device_code_SECRET_000000000000000000000000",
                        "user_code": "BCDF-GHJK",
                        "verification_uri": "https://auth.example/device",
                        "verification_uri_complete": "https://auth.example/device?user_code=BCDF-GHJK",
                        "expires_in": 300,
                        "interval": 1
                    })
                    .to_string()
                    .into_bytes(),
                ));
            }
            if self.blocked {
                let gate = state.gate.take();
                drop(state);
                if let Some(gate) = gate {
                    let _ = gate.recv();
                }
            }
            if let Some(error) = self.token_error {
                return Ok((400, json!({"error": error}).to_string().into_bytes()));
            }
            Ok((
                200,
                json!({
                    "user": {"id": "user_0001"},
                    "access_token": "access_token_SECRET_000000000000000000000000"
                })
                .to_string()
                .into_bytes(),
            ))
        }
    }

    #[derive(Clone)]
    struct ScriptedExchange {
        calls: Arc<AtomicUsize>,
    }

    impl ProductSessionExchange for ScriptedExchange {
        fn exchange(&self, _provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ProductSession::from_trusted_exchange("acct_desktop0001", Secret::new("product_session_test_0000000000"), None))
        }
    }

    #[derive(Clone, Default)]
    struct RecordingBrowser {
        opened: Arc<Mutex<Vec<String>>>,
    }

    impl BrowserLauncher for RecordingBrowser {
        fn open(&self, url: &str) -> Result<(), Refusal> {
            self.opened.lock().map_err(|_| account_unavailable("browser test lock"))?.push(url.to_owned());
            Ok(())
        }
    }

    fn test_config() -> DesktopAccountConfig {
        DesktopAccountConfig::new("client_test", "https://product.example").expect("test configuration")
    }

    fn test_runner(transport: ScriptedDeviceTransport, exchange: ScriptedExchange) -> (LoginRunner, Arc<AtomicUsize>) {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let runner: LoginRunner = Arc::new(move |task| {
            counter.fetch_add(1, Ordering::SeqCst);
            let transport = transport.clone();
            let exchange = exchange.clone();
            std::thread::Builder::new()
                .name("intcad-account-login-test".into())
                .spawn(move || {
                    run_login(task, transport, &exchange, &|_| {});
                })
                .map_err(|_| account_unavailable("the test sign-in worker could not start"))
        });
        (runner, attempts)
    }

    fn sequence_runner(transports: Vec<ScriptedDeviceTransport>, exchange: ScriptedExchange) -> (LoginRunner, Arc<AtomicUsize>) {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let transports = Arc::new(Mutex::new(transports));
        let runner: LoginRunner = Arc::new(move |task| {
            let transport = transports.lock().map_err(|_| account_unavailable("test transport lock"))?.remove(0);
            counter.fetch_add(1, Ordering::SeqCst);
            let exchange = exchange.clone();
            std::thread::Builder::new()
                .name("intcad-account-login-test".into())
                .spawn(move || {
                    run_login(task, transport, &exchange, &|_| {});
                })
                .map_err(|_| account_unavailable("the test sign-in worker could not start"))
        });
        (runner, attempts)
    }

    fn wait_for_login(port: &mut DesktopAccountPort, signed_in: bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let _ = AccountPort::poll(port);
            if port.view().signed_in == signed_in && !port.view().sign_in_state.pending() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the sign-in worker did not reach signed_in={signed_in}");
    }

    fn login_port(browser: RecordingBrowser, runner: LoginRunner, store: &TestStore) -> DesktopAccountPort {
        let runtime = AccountRuntime::with_session_config(CredentialSession::new(Box::new(store.clone())), test_config());
        let startup = runtime.startup();
        DesktopAccountPort::new(runtime, &startup).with_seams(Arc::new(browser), runner)
    }

    #[test]
    fn explicit_sign_in_verifies_the_product_before_persisting_and_never_launches_a_browser() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange { calls: Arc::new(AtomicUsize::new(0)) };
        let (runner, attempts) = test_runner(ScriptedDeviceTransport::default(), exchange.clone());
        let mut port = login_port(browser.clone(), runner, &store);

        port.sign_in().unwrap();
        wait_for_login(&mut port, true);
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "one explicit action starts one attempt");
        assert_eq!(exchange.calls.load(Ordering::SeqCst), 1, "the provider was verified once");
        assert!(browser.opened.lock().unwrap().is_empty(), "sign-in launched a browser without a click");
        assert!(port.runtime.credentials.product_session_active(), "the verified session was persisted");
        assert!(port.runtime.credentials.authenticated_scope().is_none(), "no installation authority was invented");
        assert_eq!(port.view().account_label.as_deref(), Some("acct_desktop0001"));
        assert!(!port.view().chat_available, "verification alone does not grant chat or execution authority");
    }

    #[test]
    fn expired_sign_in_is_explicitly_retryable() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange { calls: Arc::new(AtomicUsize::new(0)) };
        let (runner, attempts) = sequence_runner(vec![ScriptedDeviceTransport::expired(), ScriptedDeviceTransport::default()], exchange);
        let mut port = login_port(browser, runner, &store);

        port.sign_in().unwrap();
        wait_for_login(&mut port, false);
        assert_eq!(port.view().sign_in_state, SignInState::Expired);

        // A second explicit click starts a fresh attempt instead of replaying the old one.
        port.sign_in().unwrap();
        wait_for_login(&mut port, true);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn canceling_a_waiting_attempt_prevents_late_persistence() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange { calls: Arc::new(AtomicUsize::new(0)) };
        let (gate_tx, gate_rx) = channel();
        let (runner, _) = test_runner(ScriptedDeviceTransport::blocked(gate_rx), exchange.clone());
        let mut port = login_port(browser, runner, &store);

        port.sign_in().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while port.view().sign_in_prompt.is_none() && Instant::now() < deadline {
            let _ = AccountPort::poll(&mut port);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(port.view().sign_in_prompt.is_some(), "the provider prompt arrived");
        port.open_sign_in_browser().unwrap();
        port.cancel_sign_in();
        gate_tx.send(()).unwrap();
        port.shutdown();

        assert_eq!(exchange.calls.load(Ordering::SeqCst), 0, "a canceled attempt never reached verification");
        assert!(!port.runtime.credentials.product_session_active());
        assert_eq!(port.runtime.credentials.provider_account(), None);
        assert!(!port.view().signed_in);
    }

    #[test]
    fn signing_out_obsoletes_a_waiting_attempt() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange { calls: Arc::new(AtomicUsize::new(0)) };
        let (gate_tx, gate_rx) = channel();
        let (runner, _) = test_runner(ScriptedDeviceTransport::blocked(gate_rx), exchange.clone());
        let mut port = login_port(browser, runner, &store);
        port.sign_in().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while port.view().sign_in_prompt.is_none() && Instant::now() < deadline {
            let _ = AccountPort::poll(&mut port);
            std::thread::sleep(Duration::from_millis(5));
        }
        port.sign_out().unwrap();
        gate_tx.send(()).unwrap();
        wait_for_cleanup(&port);
        port.shutdown();
        assert_eq!(exchange.calls.load(Ordering::SeqCst), 0);
        assert!(!port.runtime.credentials.product_session_active());
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
