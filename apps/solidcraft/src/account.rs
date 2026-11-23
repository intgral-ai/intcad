//! Desktop account composition root (Task 11).
//!
//! The runtime owns the one credential session, the durable operation ledger, the conservative
//! budget and the monotonic clock, and installs the UI account port. It never opens a dialog or
//! the network on a render pass.
//!
//! Fail closed: the desktop wires an explicit sign-in action to the existing WorkOS device-code
//! flow and the reviewed `ServerVerifiedProductSession`, but this build ships no reviewed WorkOS
//! client id or product service origin. `DesktopAccountConfig` is the only source of those
//! values; without it the action refuses before any request. A verified session installs the
//! reviewed HTTP account service for the commit gate, but never promotes a provider token
//! implicitly and installs no model or chat authority. Ordinary CAD stays free and anonymous;
//! controlled entries keep their existing refusal until the host installs that service. A secure
//! store is probed before any session is restored, and sign-out, account switch or cancellation
//! ends the current attempt's authority immediately.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use solidcraft_account_native::{
    BootstrapPlatform, CredentialSession, DeviceCodeFlow, DeviceCodeMode, DeviceCodePoll, DeviceCodeTransport, FileOperationStore, FixedCredentials,
    HttpAccountService, ProductExchangeError, ProductSessionExchange, Secret, ServerVerifiedProductSession, Transport, UreqDeviceCodeTransport,
    UreqTransport, WireConnection, WireCredentials, WorkOsPublicClient,
};
use solidcraft_engine::entitlement::{
    AccountService, Admission, AuthenticatedHostScope, Availability, BillingResult, BudgetTable, Capability, CredentialErrorCode, EntryCategory,
    Grant, MonotonicClock, MonotonicInstant, OperationId, OperationStatus, OperationStore, Refusal, RefusalCode, SettlementIntent,
    UnsettledOperation,
};
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::account::{
    AccountPort, AccountView, CommitAuthority, DeleteAccountGuide, DeviceCode, LifecycleEvent, OwnModelView, SignInState, State, Timestamp,
};

#[derive(Clone, Default)]
struct RestartCancellation(Arc<AtomicBool>);
impl RestartCancellation {
    fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

type ProductTransportFactory = Arc<dyn Fn() -> Box<dyn Transport + Send + Sync> + Send + Sync>;

#[cfg(test)]
use legacy_device_tests::{BrowserOpener, DeviceTransportFactory, SignInConfig, SignInTiming};
/// A restart check is one startup decision, so it gets its own short bounds instead of the
/// 30-second limit the full sign-in flow allows. A slow or offline service must not hold launch.
const RESTART_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const RESTART_STATUS_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct RestartShared {
    /// The complete binding of an online-verified original pair. Presentation uses only its
    /// account label; the service installer requires the whole provable scope.
    scope: Option<AuthenticatedHostScope>,
    /// The bounded check has finished, failed or been retired.
    done: bool,
}

/// One bounded background restart check. It publishes the verified scope so the host can install
/// one account service for that exact session; it never invents a scope.
struct RestartJob {
    cancel: RestartCancellation,
    shared: Arc<Mutex<RestartShared>>,
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

#[cfg(test)]
mod legacy_device_tests {
    use super::*;
    use solidcraft_account_native::{DeviceCodePrompt, ProductLogin};
    /// Explicit desktop sign-in configuration. Every value comes from the host; no origin is ever
    /// derived from a token, a response field or a guessed default, and no secret is baked in.
    #[derive(Clone, Debug)]
    pub(super) struct SignInConfig {
        pub(super) client: WorkOsPublicClient,
        pub(super) authkit: AuthKitOrigin,
        pub(super) product: WireConnection,
    }

    impl SignInConfig {
        pub(super) fn new(client_id: &str, authkit_origin: &str, product_origin: &str, client_version: &str) -> Result<Self, Refusal> {
            let client = WorkOsPublicClient::new(client_id).ok_or_else(|| account_unavailable("no WorkOS public client id is configured"))?;
            let authkit = AuthKitOrigin::new(authkit_origin).ok_or_else(|| account_unavailable("no trusted HTTPS AuthKit origin is configured"))?;
            let product = WireConnection::new(product_origin, client_version)
                .map_err(|_| account_unavailable("no trusted HTTPS product service origin is configured"))?;
            Ok(Self { client, authkit, product })
        }
    }

    /// A normalized HTTPS origin that a WorkOS verification page must match exactly.
    ///
    /// WorkOS returns the address of its own hosted page. The desktop trusts it only when its scheme,
    /// host and port equal the configured AuthKit origin; a path, query or fragment on the response
    /// does not change the origin.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct AuthKitOrigin {
        pub(super) origin: String,
    }

    impl AuthKitOrigin {
        pub(super) fn new(raw: &str) -> Option<Self> {
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

        pub(super) fn matches(&self, uri: &str) -> bool {
            normalized_https_origin(uri).is_some_and(|origin| origin == self.origin)
        }
    }

    /// Parse only `https://host[:port]` (or that followed by `/`, `?` or `#`) and return a normalized
    /// lowercase origin. Userinfo, plain HTTP, control bytes and empty hosts are rejected.
    pub(super) fn normalized_https_origin(raw: &str) -> Option<String> {
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
            if host_part.is_empty() || host_part.contains(':') || port.is_empty() || port.len() > 5 || !port.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            port.parse::<u16>().ok().filter(|port| *port != 0)?;
        }
        Some(format!("https://{authority}"))
    }

    /// A cancellation flag that wakes the worker immediately, so no sign-in keeps polling for its
    /// server interval after the user switches accounts or closes the session.
    #[derive(Clone, Default)]
    pub(super) struct SignInCancellation {
        cancelled: Arc<AtomicBool>,
    }

    impl SignInCancellation {
        pub(super) fn cancel(&self) {
            self.cancelled.store(true, Ordering::SeqCst);
        }

        pub(super) fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    /// The worker's only time source. Tests supply a virtual clock; production waits on the
    /// cancellation-aware condition variable.
    pub(super) trait SignInTiming: Send + Sync {
        fn now(&self) -> Duration;
        fn wait(&self, interval: Duration, cancel: &SignInCancellation) -> bool;
    }

    pub(super) fn auth_required(message: &'static str) -> Refusal {
        Refusal::new(RefusalCode::AuthRequired, message)
    }

