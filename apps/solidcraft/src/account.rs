//! Desktop account composition root (Task 11).
//!
//! The runtime owns the one credential session, the durable operation ledger, the conservative
//! budget and the monotonic clock, and installs the UI account port. It never opens a dialog or
//! the network on a render pass.
//!
//! Fail closed: a documented server-verified `ProductSessionExchange` exists in `account-native`
//! and the desktop can run it from an explicit sign-in, but this build configures no product
//! service origin, WorkOS client id or AuthKit origin unless the host supplies all three. No
//! shipped UI action completes a provider sign-in implicitly, no browser opens without an explicit
//! browser action, and a completed flow activates only after the configured product service
//! verifies it and issues one installation binding. The desktop installs no production billing or
//! commit authority and never promotes a provider token implicitly. Ordinary CAD stays free and
//! anonymous; controlled entries keep their existing default refusal. A secure store is probed
//! before any session is restored, and sign-out or account switch ends the old session's
//! authority immediately.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use solidcraft_account_native::{
    BootstrapPlatform, CredentialSession, DeviceCodeFlow, DeviceCodeMode, DeviceCodePoll, DeviceCodePrompt, DeviceCodeTransport, FileOperationStore,
    FixedCredentials, HttpAccountService, ProductLogin, Secret, ServerVerifiedProductSession, Transport, UreqDeviceCodeTransport, UreqTransport,
    WireConnection, WireCredentials, WorkOsPublicClient,
};
use solidcraft_engine::entitlement::{
    AccountService, AuthenticatedHostScope, BillingResult, BudgetTable, MonotonicClock, MonotonicInstant, OperationStore, Refusal, RefusalCode,
    SettlementIntent, UnsettledOperation,
};
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::account::{AccountPort, AccountView, DeleteAccountGuide, DeviceCode, LifecycleEvent, OwnModelView, State, Timestamp};

/// Explicit desktop sign-in configuration. Every value comes from the host; no origin is ever
/// derived from a token, a response field or a guessed default, and no secret is baked in.
#[derive(Clone, Debug)]
struct SignInConfig {
    client: WorkOsPublicClient,
    authkit: AuthKitOrigin,
    product: WireConnection,
}

impl SignInConfig {
    fn new(client_id: &str, authkit_origin: &str, product_origin: &str, client_version: &str) -> Result<Self, Refusal> {
        let client = WorkOsPublicClient::new(client_id).ok_or_else(|| account_unavailable("no WorkOS public client id is configured"))?;
        let authkit = AuthKitOrigin::new(authkit_origin).ok_or_else(|| account_unavailable("no trusted HTTPS AuthKit origin is configured"))?;
        let product = WireConnection::new(product_origin, client_version)
            .map_err(|_| account_unavailable("no trusted HTTPS product service origin is configured"))?;
        Ok(Self { client, authkit, product })
    }

    /// The production source is three explicit host-provided values. A missing or malformed value
    /// leaves sign-in unavailable rather than falling back to any built-in address.
    fn from_environment() -> Option<Self> {
        let client_id = std::env::var("INTCAD_WORKOS_CLIENT_ID").ok()?;
        let authkit = std::env::var("INTCAD_AUTHKIT_ORIGIN").ok()?;
        let product = std::env::var("INTCAD_PRODUCT_SERVICE_ORIGIN").ok()?;
        Self::new(client_id.trim(), authkit.trim(), product.trim(), env!("CARGO_PKG_VERSION")).ok()
    }
}

/// A normalized HTTPS origin that a WorkOS verification page must match exactly.
///
/// WorkOS returns the address of its own hosted page. The desktop trusts it only when its scheme,
/// host and port equal the configured AuthKit origin; a path, query or fragment on the response
/// does not change the origin.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthKitOrigin {
    origin: String,
}

impl AuthKitOrigin {
    fn new(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        // Configuration names an origin only, with no path, query or fragment. The returned
        // verification page may of course carry a path; `matches` compares origins alone.
        let rest = raw.strip_prefix("https://")?;
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        if !(rest.len() == authority_end || rest.get(authority_end..) == Some("/")) {
            return None;
        }
        normalized_https_origin(raw).map(|origin| Self { origin })
    }

    fn matches(&self, uri: &str) -> bool {
        normalized_https_origin(uri).is_some_and(|origin| origin == self.origin)
    }
}

/// Parse only `https://host[:port]` (or that followed by `/`, `?` or `#`) and return a normalized
/// lowercase origin. Userinfo, plain HTTP, control bytes and empty hosts are rejected.
fn normalized_https_origin(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix("https://")?;
    if rest.is_empty() || rest.chars().any(|character| character.is_control() || character.is_whitespace() || matches!(character, '\\' | '@')) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.len() > 255 {
        return None;
    }
    let authority = authority.to_ascii_lowercase();
    let authority = authority.strip_suffix(":443").unwrap_or(&authority);
    let host = authority.split(':').next().unwrap_or_default();
    if host.is_empty() || !host.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-') {
        return None;
    }
    if let Some((host_part, port)) = authority.rsplit_once(':') {
        if host_part.is_empty() || host_part.contains(':') || port.is_empty() || port.len() > 5 || !port.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        port.parse::<u16>().ok().filter(|port| *port != 0)?;
    }
    Some(format!("https://{authority}"))
}

/// A cancellation flag that wakes the worker immediately, so no sign-in keeps polling for its
/// server interval after the user switches accounts or closes the session.
#[derive(Clone, Default)]
struct SignInCancellation {
    state: Arc<CancelState>,
}

#[derive(Default)]
struct CancelState {
    cancelled: AtomicBool,
    lock: Mutex<()>,
    wake: Condvar,
}

impl SignInCancellation {
    fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::SeqCst);
        self.state.wake.notify_all();
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }

    /// Wait for the interval or a cancellation, whichever comes first. Returns `false` on cancel.
    fn wait(&self, interval: Duration) -> bool {
        if self.is_cancelled() {
            return false;
        }
        let mut guard = match self.state.lock.lock() {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        };
        let now = Instant::now();
        let deadline = now.checked_add(interval).unwrap_or(now);
        loop {
            if self.is_cancelled() {
                return false;
            }
            let now = Instant::now();
            let remaining = deadline.saturating_duration_since(now);
            if remaining.is_zero() {
                return true;
            }
            let waited = match self.state.wake.wait_timeout(guard, remaining) {
                Ok(waited) => waited,
                Err(error) => error.into_inner(),
            };
            guard = waited.0;
            if waited.1.timed_out() {
                return !self.is_cancelled();
            }
        }
    }
}

/// The worker's only time source. Tests supply a virtual clock; production waits on the
/// cancellation-aware condition variable.
trait SignInTiming: Send + Sync {
    fn now(&self) -> Duration;
    fn wait(&self, interval: Duration, cancel: &SignInCancellation) -> bool;
}

struct SystemSignInTiming {
    start: Instant,
}

impl SystemSignInTiming {
    fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl SignInTiming for SystemSignInTiming {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn wait(&self, interval: Duration, cancel: &SignInCancellation) -> bool {
        cancel.wait(interval)
    }
}

fn auth_required(message: &'static str) -> Refusal {
    Refusal::new(RefusalCode::AuthRequired, message)
}

fn wall_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

/// Drive one explicit WorkOS device-code login through to a verified product session.
///
/// Only this function sends sign-in requests: it starts the documented device authorization,
/// shows the prompt for an explicit browser action, polls on the server's interval and slow-down
/// backoff until the user approves or the code's lifetime ends, then stores the provider login and
/// asks the configured product service to verify it. Cancellation is checked before every request
/// and before either credential write, so a late result after sign-out or account switch writes
/// nothing. The caller never replays a request that already completed.
fn drive_device_sign_in<D, P>(
    flow: &mut DeviceCodeFlow<D>,
    authkit: &AuthKitOrigin,
    product: &WireConnection,
    transport: P,
    timing: &dyn SignInTiming,
    cancel: &SignInCancellation,
    publish: &mut dyn FnMut(&DeviceCodePrompt),
) -> Result<ProductLogin, Refusal>
where
    D: DeviceCodeTransport,
    P: Transport,
{
    let started_at = timing.now();
    let prompt = flow.start()?;
    if !authkit.matches(prompt.verification_uri()) || prompt.verification_uri_complete().is_some_and(|uri| !authkit.matches(uri)) {
        return Err(account_unavailable("the sign-in service returned an untrusted verification address"));
    }
    let deadline = started_at.saturating_add(prompt.expires_in());
    publish(&prompt);
    let mut interval = prompt.interval();
    loop {
        if cancel.is_cancelled() {
            return Err(auth_required("the sign-in was cancelled"));
        }
        if timing.now() >= deadline {
            return Err(auth_required("the sign-in code expired"));
        }
        if !timing.wait(interval, cancel) {
            return Err(auth_required("the sign-in was cancelled"));
        }
        if timing.now() >= deadline {
            return Err(auth_required("the sign-in code expired"));
        }
        match flow.poll()? {
            DeviceCodePoll::Pending { interval: next } | DeviceCodePoll::SlowDown { interval: next } => interval = next,
            DeviceCodePoll::Authorized => break,
            DeviceCodePoll::Denied => return Err(auth_required("the sign-in was denied in the browser")),
            DeviceCodePoll::Expired => return Err(auth_required("the sign-in code expired")),
            DeviceCodePoll::Failed(_) => return Err(auth_required("the sign-in service rejected the request")),
        }
        if cancel.is_cancelled() {
            return Err(auth_required("the sign-in was cancelled"));
        }
    }
    // The browser approval may have arrived just as the session was retired. Check before the
    // provider write, and again before the product exchange, so a cancelled worker stops here.
    if cancel.is_cancelled() {
        return Err(auth_required("the sign-in was cancelled"));
    }
    flow.complete()?;
    if cancel.is_cancelled() {
        return Err(auth_required("the sign-in was cancelled"));
    }
    let exchange = ServerVerifiedProductSession::new(&transport, product.clone());
    let login =
        flow.session().activate_product_session(&exchange).map_err(|_| account_unavailable("the product service did not verify the sign-in"))?;
    // The product service has verified the exact WorkOS session. Only now does the desktop ask
    // the same trusted origin to issue its first installation identity. No account or installation
    // scope is invented before that response, and a malformed or failed bootstrap refuses the
    // sign-in rather than leaving a partially bound session.
    if cancel.is_cancelled() {
        return Err(auth_required("the sign-in was cancelled"));
    }
    let session = flow.session().clone();
    let mut account = HttpAccountService::with_transport(
        &transport,
        product.clone(),
        Box::new(session.clone()),
        Box::new(session),
        Box::new(SystemMonotonicClock::new()),
    );
    account
        .bootstrap_first_installation(current_bootstrap_platform(), None)
        .map_err(|_| account_unavailable("the installation could not be bound"))?;
    Ok(login)
}

fn current_bootstrap_platform() -> BootstrapPlatform {
    if cfg!(target_os = "windows") {
        BootstrapPlatform::Windows
    } else if cfg!(target_os = "macos") {
        BootstrapPlatform::Macos
    } else {
        BootstrapPlatform::Linux
    }
}

/// Open one validated verification page in the system browser. Only an explicit browser action
/// calls this; no prompt, refusal or noninteractive path opens a browser by itself.
fn open_system_browser(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "windows") {
        // `rundll32` takes the URL as data, with no shell string to split or interpolate.
        std::process::Command::new("rundll32.exe")
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    if cfg!(target_os = "windows") {
        command.args(["url.dll,FileProtocolHandler", url]);
    } else {
        command.arg(url);
    }
    command.spawn().map(|_| ()).map_err(|error| error.to_string())
}

type DeviceTransportFactory = Arc<dyn Fn() -> Box<dyn DeviceCodeTransport + Send + Sync> + Send + Sync>;
type ProductTransportFactory = Arc<dyn Fn() -> Box<dyn Transport + Send + Sync> + Send + Sync>;
type BrowserOpener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// A restart check is one startup decision, so it gets its own short bounds instead of the
/// 30-second limit the full sign-in flow allows. A slow or offline service must not hold launch.
const RESTART_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const RESTART_STATUS_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignInPhase {
    Starting,
    WaitingForApproval,
    SignedIn,
    Failed,
    Cancelled,
}

struct SignInShared {
    phase: SignInPhase,
    prompt: Option<DeviceCode>,
    browser_uri: Option<String>,
    account: Option<String>,
    notice: Option<Refusal>,
}

impl Default for SignInShared {
    fn default() -> Self {
        Self { phase: SignInPhase::Starting, prompt: None, browser_uri: None, account: None, notice: None }
    }
}

struct SignInJob {
    cancel: SignInCancellation,
    shared: Arc<Mutex<SignInShared>>,
}

#[derive(Default)]
struct RestartShared {
    /// The account label of an online-verified original pair, for presentation only.
    account: Option<String>,
    /// The bounded check has finished, failed or been retired.
    done: bool,
}

/// One bounded background restart check. It holds only presentation state; nothing here can
/// install commit authority.
struct RestartJob {
    cancel: SignInCancellation,
    shared: Arc<Mutex<RestartShared>>,
}

/// A signed-in view after a completed device flow and installation bootstrap. Controlled
/// authority stays closed: this checkpoint installs no account service, so chat and billed tools
/// are still unavailable even though the product session and installation are bound.
fn signed_in_view(account: &str) -> AccountView {
    AccountView {
        signed_in: true,
        sign_in_visible: false,
        account_label: Some(account.to_owned()),
        available: None,
        held: 0,
        settling: false,
        chat_available: false,
        tool_blocked: Some(RefusalCode::AuthUnavailable),
        own_model_label: None,
    }
}

/// The exact local facts a restart may try to verify: the account/installation/session scope plus
/// the two stored credentials that make it a complete pair. This is a candidate only; it never
/// backs presentation or authority.
#[derive(Clone, Debug)]
struct RestartBinding {
    scope: AuthenticatedHostScope,
    product: Secret,
    installation: Secret,
    /// The credential session's revocation generation when the binding was captured. A sign-out,
    /// switch or completed cleanup changes it, so a late result is discarded.
    generation: u64,
}

/// Account facts read once before the window exists.
#[derive(Clone, Debug)]
pub struct AccountStartup {
    secure_store: bool,
    /// A durable sign-out marker is present, or its state could not be read. Either way startup
    /// refuses to restore or admit anything until a reviewed sign-in path clears it.
    logout_pending: bool,
    /// Complete local credentials found at startup. They are not a session until an online status
    /// check accepts them.
    candidate: Option<AuthenticatedHostScope>,
    /// The exact pair whose bounded online check runs after the window exists. Startup never does
    /// that network read synchronously, so a candidate stays signed out until the check finishes.
    restart: Option<RestartBinding>,
    /// A local candidate that a synchronous in-process check accepted. No current startup path
    /// sets this; the bounded background check below is presentation only and must not fill it.
    restored: Option<AuthenticatedHostScope>,
}

/// One operation's background renewal checkpoint. The window reads only a cached result.
struct ControlRenewalJob {
    scope: AuthenticatedHostScope,
    operation: solidcraft_engine::entitlement::OperationId,
    grant: solidcraft_engine::entitlement::Grant,
    generation: u64,
    cutoff: Arc<Mutex<Option<MonotonicInstant>>>,
    cancelled: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

type ControlRenewalFactory = Box<
    dyn FnMut(
            CredentialSession,
            AuthenticatedHostScope,
            solidcraft_engine::entitlement::OperationId,
            solidcraft_engine::entitlement::Grant,
        ) -> std::io::Result<ControlRenewalJob>
        + Send
        + Sync,
>;

type RenewalWait = Arc<dyn Fn(Duration, &AtomicBool) -> bool + Send + Sync>;

struct ControlRenewalSource {
    connection: WireConnection,
    transport: ProductTransportFactory,
    clock: Arc<dyn Fn() -> Box<dyn MonotonicClock> + Send + Sync>,
    wait: RenewalWait,
}

impl Drop for ControlRenewalJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

// Re-sample the lease clock even if the OS wait itself excludes suspend.
#[cfg(any(target_os = "linux", test))]
fn native_renewal_wait(wait: Duration, cancelled: &AtomicBool) -> bool {
    if !cancelled.load(Ordering::SeqCst) {
        std::thread::park_timeout(wait.min(Duration::from_millis(250)));
    }
    !cancelled.load(Ordering::SeqCst)
}

impl ControlRenewalJob {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn<T: Transport + 'static>(
        session: CredentialSession,
        scope: AuthenticatedHostScope,
        operation: solidcraft_engine::entitlement::OperationId,
        grant: solidcraft_engine::entitlement::Grant,
        fixture_schedule: Option<(solidcraft_engine::entitlement::ConservativeDeadline, Duration)>,
        setup: impl FnOnce() -> (T, WireConnection, Box<dyn MonotonicClock>) + Send + 'static,
        mut wait_until_due: impl FnMut(Duration, &AtomicBool) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        let cutoff = Arc::new(Mutex::new(None));
        let published = Arc::clone(&cutoff);
        let cancelled = Arc::new(AtomicBool::new(false));
        let retired = Arc::clone(&cancelled);
        let generation = session.authority_generation();
        let credentials = session;
        let (worker_scope, worker_operation, worker_grant) = (scope.clone(), operation.clone(), grant.clone());
        let worker = std::thread::Builder::new().name("intcad-control-renewal".into()).spawn(move || {
            let (transport, connection, clock) = setup();
            let driver = match fixture_schedule {
                Some((initial, interval)) => solidcraft_account_native::OperationRenewal::new(
                    transport,
                    connection,
                    credentials.clone(),
                    clock,
                    worker_scope,
                    worker_operation,
                    worker_grant,
                    initial,
                    interval,
                ),
                None => solidcraft_account_native::OperationRenewal::from_admission(
                    transport,
                    connection,
                    credentials.clone(),
                    clock,
                    worker_scope,
                    worker_operation,
                    worker_grant,
                ),
            };
            let Ok(mut driver) = driver else {
                return;
            };
            while let Some(wait) = driver.wait_for_next_tick() {
                if retired.load(Ordering::SeqCst)
                    || !wait_until_due(wait, &retired)
                    || retired.load(Ordering::SeqCst)
                    || credentials.authority_generation() != generation
                {
                    break;
                }
                match driver.tick() {
                    Ok(true) => {
                        let Ok(mut slot) = published.lock() else { break };
                        if retired.load(Ordering::SeqCst) || credentials.authority_generation() != generation || credentials.is_revoked() {
                            break;
                        }
                        *slot = Some(driver.deadline().expires_at());
                    }
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        })?;
        Ok(Self { scope, operation, grant, generation, cutoff, cancelled, worker: Some(worker) })
    }

    #[cfg(test)]
    fn wait_for_fixture(&mut self) {
        if let Some(worker) = self.worker.take() {
            assert!(worker.join().is_ok());
        }
    }
}

/// The host-owned account runtime. `service` stays `None` until a reviewed account service and
/// authenticated durable settlement adapter can be installed.
pub struct AccountRuntime {
    credentials: CredentialSession,
    ledger: Option<FileOperationStore>,
    /// Measured safe-stop bounds for controlled entries. Retained unverified until a reviewed
    /// billing adapter and its evidence exist; nothing in this task makes a bound verified.
    #[allow(dead_code)]
    budget: BudgetTable,
    clock: Option<Box<dyn MonotonicClock + Send + Sync>>,
    service: Option<Box<dyn AccountService + Send + Sync>>,
    control_renewal: Option<ControlRenewalJob>,
    control_renewal_factory: Option<ControlRenewalFactory>,
    control_renewal_source: Option<ControlRenewalSource>,
    /// Explicit device sign-in settings, absent unless the host supplied all three trusted values.
    sign_in: Option<SignInConfig>,
    /// Test-injected transports. Production creates the documented HTTPS transports per job.
    sign_in_device: Option<DeviceTransportFactory>,
    sign_in_product: Option<ProductTransportFactory>,
    sign_in_timing: Option<Arc<dyn SignInTiming>>,
    sign_in_opener: Option<BrowserOpener>,
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
        let mut runtime = Self::new(credentials, ledger);
        if let Some(sign_in) = SignInConfig::from_environment() {
            // This installs transport and timing only. Without controlled admission
            // authority there is no grant and no worker or HTTPS request starts.
            #[cfg(target_os = "linux")]
            {
                runtime.control_renewal_source = Some(ControlRenewalSource {
                    connection: sign_in.product.clone(),
                    transport: Arc::new(|| Box::new(UreqTransport::with_limits(Duration::from_secs(5), Duration::from_secs(3)))),
                    clock: Arc::new(|| Box::new(SystemMonotonicClock::new())),
                    wait: Arc::new(native_renewal_wait),
                });
            }
            runtime.sign_in = Some(sign_in);
            runtime.sign_in_opener = Some(Arc::new(open_system_browser));
        } else {
            solidcraft_engine::note!("account: no complete WorkOS/AuthKit/product origin configuration; sign-in stays unavailable");
        }
        runtime
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
        Self {
            credentials,
            ledger,
            budget: BudgetTable::unverified_default(),
            clock: Some(Box::new(SystemMonotonicClock::new())),
            service: None,
            control_renewal: None,
            control_renewal_factory: None,
            control_renewal_source: None,
            sign_in: None,
            sign_in_device: None,
            sign_in_product: None,
            sign_in_timing: None,
            sign_in_opener: None,
        }
    }