    /// Drive one explicit WorkOS device-code login through to a verified product session.
    ///
    /// Only this function sends sign-in requests: it starts the documented device authorization,
    /// shows the prompt for an explicit browser action, polls on the server's interval and slow-down
    /// backoff until the user approves or the code's lifetime ends, then stores the provider login and
    /// asks the configured product service to verify it. Cancellation is checked before every request
    /// and before either credential write, so a late result after sign-out or account switch writes
    /// nothing. The caller never replays a request that already completed.
    pub(super) fn drive_device_sign_in<D, P>(
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

    pub(super) fn current_bootstrap_platform() -> BootstrapPlatform {
        if cfg!(target_os = "windows") {
            BootstrapPlatform::Windows
        } else if cfg!(target_os = "macos") {
            BootstrapPlatform::Macos
        } else {
            BootstrapPlatform::Linux
        }
    }

    pub(super) type DeviceTransportFactory = Arc<dyn Fn() -> Box<dyn DeviceCodeTransport + Send + Sync> + Send + Sync>;
    pub(super) type BrowserOpener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
}

/// The durable sign-out state read once before the window exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupLogout {
    /// No marker: ordinary signed-out or signed-in restore rules apply.
    Clear,
    /// A marker is readable and present. No prior identity or authority may be restored; the UI
    /// offers a recovery action that finishes the durable cleanup.
    ReadableTombstone,
    /// The marker or the secure store could not be read. Nothing is offered and every account
    /// action stays closed.
    Unreadable,
}

/// Account facts read once before the window exists.
#[derive(Clone, Debug)]
pub struct AccountStartup {
    secure_store: bool,
    /// A readable durable sign-out marker, or an unreadable safety state. Either way startup
    /// refuses to restore or admit anything until cleanup or a reviewed sign-in clears it.
    logout: StartupLogout,
    candidate: Option<AuthenticatedHostScope>,
    restart: Option<RestartBinding>,
    restored: Option<AuthenticatedHostScope>,
}

impl AccountStartup {
    #[cfg(test)]
    fn logout_pending(&self) -> bool {
        self.logout != StartupLogout::Clear
    }
}

/// The reviewed values a desktop provider sign-in needs. Neither value comes from an environment
/// heuristic, a token field or a server response: the WorkOS public client id and the bare HTTPS
/// product origin are supplied by explicit configuration, and malformed values refuse to load.
#[derive(Clone)]
pub struct DesktopAccountConfig {
    client: WorkOsPublicClient,
    product: WireConnection,
    verification_origin: String,
}

impl DesktopAccountConfig {
    /// The only constructor: all three values must be explicit. No reviewed production values
    /// ship in this build, so `trusted_desktop_config` passes `None`; tests and a trusted release
    /// builder can use this seam without inventing a runtime or environment fallback.
    pub fn new(workos_public_client_id: &str, product_https_origin: &str, authkit_verification_origin: &str) -> Option<Self> {
        let client = WorkOsPublicClient::new(workos_public_client_id)?;
        let product = crate::keyring_store::product_connection(product_https_origin, env!("CARGO_PKG_VERSION"))?;
        let verification = WireConnection::new(authkit_verification_origin, env!("CARGO_PKG_VERSION")).ok()?;
        Some(Self { client, product, verification_origin: verification.base_url().to_owned() })
    }
}

/// The build-time account configuration for a trusted release. The release builder supplies the
/// public WorkOS client id, the exact AuthKit verification origin and the exact product HTTPS
/// origin as build environment values. They are compiled in, cannot be changed from preferences,
/// MCP, a design file or a server response, and a missing or malformed value fails closed before
/// any request or browser launch. Local builds normally have none of them.
pub fn trusted_desktop_config() -> Option<DesktopAccountConfig> {
    config_from_build_values(
        option_env!("INTCAD_WORKOS_PUBLIC_CLIENT_ID"),
        option_env!("INTCAD_AUTHKIT_VERIFICATION_ORIGIN"),
        option_env!("INTCAD_PRODUCT_HTTPS_ORIGIN"),
    )
}

fn config_from_build_values(client: Option<&str>, authkit: Option<&str>, product: Option<&str>) -> Option<DesktopAccountConfig> {
    DesktopAccountConfig::new(client?, product?, authkit?)
}

/// The browser seam. The production launcher starts the system browser only for the exact
/// provider-issued HTTPS verification address of the current attempt.
pub trait BrowserLauncher: Send + Sync {
    /// `allowed_origin` is supplied by the host configuration; the launcher repeats the exact
    /// check rather than trusting its caller to have done so.
    fn open(&self, url: &str, allowed_origin: &str) -> Result<(), Refusal>;
}

struct SystemBrowserLauncher;

impl BrowserLauncher for SystemBrowserLauncher {
    fn open(&self, url: &str, allowed_origin: &str) -> Result<(), Refusal> {
        if !trusted_browser_url(url, allowed_origin) {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the sign-in page is not the configured HTTPS origin"));
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

/// The exact `https://host[:port]` origin of a URL, or `None` when the URL is malformed or not
/// HTTPS. Credentials, whitespace, control characters and backslashes are refused.
fn https_origin(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    if rest.is_empty() || url.len() > 2048 || url.bytes().any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\') {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(format!("https://{authority}"))
}

/// Only the configured provider verification origin may reach the system browser. The address
/// still comes from the current device-code prompt, never from a caller.
fn trusted_browser_url(url: &str, allowed_origin: &str) -> bool {
    https_origin(url).is_some_and(|origin| origin == allowed_origin)
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
    verification_origin: String,
    credentials: CredentialSession,
    status: Arc<Mutex<LoginStatus>>,
    persist_gate: Arc<Mutex<()>>,
    bootstrap_transport: Option<Box<dyn Transport + Send + Sync>>,
}

type LoginWait = dyn Fn(Duration, Instant, &dyn Fn() -> bool) -> bool + Sync;

/// Canceled attempts may still be inside one bounded provider request. Keep a small fixed number
/// of their handles so retry never joins a live worker on the window thread.
const MAX_LOGIN_WORKERS: usize = 4;

type LoginRunner = Arc<dyn Fn(LoginTask) -> Result<std::thread::JoinHandle<()>, Refusal> + Send + Sync>;

fn production_login_runner() -> LoginRunner {
    Arc::new(|mut task| {
        task.bootstrap_transport = Some(Box::new(UreqTransport::new()));
        std::thread::Builder::new()
            .name("intcad-account-login".into())
            .spawn(move || {
                let exchange = ServerVerifiedProductSession::new(UreqTransport::new(), task.connection.clone());
                run_login(task, UreqDeviceCodeTransport::new(), &exchange, &interruptible_wait);
            })
            .map_err(|_| Refusal::new(RefusalCode::AuthUnavailable, "the sign-in worker could not start"))
    })
}

/// Wait the provider's full interval, waking at most every 100 ms only to observe cancellation.
/// The endpoint is never polled before the provider's `interval` or `slow_down` interval elapsed.
fn interruptible_wait(duration: Duration, _deadline: Instant, canceled: &dyn Fn() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if canceled() {
            return false;
        }
        let elapsed = started.elapsed();
        if elapsed >= duration {
            return true;
        }
        std::thread::sleep((duration - elapsed).min(Duration::from_millis(100)));
    }
}

/// Poll one explicit device-code attempt off the window thread. Network work never holds the
/// attempt fence; only the final credential write does, together with cancellation.
fn run_login<T: DeviceCodeTransport>(task: LoginTask, transport: T, exchange: &dyn ProductSessionExchange, wait: &LoginWait) {
    let LoginTask { attempt, client, connection, verification_origin, credentials, status, persist_gate, bootstrap_transport } = task;
    let mut flow = DeviceCodeFlow::new(client, transport, credentials.clone(), DeviceCodeMode::Explicit);
    let prompt = match flow.start() {
        Ok(prompt) => prompt,
        Err(error) => {
            finish(&status, attempt, LoginPhase::Failed(error.code), None);
            return;
        }
    };
    if !trusted_browser_url(prompt.verification_uri(), &verification_origin)
        || prompt.verification_uri_complete().is_some_and(|url| !trusted_browser_url(url, &verification_origin))
    {
        // The provider returned an address outside the configured AuthKit origin. Never publish
        // it to the UI or launch it.
        finish(&status, attempt, LoginPhase::Failed(RefusalCode::AuthUnavailable), None);
        return;
    }
    let expires_at = Timestamp(unix_seconds().saturating_add(prompt.expires_in().as_secs()));
    let code = DeviceCode {
        user_code: prompt.user_code().to_owned(),
        verification_uri: prompt.verification_uri_complete().unwrap_or_else(|| prompt.verification_uri()).to_owned(),
        expires_at,
    };
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
        let now = Instant::now();
        let interval_end = now.checked_add(interval).unwrap_or(now);
        let wait_until = deadline.map_or(interval_end, |expires| expires.min(interval_end));
        let duration = wait_until.saturating_duration_since(now);
        let canceled = || !current(&status, attempt);
        if !wait(duration, wait_until, &canceled) {
            return;
        }
        if !current(&status, attempt) {
            return;
        }
        if deadline.is_some_and(|expires| Instant::now() >= expires) {
            finish(&status, attempt, LoginPhase::Expired, None);
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
    let installation = if let Some(transport) = bootstrap_transport {
        let client = HttpAccountService::with_transport(
            transport,
            connection,
            Box::new(FixedCredentials::none()),
            Box::new(credentials.clone()),
            Box::new(SystemMonotonicClock::new()),
        );
        let platform = if cfg!(target_os = "windows") {
            BootstrapPlatform::Windows
        } else if cfg!(target_os = "macos") {
            BootstrapPlatform::Macos
        } else {
            BootstrapPlatform::Linux
        };
        match client.request_first_installation(product.product_session(), platform, None) {
            Ok(result) => Some(result),
            Err(error) => {
                finish(&status, attempt, LoginPhase::Failed(error.code), None);
                return;
            }
        }
    } else {
        None
    };
    // The exchange above is network work. The write and a cancel or sign-out serialize here, so
    // a stale attempt either sees the new generation or is deleted by the new cleanup.
    let _gate = match persist_gate.lock() {
        Ok(gate) => gate,
        Err(_) => return,
    };
    if !current(&status, attempt) {
        return;
    }
    let generation_current = || current(&status, attempt) && !credentials.is_revoked();
    match credentials.persist_verified_login_with_installation(&provider, &product, installation, attempt, &generation_current) {
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

/// The host-owned account runtime. `service` stays `None` until a completed sign-in or the
/// bounded online restart check proves the current session's scope; a local persisted pair alone
/// never installs it.
pub struct AccountRuntime {
    credentials: CredentialSession,
    ledger: Option<FileOperationStore>,
    /// Measured safe-stop bounds for controlled entries. Retained unverified until a reviewed
    /// billing adapter and its evidence exist; nothing in this task makes a bound verified.
    #[allow(dead_code)]
    budget: BudgetTable,
    clock: Option<Arc<dyn MonotonicClock + Send + Sync>>,
    service: Option<Box<dyn AccountService + Send + Sync>>,
    /// The scope checked against the live credentials and its revocation generation.
    service_proof: Option<(AuthenticatedHostScope, u64)>,
    control_renewal: Option<ControlRenewalJob>,
    control_renewal_factory: Option<ControlRenewalFactory>,
    control_renewal_source: Option<ControlRenewalSource>,
    config: Option<DesktopAccountConfig>,
    restart_transport: Option<ProductTransportFactory>,
    // Keep the per-user keyring lock even when the operation ledger uses an overridden path.
    _keyring_lock: Option<std::fs::File>,
    #[cfg(test)]
    login_seams: Option<(Arc<dyn BrowserLauncher>, LoginRunner)>,
}

impl AccountRuntime {
    /// The production desktop runtime: the operating-system credential store and the durable
    /// ledger under the settings folder. Both are optional; a missing one fails closed.
    pub fn desktop(settings: Option<&Path>) -> Self {
        Self::desktop_with_config(settings, trusted_desktop_config())
    }

    /// The production desktop runtime with explicit trusted account configuration. A missing
    /// configuration keeps the same anonymous CAD and leaves the explicit sign-in refused.
    fn desktop_with_config(settings: Option<&Path>, config: Option<DesktopAccountConfig>) -> Self {
        let lock_directory = eframe::storage_dir(crate::SETTINGS_FOLDER);
        Self::desktop_with_store(settings, config, lock_directory.as_deref(), |service| {
            Box::new(solidcraft_account_native::KeyringStore::new(service))
        })
    }

    fn desktop_with_store(
        settings: Option<&Path>,
        config: Option<DesktopAccountConfig>,
        lock_directory: Option<&Path>,
        open_store: impl FnOnce(&str) -> Box<dyn solidcraft_engine::entitlement::CredentialStore + Send>,
    ) -> Self {
        // No reviewed origin means there is no keyring namespace to open. Never fall back to
        // another origin or to pre-binding entries, even when a ledger is available.
        let keyring_lock =
            config.as_ref().and(lock_directory).and_then(|directory| crate::keyring_store::try_lock_account_namespace(directory).ok().flatten());
        let credentials = match (&config, &keyring_lock) {
            (Some(config), Some(_)) => CredentialSession::new(open_store(&crate::keyring_store::account_keyring_service(&config.product))),
            (config, _) => {
                if config.is_some() {
                    solidcraft_engine::note!("account: another desktop holds the secure store, or its lock is unavailable");
                }
                CredentialSession::without_persistence()
            }
        };
        let ledger = settings.and_then(|directory| match FileOperationStore::open(directory.join("account")) {
            Ok(store) => Some(store),
            Err(error) => {
                solidcraft_engine::note!("account: the durable operation ledger could not be opened ({error})");
                None
            }
        });
        let mut runtime = Self::new(credentials, ledger, config);
        runtime._keyring_lock = keyring_lock;
        #[cfg(target_os = "linux")]
        if let Some(config) = &runtime.config {
            // This installs transport and timing only. Without controlled admission
            // authority there is no grant and no worker or HTTPS request starts.
            runtime.control_renewal_source = Some(ControlRenewalSource {
                connection: config.product.clone(),
                transport: Arc::new(|| Box::new(UreqTransport::with_limits(Duration::from_secs(5), Duration::from_secs(3)))),
                clock: Arc::new(|| Box::new(SystemMonotonicClock::new())),
                wait: Arc::new(native_renewal_wait),
            });
        }
        if runtime.config.is_none() {
            solidcraft_engine::note!("account: no complete WorkOS/AuthKit/product origin configuration; sign-in stays unavailable");
        }
        runtime
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
            clock: Some(Arc::new(SystemMonotonicClock::new())),
            service: None,
            service_proof: None,
            control_renewal: None,
            control_renewal_factory: None,
            control_renewal_source: None,
            config,
            restart_transport: None,
            _keyring_lock: None,
            #[cfg(test)]
            login_seams: None,
        }
    }

    #[cfg(test)]
    fn with_sign_in_harness(
        mut self,
        config: SignInConfig,
        device: DeviceTransportFactory,
        product: ProductTransportFactory,
        _timing: Arc<dyn SignInTiming>,
        opener: BrowserOpener,
    ) -> Self {
        struct TestBrowser(BrowserOpener);
        impl BrowserLauncher for TestBrowser {
            fn open(&self, url: &str, _: &str) -> Result<(), Refusal> {
                (self.0)(url).map_err(|_| account_unavailable("test browser failed"))
            }
        }
        self.config = Some(DesktopAccountConfig { client: config.client, product: config.product, verification_origin: config.authkit.origin });
        self.restart_transport = Some(product.clone());
        self.login_seams = Some((
            Arc::new(TestBrowser(opener)),
            Arc::new(move |mut task| {
                let device = device();
                let exchange = ServerVerifiedProductSession::new(product(), task.connection.clone());
                task.bootstrap_transport = Some(product());
                Ok(std::thread::spawn(move || run_login(task, device, &exchange, &|_, _, canceled| !canceled())))
            }),
        ));
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
    /// operation replay here. A verified result installs the reviewed account service for that
    /// exact scope; a candidate that does not verify installs nothing.
    pub fn startup(&self) -> AccountStartup {
        let secure_store = self.credentials.is_persistent();
        // A present marker is recoverable when it is readable; an unreadable one keeps the
        // process fully closed because revocation/safety cannot be confirmed.
        let logout = match self.credentials.logout_pending() {
            Ok(true) => StartupLogout::ReadableTombstone,
            Ok(false) => StartupLogout::Clear,
            Err(_) => StartupLogout::Unreadable,
        };
        let restart = (logout == StartupLogout::Clear).then(|| self.restart_binding(secure_store)).flatten();
        let candidate = restart.as_ref().map(|binding| binding.scope.clone());
        AccountStartup { secure_store, logout, candidate, restart, restored: None }
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
            product: self.config.as_ref()?.product.clone(),
            credentials: self.credentials.clone(),
            transport: self.restart_transport.clone(),
            ledger_root: self.ledger.as_ref().map(|ledger| ledger.root().to_path_buf()),
        })
    }

    /// The current safe-stop bounds, still unverified in this task.
    #[allow(dead_code)]
    pub fn budget(&self) -> &BudgetTable {
        &self.budget
    }

    /// Install the account service for the exact session a completed sign-in or a bounded online
    /// restart check proved. A locally readable pair, an unprovable scope or a scope that no
    /// longer matches the live credentials installs nothing.
    fn install_verified_service(&mut self) -> bool {
        let Some(scope) = self.authenticated_scope().filter(AuthenticatedHostScope::is_provable) else {
            return false;
        };
        self.install_verified_service_for(&scope)
    }

    /// Install the account service only when `scope` is provable and is still the live session's
    /// binding. A repeated call for the same scope keeps the installed service.
    fn install_verified_service_for(&mut self, scope: &AuthenticatedHostScope) -> bool {
        let generation = self.credentials.authority_generation();
        if !scope.is_provable() || self.authenticated_scope().as_ref() != Some(scope) || self.credentials.authority_generation() != generation {
            return false;
        }
        if self.service.is_some() && self.service_proof.as_ref().is_some_and(|(proved, epoch)| proved == scope && *epoch == generation) {
            return true;
        }
        let Some(config) = self.config.as_ref() else { return false };
        let Some(clock) = self.clock.as_ref().map(|clock| SharedClock(Arc::clone(clock))) else { return false };
        let connection = config.product.clone();
        // Reuse the restart/settlement transport seam: a test scripts one product service and
        // production stays bounded and HTTPS-only.
        let transport: ProductTransportFactory = match self.restart_transport.clone() {
            Some(factory) => factory,
            None => Arc::new(|| Box::new(UreqTransport::with_limits(RESTART_STATUS_TIMEOUT, RESTART_STATUS_CONNECT_TIMEOUT))),
        };
        self.service = Some(Box::new(HttpSessionAccountService { connection, transport, credentials: self.credentials.clone(), clock }));
        self.service_proof = Some((scope.clone(), generation));
        true
    }

    /// Drop the installed service when its session is gone, revoked or different from the one it
    /// was proved for. Called on the host's logic tick, never from a paint pass.
    fn clear_stale_service(&mut self) {
        let stale = match self.service_proof.as_ref() {
            Some((scope, generation)) => self.credentials.authority_generation() != *generation || self.authenticated_scope().as_ref() != Some(scope),
            None => self.service.is_some(),
        };
        if stale {
            self.clear_account_service();
        }
    }

    /// Remove the in-process account service and its proof, so a later controlled entry cannot
    /// run on the old identity.
    fn clear_account_service(&mut self) {
        self.service = None;
        self.service_proof = None;
    }

    /// Take the installed account service and clock only for the exact scope this runtime proved
    /// with a completed sign-in or the bounded restart check. A local candidate alone, an
    /// unprovable scope or a conflicting synchronous `restored` scope yields nothing.
    fn take_commit_authority(&mut self, restored: Option<&AuthenticatedHostScope>) -> Option<CommitAuthority> {
        let (scope, generation) = self.service_proof.as_ref()?;
        // The installation checked the exact live scope. Check the revocation barrier here
        // without returning to the keyring; subsequent service calls read live credentials.
        if restored.is_some_and(|restored| restored != scope)
            || self.credentials.is_revoked()
            || self.credentials.authority_generation() != *generation
        {
            return None;
        }
        let scope = scope.clone();
        let clock = self.clock.as_ref().map(|clock| SharedClock(Arc::clone(clock)))?;
        let service = self.service.take()?;
        self.service_proof = None;
        Some((scope, service, Box::new(clock)))
    }
}

/// A clock handle shared by the installed service and the commit gate, so a grant anchored by one
/// component is measured on the same epoch by the other.
#[derive(Clone)]
struct SharedClock(Arc<dyn MonotonicClock + Send + Sync>);

impl MonotonicClock for SharedClock {
    fn now(&self) -> MonotonicInstant {
        self.0.now()
    }

    fn wall_now(&self) -> SystemTime {
        self.0.wall_now()
    }
}

/// The sendable account service for one proved session.
///
/// `HttpAccountService` boxes its credentials, handoff and clock as plain trait objects, so it is
/// not itself `Send`. The commit authority requires a sendable service, so this adapter keeps the
/// sendable ingredients and builds the HTTP adapter per call. Every call therefore reads the live
/// [`CredentialSession`]: a signed-out, switched or expired session refuses instead of replaying a
/// token captured at install time.
struct HttpSessionAccountService {
    connection: WireConnection,
    transport: ProductTransportFactory,
    credentials: CredentialSession,
    clock: SharedClock,
}

impl HttpSessionAccountService {
    fn service(&self) -> HttpAccountService<Box<dyn Transport + Send + Sync>> {
        HttpAccountService::with_transport(
            (self.transport)(),
            self.connection.clone(),
            Box::new(self.credentials.clone()),
            Box::new(self.credentials.clone()),
            Box::new(self.clock.clone()),
        )
    }
}

impl AccountService for HttpSessionAccountService {
    fn status(&self) -> Result<Availability, Refusal> {
        self.service().status()
    }

    fn authorize(&mut self, scope: &AuthenticatedHostScope, capability: Capability) -> Result<Grant, Refusal> {
        self.service().authorize(scope, capability)
    }

    fn begin(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<Admission, Refusal> {
        self.service().begin(scope, operation, capability, entry)
    }

    fn finalize(&mut self, scope: &AuthenticatedHostScope, operation: &OperationId, result: BillingResult) -> Result<OperationStatus, Refusal> {
        self.service().finalize(scope, operation, result)
    }

    fn operation(&self, scope: &AuthenticatedHostScope, operation: &OperationId) -> Result<OperationStatus, Refusal> {
        self.service().operation(scope, operation)
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
    fn verify(&self, binding: &RestartBinding, cancel: &RestartCancellation) -> Option<AuthenticatedHostScope> {
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
    fn recover_settlement(&self, binding: &RestartBinding, cancel: &RestartCancellation) {
        const MAX_FINALIZE_ATTEMPTS: usize = 4;
        let Some(root) = &self.ledger_root else { return };
        if cancel.is_cancelled() {
            return;
        }
        let Ok(mut ledger) = FileOperationStore::open(root) else {
            solidcraft_engine::note!("account: startup settlement could not open the ledger");
            return;
        };
        let _recovery_lock = match ledger.try_lock_settlement_recovery(&binding.scope) {
            Ok(Some(lock)) => lock,
            Ok(None) => return,
            Err(_) => {
                solidcraft_engine::note!("account: startup settlement could not lock the recovery pass");
                return;
            }
        };
        let Ok(mut rows) = ledger.unsettled_operations(Some(&binding.scope)) else {
            solidcraft_engine::note!("account: startup settlement could not verify the ledger inventory");
            return;
        };
        // Inventory is sorted by operation ID, including when the prior cursor
        // no longer appears (for example, its receipt went terminal).
        if let Some(cursor) = ledger.settlement_cursor(&binding.scope) {
            let after = rows.partition_point(|row| row.record().operation <= cursor);
            rows.rotate_left(after);
        }
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
            if ledger.advance_settlement_cursor(&binding.scope, operation).is_err() {
                solidcraft_engine::note!("account: startup settlement could not advance the retry cursor");
                break;
            }
            if cancel.is_cancelled() {
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
    /// The owning generation's cleanup finished successfully and has not been adopted by `poll`.
    completed: bool,
    /// The latest generation's cleanup error; the session stays fail-closed until a reviewed
    /// sign-in path can repair it.
    error: Option<Refusal>,
}

struct DesktopAccountPort {
    runtime: AccountRuntime,
    view: AccountView,
    lifecycle: Option<LifecycleEvent>,
    restored: Option<AuthenticatedHostScope>,
    /// `true` until a reviewed sign-in path or confirmed cleanup has cleared a durable logout
    /// marker. It keeps the failed-cleanup identity fail-closed across this process too.
    logout: StartupLogout,
    secure_store: bool,
    generation: u64,
    cleanup: Arc<Mutex<CleanupStatus>>,
    /// The bounded restart check, if a complete persisted pair was captured at startup. A
    /// verified result installs the account service for that exact scope; any explicit sign-in,
    /// sign-out, switch or shutdown retires it.
    restart: Option<RestartJob>,
    /// The same restart result remains available for the view but can prepare authority only once.
    restart_service_installed: bool,
    workers: Vec<std::thread::JoinHandle<()>>,
    login_attempt: u64,
    login_status: Arc<Mutex<LoginStatus>>,
    /// Held by the worker for staging, promotion and the final commit marker. Cancel and sign-out
    /// try-lock it: a cancel during promotion is refused without waiting on keyring IO.
    persist_gate: Arc<Mutex<()>>,
    login_workers: Vec<std::thread::JoinHandle<()>>,
    browser: Arc<dyn BrowserLauncher>,
    runner: LoginRunner,
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
            restored: startup.restored.clone(),
            logout: startup.logout,
            secure_store: startup.secure_store,
            generation: 0,
            cleanup: Arc::new(Mutex::new(CleanupStatus::default())),
            restart: None,
            restart_service_installed: false,
            workers: Vec::new(),
            login_attempt: 0,
            login_status: Arc::new(Mutex::new(LoginStatus::default())),
            persist_gate: Arc::new(Mutex::new(())),
            login_workers: Vec::new(),
            browser: Arc::new(SystemBrowserLauncher),
            runner: production_login_runner(),
        };
        #[cfg(test)]
        if let Some((browser, runner)) = port.runtime.login_seams.take() {
            port.browser = browser;
            port.runner = runner;
        }
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
        let cancel = RestartCancellation::default();
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
            shared.scope = verified;
            drop(shared);
            if accepted {
                solidcraft_engine::note!(
                    "account: persisted account session verified by a bounded online status read; installing its account service"
                );
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

    /// Retire the restart check without waiting on its network request. The shared result is
    /// marked done and its cached account cleared immediately; a reply that lands after this
    /// retirement is dropped rather than publishing beside a newer explicit attempt.
    fn cancel_restart(&mut self) {
        if let Some(job) = self.restart.as_mut() {
            job.cancel.cancel();
            if let Ok(mut shared) = job.shared.lock() {
                shared.scope = None;
                shared.done = true;
            }
        }
    }

    fn verified_restart_scope(&self) -> Option<AuthenticatedHostScope> {
        self.restart.as_ref()?.shared.lock().ok()?.scope.clone()
    }

    fn verified_restart_account(&self) -> Option<String> {
        self.verified_restart_scope().map(|scope| scope.account().as_str().to_owned())
    }

    fn restart_waiting(&self) -> bool {
        self.restart.as_ref().is_some_and(|job| job.shared.lock().is_ok_and(|shared| !shared.done))
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

    /// Rerun finished handles only. A live canceled worker stays in the bounded list; no UI path
    /// joins it.
    fn reap_login_workers(&mut self) {
        let mut live = Vec::with_capacity(self.login_workers.len());
        for worker in std::mem::take(&mut self.login_workers) {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                live.push(worker);
            }
        }
        self.login_workers = live;
    }

    fn retired_login_workers(&mut self) -> usize {
        self.reap_login_workers();
        self.login_workers.len()
    }

    /// Advance the generation without taking the persistence gate, so cancel and retry never wait
    /// on a keyring write. A worker that already entered its final write observes the changed
    /// generation and removes its own attempt write off the window thread.
    fn invalidate_login(&mut self) {
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
        self.reap_login_workers();
        let mut status = self.login_status.lock().ok()?;
        if status.attempt != self.login_attempt {
            return None;
        }
        let mut verified = false;
        let mut cleared = false;
        let terminal = match status.phase {
            LoginPhase::Idle => return None,
            LoginPhase::Starting => {
                self.view.sign_in_state = SignInState::Starting;
                self.view.sign_in_prompt = None;
                self.view.sign_in_error = None;
                None
            }
            LoginPhase::Waiting => {
                self.view.sign_in_state = SignInState::Waiting;
                self.view.sign_in_prompt = status.prompt.clone();
                self.view.sign_in_error = None;
                None
            }
            LoginPhase::Done => {
                let account = status.account.clone()?;
                self.logout = StartupLogout::Clear;
                verified = true;
                self.view = signed_in_view(account);
                Some(())
            }
            LoginPhase::Expired => {
                cleared = true;
                self.view = AccountView { sign_in_state: SignInState::Expired, ..AccountView::signed_out() };
                Some(())
            }
            LoginPhase::Failed(code) => {
                cleared = true;
                self.view = AccountView { sign_in_state: SignInState::Failed, sign_in_error: Some(code), ..AccountView::signed_out() };
                Some(())
            }
        };
        if terminal.is_some() {
            // A terminal snapshot is adopted once. A later sign-out must not be resurrected by a
            // stale worker result.
            status.phase = LoginPhase::Idle;
            status.prompt = None;
            status.account = None;
        }
        // Install only after releasing the sign-in status lock: the credential read for the
        // proven scope must not run while another account action waits to publish a result.
        drop(status);
        if verified {
            // The exchange verified the product session and the installation bootstrap persisted a
            // provable scope. Install the reviewed HTTP account service for the commit gate. Chat
            // and model authority are not part of that service and stay refused until a reviewed
            // chat adapter exists.
            if !self.runtime.install_verified_service() {
                solidcraft_engine::note!("account: the verified sign-in produced no provable scope; commit authority stays closed");
            }
        } else if cleared {
            self.refuse_sign_in();
        }
        Some(self.view.clone())
    }

    fn poll_cleanup(&mut self) -> Option<AccountView> {
        let (pending, error, completed) = {
            let mut status = self.cleanup.lock().ok()?;
            let completed = if status.pending.is_some() || status.error.is_some() {
                false
            } else if status.completed {
                status.completed = false;
                true
            } else {
                // A startup recovery view or a pending action is not changed until its own worker
                // reports. An unreadable startup state has no cleanup to adopt.
                return None;
            };
            (status.pending.is_some(), status.error.as_ref().map(|refusal| refusal.code), completed)
        };
        if completed && self.secure_store {
            // The worker deleted the marker only after the old authority was gone. This is the one
            // place a readable startup tombstone becomes ordinary signed-out state.
            self.logout = StartupLogout::Clear;
        }
        self.view = if self.secure_store && !pending && error.is_none() && completed {
            AccountView::signed_out()
        } else {
            AccountView { sign_out_pending: pending, sign_out_error: error, ..AccountView::unavailable() }
        };
        Some(self.view.clone())
    }

    /// Refuse account work while a durable sign-out is unconfirmed, a cleanup is running or its
    /// latest generation failed. An unreadable status is itself a refusal.
    fn cleanup_confirmed(&self) -> Result<(), Refusal> {
        if self.logout != StartupLogout::Clear {
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
        if !self.secure_store {
            return Err(account_unavailable("no secure credential store is available"));
        }
        if self.logout == StartupLogout::Unreadable {
            return Err(account_unavailable("the account storage state cannot be read"));
        }
        let gate_holder = Arc::clone(&self.persist_gate);
        let gate = match gate_holder.try_lock() {
            Ok(gate) => gate,
            Err(_) => {
                return Err(account_unavailable("the sign-in is completing; sign out again after it finishes"));
            }
        };
        // Publish the new generation and set the barrier in the same short lock. A finishing
        // older worker takes that lock before clearing anything, so it can never clear a newer
        // barrier between generation publication and its own stale-result check. No keyring IO
        // happens while this lock is held.
        let cleanup = Arc::clone(&self.cleanup);
        let mut status = match cleanup.lock() {
            Ok(status) => status,
            Err(_) => {
                self.runtime.credentials.revoke_in_process();
                self.runtime.clear_account_service();
                self.cancel_restart();
                self.retire_control_renewal();
                self.invalidate_login();
                self.lifecycle = Some(event);
                let error = account_unavailable("the account cleanup state is unavailable");
                self.view = AccountView { sign_out_error: Some(error.code), ..AccountView::unavailable() };
                return Err(error);
            }
        };
        if status.pending.is_some() {
            return Err(account_unavailable("a sign-out is already pending; wait for it to finish or try again"));
        }
        // Authority is revoked in this process now. The durable tombstone and delete run on one
        // worker; this action does not claim durable completion until that worker confirms its
        // own generation. No keyring write or delete happens on the window thread.
        self.runtime.credentials.revoke_in_process();
        self.runtime.clear_account_service();
        self.cancel_restart();
        self.retire_control_renewal();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        status.pending = Some(generation);
        status.completed = false;
        status.error = None;
        drop(status);
        self.invalidate_login();
        drop(gate);
        self.lifecycle = Some(event);
        self.view = AccountView { sign_out_pending: true, ..AccountView::unavailable() };

        let credentials = self.runtime.credentials.clone();
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
                    status.completed = true;
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
                let error = account_unavailable("the sign-out cleanup could not start");
                self.view.sign_out_pending = false;
                self.view.sign_out_error = Some(error.code);
                if let Ok(mut status) = self.cleanup.lock() {
                    status.pending = None;
                    status.completed = false;
                    status.error = Some(error.clone());
                }
                Err(error)
            }
        }
    }

    fn join_workers(&mut self) {
        // Exit-only serialization: wait for an in-progress promotion to become the documented
        // winner before invalidating the generation. Never invalidate first and then accept a
        // commit that was already inside the gate.
        let gate_holder = Arc::clone(&self.persist_gate);
        let gate = gate_holder.lock();
        self.invalidate_login();
        drop(gate);
        let login_workers = std::mem::take(&mut self.login_workers);
        for worker in login_workers {
            let _ = worker.join();
        }
        let workers = std::mem::take(&mut self.workers);
        for worker in workers {
            let _ = worker.join();
        }
    }

    /// Install the account service for an online-verified restart, or drop one whose session has
    /// expired or been revoked. Called from the host's logic tick, never from a paint pass.
    fn reconcile_account_service(&mut self) {
        if !self.restart_service_installed
            && let Some(scope) = self.verified_restart_scope()
        {
            self.restart_service_installed = self.runtime.install_verified_service_for(&scope);
        } else {
            self.runtime.clear_stale_service();
        }
    }

    fn refuse_sign_in(&mut self) {
        self.runtime.clear_account_service();
        self.retire_control_renewal();
        self.lifecycle.get_or_insert(LifecycleEvent::SignInRefused);
    }
}

fn signed_in_view(account: String) -> AccountView {
    AccountView {
        signed_in: true,
        sign_in_visible: false,
        sign_in_state: SignInState::Idle,
        sign_in_prompt: None,
        sign_in_error: None,
        sign_out_pending: false,
        sign_out_recovery: false,
        sign_out_error: None,
        account_label: Some(account),
        available: None,
        held: 0,
        settling: false,
        // An authenticated account still needs an online status and a model to say chat is
        // available; the commit-gate account service proves neither by itself.
        chat_available: false,
        tool_blocked: Some(RefusalCode::AuthUnavailable),
        own_model_label: None,
    }
}

fn current_view(startup: &AccountStartup) -> AccountView {
    if !startup.secure_store {
        return AccountView::unavailable();
    }
    match startup.logout {
        // An unreadable store or marker cannot prove that recovery is possible: no action at all.
        StartupLogout::Unreadable => return AccountView::unavailable(),
        // A readable marker is recoverable, but the previous identity and authority stay gone.
        StartupLogout::ReadableTombstone => {
            return AccountView { sign_out_recovery: true, ..AccountView::unavailable() };
        }
        StartupLogout::Clear => {}
    }
    if let Some(scope) = startup.restored.as_ref() {
        return signed_in_view(scope.account().as_str().to_owned());
    }
    AccountView::signed_out()
}

/// While a provider attempt or a durable sign-out cleanup is pending, an idle window cannot rely
/// on unrelated input to poll the worker. Ask the host to repaint after a bounded interval so a
/// prompt, expiry, completion or confirmation reaches the screen; return `None` once idle.
pub fn poll_repaint_after(view: &AccountView) -> Option<Duration> {
    (view.sign_in_state.pending() || view.sign_out_pending).then_some(Duration::from_secs(1))
}

impl AccountPort for DesktopAccountPort {
    fn take_commit_authority(&mut self) -> Option<CommitAuthority> {
        self.runtime.take_commit_authority(self.restored.as_ref())
    }

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
        if let Some(account) = self.verified_restart_account() {
            return signed_in_view(account);
        }
        self.view.clone()
    }

    fn refresh_status(&mut self) -> Result<AccountView, Refusal> {
        self.cleanup_confirmed()?;
        self.reconcile_account_service();
        // A completed device flow or an online-verified restart is recognized from cached worker
        // state. Neither is replayed or exchanged again. The account service is installed for the
        // commit gate; this task still does not claim an online chat or model snapshot.
        if self.view.signed_in || self.verified_restart_account().is_some() {
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
        // A product session exists, but no online verification has accepted it. Never invent a
        // quota or a successful online status: keep the cached view and report the stable refusal.
        Err(account_unavailable("no authenticated account service is installed"))
    }

    fn sign_in(&mut self) -> Result<(), Refusal> {
        self.start_sign_in().inspect_err(|_| self.refuse_sign_in())
    }

    fn open_sign_in_browser(&mut self) -> Result<(), Refusal> {
        let config = self.runtime.config.as_ref().ok_or_else(|| account_unavailable("no configured sign-in verification origin"))?;
        let prompt = self.begin_device_code()?;
        if prompt.expires_at.0 <= unix_seconds() {
            return Err(Refusal::new(RefusalCode::AuthRequired, "the sign-in code expired"));
        }
        let url = prompt.verification_uri.as_str();
        // Re-check the exact configured origin at the launch boundary, not only when the prompt
        // was published.
        if !trusted_browser_url(url, &config.verification_origin) {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the sign-in page is not the configured verification origin"));
        }
        self.browser.open(url, &config.verification_origin)
    }

    fn cancel_sign_in(&mut self) -> Result<(), Refusal> {
        if self.view.sign_out_pending || self.view.sign_out_error.is_some() {
            return Err(account_unavailable("sign-out cleanup has not been confirmed"));
        }
        let gate_holder = Arc::clone(&self.persist_gate);
        let gate = match gate_holder.try_lock() {
            Ok(gate) => gate,
            Err(_) => {
                // Promotion owns the gate from its first staged write through the commit marker.
                // Cancelling now is refused, not reported as success and not waited for.
                return Err(account_unavailable("the sign-in is completing; it can no longer be canceled"));
            }
        };
        let completed = self.view.signed_in || self.login_status.lock().map(|status| status.phase == LoginPhase::Done).unwrap_or(false);
        if completed {
            return Err(account_unavailable("the sign-in has completed; sign out instead"));
        }
        self.invalidate_login();
        self.view = if self.secure_store { AccountView::signed_out() } else { AccountView::unavailable() };
        drop(gate);
        Ok(())
    }

    fn poll(&mut self) -> Option<AccountView> {
        if let Some(view) = self.poll_login() {
            return Some(view);
        }
        self.reconcile_account_service();
        self.poll_cleanup().or_else(|| self.verified_restart_account().map(signed_in_view))
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        self.revoke(LifecycleEvent::SignedOut)
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        self.revoke(LifecycleEvent::AccountSwitched)
    }

    fn delete_account_guide(&self) -> DeleteAccountGuide {
        DeleteAccountGuide { help_url: "", steps: &["account.delete_request", "account.delete_request_safety"] }
    }

    fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        let status = self.login_status.lock().map_err(|_| account_unavailable("the sign-in state is unavailable"))?;
        status
            .prompt
            .clone()
            .filter(|_| status.attempt == self.login_attempt && status.phase == LoginPhase::Waiting)
            .ok_or_else(|| Refusal::new(RefusalCode::AuthRequired, "no current sign-in verification address"))
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
        self.runtime.clear_account_service();
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

impl DesktopAccountPort {
    fn start_sign_in(&mut self) -> Result<(), Refusal> {
        if self.logout != StartupLogout::Clear {
            return Err(account_unavailable("previous sign-out must be confirmed before signing in"));
        }
        if self.restart_waiting() {
            return Err(account_unavailable("the previous session is still being checked or settled"));
        }
        if self.login_pending() {
            return Err(account_unavailable("a sign-in is already in progress"));
        }
        if self.view().signed_in {
            // Lifecycle rule: a new provider login never replaces an active account implicitly.
            return Err(account_unavailable("sign out or switch account before signing in again"));
        }
        if !self.secure_store {
            return Err(account_unavailable("no secure credential store is available"));
        }
        if let Ok(mut status) = self.cleanup.lock() {
            if status.pending.is_some() {
                return Err(account_unavailable("the sign-out cleanup is still running"));
            }
            if let Some(error) = &status.error {
                return Err(error.clone());
            }
            // A new attempt supersedes an unadopted cleanup confirmation, so the later poll can
            // never overwrite the signed-in view with the previous sign-out.
            status.completed = false;
        } else {
            return Err(account_unavailable("the account cleanup state is unavailable"));
        }
        if self.runtime.credentials.is_revoked() {
            return Err(account_unavailable("the account authority is revoked"));
        }
        let config = self.runtime.config.clone().ok_or_else(|| account_unavailable("no configured product-session verification origin"))?;
        if self.retired_login_workers() >= MAX_LOGIN_WORKERS {
            return Err(account_unavailable("previous sign-in workers are still finishing"));
        }

        self.cancel_restart();
        self.login_attempt = self.login_attempt.wrapping_add(1);
        let attempt = self.login_attempt;
        match self.login_status.lock() {
            Ok(mut status) => {
                *status = LoginStatus { attempt, phase: LoginPhase::Starting, prompt: None, account: None };
            }
            Err(_) => return Err(account_unavailable("the sign-in state is unavailable")),
        }
        self.view = AccountView { sign_in_state: SignInState::Starting, ..AccountView::signed_out() };

        let task = LoginTask {
            attempt,
            client: config.client,
            connection: config.product,
            verification_origin: config.verification_origin,
            credentials: self.runtime.credentials.clone(),
            status: Arc::clone(&self.login_status),
            persist_gate: Arc::clone(&self.persist_gate),
            bootstrap_transport: None,
        };
        match (self.runner)(task) {
            Ok(worker) => {
                self.login_workers.push(worker);
                Ok(())
            }
            Err(error) => {
                // A failed spawn is not a pending login and must not leave the menu in Starting.
                self.invalidate_login();
                self.view = AccountView::signed_out();
                Err(error)
            }
        }
    }
}

/// Install the desktop account runtime into the app.
///
/// The UI port is always installed so the start page and title bar have one host-owned seam. The
/// durable ledgers are installed when they opened. Commit authority is handed over by the port
/// on the logic tick only after a bounded restart check or sign-in verifies the session.
pub fn install(app: &mut SolveApp, runtime: AccountRuntime, startup: AccountStartup) {
    if startup.logout == StartupLogout::ReadableTombstone {
        solidcraft_engine::note!(
            "account: a readable sign-out marker is present; recovery is offered and no prior identity or authority is restored"
        );
    } else if startup.logout == StartupLogout::Unreadable {
        solidcraft_engine::note!("account: the sign-out state could not be read; account restore and actions stay closed");
    } else if startup.candidate.is_some() {
        solidcraft_engine::note!("account: local restart candidate stays signed out until its bounded online check finishes");
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

/// Adopt worker results, retire any old authority, then install authority for a proved session.
/// Runs before controlled dispatch, never in the paint pass.
pub fn poll(app: &mut SolveApp) -> bool {
    app.account.process_actions();
    app.drain_account_lifecycle();
    let Some((scope, service, clock)) = app.account.take_commit_authority() else { return false };
    app.install_commit_authority(scope, service, clock);
    solidcraft_engine::note!("account: authenticated commit authority installed for the verified session");
    true
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

    use serde_json::{Value, json};
    use solidcraft_account_native::{
        CredentialHandoff, CredentialSession, DeviceCodePrompt, DeviceCodeTransport, MemorySession, NoPersistentStore, ProductExchangeError,
        ProductLogin, ProductSession, ProductSessionExchange, ProviderSession, Secret, Transport, TransportFailure, WireCredentials, WireRequest,
        WireResponse,
    };
    use solidcraft_engine::entitlement::{
        AccountId, AccountService, Admission, Availability, BillingResult, Capability, CredentialError, CredentialErrorCode, CredentialStore,
        EntryCategory, Grant, HostSessionBinding, InstallationId, OperationId, OperationStatus, Refusal, RefusalCode,
    };
    use solidcraft_ui_egui::account::{AccountPort, Action};
    use solidcraft_ui_egui::control;

    use super::legacy_device_tests::*;
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
        block_set: Arc<Mutex<Option<(String, Receiver<()>)>>>,
        fail_set: Arc<Mutex<Option<String>>>,
        persistent: Arc<AtomicBool>,
        get_calls: Arc<AtomicUsize>,
        set_calls: Arc<AtomicUsize>,
        delete_calls: Arc<AtomicUsize>,
        delete_waiting: Arc<AtomicUsize>,
        get_waiting: Arc<AtomicUsize>,
        set_waiting: Arc<AtomicUsize>,
    }

    impl Default for TestStore {
        fn default() -> Self {
            Self {
                entries: Arc::new(Mutex::new(BTreeMap::new())),
                fail_delete: Arc::new(AtomicBool::new(false)),
                fail_get: Arc::new(AtomicBool::new(false)),
                block_delete: Arc::new(Mutex::new(VecDeque::new())),
                block_get: Arc::new(Mutex::new(None)),
                block_set: Arc::new(Mutex::new(None)),
                fail_set: Arc::new(Mutex::new(None)),
                persistent: Arc::new(AtomicBool::new(true)),
                get_calls: Arc::new(AtomicUsize::new(0)),
                set_calls: Arc::new(AtomicUsize::new(0)),
                delete_calls: Arc::new(AtomicUsize::new(0)),
                delete_waiting: Arc::new(AtomicUsize::new(0)),
                get_waiting: Arc::new(AtomicUsize::new(0)),
                set_waiting: Arc::new(AtomicUsize::new(0)),
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

        /// Make the next write of `key` wait until the test releases it.
        fn block_next_set(&self, key: &str, gate: Receiver<()>) {
            *self.block_set.lock().unwrap() = Some((key.to_owned(), gate));
        }

        /// Fail writes of `key` while set; clear with `None` to allow a retry.
        fn set_fail_key(&self, key: Option<&str>) {
            *self.fail_set.lock().unwrap() = key.map(str::to_owned);
        }

        /// Model a failed secure-store probe without changing what is readable.
        fn set_persistent(&self, persistent: bool) {
            self.persistent.store(persistent, Ordering::SeqCst);
        }

        fn mutation_count(&self) -> usize {
            self.set_calls.load(Ordering::SeqCst) + self.delete_calls.load(Ordering::SeqCst)
        }

        fn wait_until_set_blocked(&self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.set_waiting.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(self.set_waiting.load(Ordering::SeqCst) > 0, "the staged/commit write was never reached");
        }
    }

    impl CredentialStore for TestStore {
        fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
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
            self.set_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_set.lock().unwrap().as_deref() == Some(key) {
                return Err(store_error());
            }
            let gate = {
                let mut blocked = self.block_set.lock().unwrap();
                match blocked.as_ref() {
                    Some((expected, _)) if expected == key => blocked.take().map(|(_, gate)| gate),
                    _ => None,
                }
            };
            if let Some(gate) = gate {
                self.set_waiting.fetch_add(1, Ordering::SeqCst);
                let _ = gate.recv();
                self.set_waiting.fetch_sub(1, Ordering::SeqCst);
            }
            self.entries.lock().unwrap().insert(key.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
            self.delete_calls.fetch_add(1, Ordering::SeqCst);
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
            self.persistent.load(Ordering::SeqCst)
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
    fn desktop_deletion_guide_has_localized_steps_without_a_url() {
        let runtime = AccountRuntime::with_store(NoPersistentStore);
        let startup = runtime.startup();
        let port = DesktopAccountPort::new(runtime, &startup);
        let guide = port.delete_account_guide();
        assert!(guide.help_url.is_empty());
        assert_eq!(guide.steps, &["account.delete_request", "account.delete_request_safety"]);
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
    fn desktop_does_not_open_the_keyring_without_a_reviewed_origin_or_the_lock() {
        let open_store = |_: &str| -> Box<dyn CredentialStore + Send> { panic!("the OS keyring must not open") };
        let lock_dir = std::env::temp_dir().join(format!("intcad-no-origin-{}", std::process::id()));
        let no_origin = AccountRuntime::desktop_with_store(None, None, Some(&lock_dir), open_store);
        assert!(!no_origin.startup().secure_store);
        assert!(!lock_dir.exists(), "no reviewed origin need not reserve the keyring");
        let config = DesktopAccountConfig::new("client_test", "https://api.example", "https://auth.example").unwrap();
        let no_lock = AccountRuntime::desktop_with_store(None, Some(config), None, open_store);
        assert!(!no_lock.startup().secure_store);
    }

    #[test]
    fn desktop_restart_never_reads_or_sends_another_origins_credentials() {
        use solidcraft_engine::entitlement::{OperationStore, TransportCorrelation};

        let path = std::env::temp_dir().join(format!("intcad-origin-binding-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(path);
        let stores = std::cell::RefCell::new(BTreeMap::<String, TestStore>::new());
        let store_for = |service: &str| stores.borrow_mut().entry(service.to_owned()).or_default().clone();
        let open = |origin, product: ScriptedProduct| {
            let config = DesktopAccountConfig::new("client_test", origin, "https://auth.example").unwrap();
            let mut runtime =
                AccountRuntime::desktop_with_store(Some(&scratch.0), Some(config), Some(&scratch.0), |service| Box::new(store_for(service)));
            runtime.restart_transport = Some(Arc::new(move || Box::new(product.clone())));
            runtime
        };
        let original = open("https://api.example", ScriptedProduct::default());
        let mut session = original.credentials.clone();
        session.complete_provider_login("provider_account", "provider_token").unwrap();
        session.activate_product_session(&TestExchange).unwrap();
        CredentialHandoff::installation_credential(&mut session, "installation_desktop", Secret::new("installation_credential")).unwrap();
        let scope = session.authenticated_scope().unwrap();
        let mut ledger = FileOperationStore::open(scratch.0.join("account")).unwrap();
        let row =
            ledger.persist_intent(&scope, &TransportCorrelation::new("origin_binding"), Capability::Review, EntryCategory::ReviewPanel).unwrap();
        ledger.record_settlement_intent(&row.operation, SettlementIntent::Commit(BillingResult::Succeeded)).unwrap();
        CredentialHandoff::settlement_credential(&mut session, &row.operation, Secret::new("fixture_settlement")).unwrap();
        drop(session);
        drop(original);

        let original_store = stores.borrow().values().next().unwrap().clone();
        let legacy = store_for(solidcraft_account_native::ACCOUNT_KEYRING_SERVICE);
        let unscoped_entries = original_store.entries.lock().unwrap().clone();
        *legacy.entries.lock().unwrap() = unscoped_entries;
        let original_reads = original_store.get_calls.load(Ordering::SeqCst);
        let recovery_script = || {
            ScriptedProduct::scripted(vec![
                Ok(WireResponse::new(200, status_body_for(scope.account().as_str()).to_string().into_bytes())),
                Ok(WireResponse::new(
                    200,
                    json!({
                        "operation_id": row.operation.as_str(), "state":"CHARGED", "result":"SUCCEEDED", "terminal":true,
                        "finalized_at":"2026-01-01T00:00:30Z", "settlement_deadline":"2026-01-01T00:11:00Z"
                    })
                    .to_string()
                    .into_bytes(),
                )),
            ])
        };

        // A shared ledger and old unscoped entries are not evidence that the new origin owns
        // the previous server's bearer or settlement credential, even if it would accept them.
        for origin in ["https://other.example", "https://api.example:8443"] {
            let product = recovery_script();
            let runtime = open(origin, product.clone());
            let startup = runtime.startup();
            assert!(startup.secure_store);
            let mut port = DesktopAccountPort::new(runtime, &startup);
            for worker in std::mem::take(&mut port.workers) {
                worker.join().unwrap();
            }
            assert!(product.requests().is_empty(), "a different origin must send neither status nor finalize");
            assert!(startup.candidate.is_none());
            assert!(!port.view().signed_in);
            port.shutdown();
            assert_eq!(original_store.get_calls.load(Ordering::SeqCst), original_reads);
            assert_eq!(legacy.get_calls.load(Ordering::SeqCst), 0);
            assert_eq!(ledger.load(&row.operation).unwrap().unwrap().status, OperationStatus::Held);
        }

        let product = recovery_script();
        let same = open("https://API.Example:443/", product.clone());
        let startup = same.startup();
        assert_eq!(startup.candidate.as_ref(), Some(&scope));
        let mut port = DesktopAccountPort::new(same, &startup);
        for worker in std::mem::take(&mut port.workers) {
            worker.join().unwrap();
        }
        assert!(port.view().signed_in);
        port.shutdown();
        let requests = product.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url(), "https://api.example/v1/status");
        assert_eq!(requests[0].header("Authorization"), Some("Bearer product_session_test_0000000000"));
        assert_eq!(requests[1].url(), format!("https://api.example/v1/operations/{}/finalize", row.operation.as_str()));
        assert_eq!(requests[1].header("X-Settlement-Credential"), Some("fixture_settlement"));
        assert_eq!(ledger.load(&row.operation).unwrap().unwrap().status, OperationStatus::Charged);
        assert_eq!(legacy.get_calls.load(Ordering::SeqCst), 0);
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
    fn startup_settlement_rotates_past_repeated_failures_without_exceeding_four_attempts() {
        use solidcraft_engine::entitlement::TransportCorrelation;

        let path = std::env::temp_dir().join(format!("intcad-settlement-backlog-{}", std::process::id()));
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
        let mut ledger = FileOperationStore::open(&scratch.0).unwrap();
        let intent = SettlementIntent::Commit(BillingResult::Succeeded);
        let mut operations = std::array::from_fn::<_, 5, _>(|i| {
            let row = ledger
                .persist_intent(&scope, &TransportCorrelation::new(format!("backlog_{i}")), Capability::Review, EntryCategory::ReviewPanel)
                .unwrap();
            ledger.record_settlement_intent(&row.operation, intent).unwrap();
            CredentialHandoff::settlement_credential(&mut session, &row.operation, Secret::new("fixture_settlement")).unwrap();
            row.operation
        });
        operations.sort();
        drop(ledger);
        drop(session);

        // All finalize calls fail, so no row disappears and each restart must
        // make progress through the unchanged backlog on its own.
        for (launch, expected) in [[0, 1, 2, 3], [4, 0, 1, 2], [3, 4, 0, 1]].into_iter().enumerate() {
            let mut responses = vec![Ok(WireResponse::new(200, status_body_for(scope.account().as_str()).to_string().into_bytes()))];
            responses.extend((0..5).map(|_| Err(TransportFailure::Timeout)));
            let mut product = ScriptedProduct::scripted(responses);
            let (entered_tx, entered_rx) = channel();
            let (release_tx, release_rx) = channel();
            if launch == 0 {
                let release_rx = Mutex::new(release_rx);
                let sends = AtomicUsize::new(0);
                product.on_send(move || {
                    if sends.fetch_add(1, Ordering::SeqCst) == 1 {
                        entered_tx.send(()).unwrap();
                        let _ = release_rx.lock().unwrap().recv_timeout(Duration::from_secs(10));
                    }
                });
            }
            let opener = RecordingOpener::default();
            let mut runtime = restart_runtime(CredentialSession::new(Box::new(store.clone())), product.clone(), &opener);
            runtime.ledger = Some(FileOperationStore::open(&scratch.0).unwrap());
            let startup = runtime.startup();
            let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
            install(&mut app, runtime, startup);
            if launch == 0 {
                entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                let competing = ScriptedProduct::scripted(vec![
                    Ok(WireResponse::new(200, status_body_for(scope.account().as_str()).to_string().into_bytes())),
                    Err(TransportFailure::Timeout),
                ]);
                let mut runtime = restart_runtime(CredentialSession::new(Box::new(store.clone())), competing.clone(), &opener);
                let shared_ledger = FileOperationStore::open(&scratch.0).unwrap();
                runtime.ledger = Some(FileOperationStore::open(&scratch.0).unwrap());
                let cursor_before = shared_ledger.settlement_cursor(&scope);
                let startup = runtime.startup();
                let mut port = DesktopAccountPort::new(runtime, &startup);
                wait_for_restart(&port);
                assert!(port.view().signed_in);
                let requests = competing.requests();
                let cursor_after = shared_ledger.settlement_cursor(&scope);
                release_tx.send(()).unwrap();
                port.shutdown();
                assert_eq!(requests.len(), 1, "overlapping startup must not send finalize");
                assert_eq!(requests[0].url(), "https://api.example/v1/status");
                assert_eq!(cursor_after, cursor_before, "overlapping startup must not move the cursor");
            }
            wait_until("startup settlement completes", || {
                app.account.process_actions();
                app.account.view().signed_in
            });
            app.account.shutdown();
            let requests = product.requests();
            assert_eq!(requests.len(), 5, "one status read plus four finalize attempts");
            assert_eq!(requests[0].url(), "https://api.example/v1/status");
            for (request, index) in requests[1..].iter().zip(expected) {
                assert_eq!(request.method(), "POST");
                assert_eq!(
                    request.url(),
                    format!("https://api.example/v1/operations/{}/finalize", operations[index].as_str()),
                    "startup {} must resume the bounded scan",
                    launch + 1
                );
            }
        }
        let ledger = FileOperationStore::open(&scratch.0).unwrap();
        for operation in &operations {
            assert_eq!(ledger.load(operation).unwrap().unwrap().status, OperationStatus::Held);
            assert_eq!(ledger.settlement_intent(operation).unwrap(), Some(intent));
        }
    }

    #[test]
    fn startup_settlement_lock_survives_process_contention_and_releases_on_exit() {
        use solidcraft_engine::entitlement::{TransportCorrelation, UnsettledOperation};
        use std::process::{Child, Command, Stdio};

        const ROOT: &str = "INTCAD_TEST_SETTLEMENT_PROCESS_ROOT";
        const ROLE: &str = "INTCAD_TEST_SETTLEMENT_PROCESS_ROLE";
        let store = TestStore::default();
        let mut session = activate_session(&store);
        let scope = session.authenticated_scope().unwrap();
        if let Some(root) = std::env::var_os(ROOT) {
            let role = std::env::var(ROLE).unwrap();
            let ledger = FileOperationStore::open(root).unwrap();
            let rows = ledger.unsettled_operations(Some(&scope)).unwrap();
            assert_eq!(rows.len(), 5);
            for row in &rows {
                let UnsettledOperation::Owned(record) = row else { panic!("expected an owned fixture row") };
                CredentialHandoff::settlement_credential(&mut session, &record.operation, Secret::new("fixture_settlement")).unwrap();
            }
            let mut responses = vec![Ok(WireResponse::new(200, status_body_for(scope.account().as_str()).to_string().into_bytes()))];
            responses.extend((0..5).map(|_| Err(TransportFailure::Timeout)));
            let mut product = ScriptedProduct::scripted(responses);
            if role == "holder" {
                let ready = ledger.root().join("holder-ready");
                let calls = AtomicUsize::new(0);
                product.on_send(move || {
                    if calls.fetch_add(1, Ordering::SeqCst) == 4 {
                        // The fourth finalize has reached the transport, after the cursor write.
                        std::fs::write(&ready, []).unwrap();
                        let _ = std::io::Read::read(&mut std::io::stdin(), &mut [0u8]);
                    }
                });
            }
            let opener = RecordingOpener::default();
            let mut runtime = restart_runtime(session, product.clone(), &opener);
            runtime.ledger = Some(ledger);
            let startup = runtime.startup();
            let mut port = DesktopAccountPort::new(runtime, &startup);
            if role == "holder" {
                port.join_workers();
                panic!("holder must be terminated while its fourth finalize is in flight");
            }
            wait_for_restart(&port);
            assert!(port.view().signed_in);
            port.shutdown();
            let requests = product.requests();
            assert_eq!(requests[0].url(), "https://api.example/v1/status");
            let expected: &[usize] = match role.as_str() {
                "contender" => &[],
                "successor" => &[4, 0, 1, 2],
                _ => panic!("unexpected process completion for role {role}"),
            };
            assert_eq!(requests.len(), 1 + expected.len(), "{role}: unexpected finalize count");
            for (request, index) in requests[1..].iter().zip(expected) {
                assert_eq!(request.method(), "POST");
                assert_eq!(request.url(), format!("https://api.example/v1/operations/{}/finalize", rows[*index].record().operation.as_str()));
            }
            return;
        }

        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        struct Holder(Child);
        impl Drop for Holder {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let path = std::env::temp_dir().join(format!("intcad-settlement-process-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let scratch = Scratch(path);
        let mut ledger = FileOperationStore::open(&scratch.0).unwrap();
        let intent = SettlementIntent::Commit(BillingResult::Succeeded);
        let mut operations = std::array::from_fn::<_, 5, _>(|i| {
            let row = ledger
                .persist_intent(&scope, &TransportCorrelation::new(format!("process_backlog_{i}")), Capability::Review, EntryCategory::ReviewPanel)
                .unwrap();
            ledger.record_settlement_intent(&row.operation, intent).unwrap();
            row.operation
        });
        operations.sort();

        let child = |role: &str| {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args(["--exact", "account::tests::startup_settlement_lock_survives_process_contention_and_releases_on_exit"]);
            cmd.env(ROOT, &scratch.0).env(ROLE, role);
            cmd
        };
        let verify_child = |role: &str| {
            let output = child(role).output().unwrap();
            assert!(output.status.success(), "{role}: {}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "{role}: child test did not run");
        };
        let mut holder = Holder(child("holder").stdin(Stdio::piped()).spawn().unwrap());
        wait_until("holder process enters its fourth finalize", || scratch.0.join("holder-ready").is_file());
        assert_eq!(ledger.settlement_cursor(&scope).as_ref(), Some(&operations[3]));
        verify_child("contender");
        assert_eq!(ledger.settlement_cursor(&scope).as_ref(), Some(&operations[3]), "contender must not modify the persisted cursor");

        // Exit without unwinding: the kernel must release the lock, not a Rust cleanup hook.
        holder.0.kill().unwrap();
        assert!(!holder.0.wait().unwrap().success());
        verify_child("successor");
        assert_eq!(ledger.settlement_cursor(&scope).as_ref(), Some(&operations[2]));
        for operation in &operations {
            assert_eq!(ledger.load(operation).unwrap().unwrap().status, OperationStatus::Held);
            assert_eq!(ledger.settlement_intent(operation).unwrap(), Some(intent));
        }
    }

    #[test]
    fn switching_accounts_during_startup_finalize_discards_the_receipt_and_stops_recovery() {
        use solidcraft_engine::entitlement::TransportCorrelation;

        let path = std::env::temp_dir().join(format!("intcad-startup-finalize-switch-{}", std::process::id()));
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
        let mut ledger = FileOperationStore::open(&scratch.0).unwrap();
        let intent = SettlementIntent::Commit(BillingResult::Succeeded);
        let mut operations = ["settlement_switch_a", "settlement_switch_b"].map(|correlation| {
            let row = ledger.persist_intent(&scope, &TransportCorrelation::new(correlation), Capability::Review, EntryCategory::ReviewPanel).unwrap();
            ledger.record_settlement_intent(&row.operation, intent).unwrap();
            CredentialHandoff::settlement_credential(&mut session, &row.operation, Secret::new("fixture_settlement")).unwrap();
            row.operation
        });
        // Startup inventories rows in operation-ID order.
        operations.sort();
        let [first, second] = &operations;
        drop(ledger);
        drop(session);

        let receipt = |operation: &OperationId| {
            Ok(WireResponse::new(
                200,
                json!({
                    "operation_id": operation.as_str(), "state":"CHARGED", "result":"SUCCEEDED", "terminal":true,
                    "finalized_at":"2026-01-01T00:00:30Z", "settlement_deadline":"2026-01-01T00:11:00Z"
                })
                .to_string()
                .into_bytes(),
            ))
        };
        let mut product = ScriptedProduct::scripted(vec![
            Ok(WireResponse::new(200, status_body_for(scope.account().as_str()).to_string().into_bytes())),
            receipt(first),
            receipt(second),
        ]);
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let release_rx = Mutex::new(release_rx);
        let sends = AtomicUsize::new(0);
        product.on_send(move || {
            if sends.fetch_add(1, Ordering::SeqCst) == 1 {
                entered_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
            }
        });
        let opener = RecordingOpener::default();
        let mut runtime = restart_runtime(CredentialSession::new(Box::new(store)), product.clone(), &opener);
        runtime.ledger = Some(FileOperationStore::open(&scratch.0).unwrap());
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        port.switch_account().unwrap();
        assert_eq!(port.take_lifecycle_event(), Some(LifecycleEvent::AccountSwitched));
        release_tx.send(()).unwrap();
        // Join without invoking a second cancellation via shutdown.
        port.join_workers();

        let ledger = FileOperationStore::open(&scratch.0).unwrap();
        for operation in &operations {
            assert_eq!(ledger.load(operation).unwrap().unwrap().status, OperationStatus::Held);
            assert_eq!(ledger.settlement_intent(operation).unwrap(), Some(intent));
        }
        let requests = product.requests();
        assert_eq!(requests.len(), 2, "status and the in-flight finalize only");
        assert_eq!(requests[0].url(), "https://api.example/v1/status");
        assert_eq!(requests[1].method(), "POST");
        assert_eq!(requests[1].url(), format!("https://api.example/v1/operations/{}/finalize", first.as_str()));
        assert!(!port.view().signed_in);
        assert!(opener.urls().is_empty());
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
        assert!(!poll(&mut app), "the app has no authority while the check is pending");
        assert!(!app.account.view().signed_in, "a pending check is never presented as signed in");
        assert!(app.account.startup_waiting(), "the host should wake while the bounded check runs");

        gate_tx.send(()).unwrap();
        wait_until("the gated restart authority", || poll(&mut app));
        assert_eq!(app.account.view().account_label.as_deref(), Some("acct_desktop_0001"));
        assert!(!app.account.startup_waiting(), "a finished check stops the bounded wake");
        assert_eq!(product.requests().len(), 1, "one status read and no bootstrap or replay");
        assert_eq!(product.requests()[0].method(), "GET");
        assert_eq!(product.requests()[0].url(), "https://api.example/v1/status");
        assert!(opener.urls().is_empty(), "the restart check never opens a browser");
        app.end_controlled_session();
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
    fn explicit_sign_in_cannot_replace_credentials_during_restart_check() {
        let (gate_tx, gate_rx) = channel();
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
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
        assert_eq!(port.sign_in().unwrap_err().code, RefusalCode::AuthUnavailable);
        assert!(port.startup_waiting(), "sign-in must not retire the old check");
        assert!(device.requests().is_empty());

        gate_tx.send(()).unwrap();
        wait_for_restart(&port);
        assert_eq!(port.view().account_label.as_deref(), Some("acct_desktop_0001"));
        port.shutdown();
        assert_eq!(product.requests().len(), 1, "only the original status read is allowed");
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
        assert!(startup.logout_pending(), "startup sees the unconfirmed sign-out");
        assert_eq!(startup.logout, StartupLogout::ReadableTombstone);
        assert!(startup.candidate.is_none());
        assert!(startup.restored.is_none());

        // Once the inert store is healthy, the marker still suppresses restore until a reviewed
        // sign-in path clears it.
        store.fail_delete.store(false, Ordering::SeqCst);
        let startup = AccountRuntime::with_session(CredentialSession::new(Box::new(store))).startup();
        assert!(startup.logout_pending());
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
        assert!(startup.logout_pending(), "an unreadable safety marker is treated as unsafe");
        assert_eq!(startup.logout, StartupLogout::Unreadable);
        assert!(startup.restored.is_none());
        let mut port = DesktopAccountPort::new(runtime, &startup);
        let view = port.view();
        assert!(!view.sign_out_recovery, "unreadable storage must not expose recovery");
        assert!(!view.sign_in_visible, "unreadable storage must stay closed");
        assert!(port.sign_out().is_err(), "no write may be attempted when storage is unreadable");
    }

    #[test]
    fn a_failed_store_probe_refuses_sign_out_and_switch_without_mutating_storage() {
        let store = TestStore::default();
        let session = activate_session(&store);
        store.fail_delete.store(true, Ordering::SeqCst);
        assert_eq!(session.sign_out().unwrap_err().code, CredentialErrorCode::Unavailable);
        assert!(session.logout_pending().unwrap(), "a readable marker remains");
        store.fail_delete.store(false, Ordering::SeqCst);
        // The probe failed: storage is usable for reads, but this process may not write or spawn.
        store.set_persistent(false);

        let entries = store.entries.lock().unwrap().clone();
        let mutations = store.mutation_count();
        for (action, revoke) in [
            (LifecycleEvent::SignedOut, DesktopAccountPort::sign_out as fn(&mut DesktopAccountPort) -> Result<(), Refusal>),
            (LifecycleEvent::AccountSwitched, DesktopAccountPort::switch_account),
        ] {
            let runtime = AccountRuntime::with_session(CredentialSession::new(Box::new(store.clone())));
            let startup = runtime.startup();
            assert!(!startup.secure_store, "the failed probe is visible at startup");
            assert_eq!(startup.logout, StartupLogout::ReadableTombstone, "the marker is still readable");
            let mut port = DesktopAccountPort::new(runtime, &startup);
            assert!(!port.view().sign_out_recovery, "a failed probe must not offer recovery");
            assert!(!port.view().sign_in_visible, "a failed probe must not offer sign-in");
            let before = port.view().clone();
            let refused = revoke(&mut port);
            assert!(refused.is_err(), "{action:?} must fail closed without a secure store");
            assert_eq!(port.view(), before, "{action:?} changed the view");
            assert!(port.lifecycle.is_none(), "{action:?} raised a lifecycle event");
            assert!(port.workers.is_empty(), "{action:?} spawned cleanup");
            assert!(!port.runtime.credentials.is_revoked(), "{action:?} revoked authority");
            assert!(port.cleanup.lock().unwrap().pending.is_none(), "{action:?} started cleanup");
            assert_eq!(store.mutation_count(), mutations, "{action:?} wrote or deleted storage");
            assert_eq!(*store.entries.lock().unwrap(), entries, "{action:?} changed stored entries");
        }
    }

    #[test]
    fn a_readable_startup_tombstone_offers_recovery_and_clears_only_after_confirmed_cleanup() {
        let store = TestStore::default();
        let session = activate_session(&store);
        store.fail_delete.store(true, Ordering::SeqCst);
        assert_eq!(session.sign_out().unwrap_err().code, CredentialErrorCode::Unavailable);

        let runtime = AccountRuntime::with_session(CredentialSession::new(Box::new(store.clone())));
        let startup = runtime.startup();
        assert_eq!(startup.logout, StartupLogout::ReadableTombstone);
        assert!(startup.restored.is_none(), "the marker suppressed identity restore");
        let mut port = DesktopAccountPort::new(runtime, &startup);
        assert!(port.view().sign_out_recovery, "a readable tombstone must offer recovery");
        assert!(!port.view().sign_in_visible, "no sign-in before the marker is cleared");
        assert!(!port.view().signed_in, "no prior identity is restored");
        assert!(port.runtime.authenticated_scope().is_none(), "no prior authority is restored");

        store.fail_delete.store(false, Ordering::SeqCst);
        port.sign_out().unwrap();
        assert!(port.view().sign_out_pending, "the recovery action starts one cleanup");
        wait_for_cleanup(&port);
        let view = port.poll().expect("the recovery cleanup is adopted once");
        assert!(!view.sign_out_recovery);
        assert!(!view.sign_out_pending);
        assert!(view.sign_in_visible, "confirmed cleanup returns to ordinary signed-out state");
        assert_eq!(port.logout, StartupLogout::Clear);
        assert!(!port.runtime.credentials.logout_pending().unwrap(), "the durable marker is gone");
        assert!(port.runtime.authenticated_scope().is_none(), "recovery never revives authority");
        port.shutdown();
    }

    #[test]
    fn poll_repaint_is_bounded_to_pending_worker_states() {
        assert_eq!(poll_repaint_after(&AccountView::signed_out()), None, "an idle window must not schedule frames");
        assert_eq!(poll_repaint_after(&signed_in_view("alice".into())), None);
        assert_eq!(poll_repaint_after(&AccountView { sign_in_state: SignInState::Expired, ..AccountView::signed_out() }), None);
        assert_eq!(poll_repaint_after(&AccountView { sign_in_state: SignInState::Failed, ..AccountView::signed_out() }), None);
        for state in [SignInState::Starting, SignInState::Waiting] {
            let view = AccountView { sign_in_state: state, ..AccountView::signed_out() };
            assert_eq!(poll_repaint_after(&view), Some(Duration::from_secs(1)), "{state:?} needs a bounded poll");
        }
        let pending = AccountView { sign_out_pending: true, ..AccountView::unavailable() };
        assert_eq!(poll_repaint_after(&pending), Some(Duration::from_secs(1)), "sign-out completion needs a bounded poll");
        let recovery = AccountView { sign_out_recovery: true, ..AccountView::unavailable() };
        assert_eq!(poll_repaint_after(&recovery), None, "recovery waits for a person, not a worker");
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
            let first = port.view();
            assert!(first.sign_out_pending, "{action:?} must show pending before the first poll");
            assert!(!first.signed_in);
            assert!(!first.sign_in_visible);
            assert_eq!(first.account_label, None);
            assert_eq!(first.sign_out_error, None);
            wait_for_cleanup(&port);
            let confirmed = port.poll().expect("confirmed cleanup updates the pending view");
            assert!(!confirmed.sign_out_pending);
            assert!(confirmed.sign_in_visible);
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
        assert_eq!(port.logout, StartupLogout::Clear, "the startup gate was already clear");
    }

    #[test]
    fn a_repeated_sign_out_while_pending_is_refused_without_a_second_worker() {
        let store = TestStore::default();
        let (gate_tx, gate_rx) = channel();
        let runtime = AccountRuntime::with_session(activate_session(&store));
        store.block_next_delete(gate_rx);
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        port.sign_out().unwrap();
        wait_for_counter(&store.delete_waiting, 1);
        assert_eq!(port.cleanup.lock().unwrap().pending, Some(1));
        assert!(port.runtime.credentials.is_revoked());
        let attempt = port.login_attempt;
        let started = Instant::now();
        let error = port.sign_out().unwrap_err();
        assert_eq!(error.code, RefusalCode::AuthUnavailable);
        assert!(started.elapsed() < Duration::from_millis(100), "a repeated sign-out blocked on keyring IO");
        assert_eq!(port.cleanup.lock().unwrap().pending, Some(1), "a repeated sign-out started a second worker");
        assert_eq!(port.login_attempt, attempt, "refusing an action must not advance the login fence");
        assert_eq!(port.take_lifecycle_event(), Some(LifecycleEvent::SignedOut), "one sign-out raises one event");
        assert_eq!(port.take_lifecycle_event(), None);

        gate_tx.send(()).unwrap();
        wait_for_cleanup(&port);
        assert!(port.cleanup.lock().unwrap().error.is_none());
        assert!(!port.runtime.credentials.is_revoked(), "the owning generation lifts the barrier");
        port.shutdown();
    }

    #[test]
    fn shutdown_waits_for_a_blocked_promotion_and_accepts_the_serialized_commit() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
        let (runner, _) = test_runner(ScriptedDeviceTransport::default(), exchange);
        let (gate_tx, gate_rx) = channel();
        store.block_next_set("login/commit", gate_rx);
        let mut port = login_port(browser, runner, &store);
        port.sign_in().unwrap();
        store.wait_until_set_blocked();
        std::thread::scope(|scope| {
            let shutdown = scope.spawn(|| port.shutdown());
            std::thread::sleep(Duration::from_millis(50));
            assert!(!shutdown.is_finished(), "shutdown invalidated the generation before the promotion finished");
            gate_tx.send(()).unwrap();
            shutdown.join().unwrap();
        });
        assert!(port.runtime.credentials.product_session_active(), "the promotion that won the gate was not a valid commit");
    }

    #[test]
    fn a_failed_sign_out_marker_is_surfaced_and_retryable() {
        let store = TestStore::default();
        let runtime = AccountRuntime::with_session(activate_session(&store));
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        store.set_fail_key(Some("auth/logout_pending"));
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        let error = port.cleanup.lock().unwrap().error.clone().expect("the failed durable marker is surfaced");
        assert_eq!(error.code, RefusalCode::AuthUnavailable);
        let _ = AccountPort::poll(&mut port);
        assert_eq!(port.view().sign_out_error, Some(RefusalCode::AuthUnavailable));
        assert!(!port.view().sign_in_visible);
        assert!(port.cancel_sign_in().is_err(), "cancel cannot replace the cleanup error view");
        assert_eq!(port.view().sign_out_error, Some(RefusalCode::AuthUnavailable));
        assert!(port.runtime.credentials.is_revoked(), "the in-process authority stays revoked");
        assert!(!port.runtime.credentials.logout_pending().unwrap_or(false), "no durable tombstone is claimed");

        store.set_fail_key(None);
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        assert!(port.cleanup.lock().unwrap().error.is_none());
        assert!(!port.runtime.credentials.logout_pending().unwrap_or(true), "the retry wrote and cleared the marker");
        assert!(!port.runtime.credentials.product_session_active());
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
        let runtime = AccountRuntime::new(session.clone(), Some(ledger), None);
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
        ctx.run_ui(egui::RawInput { time: Some(1.0), ..Default::default() }, |ui| {
            solidcraft_ui_egui::dfm_review::poll(&mut app, ui.ctx());
        })
        .textures_delta
        .clear();
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
        let mut port = DesktopAccountPort::new(unverified, &startup);
        assert!(port.take_commit_authority().is_none(), "a local candidate alone installs no authority");
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        app.account = State::with_port(Box::new(port));
        assert!(!poll(&mut app), "there is no authority to install before verification");
    }

    /// A complete sign-in through the real worker and a scripted product service, including the
    /// installation bootstrap, so the persisted session has a provable scope.
    fn verified_sign_in_port() -> DesktopAccountPort {
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::scripted(vec![
            Ok(WireResponse::new(200, status_body().to_string().into_bytes())),
            Ok(WireResponse::new(201, installation_body().to_string().into_bytes())),
        ]);
        let runtime = harness_runtime(installed_session(), device, product, Arc::new(VirtualTiming::default()), &RecordingOpener::default());
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        port.sign_in().unwrap();
        wait_for_login(&mut port, true);
        assert!(port.runtime.service.is_some(), "the completed sign-in installed the account service");
        port
    }

    #[test]
    fn a_verified_sign_in_installs_the_commit_authority() {
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::scripted(vec![
            Ok(WireResponse::new(200, status_body().to_string().into_bytes())),
            Ok(WireResponse::new(201, installation_body().to_string().into_bytes())),
            Ok(WireResponse::new(200, status_body().to_string().into_bytes())),
        ]);
        let runtime = harness_runtime(installed_session(), device, product, Arc::new(VirtualTiming::default()), &RecordingOpener::default());
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        port.sign_in().unwrap();
        wait_for_login(&mut port, true);

        let scope = port.runtime.authenticated_scope().expect("the installed pair gives a provable scope");
        assert!(scope.is_provable());
        assert!(port.runtime.service.is_some(), "the verified sign-in installs the account service");
        let runtime_clock = port.runtime.clock.clone().unwrap();
        let before = runtime_clock.now();
        let (installed, service, clock) = port.take_commit_authority().expect("the verified sign-in gives commit authority");
        let drained = clock.now();
        let after = runtime_clock.now();
        assert!((before..=after).contains(&drained), "the handover did not keep the runtime clock's epoch");
        assert_eq!(installed, scope);
        let availability = service.status().expect("the installed service talks to the same product origin");
        assert!(availability.online, "the served status is parsed as the online snapshot");
        assert!(port.take_commit_authority().is_none(), "the same sign-in can hand over authority only once");
        port.shutdown();
    }

    #[test]
    fn session_end_clears_the_apps_commit_authority() {
        for action in [Some(Action::SignOut), Some(Action::SwitchAccount), Some(Action::SignIn), None] {
            let port = verified_sign_in_port();
            // The runtime and installed service share this clock. The handover gives the app
            // a separate handle to the same clock, and teardown drops the service and that handle.
            let runtime_clock = port.runtime.clock.clone().unwrap();
            let prepared = Arc::strong_count(&runtime_clock);
            let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
            app.account = State::with_port(Box::new(port));
            assert!(poll(&mut app), "the completed sign-in hands authority to the app");
            assert_eq!(Arc::strong_count(&runtime_clock), prepared + 1, "the app holds the service and its clock");
            assert!(!poll(&mut app), "the next frame must not reinstall authority");
            if let Some(action) = action {
                app.account.request(action);
                assert!(!poll(&mut app), "{action:?} must not reinstall authority");
            } else {
                app.end_controlled_session(); // The desktop exit hook closes authority first.
            }
            assert_eq!(Arc::strong_count(&runtime_clock), prepared - 1, "{action:?} must drop both app handles");
            assert!(app.account.take_commit_authority().is_none(), "{action:?} must leave nothing to hand over");
            app.account.shutdown();
        }
    }

    #[test]
    fn a_pending_restart_check_installs_no_commit_authority() {
        let (gate_tx, gate_rx) = channel();
        let product = ScriptedProduct::scripted(vec![Ok(WireResponse::new(200, status_body_for("acct_desktop_0001").to_string().into_bytes()))]);
        let gated = GatedStatusProduct::hold_first_status(product.clone(), gate_rx);
        let device_factory: DeviceTransportFactory = Arc::new(|| Box::new(Arc::new(ScriptedDevice::default())));
        let product_factory: ProductTransportFactory = {
            let gated = gated.clone();
            Arc::new(move || Box::new(Arc::new(gated.clone())))
        };
        let opener = RecordingOpener::default();
        let runtime = AccountRuntime::with_session(active_session()).with_sign_in_harness(
            account_config(),
            device_factory,
            product_factory,
            Arc::new(VirtualTiming::default()),
            opener.opener(),
        );
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);

        wait_for_counter(&gated.entered, 1);
        let _ = port.poll();
        assert!(port.runtime.service.is_none(), "an unfinished online check installs no account service");
        assert!(port.take_commit_authority().is_none(), "a pending check yields no authority");

        gate_tx.send(()).unwrap();
        wait_for_restart(&port);
        let _ = port.poll();
        assert!(port.runtime.service.is_some(), "the completed check installs the account service");
        assert!(port.take_commit_authority().is_some(), "the verified restart yields authority");
        let _ = port.poll();
        assert!(port.take_commit_authority().is_none(), "polling must not replay the same restart proof");
        port.shutdown();
    }

    #[test]
    fn an_unproven_or_mismatched_scope_never_installs_commit_authority() {
        let mut port = verified_sign_in_port();
        let other =
            AuthenticatedHostScope::new(AccountId::new("acct_other"), InstallationId::new("inst_other"), HostSessionBinding::new("binding_other"));
        let unprovable = AuthenticatedHostScope::new(AccountId::new(""), InstallationId::new(""), HostSessionBinding::new(""));
        assert!(!unprovable.is_provable());
        assert!(!port.runtime.install_verified_service_for(&other), "a mismatched scope installs nothing");
        assert!(!port.runtime.install_verified_service_for(&unprovable), "an unprovable scope installs nothing");
        assert!(port.runtime.service.is_some(), "a refused scope does not disturb the proved service");

        port.restored = Some(other);
        assert!(port.take_commit_authority().is_none(), "a conflicting restored scope yields no authority");
        port.restored = None;
        assert!(port.take_commit_authority().is_some(), "the scope the runtime proved still yields authority");
        port.shutdown();
    }

    #[test]
    fn the_app_installs_restart_authority_but_paid_entries_still_need_bounds() {
        let runtime = verified_restart_runtime(active_session());
        let startup = runtime.startup();
        let mut app = solidcraft_ui_egui::SolveApp::new(solidcraft_engine::Session::default(), solidcraft_ui_egui::Services::default());
        install(&mut app, runtime, startup);
        assert!(app.account.is_installed(), "the desktop port is installed");
        wait_until("the background restart authority", || poll(&mut app));
        assert!(app.account.view().signed_in, "the online-verified restart is shown");
        assert!(!poll(&mut app), "the next frame cannot reinstall the same authority");

        let operation = OperationId::parse("op_bound_fixture").unwrap();
        let review = solidcraft_ui_egui::dfm_review::start_with_operation(&mut app, operation).unwrap_err();
        assert_eq!(review, "UNSAFE_TO_START: this Review path has no verified safe-stop bound");
        let (mut control, _) = solidcraft_ui_egui::ControlRequest::trusted_control("engine.execute", json!({"command": "document.inspect"}));
        let control::Outcome::Done(reply) = control::handle(&mut app, &egui::Context::default(), &mut control) else {
            panic!("expected an immediate refusal");
        };
        assert_eq!(reply["errorDetails"]["code"], "UnsafeToStart");
        assert_eq!(reply["error"], "this controlled entry has no verified safe-stop bound");

        let (gui, _) = solidcraft_ui_egui::ControlRequest::trusted_gui("engine.execute", json!({"command": "document.inspect"}));
        assert!(matches!(control::admit_control_request(&gui, &BudgetTable::unverified_default(), &mut NoBilling), Admission::Allowed));
        app.end_controlled_session();
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

    #[derive(Clone, Default)]
    struct SequenceDeviceTransport {
        responses: Arc<Mutex<VecDeque<Result<(u16, Vec<u8>), CredentialError>>>>,
    }

    impl SequenceDeviceTransport {
        fn with_responses(polls: Vec<(u16, Value)>) -> Self {
            let mut responses = VecDeque::new();
            responses.push_back(Ok((
                200,
                json!({
                    "device_code": "device_code_SECRET_000000000000000000000000",
                    "user_code": "BCDF-GHJK",
                    "verification_uri": "https://auth.example/device",
                    "expires_in": 300,
                    "interval": 1
                })
                .to_string()
                .into_bytes(),
            )));
            for (status, poll) in polls {
                responses.push_back(Ok((status, poll.to_string().into_bytes())));
            }
            Self { responses: Arc::new(Mutex::new(responses)) }
        }
    }

    impl DeviceCodeTransport for SequenceDeviceTransport {
        fn post(&self, _url: &str, _content_type: &str, _body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError> {
            self.responses.lock().map_err(|_| store_error())?.pop_front().unwrap_or_else(|| Err(store_error()))
        }
    }

    #[derive(Clone, Default)]
    struct ScriptedExchange {
        calls: Arc<AtomicUsize>,
        reject_first: bool,
    }

    impl ScriptedExchange {
        fn accepting() -> Self {
            Self { calls: Arc::new(AtomicUsize::new(0)), reject_first: false }
        }

        fn rejecting_once() -> Self {
            Self { calls: Arc::new(AtomicUsize::new(0)), reject_first: true }
        }
    }

    impl ProductSessionExchange for ScriptedExchange {
        fn exchange(&self, _provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.reject_first && call == 0 {
                return Err(ProductExchangeError::Rejected);
            }
            Ok(ProductSession::from_trusted_exchange("acct_desktop0001", Secret::new("product_session_test_0000000000"), None))
        }
    }

    #[derive(Clone, Default)]
    struct RecordingBrowser {
        opened: Arc<Mutex<Vec<String>>>,
    }

    impl BrowserLauncher for RecordingBrowser {
        fn open(&self, url: &str, allowed_origin: &str) -> Result<(), Refusal> {
            if !trusted_browser_url(url, allowed_origin) {
                return Err(account_unavailable("the test browser refused a foreign origin"));
            }
            self.opened.lock().map_err(|_| account_unavailable("browser test lock"))?.push(url.to_owned());
            Ok(())
        }
    }

    fn test_config() -> DesktopAccountConfig {
        DesktopAccountConfig::new("client_test", "https://product.example", "https://auth.example").expect("test configuration")
    }

    fn foreign_config() -> DesktopAccountConfig {
        DesktopAccountConfig::new("client_test", "https://product.example", "https://auth.allowed.example").expect("test configuration")
    }

    fn config_port(config: DesktopAccountConfig, browser: RecordingBrowser, runner: LoginRunner, store: &TestStore) -> DesktopAccountPort {
        let runtime = AccountRuntime::with_session_config(CredentialSession::new(Box::new(store.clone())), config);
        let startup = runtime.startup();
        DesktopAccountPort::new(runtime, &startup).with_seams(Arc::new(browser), runner)
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
                    run_login(task, transport, &exchange, &no_wait);
                })
                .map_err(|_| account_unavailable("the test sign-in worker could not start"))
        });
        (runner, attempts)
    }

    fn no_wait(_duration: Duration, _deadline: Instant, _canceled: &dyn Fn() -> bool) -> bool {
        true
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
                    run_login(task, transport, &exchange, &no_wait);
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
        let exchange = ScriptedExchange::accepting();
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
        let exchange = ScriptedExchange::accepting();
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
        let exchange = ScriptedExchange::accepting();
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
        AccountPort::cancel_sign_in(&mut port).unwrap();
        gate_tx.send(()).unwrap();
        port.shutdown();

        assert_eq!(exchange.calls.load(Ordering::SeqCst), 0, "a canceled attempt never reached verification");
        assert!(!port.runtime.credentials.product_session_active());
        assert_eq!(port.runtime.credentials.provider_account(), None);
        assert!(!port.view().signed_in);
    }

    #[test]
    fn cancel_after_commit_is_refused_and_sign_out_remains_the_lifecycle_transition() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
        let (runner, _) = test_runner(ScriptedDeviceTransport::default(), exchange);
        let mut port = login_port(browser, runner, &store);
        port.sign_in().unwrap();
        wait_for_login(&mut port, true);
        let error = AccountPort::cancel_sign_in(&mut port).unwrap_err();
        assert_eq!(error.code, RefusalCode::AuthUnavailable, "a completed sign-in was reported as canceled");
        assert!(port.runtime.credentials.product_session_active(), "the committed session is not deleted");
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        assert!(!port.runtime.credentials.product_session_active());
        assert!(!port.view().signed_in);
    }

    fn authorized_token() -> Value {
        json!({"user": {"id": "user_0001"}, "access_token": "access_token_SECRET_000000000000000000000000"})
    }

    fn direct_task(config: &DesktopAccountConfig, store: &TestStore, attempt: u64) -> LoginTask {
        LoginTask {
            attempt,
            client: config.client.clone(),
            connection: config.product.clone(),
            verification_origin: config.verification_origin.clone(),
            credentials: CredentialSession::new(Box::new(store.clone())),
            status: Arc::new(Mutex::new(LoginStatus { attempt, phase: LoginPhase::Starting, prompt: None, account: None })),
            persist_gate: Arc::new(Mutex::new(())),
            bootstrap_transport: None,
        }
    }

    #[test]
    fn provider_slow_down_interval_is_honoured_and_cancellation_is_interruptible() {
        let store = TestStore::default();
        let config = test_config();
        let task = direct_task(&config, &store, 1);
        let status = Arc::clone(&task.status);
        let exchange = ScriptedExchange::accepting();
        let waits = Arc::new(Mutex::new(Vec::<Duration>::new()));
        let logged = Arc::clone(&waits);
        let transport = SequenceDeviceTransport::with_responses(vec![
            (400, json!({"error": "authorization_pending"})),
            (400, json!({"error": "slow_down"})),
            (200, authorized_token()),
        ]);
        run_login(task, transport, &exchange, &move |duration, _deadline, _canceled| {
            logged.lock().unwrap().push(duration);
            true
        });
        let waits = waits.lock().unwrap().clone();
        assert!(waits.iter().all(|duration| *duration >= Duration::from_secs(1)), "polled earlier than the provider interval: {waits:?}");
        assert!(waits.iter().any(|duration| *duration >= Duration::from_secs(6)), "slow_down was not honoured: {waits:?}");
        assert!(status.lock().unwrap().phase == LoginPhase::Done);
        assert!(store.entries.lock().unwrap().contains_key("auth/product_session"));
    }

    #[test]
    fn foreign_verification_origin_is_never_published_or_launched() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
        let (runner, _) = test_runner(ScriptedDeviceTransport::default(), exchange);
        let mut port = config_port(foreign_config(), browser.clone(), runner, &store);
        port.sign_in().unwrap();
        wait_for_login(&mut port, false);
        assert_eq!(port.view().sign_in_state, SignInState::Failed);
        assert!(port.view().sign_in_prompt.is_none(), "a foreign provider origin reached the UI");
        assert!(browser.opened.lock().unwrap().is_empty());
        assert_eq!(port.open_sign_in_browser().unwrap_err().code, RefusalCode::AuthRequired);
    }

    #[test]
    fn rejected_product_verification_can_be_retried_explicitly() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::rejecting_once();
        let (runner, attempts) = sequence_runner(vec![ScriptedDeviceTransport::default(), ScriptedDeviceTransport::default()], exchange.clone());
        let mut port = login_port(browser, runner, &store);
        port.sign_in().unwrap();
        wait_for_login(&mut port, false);
        assert_eq!(port.view().sign_in_state, SignInState::Failed);
        assert!(!port.runtime.credentials.product_session_active());

        port.sign_in().unwrap();
        wait_for_login(&mut port, true);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(exchange.calls.load(Ordering::SeqCst), 2);
        assert!(port.runtime.credentials.product_session_active());
    }

    macro_rules! joining_runner {
        ($attempts:expr, $sleep:expr) => {{
            let counter = Arc::clone($attempts);
            let runner: LoginRunner = Arc::new(move |task| {
                counter.fetch_add(1, Ordering::SeqCst);
                std::thread::Builder::new()
                    .name("intcad-account-login-joining-test".into())
                    .spawn(move || {
                        let _task = task;
                        std::thread::sleep($sleep);
                    })
                    .map_err(|_| account_unavailable("the test sign-in worker could not start"))
            });
            runner
        }};
    }

    #[test]
    fn spawn_failure_resets_starting_and_refuses() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let runner: LoginRunner = Arc::new(|_| Err(account_unavailable("spawn refused")));
        let mut port = config_port(test_config(), browser, runner, &store);
        assert!(port.sign_in().is_err());
        assert_eq!(port.view().sign_in_state, SignInState::Idle);
        assert!(!port.view().signed_in);
    }

    #[test]
    fn cancel_and_retry_do_not_join_a_live_worker_or_wait_for_persistence() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let attempts = Arc::new(AtomicUsize::new(0));
        let runner = joining_runner!(&attempts, Duration::from_millis(300));
        let mut port = config_port(test_config(), browser, runner, &store);
        port.sign_in().unwrap();
        let started = Instant::now();
        AccountPort::cancel_sign_in(&mut port).unwrap();
        assert!(started.elapsed() < Duration::from_millis(100), "cancel blocked on a live worker");
        let started = Instant::now();
        port.sign_in().unwrap();
        assert!(started.elapsed() < Duration::from_millis(100), "retry joined a live worker");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        port.shutdown();
    }

    #[test]
    fn a_terminal_snapshot_is_consumed_once_and_does_not_resurrect_a_later_view() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
        let (runner, _) = test_runner(ScriptedDeviceTransport::default(), exchange);
        let mut port = login_port(browser, runner, &store);
        port.sign_in().unwrap();
        wait_for_login(&mut port, true);
        assert!(AccountPort::poll(&mut port).is_none(), "the success snapshot was returned twice");
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        let confirmed = port.poll().expect("cleanup confirmation is a separate terminal update");
        assert!(!confirmed.sign_out_pending);
        assert!(!confirmed.signed_in, "a stale success resurrected the signed-out view");
        assert!(port.poll().is_none(), "terminal snapshots are not replayed");
    }

    #[test]
    fn readable_tombstone_requires_cleanup_before_sign_in_and_product_only_does_not_restore() {
        let store = TestStore::default();
        store.entries.lock().unwrap().insert("auth/logout_pending".into(), "sign_out".into());
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
        let (runner, _) = test_runner(ScriptedDeviceTransport::default(), exchange);
        let mut port = login_port(browser, runner, &store);
        assert_eq!(port.logout, StartupLogout::ReadableTombstone, "the startup marker is present");
        assert!(port.sign_in().is_err(), "sign-in cannot replace unconfirmed cleanup");
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        port.poll().unwrap();
        port.sign_in().unwrap();
        wait_for_login(&mut port, true);
        assert_eq!(port.logout, StartupLogout::Clear, "a verified identity clears the tombstone");
        assert!(port.runtime.credentials.product_session_active());
        assert!(port.runtime.credentials.authenticated_scope().is_none(), "identity is not installation proof");
        let restarted = AccountRuntime::with_session(CredentialSession::new(Box::new(store)));
        let startup = restarted.startup();
        assert!(!startup.logout_pending());
        assert!(startup.restored.is_none(), "restart does not restore controlled authority without installation proof");
        assert!(startup.candidate.is_none(), "identity alone is not a restart candidate");
        assert!(!current_view(&startup).signed_in);
    }

    #[test]
    fn trusted_configuration_is_build_time_and_fails_closed() {
        assert!(config_from_build_values(None, Some("https://auth.example"), Some("https://product.example")).is_none());
        assert!(config_from_build_values(Some("client_test"), None, Some("https://product.example")).is_none());
        assert!(config_from_build_values(Some("client_test"), Some("https://auth.example"), None).is_none());
        let valid = config_from_build_values(Some("client_test"), Some("https://auth.example"), Some("https://product.example"));
        assert!(valid.is_some(), "explicit complete build values load");
        assert!(DesktopAccountConfig::new("", "https://product.example", "https://auth.example").is_none());
        assert!(DesktopAccountConfig::new("client_test", "http://product.example", "https://auth.example").is_none());
        let wrong_product_path = DesktopAccountConfig::new("client_test", "https://product.example/v1", "https://auth.example");
        assert!(wrong_product_path.is_none(), "a product origin must be bare");
        let wrong_auth = DesktopAccountConfig::new("client_test", "https://product.example", "https://auth.example/path");
        assert!(wrong_auth.is_none(), "an AuthKit origin must be bare");
        assert!(!trusted_browser_url("http://auth.example/device", "https://auth.example"));
        assert!(!trusted_browser_url("https://other.example/device", "https://auth.example"));
        assert!(!trusted_browser_url("https://user@auth.example/device", "https://auth.example"));
    }

    #[test]
    fn cancel_during_blocked_persistence_is_refused_without_waiting_or_claiming_success() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
        let (runner, _) = test_runner(ScriptedDeviceTransport::default(), exchange);
        let (gate_tx, gate_rx) = channel();
        store.block_next_set("login/commit", gate_rx);
        let mut port = login_port(browser, runner, &store);
        port.sign_in().unwrap();
        store.wait_until_set_blocked();
        let started = Instant::now();
        let error = AccountPort::cancel_sign_in(&mut port).unwrap_err();
        assert_eq!(error.code, RefusalCode::AuthUnavailable);
        assert!(started.elapsed() < Duration::from_millis(100), "cancel waited on blocked persistence");
        gate_tx.send(()).unwrap();
        wait_for_login(&mut port, true);
        assert!(port.runtime.credentials.product_session_active(), "the refused cancel did not claim cancellation");
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        assert!(!port.runtime.credentials.product_session_active());
    }

    #[test]
    fn sign_out_during_blocked_persistence_is_refused_then_works_after_commit() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
        let (runner, _) = test_runner(ScriptedDeviceTransport::default(), exchange);
        let (gate_tx, gate_rx) = channel();
        store.block_next_set("login/commit", gate_rx);
        let mut port = login_port(browser, runner, &store);
        port.sign_in().unwrap();
        store.wait_until_set_blocked();
        let started = Instant::now();
        let error = port.sign_out().unwrap_err();
        assert_eq!(error.code, RefusalCode::AuthUnavailable);
        assert!(started.elapsed() < Duration::from_millis(100), "sign-out waited on blocked persistence");
        gate_tx.send(()).unwrap();
        wait_for_login(&mut port, true);
        port.sign_out().unwrap();
        wait_for_cleanup(&port);
        assert!(!port.runtime.credentials.product_session_active(), "the accepted sign-out did not remove the commit");
        assert!(!port.view().signed_in);
    }

    #[test]
    fn signing_out_obsoletes_a_waiting_attempt() {
        let store = TestStore::default();
        let browser = RecordingBrowser::default();
        let exchange = ScriptedExchange::accepting();
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
        wait_until("the refused origin", || matches!(port.login_status.lock().unwrap().phase, LoginPhase::Failed(_)));
        assert!(!port.view().signed_in);

        port.poll();
        port.sign_in().unwrap();
        wait_until("the retried verified sign-in", || port.login_status.lock().unwrap().phase == LoginPhase::Done);
        port.poll();
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
    fn bootstrap_refusal_does_not_publish_the_verified_login() {
        let store = TestStore::default();
        let session = CredentialSession::new(Box::new(store.clone()));
        let device = ScriptedDevice::scripted(vec![(200, device_body()), (200, token_body())]);
        let product = ScriptedProduct::scripted(vec![
            Ok(WireResponse::new(200, status_body().to_string().into_bytes())),
            Ok(WireResponse::new(503, b"{}".to_vec())),
        ]);
        let runtime = harness_runtime(session, device, product.clone(), Arc::new(VirtualTiming::default()), &RecordingOpener::default());
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        port.sign_in().unwrap();
        wait_until("installation refusal", || matches!(port.login_status.lock().unwrap().phase, LoginPhase::Failed(_)));
        port.poll();
        assert_eq!(product.requests().len(), 2);
        assert_eq!(port.view().sign_in_state, SignInState::Failed);
        port.shutdown();
        let reopened = CredentialSession::new(Box::new(store));
        assert!(!reopened.product_session_active());
        assert!(reopened.authenticated_scope().is_none());
    }

    #[test]
    fn desktop_sign_in_without_configuration_is_refused_before_any_request() {
        let runtime = AccountRuntime::with_session(CredentialSession::new(Box::new(TestStore::default())));
        let startup = runtime.startup();
        let mut port = DesktopAccountPort::new(runtime, &startup);
        assert_eq!(port.sign_in().unwrap_err().code, RefusalCode::AuthUnavailable);
        assert!(!port.login_pending(), "no worker starts without all three trusted values");
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

        port.open_sign_in_browser().unwrap();
        assert_eq!(
            opener.urls(),
            vec!["https://auth.example/device?user_code=BCDF-GHJK".to_owned()],
            "one explicit action opens exactly the trusted verification page"
        );

        gate_tx.send(()).unwrap();
        wait_until("the verified sign-in", || port.login_status.lock().unwrap().phase == LoginPhase::Done);
        port.poll();
        assert_eq!(port.view().account_label.as_deref(), Some("acct_product_0001"));
        assert!(port.view().sign_in_error.is_none());
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
        wait_until("the origin refusal", || matches!(port.login_status.lock().unwrap().phase, LoginPhase::Failed(_)));
        assert!(!port.view().signed_in);
        assert!(opener.urls().is_empty(), "an untrusted origin is never opened");
        assert_eq!(port.begin_device_code().unwrap_err().code, RefusalCode::AuthRequired, "a refused flow keeps no prompt");
        assert_eq!(port.open_sign_in_browser().unwrap_err().code, RefusalCode::AuthRequired, "a refused flow opens no browser");
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

        assert!(!port.login_pending(), "the retired job is gone");
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