    /// Test-only injection: fresh transport factories, a virtual clock and an inert browser
    /// opener, so orchestration tests never touch a network, a credential or a real browser.
    #[cfg(test)]
    fn with_sign_in_harness(
        mut self,
        config: SignInConfig,
        device: DeviceTransportFactory,
        product: ProductTransportFactory,
        timing: Arc<dyn SignInTiming>,
        opener: BrowserOpener,
    ) -> Self {
        self.sign_in = Some(config);
        self.sign_in_device = Some(device);
        self.sign_in_product = Some(product);
        self.sign_in_timing = Some(timing);
        self.sign_in_opener = Some(opener);
        self
    }

    /// The host's authenticated account, installation and session binding, or `None`.
    pub fn authenticated_scope(&self) -> Option<AuthenticatedHostScope> {
        self.credentials.authenticated_scope()
    }

    /// Read the secure-store facts once, before the window exists, so `install` does no keyring
    /// work on the window's thread.
    ///
    /// A persisted pair is only a restart candidate and this call performs no network work. The
    /// window can open at once; after it exists, [`DesktopAccountPort`] runs one bounded,
    /// account-bound `GET /v1/status` on its own native worker. There is no bootstrap, browser or
    /// operation replay here, and a verified result is presentation only: controlled authority
    /// stays closed unless a separately reviewed account service is installed synchronously.
    pub fn startup(&self) -> AccountStartup {
        let secure_store = self.credentials.is_persistent();
        // An unreadable tombstone is treated as present: revocation/safety cannot be confirmed.
        let logout_pending = self.credentials.logout_pending().unwrap_or(true);
        let restart = (!logout_pending).then(|| self.restart_binding(secure_store)).flatten();
        let candidate = restart.as_ref().map(|binding| binding.scope.clone());
        if candidate.is_some() {
            solidcraft_engine::note!("account: a complete local pair is a restart candidate; the bounded online check stays off the window thread");
        }
        AccountStartup { secure_store, logout_pending, candidate, restart, restored: None }
    }

    /// Capture the complete persisted pair and its revocation generation. A missing installation
    /// credential, a product-only login, a failed read and a store change during the read all
    /// yield no candidate.
    fn restart_binding(&self, secure_store: bool) -> Option<RestartBinding> {
        if !secure_store {
            return None;
        }
        let generation = self.credentials.authority_generation();
        let scope = self.authenticated_scope()?;
        if !crate::startup::can_restore_account_session(secure_store, self.credentials.product_session_active(), scope.is_provable()) {
            return None;
        }
        let product = WireCredentials::product_session(&self.credentials)?;
        if !WireCredentials::installation_bound(&self.credentials) {
            return None;
        }
        let installation = WireCredentials::installation_credential(&self.credentials)?;
        if self.credentials.authority_generation() != generation {
            return None;
        }
        Some(RestartBinding { scope, product, installation, generation })
    }

    /// Collect the immutable pieces a bounded restart worker needs. A missing sign-in
    /// configuration means no check can run and the candidate stays signed out.
    fn restart_verifier(&self) -> Option<RestartVerifier> {
        Some(RestartVerifier {
            product: self.sign_in.as_ref()?.product.clone(),
            credentials: self.credentials.clone(),
            transport: self.sign_in_product.clone(),
            ledger_root: self.ledger.as_ref().map(|ledger| ledger.root().to_path_buf()),
        })
    }

    /// The current safe-stop bounds, still unverified in this task.
    #[allow(dead_code)]
    pub fn budget(&self) -> &BudgetTable {
        &self.budget
    }

    /// Take the installed account service and clock only when an explicit synchronous checkpoint
    /// accepted the exact session. The bounded background restart check never fills that
    /// checkpoint, so its result can never install controlled authority.
    fn take_commit_authority(
        &mut self,
        startup: &AccountStartup,
    ) -> Option<(AuthenticatedHostScope, Box<dyn AccountService + Send>, Box<dyn MonotonicClock + Send>)> {
        // Check the service first: with no reviewed adapter this task performs no credential
        // read while the window is being created.
        let service = self.service.take()?;
        let clock = self.clock.take()?;
        let scope = startup.restored.clone()?;
        let current = self.authenticated_scope()?;
        (current == scope).then_some((scope, service, clock))
    }
}

fn account_unavailable(message: &'static str) -> Refusal {
    Refusal::new(RefusalCode::AuthUnavailable, message)
}

/// The immutable pieces of one bounded restart check that can move to a native worker.
struct RestartVerifier {
    product: WireConnection,
    credentials: CredentialSession,
    transport: Option<ProductTransportFactory>,
    ledger_root: Option<std::path::PathBuf>,
}

impl RestartVerifier {
    fn transport(&self) -> Box<dyn Transport + Send + Sync> {
        match self.transport.as_ref() {
            Some(factory) => factory(),
            None => Box::new(UreqTransport::with_limits(RESTART_STATUS_TIMEOUT, RESTART_STATUS_CONNECT_TIMEOUT)),
        }
    }

    /// Ask the configured service whether the exact local bearer belongs to the expected account.
    /// Only a 200 snapshot for that account, followed by an unchanged pair and revocation
    /// generation, yields a presentable scope. This check never writes credentials or opens a browser.
    fn verify(&self, binding: &RestartBinding, cancel: &SignInCancellation) -> Option<AuthenticatedHostScope> {
        if cancel.is_cancelled() {
            return None;
        }
        let service = HttpAccountService::with_transport(
            self.transport(),
            self.product.clone(),
            Box::new(FixedCredentials::none().with_product_session(binding.product.expose_secret())),
            Box::new(self.credentials.clone()),
            Box::new(SystemMonotonicClock::new()),
        );
        service.restart_status(binding.scope.account().as_str()).ok()?;
        if cancel.is_cancelled() {
            return None;
        }
        restart_binding_unchanged(&self.credentials, binding)
    }

    /// Replay durable intent only. The original settlement credential is deliberately the
    /// only credential supplied to this adapter, and a refusal leaves the row unresolved.
    fn recover_settlement(&self, binding: &RestartBinding, cancel: &SignInCancellation) {
        const MAX_FINALIZE_ATTEMPTS: usize = 4;
        let Some(root) = &self.ledger_root else { return };
        if cancel.is_cancelled() {
            return;
        }
        let Ok(mut ledger) = FileOperationStore::open(root) else {
            solidcraft_engine::note!("account: startup settlement could not open the ledger");
            return;
        };
        let Ok(rows) = ledger.unsettled_operations(Some(&binding.scope)) else {
            solidcraft_engine::note!("account: startup settlement could not verify the ledger inventory");
            return;
        };
        let mut attempts = 0;
        for row in rows {
            if cancel.is_cancelled() || attempts == MAX_FINALIZE_ATTEMPTS {
                break;
            }
            let UnsettledOperation::Owned(record) = row else { continue };
            let operation = &record.operation;
            let Ok(Some(intent)) = ledger.settlement_intent(operation) else { continue };
            let result = match intent {
                SettlementIntent::Commit(result) => result,
                SettlementIntent::Release(_) => BillingResult::FailedNoEffect,
            };
            // Another process may already have persisted a receipt since the inventory read.
            if !matches!(ledger.load(operation), Ok(Some(current)) if !current.status.is_terminal()) {
                continue;
            }
            let Some(credential) = WireCredentials::settlement_credential(&self.credentials, operation) else { continue };
            let mut service = HttpAccountService::with_transport(
                self.transport(),
                self.product.clone(),
                Box::new(FixedCredentials::none().with_settlement_credential(credential.expose_secret())),
                Box::new(self.credentials.clone()),
                Box::new(SystemMonotonicClock::new()),
            );
            if restart_binding_unchanged(&self.credentials, binding).is_none() || cancel.is_cancelled() {
                break;
            }
            attempts += 1;
            let receipt = service.finalize(&binding.scope, operation, result);
            if cancel.is_cancelled() || restart_binding_unchanged(&self.credentials, binding).is_none() {
                break;
            }
            if let Ok(status) = receipt
                && status.is_terminal()
                && let Err(error) = ledger.mark_terminal(operation, status)
                && error != solidcraft_engine::entitlement::OperationStoreError::TerminalState
            {
                solidcraft_engine::note!("account: startup settlement could not persist the terminal receipt");
            }
        }
    }
}

/// Re-read the exact scope, bearer, installation credential and revocation generation after the
/// network read. Any difference means the result belongs to a session that is no longer current.
fn restart_binding_unchanged(credentials: &CredentialSession, binding: &RestartBinding) -> Option<AuthenticatedHostScope> {
    if credentials.authority_generation() != binding.generation {
        return None;
    }
    let scope = credentials.authenticated_scope()?;
    if scope != binding.scope {
        return None;
    }
    if WireCredentials::product_session(credentials).as_ref() != Some(&binding.product) {
        return None;
    }
    if WireCredentials::installation_credential(credentials).as_ref() != Some(&binding.installation) {
        return None;
    }
    if credentials.authority_generation() != binding.generation {
        return None;
    }
    Some(scope)
}

/// Linux account deadlines share the suspend-inclusive boot epoch. Other platforms retain a
/// process-relative clock for the existing account paths; it is not approved for lease renewal.
struct SystemMonotonicClock {
    #[cfg(not(target_os = "linux"))]
    start: Instant,
    #[cfg(target_os = "linux")]
    failed: AtomicBool,
}

impl SystemMonotonicClock {
    fn new() -> Self {
        Self {
            #[cfg(not(target_os = "linux"))]
            start: Instant::now(),
            #[cfg(target_os = "linux")]
            failed: AtomicBool::new(false),
        }
    }
}

impl MonotonicClock for SystemMonotonicClock {
    #[cfg(target_os = "linux")]
    fn now(&self) -> MonotonicInstant {
        // Never fall back to a different epoch on a clock error. At the saturated value
        // no half-open deadline (including one constructed from this reading) is live.
        let expired = MonotonicInstant::from_millis(0).saturating_add(Duration::MAX);
        if self.failed.load(Ordering::SeqCst) {
            return expired;
        }
        let sample = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Boottime).ok().and_then(|time| {
            let seconds = u64::try_from(time.tv_sec).ok()?;
            let nanos = u32::try_from(time.tv_nsec).ok().filter(|nanos| *nanos < 1_000_000_000)?;
            Some(MonotonicInstant::from_millis(0).saturating_add(Duration::new(seconds, nanos)))
        });
        if let Some(sample) = sample {
            if !self.failed.load(Ordering::SeqCst) {
                return sample;
            }
        } else {
            self.failed.store(true, Ordering::SeqCst);
        }
        expired
    }

    #[cfg(not(target_os = "linux"))]
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_millis(self.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// The desktop implementation of the UI account seam.
///
/// `sign_in` starts one explicit, cancellable worker; every network and credential step happens
/// off the window thread, and the snapshot methods only read cached worker state. Sign-out and
/// account switch cancel the worker and revoke the session before the durable cleanup starts.
/// The view is cached; the render path never calls the port.
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
    /// One explicit device sign-in at a time. The worker owns the network; this only holds the
    /// cancellation flag and the state the window may read.
    sign_in: Option<SignInJob>,
    /// The bounded restart check, if a complete persisted pair was captured at startup. It fills
    /// presentation only and is retired by any explicit sign-in, sign-out, switch or shutdown.
    restart: Option<RestartJob>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl DesktopAccountPort {
    fn retire_control_renewal(&self) {
        if let Some(job) = &self.runtime.control_renewal {
            job.cancel();
        }
    }

    fn new(runtime: AccountRuntime, startup: &AccountStartup) -> Self {
        let view = current_view(startup);
        let mut port = Self {
            runtime,
            view,
            lifecycle: None,
            logout_pending: startup.logout_pending,
            secure_store: startup.secure_store,
            generation: 0,
            cleanup: Arc::new(Mutex::new(CleanupStatus::default())),
            sign_in: None,
            restart: None,
            workers: Vec::new(),
        };
        if let Some(binding) = startup.restart.clone() {
            port.start_restart(binding);
        }
        port
    }

    /// Start the one bounded restart check on its own native worker and return at once. The
    /// window's thread never performs this HTTP read.
    fn start_restart(&mut self, binding: RestartBinding) {
        let Some(verifier) = self.runtime.restart_verifier() else {
            solidcraft_engine::note!("account: a restart candidate has no complete service configuration; signed-in view stays closed");
            return;
        };
        let shared = Arc::new(Mutex::new(RestartShared::default()));
        let cancel = SignInCancellation::default();
        let worker_shared = Arc::clone(&shared);
        let worker_cancel = cancel.clone();
        let worker = std::thread::Builder::new().name("intcad-account-restart".into()).spawn(move || {
            let mut verified = verifier.verify(&binding, &worker_cancel);
            if verified.is_some() && verifier.ledger_root.is_some() {
                verifier.recover_settlement(&binding, &worker_cancel);
                verified = restart_binding_unchanged(&verifier.credentials, &binding);
            }
            let accepted = verified.is_some();
            // Retirement already marks the shared result done and clears any cached account. A
            // late reply is dropped instead of writing after that decision.
            if worker_cancel.is_cancelled() {
                return;
            }
            let Ok(mut shared) = worker_shared.lock() else { return };
            if worker_cancel.is_cancelled() {
                return;
            }
            shared.done = true;
            shared.account = if accepted { verified.map(|scope| scope.account().as_str().to_owned()) } else { None };
            drop(shared);
            if accepted {
                solidcraft_engine::note!("account: persisted account session verified by a bounded online status read; presentation only");
            } else {
                solidcraft_engine::note!("account: a restart candidate was not verified online; signed-in view stays closed");
            }
        });
        match worker {
            Ok(handle) => {
                self.workers.push(handle);
                self.restart = Some(RestartJob { cancel, shared });
            }
            Err(_) => solidcraft_engine::note!("account: the bounded restart check could not start; signed-in view stays closed"),
        }
    }

    /// Stop a device worker without waiting on its network request. The session's revocation
    /// barrier makes a late result write nothing, and the handle is joined on shutdown.
    fn cancel_sign_in(&mut self) {
        if let Some(job) = self.sign_in.take() {
            job.cancel.cancel();
        }
    }

    /// Retire the restart check without waiting on its network request. The shared result is
    /// marked done and its cached account cleared immediately; a reply that lands after this
    /// retirement is dropped rather than publishing beside a newer explicit attempt.
    fn cancel_restart(&mut self) {
        if let Some(job) = self.restart.as_mut() {
            job.cancel.cancel();
            if let Ok(mut shared) = job.shared.lock() {
                shared.account = None;
                shared.done = true;
            }
        }
    }

    fn completed_sign_in_account(&self) -> Option<String> {
        self.sign_in.as_ref()?.shared.lock().ok()?.account.clone()
    }

    fn verified_restart_account(&self) -> Option<String> {
        self.restart.as_ref()?.shared.lock().ok()?.account.clone()
    }

    fn restart_waiting(&self) -> bool {
        self.restart.as_ref().is_some_and(|job| job.shared.lock().is_ok_and(|shared| !shared.done))
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
        self.retire_control_renewal();
        // A running device flow or restart check belongs to the session being retired. Cancel
        // both before the generation barrier is published, so no later result can write for the
        // old session.
        self.cancel_restart();
        self.cancel_sign_in();
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
        let workers = std::mem::take(&mut self.workers);
        for worker in workers {
            let _ = worker.join();
        }
    }

    /// Start one explicit device flow on its own worker and return at once. The prompt becomes
    /// readable through [`AccountPort::begin_device_code`]; only an explicit browser action opens
    /// it. No request is sent before this call succeeds its configuration and sign-out gates.
    fn start_sign_in(&mut self) -> Result<(), Refusal> {
        self.cleanup_confirmed()?;
        if !self.secure_store {
            return Err(account_unavailable("no secure credential store is available"));
        }
        let Some(config) = self.runtime.sign_in.clone() else {
            return Err(account_unavailable("no configured sign-in service origins"));
        };
        if let Some(job) = self.sign_in.as_ref() {
            let phase = job.shared.lock().map_err(|_| account_unavailable("the sign-in state is unavailable"))?.phase;
            match phase {
                SignInPhase::Starting | SignInPhase::WaitingForApproval => {
                    return Err(account_unavailable("a sign-in is already in progress"));
                }
                SignInPhase::SignedIn => {
                    return Err(account_unavailable("the account is already signed in"));
                }
                // A refused or cancelled flow may be retried by a new explicit action. It is
                // never replayed: the old worker is dropped and a new device authorization starts.
                SignInPhase::Failed | SignInPhase::Cancelled => self.cancel_sign_in(),
            }
        }
        // The explicit sign-in becomes the current attempt; an older bounded restart result may
        // not land beside it, even if it carries the same pair.
        self.cancel_restart();
        let device_factory = self.runtime.sign_in_device.clone();
        let product_factory = self.runtime.sign_in_product.clone();
        let timing = self.runtime.sign_in_timing.clone();
        let session = self.runtime.credentials.clone();
        let shared = Arc::new(Mutex::new(SignInShared::default()));
        let cancel = SignInCancellation::default();
        let worker_shared = Arc::clone(&shared);
        let worker_cancel = cancel.clone();
        let worker = std::thread::Builder::new().name("intcad-account-sign-in".into()).spawn(move || {
            let device: Box<dyn DeviceCodeTransport + Send + Sync> = match device_factory.as_ref() {
                Some(factory) => factory(),
                None => Box::new(UreqDeviceCodeTransport::new()),
            };
            let product_transport: Box<dyn Transport + Send + Sync> = match product_factory.as_ref() {
                Some(factory) => factory(),
                None => Box::new(UreqTransport::new()),
            };
            let timing: Arc<dyn SignInTiming> = timing.unwrap_or_else(|| Arc::new(SystemSignInTiming::new()));
            let mut flow = DeviceCodeFlow::new(config.client, device, session, DeviceCodeMode::Explicit);
            let mut publish = |prompt: &DeviceCodePrompt| publish_device_prompt(&worker_shared, prompt);
            let result = drive_device_sign_in(&mut flow, &config.authkit, &config.product, product_transport, &*timing, &worker_cancel, &mut publish);
            apply_sign_in_result(&worker_shared, &worker_cancel, result);
        });
        match worker {
            Ok(handle) => {
                self.workers.push(handle);
                self.sign_in = Some(SignInJob { cancel, shared });
                Ok(())
            }
            Err(_) => Err(account_unavailable("the sign-in worker could not start")),
        }
    }
}

fn publish_device_prompt(shared: &Arc<Mutex<SignInShared>>, prompt: &DeviceCodePrompt) {
    let expires_at = wall_seconds().saturating_add(prompt.expires_in().as_secs());
    let code = DeviceCode {
        user_code: prompt.user_code().to_owned(),
        verification_uri: prompt.verification_uri().to_owned(),
        expires_at: Timestamp(expires_at),
    };
    let browser_uri = prompt.verification_uri_complete().unwrap_or_else(|| prompt.verification_uri()).to_owned();
    if let Ok(mut shared) = shared.lock() {
        shared.prompt = Some(code);
        shared.browser_uri = Some(browser_uri);
        shared.phase = SignInPhase::WaitingForApproval;
        shared.notice = None;
    }
}

fn apply_sign_in_result(shared: &Arc<Mutex<SignInShared>>, cancel: &SignInCancellation, result: Result<ProductLogin, Refusal>) {
    if let Ok(mut shared) = shared.lock() {
        // Every terminal phase retires the challenge, so a later panel or refresh never shows a
        // stale code or offers a browser action for a flow that already ended.
        shared.prompt = None;
        shared.browser_uri = None;
        if cancel.is_cancelled() {
            shared.phase = SignInPhase::Cancelled;
            return;
        }
        match result {
            Ok(login) => {
                shared.account = Some(login.account_id);
                shared.phase = SignInPhase::SignedIn;
                shared.notice = None;
            }
            Err(refusal) => {
                shared.phase = SignInPhase::Failed;
                shared.notice = Some(refusal);
            }
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
    fn control_admitted(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &solidcraft_engine::entitlement::OperationId,
        grant: &solidcraft_engine::entitlement::Grant,
    ) {
        // One slot, including a retired worker whose bounded request is still returning.
        if self
            .runtime
            .control_renewal
            .as_ref()
            .is_some_and(|job| !job.cancelled.load(Ordering::SeqCst) || job.worker.as_ref().is_some_and(|worker| !worker.is_finished()))
        {
            return;
        }
        if let Some(source) = &self.runtime.control_renewal_source {
            if grant.renewal_schedule(scope, operation).is_none() {
                return;
            }
            let connection = source.connection.clone();
            let transport = source.transport.clone();
            let clock = source.clock.clone();
            let wait = source.wait.clone();
            self.runtime.control_renewal = ControlRenewalJob::spawn(
                self.runtime.credentials.clone(),
                scope.clone(),
                operation.clone(),
                grant.clone(),
                None,
                move || (transport(), connection, clock()),
                move |interval, cancelled| wait(interval, cancelled),
            )
            .ok();
        } else if let Some(factory) = self.runtime.control_renewal_factory.as_mut() {
            self.runtime.control_renewal = factory(self.runtime.credentials.clone(), scope.clone(), operation.clone(), grant.clone()).ok();
        }
    }

    fn control_finished(&mut self, operation: &solidcraft_engine::entitlement::OperationId) {
        if self.runtime.control_renewal.as_ref().is_some_and(|job| &job.operation == operation) {
            self.retire_control_renewal();
        }
    }

    fn renewed_control_cutoff(
        &self,
        scope: &AuthenticatedHostScope,
        operation: &solidcraft_engine::entitlement::OperationId,
        grant: &solidcraft_engine::entitlement::Grant,
    ) -> Option<MonotonicInstant> {
        let job = self.runtime.control_renewal.as_ref()?;
        let credentials = &self.runtime.credentials;
        if &job.scope != scope || &job.operation != operation || &job.grant != grant || credentials.is_revoked() {
            return None;
        }
        let cutoff = *job.cutoff.lock().ok()?;
        (credentials.authority_generation() == job.generation && !job.cancelled.load(Ordering::SeqCst)).then_some(cutoff).flatten()
    }

    fn renewal_refusal(&self, scope: &AuthenticatedHostScope, operation: &solidcraft_engine::entitlement::OperationId) -> Option<RefusalCode> {
        self.runtime.credentials.renewal_refusal(scope, operation)
    }

    fn view(&self) -> AccountView {
        if let Some(account) = self.completed_sign_in_account() {
            return signed_in_view(&account);
        }
        if let Some(account) = self.verified_restart_account() {
            return signed_in_view(&account);
        }
        self.view.clone()
    }

    fn refresh_status(&mut self) -> Result<AccountView, Refusal> {
        self.cleanup_confirmed()?;
        // A completed device flow or an online-verified restart is recognized from cached worker
        // state. Neither is replayed or exchanged again; this process still installs no account
        // service, so the snapshot stays the one the check produced.
        if self.completed_sign_in_account().is_some() || self.verified_restart_account().is_some() {
            return Ok(self.view());
        }
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
        self.start_sign_in()
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
        let Some(job) = self.sign_in.as_ref() else {
            return Err(Refusal::new(RefusalCode::AuthRequired, "an explicit sign-in command is required"));
        };
        let shared = job.shared.lock().map_err(|_| account_unavailable("the sign-in state is unavailable"))?;
        shared.prompt.clone().ok_or_else(|| auth_required("the sign-in prompt is not ready"))
    }

    fn open_verification(&mut self) -> Result<(), Refusal> {
        let Some(job) = self.sign_in.as_ref() else {
            return Err(auth_required("start a sign-in first"));
        };
        let (phase, browser_uri, prompt_expires) = {
            let shared = job.shared.lock().map_err(|_| account_unavailable("the sign-in state is unavailable"))?;
            (shared.phase, shared.browser_uri.clone(), shared.prompt.as_ref().map(|prompt| prompt.expires_at.0))
        };
        if phase != SignInPhase::WaitingForApproval {
            return Err(auth_required("no sign-in is waiting for browser approval"));
        }
        let Some(browser_uri) = browser_uri else {
            return Err(auth_required("the sign-in prompt is not ready"));
        };
        if prompt_expires.is_some_and(|expires_at| expires_at <= wall_seconds()) {
            return Err(auth_required("the sign-in code expired"));
        }
        // The worker validated the response; the explicit action rechecks it against the same
        // configured origin because a browser is opened only here.
        if !self.runtime.sign_in.as_ref().is_some_and(|config| config.authkit.matches(&browser_uri)) {
            return Err(account_unavailable("the verification address is not trusted"));
        }
        let Some(opener) = self.runtime.sign_in_opener.clone() else {
            return Err(account_unavailable("no system browser opener is installed"));
        };
        opener(&browser_uri).map_err(|_| account_unavailable("the system browser could not be opened"))
    }

    fn take_sign_in_notice(&mut self) -> Option<Refusal> {
        self.sign_in.as_ref()?.shared.lock().ok()?.notice.take()
    }

    fn own_model_view(&self) -> Option<OwnModelView> {
        None
    }

    fn take_lifecycle_event(&mut self) -> Option<LifecycleEvent> {
        self.lifecycle.take()
    }

    fn startup_waiting(&self) -> bool {
        self.restart_waiting()
    }

    fn shutdown(&mut self) {
        self.cancel_restart();
        if let Some(mut job) = self.runtime.control_renewal.take() {
            job.cancel();
            if let Some(worker) = job.worker.take() {
                let _ = worker.join();
            }
        }
        self.join_workers();
    }
}

/// Install the desktop account runtime into the app.
///
/// The UI port is always installed so the start page and title bar have one host-owned seam. The
/// durable ledgers are installed when they opened. Commit and billing authority are installed
/// only from a synchronous verified scope; startup provides none, and the bounded background
/// restart check may change presentation but never authority. Controlled entries therefore keep
/// the existing default refusal.
pub fn install(app: &mut SolveApp, mut runtime: AccountRuntime, startup: AccountStartup) {
    if let Some((scope, service, clock)) = runtime.take_commit_authority(&startup) {
        app.install_commit_authority(scope, service, clock);
        solidcraft_engine::note!("account: authenticated commit authority installed for an explicit synchronous session");
    } else if startup.logout_pending {
        solidcraft_engine::note!("account: a previous sign-out is not confirmed; restore and controlled authority stay closed");
    } else if startup.candidate.is_some() {
        solidcraft_engine::note!("account: local restart candidate stays signed out until its bounded online check finishes; authority stays closed");
    } else {
        solidcraft_engine::note!("account: no account session restored; CAD stays anonymous and offline");
    }
    // Start the port while the runtime can still supply the ledger path. Recovery opens its own
    // handle on the worker; the window gets the existing handle for normal Review queries.
    let mut port = DesktopAccountPort::new(runtime, &startup);
    if let Some(ledger) = port.runtime.ledger.take() {
        app.install_review_store(Box::new(ledger));
    }
    app.account = State::with_port(Box::new(port));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_account_clock_reads_the_suspend_inclusive_boot_epoch() {
        let uptime = std::fs::read_to_string("/proc/uptime").unwrap();
        let boot_seconds: f64 = uptime.split_whitespace().next().unwrap().parse().unwrap();
        let first = SystemMonotonicClock::new().now();
        let second = SystemMonotonicClock::new().now();
        assert!(first.as_duration().as_secs_f64() >= boot_seconds, "{first:?} must use the OS boot epoch ({boot_seconds}s)");
        assert!(second >= first, "independent runtime clocks must share a monotonic epoch");
    }

    use std::collections::{BTreeMap, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use serde_json::json;
    use solidcraft_account_native::{
        CredentialHandoff, CredentialSession, DeviceCodeTransport, MemorySession, NoPersistentStore, ProductExchangeError, ProductLogin,
        ProductSession, ProductSessionExchange, ProviderSession, Secret, Transport, TransportFailure, WireCredentials, WireRequest, WireResponse,
    };
    use solidcraft_engine::entitlement::{
        AccountId, AccountService, Admission, Availability, BillingResult, Capability, CredentialError, CredentialErrorCode, CredentialStore,
        EntryCategory, Grant, HostSessionBinding, InstallationId, OperationId, OperationStatus, Refusal, RefusalCode,
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
            Ok(ProductSession::from_trusted_exchange("acct_desktop_0001", Secret::new("product_session_test_0000000000"), None))
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

    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(condition(), "timed out waiting for {what}");
    }

    fn wait_for_restart(port: &DesktopAccountPort) {
        wait_until("the bounded restart check", || !port.startup_waiting());
    }

    fn account_config() -> SignInConfig {
        SignInConfig::new("client_test", "https://auth.example", "https://api.example", "1.0.0").unwrap()
    }

    fn device_body() -> serde_json::Value {
        json!({
            "device_code": "device_code_SECRET_000000000000000000000000",
            "user_code": "BCDF-GHJK",
            "verification_uri": "https://auth.example/device",
            "verification_uri_complete": "https://auth.example/device?user_code=BCDF-GHJK",
            "expires_in": 300,
            "interval": 5
        })
    }

    fn device_body_until(expires_in: u64) -> serde_json::Value {
        let mut body = device_body();
        body["expires_in"] = json!(expires_in);
        body
    }

    fn token_body() -> serde_json::Value {
        json!({
            "user": {"object": "user", "id": "user_01E4ZCR3C56J083X43JQXF3JK5", "email_verified": true},
            "organization_id": "org_01H945H0YD4F97JN9MATX7BYAG",
            "access_token": "access_token_SECRET_000000000000000000000000",
            "authentication_method": "Password"
        })
    }

    fn status_body_for(account: &str) -> serde_json::Value {
        json!({
            "account": {"id": account, "email_verified": true, "status": "active"},
            "client": {"version": "1.0.0", "supported": true},
            "protocol": {"version": "intcad-auth/1.0", "supported": true},
            "eligible": true,
            "reasons": [],
            "capabilities": {"chat": true},
            "balance": 10,
            "held": 0,
            "available": 10
        })
    }

    fn status_body() -> serde_json::Value {
        status_body_for("acct_product_0001")
    }

    #[derive(Default)]
    struct DeviceScript {
        responses: VecDeque<Result<(u16, Vec<u8>), CredentialError>>,
        requests: Vec<String>,
        token_gate: Option<Receiver<()>>,
        cancel_on_authorized: Option<SignInCancellation>,
    }

    #[derive(Clone, Default)]
    struct ScriptedDevice {
        state: Arc<Mutex<DeviceScript>>,
        authorized_waiting: Arc<AtomicUsize>,
    }

    impl ScriptedDevice {
        fn scripted(responses: Vec<(u16, serde_json::Value)>) -> Self {
            let device = Self::default();
            {
                let mut state = device.state.lock().unwrap();
                for (status, body) in responses {
                    state.responses.push_back(Ok((status, body.to_string().into_bytes())));
                }
            }
            device
        }

        fn hold_token_success(&self, gate: Receiver<()>) {
            self.state.lock().unwrap().token_gate = Some(gate);
        }

        fn cancel_on_authorized(&self, cancel: SignInCancellation) {
            self.state.lock().unwrap().cancel_on_authorized = Some(cancel);
        }

        fn requests(&self) -> Vec<String> {
            self.state.lock().unwrap().requests.clone()
        }
    }

    impl DeviceCodeTransport for ScriptedDevice {
        fn post(&self, url: &str, _content_type: &str, _body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError> {
            let mut state = self.state.lock().unwrap();
            state.requests.push(url.to_owned());
            let response = state.responses.pop_front().unwrap_or_else(|| {
                Err(CredentialError { code: CredentialErrorCode::Unavailable, message: "inert scripted device transport".into() })
            });
            let authorized = match &response {
                Ok((status, body)) => {
                    *status == 200 && serde_json::from_slice::<serde_json::Value>(body).is_ok_and(|value| value.get("access_token").is_some())
                }
                Err(_) => false,
            };
            let gate = if authorized {
                if let Some(cancel) = state.cancel_on_authorized.clone() {
                    cancel.cancel();
                }
                state.token_gate.take()
            } else {
                None
            };
            drop(state);
            if let Some(gate) = gate {
                self.authorized_waiting.fetch_add(1, Ordering::SeqCst);
                let _ = gate.recv();
            }
            response
        }
    }

    fn installation_body() -> serde_json::Value {
        json!({
            "installation_id": "inst_product_0001",
            "installation_credential": "installation_credential_product_0000000000000000",
            "created_at": "2026-01-01T00:00:00.000Z"
        })
    }

    #[derive(Default)]
    struct ProductScript {
        responses: VecDeque<Result<WireResponse, TransportFailure>>,
        requests: Vec<WireRequest>,
    }

    #[derive(Clone, Default)]
    struct ScriptedProduct {
        state: Arc<Mutex<ProductScript>>,
        on_send: Option<Arc<dyn Fn() + Send + Sync>>,
    }

    impl ScriptedProduct {
        fn scripted(responses: Vec<Result<WireResponse, TransportFailure>>) -> Self {
            let product = Self::default();
            product.state.lock().unwrap().responses = responses.into();
            product
        }

        fn ok() -> Self {
            Self::scripted(vec![
                Ok(WireResponse::new(200, status_body().to_string().into_bytes())),
                Ok(WireResponse::new(201, installation_body().to_string().into_bytes())),
            ])
        }

        /// Run one inert fixture action while the request is in flight, so a test can retire the
        /// session before the response is applied.
        fn on_send(&mut self, action: impl Fn() + Send + Sync + 'static) {
            self.on_send = Some(Arc::new(action));
        }

        fn requests(&self) -> Vec<WireRequest> {
            self.state.lock().unwrap().requests.clone()
        }
    }

    impl Transport for ScriptedProduct {
        fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
            if let Some(action) = self.on_send.as_ref() {
                action();
            }
            let mut state = self.state.lock().unwrap();
            state.requests.push(request.clone());
            state.responses.pop_front().unwrap_or(Err(TransportFailure::Network))
        }
    }

    /// An inert product transport that holds only the first `GET /v1/status` until the test
    /// releases it. Later sign-in requests share the same scripted responses without waiting.
    #[derive(Clone)]
    struct GatedStatusProduct {
        inner: ScriptedProduct,
        gate: Arc<Mutex<Option<Receiver<()>>>>,
        entered: Arc<AtomicUsize>,
    }

    impl GatedStatusProduct {
        fn hold_first_status(inner: ScriptedProduct, gate: Receiver<()>) -> Self {
            Self { inner, gate: Arc::new(Mutex::new(Some(gate))), entered: Arc::new(AtomicUsize::new(0)) }
        }
    }

    impl Transport for GatedStatusProduct {
        fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
            if request.method() == "GET" && request.url().ends_with("/v1/status") {
                let gate = self.gate.lock().unwrap().take();
                if let Some(gate) = gate {
                    self.entered.fetch_add(1, Ordering::SeqCst);
                    let _ = gate.recv();
                }
            }
            self.inner.send(request)
        }
    }

    #[derive(Default)]
    struct VirtualTiming {
        millis: Mutex<u64>,
        waits: Mutex<Vec<Duration>>,
    }

    impl VirtualTiming {
        fn waits(&self) -> Vec<Duration> {
            self.waits.lock().unwrap().clone()
        }
    }

    impl SignInTiming for VirtualTiming {
        fn now(&self) -> Duration {
            Duration::from_millis(*self.millis.lock().unwrap())
        }

        fn wait(&self, interval: Duration, cancel: &SignInCancellation) -> bool {
            self.waits.lock().unwrap().push(interval);
            let mut millis = self.millis.lock().unwrap();
            *millis = millis.saturating_add(u64::try_from(interval.as_millis()).unwrap_or(0));
            !cancel.is_cancelled()
        }
    }

    #[derive(Clone, Default)]
    struct RecordingOpener {
        urls: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingOpener {
        fn opener(&self) -> BrowserOpener {
            let urls = Arc::clone(&self.urls);
            Arc::new(move |url: &str| {
                if let Ok(mut urls) = urls.lock() {
                    urls.push(url.to_owned());
                }
                Ok(())
            })
        }

        fn urls(&self) -> Vec<String> {
            self.urls.lock().unwrap().clone()
        }
    }

    fn harness_runtime(
        session: CredentialSession,
        device: ScriptedDevice,
        product: ScriptedProduct,
        timing: Arc<VirtualTiming>,
        opener: &RecordingOpener,
    ) -> AccountRuntime {
        let device_factory: DeviceTransportFactory = {
            let device = device.clone();
            Arc::new(move || Box::new(Arc::new(device.clone())))
        };
        let product_factory: ProductTransportFactory = {
            let product = product.clone();
            Arc::new(move || Box::new(Arc::new(product.clone())))
        };
        AccountRuntime::with_session(session).with_sign_in_harness(account_config(), device_factory, product_factory, timing, opener.opener())
    }

    /// A restart runtime with a configured scripted product service and an inert opener. Startup
    /// consumes exactly the scripted `/v1/status` response.
    fn restart_runtime(session: CredentialSession, product: ScriptedProduct, opener: &RecordingOpener) -> AccountRuntime {
        harness_runtime(session, ScriptedDevice::default(), product, Arc::new(VirtualTiming::default()), opener)
    }

    fn verified_restart_runtime(session: CredentialSession) -> AccountRuntime {
        let opener = RecordingOpener::default();
        restart_runtime(
            session,
            ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]),
            &opener,
        )
    }

    fn run_drive(
        device: ScriptedDevice,
        product: ScriptedProduct,
        timing: Arc<VirtualTiming>,
        cancel: SignInCancellation,
    ) -> (Result<ProductLogin, Refusal>, CredentialSession, Vec<String>, Vec<DeviceCodePrompt>) {
        let config = account_config();
        let session = CredentialSession::new(Box::new(MemorySession::default()));
        let mut flow = DeviceCodeFlow::new(config.client.clone(), device.clone(), session.clone(), DeviceCodeMode::Explicit);
        let mut prompts = Vec::new();
        let mut publish = |prompt: &DeviceCodePrompt| prompts.push(prompt.clone());
        let result = drive_device_sign_in(&mut flow, &config.authkit, &config.product, product.clone(), &*timing, &cancel, &mut publish);
        (result, session, device.requests(), prompts)
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
        let startup = runtime.startup();
        assert!(!startup.secure_store);
        assert!(startup.candidate.is_none());
        assert!(startup.restored.is_none());
    }

    #[test]
    fn a_persisted_session_is_not_signed_in_until_online_status_verifies_it() {
        // Restart fixture: a complete durable session exists, but no configured transport has
        // confirmed it with the product service in this process.
        let runtime = AccountRuntime::with_session(active_session());
        let startup = runtime.startup();
        assert!(startup.candidate.is_some(), "the complete pair is a local restart candidate");
        assert!(startup.restored.is_none(), "no online read accepted it");
        let port = DesktopAccountPort::new(runtime, &startup);
        assert!(!port.view().signed_in, "local credentials alone must not present a signed-in account");
        assert!(port.view().sign_in_visible, "an unverified restart stays signed out and offers an explicit sign-in");
    }

    #[test]
    fn a_restart_candidate_requires_an_active_product_session_and_a_complete_pair() {
        let no_product = AccountRuntime::with_session(installed_session());
        let startup = no_product.startup();
        assert!(startup.candidate.is_none(), "a provider-only or empty session is never a candidate");

        let active = AccountRuntime::with_session(active_session());
        assert!(active.authenticated_scope().is_some_and(|scope| scope.is_provable()));
        assert!(active.startup().candidate.is_some(), "the persisted product session and installation pair is complete");

        // A derived scope alone is not enough: the installation credential must be readable.
        let session = installed_session();
        session.complete_provider_login("provider_account", "provider_token").unwrap();
        session.activate_product_session(&TestExchange).unwrap();
        let scope = AuthenticatedHostScope::new(
            AccountId::new("acct_desktop_0001"),
            InstallationId::new("installation_desktop"),
            HostSessionBinding::new("local_scope_only"),
        );
        session.bind_scope(&scope).unwrap();
        assert!(session.authenticated_scope().is_some(), "the generic scope can be derived without a credential");
        assert!(WireCredentials::installation_credential(&session).is_none());
        let product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        let opener = RecordingOpener::default();
        let runtime = restart_runtime(session, product.clone(), &opener);
        let startup = runtime.startup();
        assert!(startup.candidate.is_none(), "a missing installation credential is not a complete pair");
        assert!(startup.restored.is_none());
        assert!(product.requests().is_empty(), "an incomplete pair sends no restart status request");
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn a_fresh_process_restores_the_persisted_scope_after_one_exact_online_status_read() {
        let store = TestStore::default();
        let previous = activate_session(&store);
        let previous_binding = previous.authenticated_scope().unwrap().session_binding().clone();
        let product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        let opener = RecordingOpener::default();

        // A new process has empty in-memory labels. Only the secure store proves the account,
        // installation and the exact product session; the binding must match the old one. Startup
        // returns before the network read; the port's bounded worker verifies the pair.
        let restarted = restart_runtime(CredentialSession::new(Box::new(store)), product.clone(), &opener);
        let startup = restarted.startup();
        assert!(startup.candidate.is_some(), "the complete pair was captured before the window");
        assert!(startup.restored.is_none(), "startup performs no synchronous online read");
        let mut port = DesktopAccountPort::new(restarted, &startup);
        wait_for_restart(&port);
        assert!(port.view().signed_in, "the bounded online check presents the exact persisted session");
        let scope = port.runtime.authenticated_scope().expect("the unchanged pair is the current session");
        assert_eq!(scope.account().as_str(), "acct_desktop_0001");
        assert_eq!(scope.installation().as_str(), "installation_desktop");
        assert_eq!(scope.session_binding(), &previous_binding);
        let requests = product.requests();
        assert_eq!(requests.len(), 1, "restart performs one status read and no bootstrap or replay");
        assert_eq!(requests[0].method(), "GET");
        assert_eq!(requests[0].url(), "https://api.example/v1/status");
        assert_eq!(requests[0].header("Authorization"), Some("Bearer product_session_test_0000000000"));
        assert!(requests[0].body().is_none());
        assert!(opener.urls().is_empty(), "restart never opens a browser");
        port.shutdown();
    }

    #[test]
    fn desktop_restart_replays_only_the_original_settlement_intent() {
        use solidcraft_engine::entitlement::{OperationStore, SettlementIntent, TransportCorrelation};

        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("intcad-restart-settlement-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(path);
        let store = TestStore::default();
        let mut session = activate_session(&store);
        let scope = session.authenticated_scope().unwrap();
        let foreign = AuthenticatedHostScope::new(scope.account().clone(), scope.installation().clone(), HostSessionBinding::new("foreign_session"));
        let mut ledger = FileOperationStore::open(&scratch.0).unwrap();
        let mut add = |owner, correlation, has_intent| {
            let row = ledger.persist_intent(owner, &TransportCorrelation::new(correlation), Capability::Review, EntryCategory::ReviewPanel).unwrap();
            if has_intent {
                ledger.record_settlement_intent(&row.operation, SettlementIntent::Commit(BillingResult::Succeeded)).unwrap();
            }
            CredentialHandoff::settlement_credential(&mut session, &row.operation, Secret::new("fixture_settlement")).unwrap();
            row.operation
        };
        let original = add(&scope, "original_settlement", true);
        let foreign = add(&foreign, "foreign_settlement", true);
        let no_intent = add(&scope, "missing_settlement", false);
        drop(ledger);
        drop(session);

        for (reply, expected, local_state) in [
            (Some(Err(TransportFailure::Timeout)), 2, OperationStatus::Held),
            (
                Some(Ok(WireResponse::new(
                    200,
                    json!({
                        "operation_id": original.as_str(), "state":"CHARGED", "result":"SUCCEEDED", "terminal":true,
                        "finalized_at":"2026-01-01T00:00:30Z", "settlement_deadline":"2026-01-01T00:11:00Z"
                    })
                    .to_string()
                    .into_bytes(),
                ))),
                2,
                OperationStatus::Charged,
            ),
            (None, 1, OperationStatus::Charged),
        ] {
            let mut responses = vec![Ok(WireResponse::new(200, status_body_for(scope.account().as_str()).to_string().into_bytes()))];
            responses.extend(reply);
            let product = ScriptedProduct::scripted(responses);
            let opener = RecordingOpener::default();
            let mut runtime = restart_runtime(CredentialSession::new(Box::new(store.clone())), product.clone(), &opener);
            runtime.ledger = Some(FileOperationStore::open(&scratch.0).unwrap());
            let startup = runtime.startup();
            assert!(startup.restored.is_none());
            let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
            install(&mut app, runtime, startup);
            wait_until("verified startup", || {
                app.account.process_actions();
                app.account.view().signed_in
            });
            let requests = product.requests();
            assert_eq!(requests.len(), expected);
            assert_eq!(requests[0].method(), "GET");
            assert_eq!(requests[0].url(), "https://api.example/v1/status");
            if let Some(request) = requests.get(1) {
                assert_eq!(request.method(), "POST");
                assert_eq!(request.url(), format!("https://api.example/v1/operations/{}/finalize", original.as_str()));
                assert_eq!(request.header("X-Settlement-Credential"), Some("fixture_settlement"));
                assert_eq!(request.header("Authorization"), None);
                assert_eq!(request.header("X-Execution-Session-Proof"), None);
                assert_eq!(serde_json::from_slice::<serde_json::Value>(request.body().unwrap()).unwrap(), json!({"result":"SUCCEEDED"}));
            }
            let ledger = FileOperationStore::open(&scratch.0).unwrap();
            assert_eq!(ledger.load(&original).unwrap().unwrap().status, local_state);
            for skipped in [&foreign, &no_intent] {
                assert_eq!(ledger.load(skipped).unwrap().unwrap().status, OperationStatus::Held);
            }
            assert!(!app.dfm_review.running);
            assert!(opener.urls().is_empty());
            app.account.shutdown();
        }
    }

    #[test]
    fn a_restart_status_reply_for_another_account_is_refused_without_replay() {
        let product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_other_0001").to_string().into_bytes()))]);
        let opener = RecordingOpener::default();
        let runtime = restart_runtime(active_session(), product.clone(), &opener);
        let startup = runtime.startup();
        assert!(startup.candidate.is_some());
        assert!(startup.restored.is_none(), "startup performs no synchronous online read");
        let port = DesktopAccountPort::new(runtime, &startup);
        wait_for_restart(&port);
        assert!(!port.view().signed_in, "a reply for another account never becomes this session");
        let requests = product.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url(), "https://api.example/v1/status");
        assert_eq!(requests[0].method(), "GET");
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn restart_status_matches_fresh_login_activation_conditions() {
        let mut unverified_email = status_body_for("acct_desktop_0001");
        unverified_email["account"]["email_verified"] = json!(false);
        unverified_email["eligible"] = json!(false);
        unverified_email["reasons"] = json!(["EMAIL_UNVERIFIED"]);

        let mut ineligible_client = status_body_for("acct_desktop_0001");
        ineligible_client["client"]["supported"] = json!(false);
        ineligible_client["eligible"] = json!(false);
        ineligible_client["reasons"] = json!(["VERSION_UNSUPPORTED"]);

        let mut wrong_protocol_version = status_body_for("acct_desktop_0001");
        wrong_protocol_version["protocol"]["version"] = json!("intcad-auth/2.0");

        let mut unsupported_protocol = status_body_for("acct_desktop_0001");
        unsupported_protocol["protocol"]["supported"] = json!(false);
        unsupported_protocol["eligible"] = json!(false);
        unsupported_protocol["reasons"] = json!(["PROTOCOL_UNSUPPORTED"]);

        for (case, body) in [
            ("unverified email", unverified_email),
            ("ineligible client", ineligible_client),
            ("wrong protocol version", wrong_protocol_version),
            ("unsupported protocol", unsupported_protocol),
        ] {
            let product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, body.to_string().into_bytes()))]);
            let opener = RecordingOpener::default();
            let runtime = restart_runtime(active_session(), product.clone(), &opener);
            let startup = runtime.startup();
            assert!(startup.candidate.is_some(), "{case}: the local pair is still a candidate");
            assert!(startup.restored.is_none(), "{case}: startup performs no synchronous online read");
            let port = DesktopAccountPort::new(runtime, &startup);
            wait_for_restart(&port);
            assert!(!port.view().signed_in, "{case}: an ineligible restart is not presented as signed in");
            assert_eq!(product.requests().len(), 1, "{case}: still one bounded status read");
            assert!(opener.urls().is_empty(), "{case}: restart opens no browser");
        }
    }

    #[test]
    fn restart_status_timeout_and_refusal_leave_the_candidate_closed() {
        for response in [
            Err(TransportFailure::Timeout),
            Err(TransportFailure::Network),
            Ok(WireResponse::new(401, Vec::new())),
            Ok(WireResponse::new(503, Vec::new())),
        ] {
            let product = ScriptedProduct::scripted(vec![response]);
            let opener = RecordingOpener::default();
            let runtime = restart_runtime(active_session(), product.clone(), &opener);
            let startup = runtime.startup();
            assert!(startup.candidate.is_some());
            assert!(startup.restored.is_none(), "startup performs no synchronous online read");
            let port = DesktopAccountPort::new(runtime, &startup);
            wait_for_restart(&port);
            assert!(!port.view().signed_in, "an unavailable or refused status never promotes presentation");
            assert_eq!(product.requests().len(), 1, "one bounded attempt, no retry or replay");
            assert!(opener.urls().is_empty());
        }
    }

    #[test]
    fn a_status_reply_that_arrives_after_revocation_is_stale_and_never_restores() {
        let session = active_session();
        let stale = session.clone();
        let mut product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        product.on_send(move || {
            stale.switch_account("account_other").unwrap();
        });
        let opener = RecordingOpener::default();
        let runtime = restart_runtime(session, product.clone(), &opener);
        let startup = runtime.startup();
        assert!(startup.candidate.is_some(), "the old pair was captured before the request");
        assert!(startup.restored.is_none(), "startup performs no synchronous online read");
        let port = DesktopAccountPort::new(runtime, &startup);
        wait_for_restart(&port);
        assert!(!port.view().signed_in, "a stale status result never promotes a signed-in view");
        assert_eq!(product.requests().len(), 1);
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn a_gated_restart_check_returns_from_startup_and_a_logic_tick_before_http() {
        let (gate_tx, gate_rx) = channel();
        let gate = Arc::new(Mutex::new(gate_rx));
        let entered = Arc::new(AtomicUsize::new(0));
        let mut product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        let worker_entered = Arc::clone(&entered);
        let worker_gate = Arc::clone(&gate);
        product.on_send(move || {
            worker_entered.fetch_add(1, Ordering::SeqCst);
            let _ = worker_gate.lock().unwrap().recv();
        });
        let opener = RecordingOpener::default();
        let runtime = restart_runtime(active_session(), product.clone(), &opener);

        // Startup reads only local facts, so it must complete with no request sent.
        let startup = runtime.startup();
        assert!(startup.candidate.is_some());
        assert!(startup.restored.is_none());
        assert!(product.requests().is_empty(), "startup sent a request before the window existed");

        let mut app = solidcraft_ui_egui::SolveApp::new(solidcraft_engine::Session::default(), solidcraft_ui_egui::Services::default());
        install(&mut app, runtime, startup);
        wait_for_counter(&entered, 1);
        // The transport is still blocked. Both installation and this logic tick already returned.
        app.account.process_actions();
        assert!(!app.account.view().signed_in, "a pending check is never presented as signed in");
        assert!(app.account.startup_waiting(), "the host should wake while the bounded check runs");

        gate_tx.send(()).unwrap();
        wait_until("the gated restart view", || {
            app.account.process_actions();
            app.account.view().signed_in
        });
        assert_eq!(app.account.view().account_label.as_deref(), Some("acct_desktop_0001"));
        assert!(!app.account.startup_waiting(), "a finished check stops the bounded wake");
        assert_eq!(product.requests().len(), 1, "one status read and no bootstrap or replay");
        assert_eq!(product.requests()[0].method(), "GET");
        assert_eq!(product.requests()[0].url(), "https://api.example/v1/status");
        assert!(opener.urls().is_empty(), "the restart check never opens a browser");
        app.account.shutdown();
    }

    #[test]
    fn a_retired_restart_reply_after_sign_out_is_discarded_without_replay() {
        let (gate_tx, gate_rx) = channel();
        let gate = Arc::new(Mutex::new(gate_rx));
        let entered = Arc::new(AtomicUsize::new(0));
        let mut product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        let worker_entered = Arc::clone(&entered);
        let worker_gate = Arc::clone(&gate);
        product.on_send(move || {
            worker_entered.fetch_add(1, Ordering::SeqCst);
            let _ = worker_gate.lock().unwrap().recv();
        });
        let opener = RecordingOpener::default();
        let store = TestStore::default();
        let runtime = restart_runtime(activate_session(&store), product.clone(), &opener);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        wait_for_counter(&entered, 1);
        assert!(!port.view().signed_in, "a pending check is never presented as signed in");
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        gate_tx.send(()).unwrap();
        port.shutdown();
        assert!(!port.view().signed_in, "a reply for the retired pair is discarded");
        assert!(port.runtime.credentials.authenticated_scope().is_none(), "the sign-out pair stays gone");
        assert_eq!(product.requests().len(), 1, "the in-flight request is never replayed");
        assert_eq!(product.requests()[0].method(), "GET");
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn an_explicit_sign_in_retires_the_restart_check_and_drops_its_late_reply() {
        let (gate_tx, gate_rx) = channel();
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::scripted(vec![
            Ok(WireResponse::new(200, status_body().to_string().into_bytes())),
            Ok(WireResponse::new(201, installation_body().to_string().into_bytes())),
            Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes())),
        ]);
        let gated = GatedStatusProduct::hold_first_status(product.clone(), gate_rx);
        let device_factory: DeviceTransportFactory = {
            let device = device.clone();
            Arc::new(move || Box::new(Arc::new(device.clone())))
        };
        let product_factory: ProductTransportFactory = {
            let gated = gated.clone();
            Arc::new(move || Box::new(Arc::new(gated.clone())))
        };
        let opener = RecordingOpener::default();
        let store = TestStore::default();
        let runtime = AccountRuntime::with_session(activate_session(&store)).with_sign_in_harness(
            account_config(),
            device_factory,
            product_factory,
            Arc::new(VirtualTiming::default()),
            opener.opener(),
        );
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        wait_for_counter(&gated.entered, 1);
        port.sign_in().unwrap();
        wait_until("the explicit sign-in", || port.completed_sign_in_account().is_some());
        assert_eq!(port.view().account_label.as_deref(), Some("acct_product_0001"));
        assert!(!port.startup_waiting(), "retirement marks the old check done before its network returns");

        gate_tx.send(()).unwrap();
        port.shutdown();
        assert!(port.verified_restart_account().is_none(), "a late restart reply is dropped after an explicit sign-in");
        assert_eq!(port.view().account_label.as_deref(), Some("acct_product_0001"), "the completed sign-in stays current");
        assert_eq!(product.requests().len(), 3, "the late status read is consumed once and never replayed");
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn a_retired_restart_reply_after_switch_is_discarded_without_replay() {
        let (gate_tx, gate_rx) = channel();
        let gate = Arc::new(Mutex::new(gate_rx));
        let entered = Arc::new(AtomicUsize::new(0));
        let mut product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        let worker_entered = Arc::clone(&entered);
        let worker_gate = Arc::clone(&gate);
        product.on_send(move || {
            worker_entered.fetch_add(1, Ordering::SeqCst);
            let _ = worker_gate.lock().unwrap().recv();
        });
        let opener = RecordingOpener::default();
        let store = TestStore::default();
        let runtime = restart_runtime(activate_session(&store), product.clone(), &opener);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        wait_for_counter(&entered, 1);
        assert!(!port.view().signed_in);
        port.switch_account().unwrap();
        wait_for_cleanup(&port);
        gate_tx.send(()).unwrap();
        port.shutdown();
        assert!(port.verified_restart_account().is_none(), "a reply for the switched-away pair is discarded");
        assert_eq!(product.requests().len(), 1, "the in-flight request is never replayed");
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn retiring_a_restart_check_marks_done_immediately_and_drops_the_late_reply() {
        let (gate_tx, gate_rx) = channel();
        let gate = Arc::new(Mutex::new(gate_rx));
        let entered = Arc::new(AtomicUsize::new(0));
        let mut product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        let worker_entered = Arc::clone(&entered);
        let worker_gate = Arc::clone(&gate);
        product.on_send(move || {
            worker_entered.fetch_add(1, Ordering::SeqCst);
            let _ = worker_gate.lock().unwrap().recv();
        });
        let opener = RecordingOpener::default();
        let runtime = restart_runtime(active_session(), product.clone(), &opener);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        wait_for_counter(&entered, 1);
        assert!(port.startup_waiting());
        // Shutdown follows this same retirement step before joining its worker.
        port.cancel_restart();
        assert!(!port.startup_waiting(), "retirement marks shared done immediately");
        gate_tx.send(()).unwrap();
        port.shutdown();
        assert!(port.verified_restart_account().is_none(), "the late reply is dropped");
        assert!(!port.view().signed_in);
        assert_eq!(product.requests().len(), 1);
        assert!(opener.urls().is_empty());
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
        assert!(startup.candidate.is_none());
        assert!(startup.restored.is_none(), "an in-memory session must not survive a restart");
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
        assert!(startup.candidate.is_none());
        assert!(startup.restored.is_none());

        // Once the inert store is healthy, the marker still suppresses restore until a reviewed
        // sign-in path clears it.
        store.fail_delete.store(false, Ordering::SeqCst);
        let startup = AccountRuntime::with_session(CredentialSession::new(Box::new(store))).startup();
        assert!(startup.logout_pending);
        assert!(startup.candidate.is_none());
        assert!(startup.restored.is_none());
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
        let startup = runtime.startup();
        assert!(startup.candidate.is_none(), "a provider login alone is never a restart candidate");
        assert!(startup.restored.is_none());
    }

    #[test]
    fn sign_out_and_switch_clear_credentials_and_raise_one_lifecycle_event() {
        for (action, expected) in [(Action::SignOut, LifecycleEvent::SignedOut), (Action::SwitchAccount, LifecycleEvent::AccountSwitched)] {
            let runtime = verified_restart_runtime(active_session());
            let startup = runtime.startup();
            let mut port = DesktopAccountPort::new(runtime, &startup);
            wait_for_restart(&port);
            assert!(port.view().signed_in, "the online-verified restart is presented");
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
    fn a_wire_401_stops_the_original_control_operation_before_lease_expiry() {
        use solidcraft_account_native::OperationRenewal;
        use solidcraft_engine::entitlement::{ConservativeDeadline, GrantId};
        use std::sync::atomic::AtomicU64;

        struct FixtureBilling;
        impl solidcraft_engine::entitlement::BillingPort for FixtureBilling {
            fn admit(&mut self, _: &OperationId, capability: Capability, _: EntryCategory) -> Result<Grant, Refusal> {
                Ok(Grant::from_server(GrantId::new("grant_desktop_test"), capability, MonotonicInstant::from_millis(60_000)))
            }
            fn commit(&mut self, _: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
                Ok(result)
            }
            fn release(&mut self, _: &OperationId, _: RefusalCode) -> Result<(), Refusal> {
                Ok(())
            }
        }
        struct Clock;
        impl MonotonicClock for Clock {
            fn now(&self) -> MonotonicInstant {
                MonotonicInstant::from_millis(20_000)
            }
            fn wall_now(&self) -> SystemTime {
                UNIX_EPOCH
            }
        }
        let mut session = active_session();
        let scope = session.authenticated_scope().unwrap();
        let (mut req, _) = solidcraft_ui_egui::ControlRequest::trusted_control("engine.execute", json!({"command":"document.inspect"}));
        let operation = req.operation().unwrap().clone();
        CredentialHandoff::execution_session_proof(&mut session, Capability::Control, &operation, Secret::new("test_proof")).unwrap();
        fn tick_unauthorized(session: CredentialSession, scope: AuthenticatedHostScope, operation: OperationId) {
            struct DriverClock(Arc<AtomicU64>);
            impl MonotonicClock for DriverClock {
                fn now(&self) -> MonotonicInstant {
                    MonotonicInstant::from_millis(self.0.load(Ordering::Relaxed))
                }
                fn wall_now(&self) -> SystemTime {
                    UNIX_EPOCH
                }
            }
            let time = Arc::new(AtomicU64::new(0));
            let grant = Grant::from_server(GrantId::new("grant_desktop_test"), Capability::Control, MonotonicInstant::from_millis(60_000));
            let mut renewal = OperationRenewal::new(
                ScriptedProduct::scripted(vec![Ok(WireResponse::new(401, br#"{"error":{"code":"AUTH_REQUIRED"}}"#.to_vec()))]),
                WireConnection::new("https://product.invalid", "1.0.0").unwrap(),
                session,
                Box::new(DriverClock(Arc::clone(&time))),
                scope,
                operation,
                grant,
                ConservativeDeadline::new(MonotonicInstant::from_millis(0), Duration::from_secs(60)),
                Duration::from_secs(20),
            )
            .unwrap();
            time.store(20_000, Ordering::Relaxed);
            assert_eq!(renewal.tick().unwrap_err().code, RefusalCode::AuthRequired);
            assert_eq!(renewal.deadline().expires_at(), MonotonicInstant::from_millis(60_000));
        }
        let runtime = AccountRuntime::with_session(session.clone());
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        let startup = runtime.startup();
        install(&mut app, runtime, startup);
        let mut budget = BudgetTable::unverified_default();
        budget.set_verified(Capability::Control, EntryCategory::DirectControl, Duration::from_secs(1));
        app.install_billing(Box::new(FixtureBilling), budget);
        app.install_commit_authority(scope.clone(), Box::new(InertService), Box::new(Clock));

        tick_unauthorized(session.clone(), scope.clone(), operation);
        let control::Outcome::Done(reply) = control::handle(&mut app, &egui::Context::default(), &mut req) else {
            panic!("a rejected operation must return an immediate refusal");
        };
        assert_eq!(reply["errorDetails"]["code"], "AuthRequired", "{reply}");

        let new_request = || solidcraft_ui_egui::ControlRequest::trusted_control("document.inspect", json!({})).0;
        let mut finishing = new_request();
        let finishing_id = finishing.operation().unwrap().clone();
        CredentialHandoff::execution_session_proof(&mut session, Capability::Control, &finishing_id, Secret::new("test_proof")).unwrap();
        let worker_session = session.clone();
        let worker_scope = scope.clone();
        let callback = move || tick_unauthorized(worker_session.clone(), worker_scope.clone(), finishing_id.clone());
        app.install_commit_authority(
            scope.clone(),
            Box::new(RenewalTestAuthority { on_third_lookup: Box::new(callback), lookups: AtomicUsize::new(0) }),
            Box::new(Clock),
        );
        let control::Outcome::Done(reply) = control::handle(&mut app, &egui::Context::default(), &mut finishing) else {
            panic!("a rejected result must return an immediate refusal");
        };
        assert_eq!(reply["errorDetails"]["code"], "AuthRequired", "{reply}");
        assert!(reply["error"].as_str().unwrap().starts_with("refused before writing the result"), "{reply}");

        let control::Outcome::Done(reply) = control::handle(&mut app, &egui::Context::default(), &mut new_request()) else {
            panic!("an unrelated inspection must return its result");
        };
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(session.authenticated_scope(), Some(scope), "401 on one operation does not sign out the session");
    }

    #[test]
    fn direct_control_admission_calls_the_optional_renewal_factory() {
        let session = active_session();
        let scope = session.authenticated_scope().unwrap();
        let (mut request, _) = solidcraft_ui_egui::ControlRequest::trusted_control("document.inspect", json!({}));
        let operation = request.operation().unwrap().clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let mut runtime = AccountRuntime::with_session(session);
        runtime.control_renewal_factory = Some(Box::new(move |session, scope, admitted, grant| {
            assert_eq!(admitted, operation);
            assert_eq!(session.authenticated_scope(), Some(scope));
            assert_eq!(grant.capability(), Capability::Control);
            observed.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::other("fixture: no renewal worker"))
        }));
        let startup = runtime.startup();
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        install(&mut app, runtime, startup);
        let mut budget = BudgetTable::unverified_default();
        budget.set_verified(Capability::Control, EntryCategory::DirectControl, Duration::from_secs(1));
        struct Billing(MonotonicInstant);
        impl solidcraft_engine::entitlement::BillingPort for Billing {
            fn admit(&mut self, _: &OperationId, capability: Capability, _: EntryCategory) -> Result<Grant, Refusal> {
                Ok(Grant::from_server(solidcraft_engine::entitlement::GrantId::new("grant_registration_fixture"), capability, self.0))
            }
            fn commit(&mut self, _: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
                Ok(result)
            }
            fn release(&mut self, _: &OperationId, _: RefusalCode) -> Result<(), Refusal> {
                Ok(())
            }
        }
        let clock = SystemMonotonicClock::new();
        app.install_billing(Box::new(Billing(clock.now().saturating_add(Duration::from_secs(60)))), budget);
        app.install_commit_authority(
            scope,
            Box::new(RenewalTestAuthority { on_third_lookup: Box::new(|| {}), lookups: AtomicUsize::new(0) }),
            Box::new(clock),
        );
        let control::Outcome::Done(reply) = control::handle(&mut app, &egui::Context::default(), &mut request) else {
            panic!("expected a synchronous inspection");
        };
        assert_eq!(reply["ok"], true, "a failed worker start does not extend or void the original lease: {reply}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn direct_control_admission_starts_a_worker_that_rejects_a_reply_after_suspend() {
        use std::sync::atomic::AtomicU64;

        struct ObservedTransport(ScriptedProduct, std::sync::mpsc::Sender<()>);
        impl Transport for ObservedTransport {
            fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
                self.0.send(request)
            }
        }
        impl Drop for ObservedTransport {
            fn drop(&mut self) {
                let _ = self.1.send(());
            }
        }

        #[derive(Clone)]
        struct Clock(Arc<AtomicU64>);
        impl MonotonicClock for Clock {
            fn now(&self) -> MonotonicInstant {
                MonotonicInstant::from_millis(self.0.load(Ordering::SeqCst))
            }
            fn wall_now(&self) -> SystemTime {
                UNIX_EPOCH + Duration::from_secs(1_767_225_600)
            }
        }
        struct Billing(Grant, Arc<AtomicUsize>);
        impl solidcraft_engine::entitlement::BillingPort for Billing {
            fn admit(&mut self, _: &OperationId, _: Capability, _: EntryCategory) -> Result<Grant, Refusal> {
                Ok(self.0.clone())
            }
            fn commit(&mut self, _: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
                self.1.fetch_add(1, Ordering::SeqCst);
                Ok(result)
            }
            fn release(&mut self, _: &OperationId, _: RefusalCode) -> Result<(), Refusal> {
                Ok(())
            }
        }
        for (received_at, accepted) in [(21_000, true), (60_001, false)] {
            let time = Arc::new(AtomicU64::new(0));
            let clock = Clock(time.clone());
            let session = active_session();
            let scope = session.authenticated_scope().unwrap();
            let (mut request, _) = solidcraft_ui_egui::ControlRequest::trusted_control("document.inspect", json!({}));
            let operation = request.operation().unwrap().clone();
            let connection = WireConnection::new("https://product.invalid", "1.0.0").unwrap();
            let admission_body = json!({
                "outcome":"NEWLY_ADMITTED","operation_id":operation.as_str(),"installation_id":scope.installation().as_str(),
                "state":"HELD","settlement_deadline":null,"authorization_expires_at":"2026-01-01T00:01:00.000Z",
                "execution_grant":{"grant_id":"grant_admitted_fixture","capability":"control","expires_at":"2026-01-01T00:01:00.000Z",
                    "renew_after_seconds":17,"session_proof":"admitted_proof"},
                "settlement_credential":"fixture_settlement"
            });
            let mut admission_service = HttpAccountService::with_transport(
                ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, admission_body.to_string().into_bytes()))]),
                connection.clone(),
                Box::new(session.clone()),
                Box::new(session.clone()),
                Box::new(clock.clone()),
            );
            let Admission::NewlyAdmitted { grant, .. } =
                admission_service.begin(&scope, &operation, Capability::Control, EntryCategory::DirectControl).unwrap()
            else {
                panic!("expected scripted admission")
            };
            let body = json!({
                "authorization_id":grant.id().as_str(), "operation_id":operation.as_str(),
                "capability":"control", "renewed":true, "remaining_seconds":60, "renew_after_seconds":20,
                "execution_session_proof":null, "expires_at":"2026-01-01T00:01:20.000Z"
            });
            let mut wire = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, body.to_string().into_bytes()))]);
            let reply_time = time.clone();
            wire.on_send(move || reply_time.store(received_at, Ordering::SeqCst));
            let (finished_tx, finished_rx) = channel();
            let finished_rx = Mutex::new(finished_rx);
            let worker_wire = wire.clone();
            let worker_clock = clock.clone();
            let sent_time = time.clone();
            let mut runtime = AccountRuntime::with_session(session);
            runtime.control_renewal_source = Some(ControlRenewalSource {
                connection,
                transport: Arc::new(move || Box::new(ObservedTransport(worker_wire.clone(), finished_tx.clone()))),
                clock: Arc::new(move || Box::new(worker_clock.clone())),
                wait: Arc::new(move |wait, _| {
                    if sent_time.load(Ordering::SeqCst) != 0 {
                        return false;
                    }
                    assert_eq!(wait, Duration::from_secs(17));
                    sent_time.store(17_000, Ordering::SeqCst);
                    true
                }),
            });
            let startup = runtime.startup();
            let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
            install(&mut app, runtime, startup);
            let mut budget = BudgetTable::unverified_default();
            budget.set_verified(Capability::Control, EntryCategory::DirectControl, Duration::from_secs(1));
            let commits = Arc::new(AtomicUsize::new(0));
            app.install_billing(Box::new(Billing(grant, commits.clone())), budget);
            app.install_commit_authority(
                scope,
                Box::new(RenewalTestAuthority {
                    on_third_lookup: Box::new(move || {
                        finished_rx.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
                        time.store(65_000, Ordering::SeqCst);
                    }),
                    lookups: AtomicUsize::new(0),
                }),
                Box::new(clock),
            );
            let control::Outcome::Done(reply) = control::handle(&mut app, &egui::Context::default(), &mut request) else {
                panic!("expected a synchronous inspection");
            };
            assert_eq!(reply["ok"], accepted, "{reply}");
            assert_eq!(commits.load(Ordering::SeqCst), usize::from(accepted));
            let requests = wire.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].header("X-Execution-Session-Proof"), Some("admitted_proof"));
            assert_eq!(requests[0].header("X-Settlement-Credential"), None);
        }
    }

    #[test]
    fn native_control_renewal_wait_stops_without_sending_after_finish_or_switch() {
        use solidcraft_engine::entitlement::{ConservativeDeadline, GrantId};

        for switch in [false, true] {
            let mut session = active_session();
            let scope = session.authenticated_scope().unwrap();
            let operation = OperationId::parse("op_cancel_wait_fixture").unwrap();
            CredentialHandoff::execution_session_proof(&mut session, Capability::Control, &operation, Secret::new("fixture_proof")).unwrap();
            let clock = SystemMonotonicClock::new();
            let initial = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
            let grant = Grant::from_server(GrantId::new("grant_cancel_fixture"), Capability::Control, initial.expires_at());
            let wire = ScriptedProduct::default();
            let worker_wire = wire.clone();
            let (waiting_tx, waiting_rx) = channel();
            let (returned_tx, returned_rx) = channel();
            let job = ControlRenewalJob::spawn(
                session.clone(),
                scope,
                operation.clone(),
                grant,
                Some((initial, Duration::from_secs(30))),
                move || (worker_wire, WireConnection::new("https://product.invalid", "1.0.0").unwrap(), Box::new(clock)),
                move |wait, cancelled| {
                    waiting_tx.send(()).unwrap();
                    let elapsed = native_renewal_wait(wait, cancelled);
                    returned_tx.send(elapsed).unwrap();
                    elapsed
                },
            )
            .unwrap();
            let mut runtime = AccountRuntime::with_session(session);
            runtime.control_renewal = Some(job);
            let startup = runtime.startup();
            let mut port = DesktopAccountPort::new(runtime, &startup);
            waiting_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if switch {
                port.switch_account().unwrap();
            } else {
                port.control_finished(&operation);
            }
            assert!(!returned_rx.recv_timeout(Duration::from_secs(5)).unwrap());
            let job = port.runtime.control_renewal.as_mut().unwrap();
            job.wait_for_fixture();
            assert!(wire.requests().is_empty());
            assert_eq!(*job.cutoff.lock().unwrap(), None);
            port.shutdown();
        }
    }

    #[test]
    fn background_control_renewal_only_publishes_a_matching_timely_reply() {
        use solidcraft_engine::entitlement::{ConservativeDeadline, GrantId};
        use std::sync::atomic::AtomicU64;

        #[derive(Clone)]
        struct Clock(Arc<AtomicU64>);
        impl MonotonicClock for Clock {
            fn now(&self) -> MonotonicInstant {
                MonotonicInstant::from_millis(self.0.load(Ordering::SeqCst))
            }
            fn wall_now(&self) -> SystemTime {
                UNIX_EPOCH
            }
        }
        struct Billing(Grant);
        impl solidcraft_engine::entitlement::BillingPort for Billing {
            fn admit(&mut self, _: &OperationId, _: Capability, _: EntryCategory) -> Result<Grant, Refusal> {
                Ok(self.0.clone())
            }
            fn commit(&mut self, _: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
                Ok(result)
            }
            fn release(&mut self, _: &OperationId, _: RefusalCode) -> Result<(), Refusal> {
                Ok(())
            }
        }
        #[derive(Clone, Copy)]
        enum Retirement {
            Finish,
            SignOut,
            Switch,
        }
        for (status, received, matching, second_reply, dispatch_at, expected, responses, retire) in [
            (200, 21_000, true, 41_000, 85_000, None, 2, None),
            (200, 21_000, true, 80_000, 85_000, Some("UnsafeToStart"), 2, None),
            (200, 60_000, true, 41_000, 65_000, Some("UnsafeToStart"), 1, None),
            (200, 21_000, false, 41_000, 65_000, Some("UnsafeToStart"), 1, None),
            (401, 21_000, true, 41_000, 21_000, Some("AuthRequired"), 1, None),
            (200, 21_000, true, 41_000, 65_000, None, 1, Some(Retirement::Finish)),
            (200, 21_000, true, 41_000, 65_000, None, 1, Some(Retirement::SignOut)),
            (200, 21_000, true, 41_000, 65_000, None, 1, Some(Retirement::Switch)),
        ] {
            let time = Arc::new(AtomicU64::new(0));
            let clock = Clock(time.clone());
            let mut session = active_session();
            let scope = session.authenticated_scope().unwrap();
            let (mut request, _) = solidcraft_ui_egui::ControlRequest::trusted_control("document.inspect", json!({}));
            let operation = request.operation().unwrap().clone();
            CredentialHandoff::execution_session_proof(&mut session, Capability::Control, &operation, Secret::new("fixture_control_proof")).unwrap();
            let grant = Grant::from_server(GrantId::new("grant_control_fixture"), Capability::Control, MonotonicInstant::from_millis(60_000));
            let body = json!({
                "authorization_id":"grant_control_fixture", "operation_id": if matching { operation.as_str() } else { "op_foreign_fixture" },
                "capability":"control","renewed":true,"remaining_seconds":60,"renew_after_seconds":20,
                "execution_session_proof":null,"expires_at":"2026-01-01T00:01:20.000Z"
            });
            let body = if status == 401 { json!({"error":{"code":"AUTH_REQUIRED"}}) } else { body };
            let mut wire = ScriptedProduct::scripted((0..responses).map(|_| Ok(WireResponse::new(status, body.to_string().into_bytes()))).collect());
            let response_time = time.clone();
            let (entered_tx, entered_rx) = channel();
            let (release_tx, release_rx) = channel();
            let release_rx = Mutex::new(release_rx);
            wire.on_send(move || {
                if retire.is_some() {
                    entered_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
                }
                let at = response_time.load(Ordering::SeqCst);
                response_time.store(if at == 20_000 { received } else { second_reply }, Ordering::SeqCst);
            });
            let worker_clock = clock.clone();
            let worker_wire = wire.clone();
            let send_time = time.clone();
            let mut job = ControlRenewalJob::spawn(
                session.clone(),
                scope.clone(),
                operation.clone(),
                grant.clone(),
                Some((ConservativeDeadline::new(MonotonicInstant::from_millis(0), Duration::from_secs(60)), Duration::from_secs(20))),
                move || (worker_wire, WireConnection::new("https://product.invalid", "1.0.0").unwrap(), Box::new(worker_clock)),
                {
                    let mut attempts = 0;
                    move |interval, _| {
                        if attempts == responses {
                            return false;
                        }
                        attempts += 1;
                        send_time.fetch_add(u64::try_from(interval.as_millis()).unwrap(), Ordering::SeqCst);
                        true
                    }
                },
            )
            .unwrap();
            if let Some(action) = retire {
                let cache = job.cutoff.clone();
                let mut runtime = AccountRuntime::with_session(session);
                runtime.control_renewal = Some(job);
                let startup = runtime.startup();
                let mut port = DesktopAccountPort::new(runtime, &startup);
                entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                match action {
                    Retirement::Finish => port.control_finished(&operation),
                    Retirement::SignOut => port.sign_out().unwrap(),
                    Retirement::Switch => port.switch_account().unwrap(),
                }
                release_tx.send(()).unwrap();
                port.runtime.control_renewal.as_mut().unwrap().wait_for_fixture();
                assert_eq!(*cache.lock().unwrap(), None, "a retired worker did not publish its late reply");
                assert_eq!(wire.requests().len(), 1, "retirement prevents a second exchange");
                port.shutdown();
                continue;
            }
            job.wait_for_fixture();
            assert_eq!(wire.requests().len(), responses);
            assert_eq!(wire.requests()[0].header("X-Execution-Session-Proof"), Some("fixture_control_proof"));
            time.store(dispatch_at, Ordering::SeqCst);
            let mut runtime = AccountRuntime::with_session(session);
            runtime.control_renewal = Some(job);
            let cancelled = runtime.control_renewal.as_ref().unwrap().cancelled.clone();
            let startup = runtime.startup();
            let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
            install(&mut app, runtime, startup);
            let mut budget = BudgetTable::unverified_default();
            budget.set_verified(Capability::Control, EntryCategory::DirectControl, Duration::from_secs(1));
            app.install_billing(Box::new(Billing(grant)), budget);
            app.install_commit_authority(
                scope,
                Box::new(RenewalTestAuthority { on_third_lookup: Box::new(|| {}), lookups: AtomicUsize::new(0) }),
                Box::new(clock),
            );
            let control::Outcome::Done(reply) = control::handle(&mut app, &egui::Context::default(), &mut request) else {
                panic!("expected a synchronous inspection");
            };
            assert_eq!(reply["ok"], expected.is_none(), "{reply}");
            if let Some(code) = expected {
                assert_eq!(reply["errorDetails"]["code"], code, "{reply}");
            }
            assert!(cancelled.load(Ordering::SeqCst), "both completion and pre-dispatch refusal retire the operation");
        }
    }

    struct RenewalTestAuthority {
        on_third_lookup: Box<dyn Fn() + Send + Sync>,
        lookups: AtomicUsize,
    }

    impl AccountService for RenewalTestAuthority {
        fn status(&self) -> Result<Availability, Refusal> {
            Ok(Availability { online: true, version_supported: true, model_available: true, balance: 10, held: 0, reasons: vec![] })
        }

        fn authorize(&mut self, scope: &AuthenticatedHostScope, capability: Capability) -> Result<Grant, Refusal> {
            InertService.authorize(scope, capability)
        }

        fn begin(
            &mut self,
            scope: &AuthenticatedHostScope,
            operation: &OperationId,
            capability: Capability,
            entry: EntryCategory,
        ) -> Result<Admission, Refusal> {
            InertService.begin(scope, operation, capability, entry)
        }

        fn finalize(&mut self, scope: &AuthenticatedHostScope, operation: &OperationId, result: BillingResult) -> Result<OperationStatus, Refusal> {
            InertService.finalize(scope, operation, result)
        }

        fn operation(&self, _scope: &AuthenticatedHostScope, _operation: &OperationId) -> Result<OperationStatus, Refusal> {
            if self.lookups.fetch_add(1, Ordering::SeqCst) == 2 {
                (self.on_third_lookup)();
            }
            Ok(OperationStatus::Held)
        }
    }

    #[test]
    fn review_wire_401_blocks_dispatch_and_late_report_before_expiry() {
        use solidcraft_engine::dfm::run::{RunMeta, RunOutcome, RunStore};
        use solidcraft_engine::entitlement::{GrantId, OperationRecord, OperationStore, RunId};
        use std::sync::atomic::AtomicUsize;

        struct Clock;
        impl MonotonicClock for Clock {
            fn now(&self) -> MonotonicInstant {
                MonotonicInstant::from_millis(20_000)
            }
            fn wall_now(&self) -> SystemTime {
                UNIX_EPOCH
            }
        }

        struct Billing {
            on_admit: Box<dyn Fn(&OperationId) + Send>,
            calls: Arc<Mutex<Vec<&'static str>>>,
        }
        impl solidcraft_engine::entitlement::BillingPort for Billing {
            fn admit(&mut self, operation: &OperationId, capability: Capability, _: EntryCategory) -> Result<Grant, Refusal> {
                self.calls.lock().unwrap().push("admit");
                (self.on_admit)(operation);
                Ok(Grant::from_server(GrantId::new("grant_review_fixture"), capability, MonotonicInstant::from_millis(60_000)))
            }
            fn commit(&mut self, _: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
                self.calls.lock().unwrap().push("commit");
                Ok(result)
            }
            fn release(&mut self, _: &OperationId, _: RefusalCode) -> Result<(), Refusal> {
                self.calls.lock().unwrap().push("release");
                Ok(())
            }
        }

        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("intcad-review-wire-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        let scratch = Scratch(path);
        let project = solidcraft_engine::project::create(&scratch.0, "review-project", "part").unwrap();
        let journal = scratch.0.join("journal");
        let mut ledger = FileOperationStore::open(&journal).unwrap();
        let mut session = active_session();
        let scope = session.authenticated_scope().unwrap();
        let operation = OperationId::parse("op_review_wire_fixture").unwrap();
        ledger
            .save_new(&OperationRecord::held(operation.clone(), scope.account().clone(), Capability::Review, EntryCategory::ReviewPanel, 0))
            .unwrap();
        CredentialHandoff::execution_session_proof(&mut session, Capability::Review, &operation, Secret::new("fixture_review_proof")).unwrap();

        let scripted = ScriptedProduct::scripted(vec![Ok(WireResponse::new(
            401,
            br#"{"error":{"code":"AUTH_REQUIRED","message":"Authentication or credential is not valid.","retryable":false,"request_id":"req_example_0001"}}"#
                .to_vec(),
        ))]);
        let runtime = AccountRuntime::new(session.clone(), Some(ledger));
        let startup = runtime.startup();
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        app.session.path = Some(project.design.to_string_lossy().into_owned());
        install(&mut app, runtime, startup);
        app.install_commit_authority(scope.clone(), Box::new(InertService), Box::new(Clock));
        let mut budget = BudgetTable::unverified_default();
        budget.set_verified(Capability::Review, EntryCategory::ReviewPanel, Duration::from_secs(1));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let renew_session = session.clone();
        let renew_scope = scope.clone();
        let transport = scripted.clone();
        app.install_billing(
            Box::new(Billing {
                calls: Arc::clone(&calls),
                on_admit: Box::new(move |operation| {
                    let mut service = HttpAccountService::with_transport(
                        transport.clone(),
                        WireConnection::new("https://product.invalid", "1.0.0").unwrap(),
                        Box::new(renew_session.clone()),
                        Box::new(renew_session.clone()),
                        Box::new(Clock),
                    );
                    assert_eq!(
                        service.renew(&renew_scope, operation, &GrantId::new("grant_review_fixture"), Capability::Review).unwrap_err().code,
                        RefusalCode::AuthRequired
                    );
                }),
            }),
            budget,
        );
        let error = solidcraft_ui_egui::dfm_review::start_with_operation(&mut app, operation.clone()).unwrap_err();
        assert!(error.starts_with("AUTH_REQUIRED:"), "{error}");
        assert!(!app.dfm_review.running);
        assert_eq!(*calls.lock().unwrap(), ["admit"]);
        let requests = scripted.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].header("X-Execution-Session-Proof"), Some("fixture_review_proof"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(requests[0].body().unwrap()).unwrap(),
            json!({"capability":"review","operation_id":operation.as_str(),"renewal":{"authorization_id":"grant_review_fixture"}}),
        );

        let journal = FileOperationStore::open(journal).unwrap();
        let run = journal.load(&operation).unwrap().unwrap().review_run_id.unwrap();
        let store = RunStore::new(project.project.root());
        let meta = RunMeta::new(
            run.as_str(),
            "review-project",
            "sha256:candidate",
            "sha256:config",
            "model",
            "2026-11-21T00:00:00.000Z",
            "2026-11-21T00:00:01.000Z",
            RunOutcome::Superseded,
            None,
        );
        store.write_stopped(&meta).unwrap();
        std::fs::write(
            store.run_dir(run.as_str()).unwrap().join("findings.json"),
            json!({
                "schemaVersion":2, "runId":run.as_str(), "candidateDigest":"sha256:candidate",
                "profileDigest":"sha256:profile", "configDigest":"sha256:config", "modelVersion":"model",
                "projectId":"review-project", "source":{"kind":"native"}, "findings":[], "rules":[], "notChecked":"",
            })
            .to_string(),
        )
        .unwrap();
        // Mark the mapped run as awaiting completion to exercise the live poll without
        // spawning a worker. The scripted result arrives while the clock is still at 20 s.
        app.dfm_review.running = true;
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput { time: Some(1.0), ..Default::default() }, |ui| {
            solidcraft_ui_egui::dfm_review::poll(&mut app, ui.ctx());
        });
        assert!(!app.dfm_review.running, "the late report reached the completion branch");
        assert!(app.dfm_review.error.as_deref().unwrap().starts_with("AUTH_REQUIRED:"));
        assert!(app.dfm_review.card.is_none());
        assert_eq!(*calls.lock().unwrap(), ["admit"]);
        assert_eq!(journal.review_operation(&RunId::new(run.as_str())).unwrap(), Some(operation.clone()));
        assert_eq!(journal.settlement_intent(&operation).unwrap(), None);
        assert_eq!(journal.load(&operation).unwrap().unwrap().status, OperationStatus::Held);
        assert_eq!(session.authenticated_scope(), Some(scope));
    }

    struct InertService;

    impl AccountService for InertService {
        fn status(&self) -> Result<Availability, Refusal> {
            Err(account_unavailable("inert test service"))
        }

        fn authorize(&mut self, _scope: &AuthenticatedHostScope, _capability: Capability) -> Result<Grant, Refusal> {
            Err(account_unavailable("inert test service"))
        }

        fn begin(
            &mut self,
            _scope: &AuthenticatedHostScope,
            _operation: &OperationId,
            _capability: Capability,
            _entry: EntryCategory,
        ) -> Result<Admission, Refusal> {
            Err(account_unavailable("inert test service"))
        }

        fn finalize(
            &mut self,
            _scope: &AuthenticatedHostScope,
            _operation: &OperationId,
            _result: BillingResult,
        ) -> Result<OperationStatus, Refusal> {
            Err(account_unavailable("inert test service"))
        }

        fn operation(&self, _scope: &AuthenticatedHostScope, _operation: &OperationId) -> Result<OperationStatus, Refusal> {
            Err(account_unavailable("inert test service"))
        }
    }

    #[test]
    fn local_restart_candidate_alone_never_installs_commit_authority() {
        let mut unverified = AccountRuntime::with_session(active_session());
        unverified.service = Some(Box::new(InertService));
        let startup = unverified.startup();
        assert!(startup.candidate.is_some());
        assert!(startup.restored.is_none(), "startup carries no synchronously verified scope");
        assert!(unverified.take_commit_authority(&startup).is_none(), "a local candidate alone installs no authority");
    }

    #[test]
    fn a_background_restart_result_presents_without_installing_commit_authority() {
        let mut runtime = verified_restart_runtime(active_session());
        runtime.service = Some(Box::new(InertService));
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        wait_for_restart(&port);
        assert!(port.view().signed_in, "the bounded online check is presentable");
        assert!(port.runtime.take_commit_authority(&startup).is_none(), "a background result never fills the synchronous authority checkpoint");
        port.shutdown();
    }

    #[test]
    fn installing_the_runtime_leaves_controlled_authority_closed_without_a_service() {
        let runtime = verified_restart_runtime(active_session());
        let startup = runtime.startup();
        let mut app = solidcraft_ui_egui::SolveApp::new(solidcraft_engine::Session::default(), solidcraft_ui_egui::Services::default());
        install(&mut app, runtime, startup);
        assert!(app.account.is_installed(), "the desktop port is installed");
        wait_until("the background restart view", || {
            app.account.process_actions();
            app.account.view().signed_in
        });
        assert!(app.account.view().signed_in, "the online-verified restart is shown");

        let (gui, _) = solidcraft_ui_egui::ControlRequest::trusted_gui("engine.execute", json!({"command": "document.inspect"}));
        assert!(matches!(control::admit_control_request(&gui, &BudgetTable::unverified_default(), &mut NoBilling), Admission::Allowed));
        app.account.shutdown();
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
    fn sign_in_configuration_requires_https_origins_and_clean_values() {
        assert_eq!(SignInConfig::new("", "https://auth.example", "https://api.example", "1.0.0").unwrap_err().code, RefusalCode::AuthUnavailable);
        assert_eq!(
            SignInConfig::new("client", "http://auth.example", "https://api.example", "1.0.0").unwrap_err().code,
            RefusalCode::AuthUnavailable
        );
        assert_eq!(
            SignInConfig::new("client", "https://auth.example/path", "https://api.example", "1.0.0").unwrap_err().code,
            RefusalCode::AuthUnavailable,
            "the configured AuthKit value is an origin, not a page"
        );
        assert_eq!(
            SignInConfig::new("client", "https://auth.example", "https://api.example/v1", "1.0.0").unwrap_err().code,
            RefusalCode::AuthUnavailable,
            "the product value is a clean HTTPS base, never a path prefix"
        );
        let config = SignInConfig::new("client", "https://auth.example:443/", "https://api.example", "1.0.0").unwrap();
        assert!(config.authkit.matches("https://auth.example/device?user_code=BCDF-GHJK"));
        assert!(!config.authkit.matches("https://auth.example.evil.example/device"));
        assert!(!config.authkit.matches("http://auth.example/device"));
    }

    #[test]
    fn a_failed_sign_in_can_be_retried_without_replaying_the_old_flow() {
        let mut untrusted = device_body();
        untrusted["verification_uri"] = json!("https://untrusted.example/device");
        untrusted["verification_uri_complete"] = json!(null);
        let device = ScriptedDevice::scripted(vec![(200, untrusted), (200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::ok();
        let timing = Arc::new(VirtualTiming::default());
        let opener = RecordingOpener::default();
        let runtime = harness_runtime(installed_session(), device.clone(), product.clone(), timing, &opener);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        port.sign_in().unwrap();
        wait_until("the refused origin", || port.take_sign_in_notice().is_some());
        assert!(!port.view().signed_in);

        port.sign_in().unwrap();
        wait_until("the retried verified sign-in", || port.view().signed_in);
        assert_eq!(device.requests().len(), 3, "the retry starts a new authorization; the old flow is not replayed");
        assert_eq!(product.requests().len(), 2, "the retried verified sign-in also binds one installation");
        assert!(opener.urls().is_empty());
        port.shutdown();
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

    #[test]
    fn desktop_sign_in_without_configuration_is_refused_before_any_request() {
        let runtime = AccountRuntime::with_session(CredentialSession::new(Box::new(TestStore::default())));
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        assert_eq!(port.sign_in().unwrap_err().code, RefusalCode::AuthUnavailable);
        assert!(port.sign_in.is_none(), "no worker starts without all three trusted values");
        assert_eq!(port.begin_device_code().unwrap_err().code, RefusalCode::AuthRequired);
    }

    #[test]
    fn explicit_sign_in_shows_the_prompt_and_opens_the_browser_only_on_an_explicit_action() {
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let (gate_tx, gate_rx) = channel();
        device.hold_token_success(gate_rx);
        let product = ScriptedProduct::ok();
        let timing = Arc::new(VirtualTiming::default());
        let opener = RecordingOpener::default();
        let runtime = harness_runtime(installed_session(), device.clone(), product.clone(), timing, &opener);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        port.sign_in().unwrap();
        wait_until("the device prompt", || port.begin_device_code().is_ok());
        let prompt = port.begin_device_code().unwrap();
        assert_eq!(prompt.user_code, "BCDF-GHJK");
        assert!(prompt.verification_uri.starts_with("https://auth.example/"));
        assert!(opener.urls().is_empty(), "showing a code never opens a browser");

        port.open_verification().unwrap();
        assert_eq!(
            opener.urls(),
            vec!["https://auth.example/device?user_code=BCDF-GHJK".to_owned()],
            "one explicit action opens exactly the trusted verification page"
        );

        gate_tx.send(()).unwrap();
        wait_until("the verified sign-in", || port.view().signed_in);
        assert_eq!(port.view().account_label.as_deref(), Some("acct_product_0001"));
        assert_eq!(port.take_sign_in_notice(), None);
        assert!(port.runtime.credentials.product_session_active(), "a verified product session is recognized");
        let scope = port.runtime.credentials.authenticated_scope().expect("the verified sign-in bound one installation");
        assert_eq!(scope.account().as_str(), "acct_product_0001");
        assert_eq!(scope.installation().as_str(), "inst_product_0001");
        assert_eq!(port.view().tool_blocked, Some(RefusalCode::AuthUnavailable), "no account service is installed");
        assert_eq!(product.requests().len(), 2, "the product service verifies the token and then issues one installation");
        assert_eq!(port.begin_device_code().unwrap_err().code, RefusalCode::AuthRequired, "a completed flow retires its prompt");
        assert!(opener.urls().len() == 1, "only the explicit action opened the browser");

        // Recognizing the completed flow is cached: refreshing never replays the exchange.
        let view = port.refresh_status().unwrap();
        assert!(view.signed_in);
        assert_eq!(product.requests().len(), 2, "a refresh must not replay product verification or installation bootstrap");
        port.shutdown();
    }

    #[test]
    fn an_untrusted_verification_origin_reports_an_in_place_refusal_and_never_opens_a_browser() {
        let mut body = device_body();
        body["verification_uri"] = json!("https://untrusted.example/device");
        body["verification_uri_complete"] = json!("https://untrusted.example/device?user_code=BCDF-GHJK");
        let device = ScriptedDevice::scripted(vec![(200, body)]);
        let product = ScriptedProduct::ok();
        let timing = Arc::new(VirtualTiming::default());
        let opener = RecordingOpener::default();
        let runtime = harness_runtime(installed_session(), device.clone(), product.clone(), timing, &opener);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        port.sign_in().unwrap();
        wait_until("the origin refusal", || port.take_sign_in_notice().is_some());
        assert!(!port.view().signed_in);
        assert!(opener.urls().is_empty(), "an untrusted origin is never opened");
        assert_eq!(port.begin_device_code().unwrap_err().code, RefusalCode::AuthRequired, "a refused flow keeps no prompt");
        assert_eq!(port.open_verification().unwrap_err().code, RefusalCode::AuthRequired, "a refused flow opens no browser");
        assert!(opener.urls().is_empty());
        assert_eq!(device.requests().len(), 1, "only the authorization request is sent");
        assert_eq!(device.requests()[0], solidcraft_account_native::WORKOS_DEVICE_AUTHORIZATION_URL);
        port.shutdown();
    }

    #[test]
    fn switching_account_cancels_the_worker_and_a_late_approval_writes_nothing() {
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let (gate_tx, gate_rx) = channel();
        device.hold_token_success(gate_rx);
        let product = ScriptedProduct::ok();
        let timing = Arc::new(VirtualTiming::default());
        let opener = RecordingOpener::default();
        let runtime = harness_runtime(installed_session(), device.clone(), product.clone(), timing, &opener);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        port.sign_in().unwrap();
        wait_until("the polled token request", || device.authorized_waiting.load(Ordering::SeqCst) >= 1);
        port.switch_account().unwrap();
        wait_for_cleanup(&port);
        gate_tx.send(()).unwrap();
        port.shutdown();

        assert!(port.sign_in.is_none(), "the retired job is gone");
        assert!(port.runtime.credentials.provider_account().is_none(), "a late approval must not store provider credentials");
        assert!(!port.runtime.credentials.product_session_active(), "a late approval must not activate a product session");
        assert!(!port.view().signed_in);
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn device_sign_in_rejects_a_prompt_outside_the_configured_authkit_origin() {
        let mut body = device_body();
        body["verification_uri"] = json!("https://untrusted.example/device");
        body["verification_uri_complete"] = json!(null);
        let device = ScriptedDevice::scripted(vec![(200, body)]);
        let timing = Arc::new(VirtualTiming::default());
        let (result, session, requests, prompts) = run_drive(device, ScriptedProduct::ok(), timing, SignInCancellation::default());

        assert_eq!(result.unwrap_err().code, RefusalCode::AuthUnavailable);
        assert!(prompts.is_empty(), "a rejected address is never shown for a browser action");
        assert_eq!(requests.len(), 1, "nothing is polled after an untrusted prompt");
        assert!(session.provider_account().is_none());
        assert!(!session.product_session_active());
    }

    #[test]
    fn device_sign_in_waits_the_server_interval_and_slow_down_backoff() {
        let device = ScriptedDevice::scripted(vec![
            (200, device_body()),
            (400, json!({"error": "authorization_pending"})),
            (400, json!({"error": "slow_down"})),
            (200, token_body()),
        ]);
        let timing = Arc::new(VirtualTiming::default());
        let (result, session, requests, prompts) = run_drive(device, ScriptedProduct::ok(), timing.clone(), SignInCancellation::default());

        assert_eq!(result.unwrap().account_id, "acct_product_0001");
        assert_eq!(
            timing.waits(),
            vec![Duration::from_secs(5), Duration::from_secs(5), Duration::from_secs(10)],
            "slow_down increases each later interval"
        );
        assert_eq!(requests.len(), 4);
        assert_eq!(prompts.len(), 1, "one prompt is shown for one explicit sign-in");
        assert_eq!(session.provider_account().as_deref(), Some("user_01E4ZCR3C56J083X43JQXF3JK5"));
        assert!(session.product_session_active());
    }

    #[test]
    fn device_sign_in_stops_at_the_code_deadline_without_an_extra_poll() {
        let device = ScriptedDevice::scripted(vec![
            (200, device_body_until(12)),
            (400, json!({"error": "authorization_pending"})),
            (400, json!({"error": "authorization_pending"})),
        ]);
        let timing = Arc::new(VirtualTiming::default());
        let (result, session, requests, _prompts) = run_drive(device, ScriptedProduct::ok(), timing.clone(), SignInCancellation::default());

        assert_eq!(result.unwrap_err().code, RefusalCode::AuthRequired);
        assert_eq!(timing.waits().len(), 3);
        assert_eq!(requests.len(), 3, "the authorization request and two polls, then no request past the deadline");
        assert!(session.provider_account().is_none());
        assert!(!session.product_session_active());
    }

    #[test]
    fn device_sign_in_cancelled_after_authorization_writes_no_credential() {
        let cancel = SignInCancellation::default();
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        device.cancel_on_authorized(cancel.clone());
        let timing = Arc::new(VirtualTiming::default());
        let (result, session, _requests, _prompts) = run_drive(device, ScriptedProduct::ok(), timing, cancel);

        assert_eq!(result.unwrap_err().code, RefusalCode::AuthRequired);
        assert!(session.provider_account().is_none(), "the provider login is not written after cancellation");
        assert!(!session.product_session_active());
    }

    #[test]
    fn provider_success_requires_product_service_verification_before_activation() {
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(
            403,
            json!({"error": {"code": "AUTH_REQUIRED", "message": "refused", "retryable": false, "request_id": "req_test"}}).to_string().into_bytes(),
        ))]);
        let timing = Arc::new(VirtualTiming::default());
        let (result, session, _requests, _prompts) = run_drive(device, product.clone(), timing, SignInCancellation::default());

        assert_eq!(result.unwrap_err().code, RefusalCode::AuthUnavailable);
        assert_eq!(session.provider_account().as_deref(), Some("user_01E4ZCR3C56J083X43JQXF3JK5"));
        assert!(!session.product_session_active(), "provider success alone never activates a product session");
        let requests = product.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url(), "https://api.example/v1/status");
    }

    #[test]
    fn verified_sign_in_requests_each_step_once_and_stores_the_product_session() {
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (400, json!({"error": "authorization_pending"})), (200, token_body())]);
        let product = ScriptedProduct::ok();
        let timing = Arc::new(VirtualTiming::default());
        let (result, session, requests, prompts) = run_drive(device, product.clone(), timing, SignInCancellation::default());

        assert_eq!(result.unwrap().account_id, "acct_product_0001");
        assert_eq!(requests.len(), 3, "one authorization request and two polls; no step repeats");
        assert_eq!(prompts.len(), 1);
        let product_requests = product.requests();
        assert_eq!(product_requests.len(), 2, "verification then one installation bootstrap; no step repeats");
        assert_eq!(product_requests[0].url(), "https://api.example/v1/status");
        assert_eq!(product_requests[1].url(), "https://api.example/v1/installations");
        assert_eq!(
            product_requests[1].header("Authorization"),
            Some("Bearer access_token_SECRET_000000000000000000000000"),
            "the installation request uses the verified product session"
        );
        assert!(product_requests[1].header("X-Installation-Credential").is_none());
        let installation: serde_json::Value = serde_json::from_slice(product_requests[1].body().unwrap()).unwrap();
        assert!(installation.get("label").is_none(), "the desktop sends no hostname, username or path");
        assert_eq!(WireCredentials::product_session(&session).unwrap().expose_secret(), "access_token_SECRET_000000000000000000000000");
        let bound = session.authenticated_scope().expect("the installation pair is durable and readable");
        assert_eq!(bound.installation().as_str(), "inst_product_0001");
        assert_eq!(WireCredentials::installation_credential(&session).unwrap().expose_secret(), "installation_credential_product_0000000000000000");
    }

    #[test]
    fn a_refused_installation_bootstrap_leaves_the_sign_in_unverified_and_scope_closed() {
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::scripted(vec![
            Ok(WireResponse::new(200, status_body().to_string().into_bytes())),
            Ok(WireResponse::new(
                503,
                json!({"error": {"code": "AUTH_UNAVAILABLE", "message": "unavailable", "retryable": true, "request_id": "req_test"}})
                    .to_string()
                    .into_bytes(),
            )),
        ]);
        let timing = Arc::new(VirtualTiming::default());
        let (result, session, _requests, _prompts) = run_drive(device, product.clone(), timing, SignInCancellation::default());

        assert_eq!(result.unwrap_err().code, RefusalCode::AuthUnavailable, "the sign-in is refused after the failed bootstrap");
        assert!(session.product_session_active(), "the verified product session is recognized");
        assert!(!WireCredentials::installation_bound(&session), "no invented installation id or scope");
        assert!(session.authenticated_scope().is_none(), "controlled authority stays closed");
        assert!(WireCredentials::installation_credential(&session).is_none());
        let requests = product.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].url(), "https://api.example/v1/installations");
    }
}
