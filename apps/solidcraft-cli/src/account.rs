//! Headless account composition. Programmatic calls never start a sign-in or open a browser.
//! The CLI may use the existing product session from the origin-bound OS credential store.
//! A reachable app control port or a request field is never authority for a charged tool.

use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use solidcraft_account_native::{
    ACCOUNT_KEYRING_SERVICE, AccountPaths, CredentialSession, DeviceCodeFlow, DeviceCodeMode, DeviceCodePrompt, IdentityDomain, IdentityLifecycle,
    KeyringStore, MemorySession, NoPersistentStore, SessionBilling, StatusSessionVerifier, SystemIdentityClock, UreqDeviceCodeTransport,
    UreqTransport, WireConnection, WorkOsPublicClient,
};
#[cfg(test)]
use solidcraft_engine::entitlement::BillingPort;
use solidcraft_engine::entitlement::{
    AuthenticatedHostScope, BudgetTable, Capability, EntryCategory, MonotonicClock, MonotonicInstant, OperationId, Refusal, RefusalCode,
    TrustedContext,
};
use solidcraft_engine::io::vfs::PathScope;
use solidcraft_mcp::{Backend, Server, ToolCallContext};
use solidcraft_suspend_clock::SuspendClock;

/// A session that refuses every durable write: for a headless process with no secure store.
pub fn no_persistent_session() -> CredentialSession {
    CredentialSession::new(Box::new(NoPersistentStore))
}

/// The explicit short session: process memory only, gone when the process exits.
#[allow(dead_code)] // used by the explicit login seam below, which has no shipped command yet
pub fn process_memory_session() -> CredentialSession {
    CredentialSession::new(Box::new(MemorySession::default()))
}

/// Ordinary program paths must use this and surface the refusal; they must not begin a flow.
#[allow(dead_code)] // the shipped CLI has no explicit sign-in command yet
pub fn programmatic_login() -> Result<DeviceCodePrompt, Refusal> {
    Err(Refusal::new(RefusalCode::AuthRequired, "an explicit sign-in command is required"))
}

/// Start an explicitly requested device-code sign-in. The caller polls and completes the returned
/// flow; the prompt contains the verification URL and user code, never the device code.
#[allow(dead_code)] // the shipped CLI has no explicit sign-in command yet; programmatic calls never start one
pub fn begin_explicit_device_login(client_id: &str) -> Result<(DeviceCodeFlow<UreqDeviceCodeTransport>, DeviceCodePrompt), Refusal> {
    let client = WorkOsPublicClient::new(client_id).ok_or_else(|| Refusal::new(RefusalCode::AuthRequired, "no WorkOS client id is configured"))?;
    let mut flow = DeviceCodeFlow::new(client, UreqDeviceCodeTransport::new(), process_memory_session(), DeviceCodeMode::Explicit);
    let prompt = flow.start()?;
    Ok((flow, prompt))
}

/// Open the same origin-bound secure store under the same process lock as desktop.
/// No runtime setting or MCP argument controls the destination of a bearer.
pub fn configured_billing() -> Option<(SessionBilling, File)> {
    let connection = WireConnection::product_origin(option_env!("INTCAD_PRODUCT_HTTPS_ORIGIN")?, env!("CARGO_PKG_VERSION")).ok()?;
    configured_billing_with_clock(connection, BootClock::default(), || None)
}

fn configured_billing_with_clock(
    connection: WireConnection,
    clock: BootClock,
    data_root: impl FnOnce() -> Option<PathBuf>,
) -> Option<(SessionBilling, File)> {
    clock.sample()?;
    let injected = data_root();
    let owned;
    let root = match injected.as_deref() {
        Some(root) => root,
        None => {
            owned = AccountPaths::for_current_user()?;
            owned.root()
        }
    };
    let (journal, lock) = AccountPaths::lock_existing(root)?;
    let service = format!("{ACCOUNT_KEYRING_SERVICE} [{}]", connection.base_url());
    let transport = || UreqTransport::with_limits(Duration::from_secs(5), Duration::from_secs(3));
    let billing =
        SessionBilling::new(CredentialSession::new(Box::new(KeyringStore::new(&service))), connection.clone(), Box::new(transport()), journal, clock);
    // Without the build-time public client ID, keep the legacy status-verified
    // product session path; it cannot refresh. A configured but unusable identity
    // must not fall back to that path.
    let billing = if let Some(client_id) = option_env!("INTCAD_WORKOS_PUBLIC_CLIENT_ID") {
        let identity = IdentityDomain::new("desktop", client_id, connection.base_url()).and_then(|domain| {
            IdentityLifecycle::new(
                domain,
                Box::new(KeyringStore::new(service)),
                connection.clone(),
                Box::new(StatusSessionVerifier::new(transport(), connection)),
                Box::new(SystemIdentityClock::new()),
                root.to_owned(),
            )
            .ok()
        });
        match identity {
            Some(mut identity) => {
                identity.set_refresh_preserves_binding(true);
                billing.with_identity(identity)
            }
            None => billing.with_unavailable_identity(),
        }
    } else {
        billing
    };
    Some((billing, lock))
}

/// The native monotonic epoch includes suspend. A missing or contradictory reading expires leases.
#[derive(Default)]
struct BootClock(SuspendClock);

impl BootClock {
    fn sample(&self) -> Option<MonotonicInstant> {
        self.0.read().map(|elapsed| MonotonicInstant::from_millis(0).saturating_add(elapsed))
    }
}

impl MonotonicClock for BootClock {
    fn now(&self) -> MonotonicInstant {
        self.sample().unwrap_or_else(|| MonotonicInstant::from_millis(0).saturating_add(Duration::MAX))
    }

    fn wall_now(&self) -> SystemTime {
        // Retain the existing conservative margin for leases derived from absolute timestamps.
        let now = SystemTime::now();
        now.checked_add(Duration::from_millis(10)).unwrap_or(now)
    }
}

/// Which production MCP backend a call reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpRoute {
    /// An in-process headless session (`--headless`, `--in`). It may settle against the CLI's own
    /// authority when a reviewed billing adapter exists.
    Headless,
    /// The running app's control port (`--connect`, discovery, or the automatic backend that may
    /// switch to it). The external control contract does not forward a host-minted operation yet.
    GuiBridge,
}

/// A **test-only** local authority for the composition wiring.
///
/// It is never installed by the shipped CLI. `billing` is an in-memory double, `budget` a
/// deterministic verified bound and `remaining` a fixed static window. It proves admission and
/// settlement plumbing, not a live account lifecycle; a production authority must revalidate
/// authorization at dispatch and commit instead.
#[cfg(test)]
struct McpAuthority {
    scope: AuthenticatedHostScope,
    billing: Box<dyn BillingPort>,
    budget: BudgetTable,
    remaining: Duration,
}

/// The MCP-side account decision for the process.
///
/// It is built once before `serve_mcp`; a request cannot add, replace or widen it.
pub struct McpAccount {
    live: Option<(SessionBilling, Option<File>)>,
    #[cfg(test)]
    synthetic: Option<McpAuthority>,
}

impl McpAccount {
    /// Read the host's authenticated scope from one credential session.
    ///
    /// All four facts must hold together: a usable secure store, no durable logout marker, an
    /// active product session (never a provider-only login), and a provable account/installation/
    /// session binding. A missing one leaves the scope `None`; the MCP server then keeps its
    /// default refusal. This mirrors the desktop startup gate without creating a second policy.
    pub fn from_session(billing: SessionBilling) -> Self {
        Self::with_live(Some((billing, None)))
    }

    pub fn configured() -> Self {
        Self::with_live(configured_billing().map(|(billing, lock)| (billing, Some(lock))))
    }

    pub fn closed() -> Self {
        Self::with_live(None)
    }

    fn with_live(live: Option<(SessionBilling, Option<File>)>) -> Self {
        Self {
            live: live.filter(|(billing, _)| billing.scope().is_some()),
            #[cfg(test)]
            synthetic: None,
        }
    }

    /// Install a local test-only authority over an in-memory scope.
    ///
    /// This proves the headless composition wiring and refusal paths only. It captures a fixed
    /// window and cannot revalidate a live account lifecycle, so it is `cfg(test)` and is never
    /// a production authorization path. A future live seam must reject static/synthetic scopes,
    /// recheck authority at dispatch/commit and settle under the durable original id.
    #[cfg(test)]
    pub fn authorized(
        scope: AuthenticatedHostScope,
        billing: Box<dyn BillingPort>,
        budget: BudgetTable,
        remaining: Duration,
    ) -> Result<Self, Refusal> {
        if !scope.is_provable() {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the host scope is not provable"));
        }
        if remaining.is_zero() {
            return Err(Refusal::new(RefusalCode::UnsafeToStart, "the host authorization window is empty"));
        }
        Ok(Self { live: None, synthetic: Some(McpAuthority { scope, billing, budget, remaining }) })
    }
}

/// Compose the MCP server from the host route and account decision.
///
/// Free helpers and protocol reads keep working through every route because the server's free
/// catalog answers them without an operation. A GUI-bridged charged call is refused before
/// dispatch: a reachable app is transport, not proven authority.
pub fn compose_mcp_server(
    backend: Box<dyn Backend>,
    path_scope: Option<PathScope>,
    account: McpAccount,
    route: McpRoute,
    budget: BudgetTable,
) -> Server {
    // Transport is not authority. The app's control port may be local and token-bearing,
    // but the CLI has no reviewed authenticated protocol that proves the app may settle a
    // charged call. Keep the pre-dispatch refusal.
    if route == McpRoute::GuiBridge {
        return Server::new(backend).with_path_scope(path_scope);
    }
    if let Some((billing, lock)) = account.live
        && let Some(scope) = billing.scope()
    {
        let backend = HostScopedBackend { inner: backend, scope, _keyring_lock: lock, remaining: None };
        return Server::new(Box::new(backend)).with_path_scope(path_scope).with_billing(Box::new(billing), budget);
    }
    #[cfg(test)]
    if let Some(authority) = account.synthetic {
        let backend = HostScopedBackend { inner: backend, scope: authority.scope, _keyring_lock: None, remaining: Some(authority.remaining) };
        return Server::new(Box::new(backend)).with_path_scope(path_scope).with_billing(authority.billing, authority.budget);
    }
    Server::new(backend).with_path_scope(path_scope)
}

/// One in-process MCP backend that mints a host operation per charged tool call.
///
/// The server calls [`Backend::tool_context`] once before dispatch. The operation, capability,
/// entry and account/session binding come from the injected host scope; tool arguments are only
/// data. A free helper returns `None`, so the server never bills it.
struct HostScopedBackend {
    inner: Box<dyn Backend>,
    scope: AuthenticatedHostScope,
    remaining: Option<Duration>,
    _keyring_lock: Option<File>,
}

impl Backend for HostScopedBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        self.inner.call(method, params)
    }

    fn call_guarded(&mut self, method: &str, params: serde_json::Value) -> Result<solidcraft_mcp::GuardedReply, solidcraft_mcp::GuardedCallError> {
        self.inner.call_guarded(method, params)
    }

    fn has_ui(&self) -> bool {
        self.inner.has_ui()
    }

    fn describe(&self) -> String {
        self.inner.describe()
    }

    fn design_path(&self) -> Option<String> {
        self.inner.design_path()
    }

    fn in_process(&self) -> bool {
        self.inner.in_process()
    }

    fn notice(&self) -> Option<String> {
        self.inner.notice()
    }

    fn param_check(&self) -> solidcraft_engine::schema::ParamCheck {
        self.inner.param_check()
    }

    fn tool_context(&self, tool: &str) -> Option<ToolCallContext> {
        if solidcraft_mcp::tool_is_free(tool) {
            return None;
        }
        let mut entropy = [0_u8; 16];
        getrandom::fill(&mut entropy).ok()?;
        let operation = OperationId::mint_from_entropy(&entropy).ok()?;
        let trusted = TrustedContext::mint(&self.scope, operation.as_str(), Capability::Mcp, EntryCategory::HeadlessMcp).ok()?;
        let context = ToolCallContext::from_trusted(trusted);
        Some(match self.remaining {
            Some(remaining) => context.with_safe_stop(remaining),
            None => context,
        })
    }

    fn begin_tool_call(&mut self, context: Option<&ToolCallContext>, helper: Option<&str>) {
        self.inner.begin_tool_call(context, helper);
    }

    fn end_tool_call(&mut self) {
        self.inner.end_tool_call();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};
    use solidcraft_account_native::{
        CredentialHandoff, ProductExchangeError, ProductSession, ProductSessionExchange, ProviderSession, Secret, Transport, TransportFailure,
        WireRequest, WireResponse,
    };
    use solidcraft_engine::entitlement::{
        AccountId, BillingResult, CredentialError, CredentialStore, Grant, GrantId, HostSessionBinding, InstallationId, MonotonicInstant,
    };
    use solidcraft_mcp::Headless;

    use super::*;
    use std::path::PathBuf;

    #[test]
    fn unavailable_boot_clock_leaves_no_live_deadline_or_configured_billing() {
        use solidcraft_engine::entitlement::ConservativeDeadline;

        let clock = BootClock(SuspendClock::new(|| None));
        let now = clock.now();
        assert_eq!(now.as_duration(), Duration::MAX);
        assert!(ConservativeDeadline::new(MonotonicInstant::from_millis(0), Duration::MAX).remaining(now).is_none());
        assert!(ConservativeDeadline::new(now, Duration::from_secs(30)).remaining(now).is_none());

        let connection = WireConnection::product_origin("https://product.invalid", "test").unwrap();
        let clock = BootClock(SuspendClock::new(|| None));
        assert!(configured_billing_with_clock(connection, clock, || panic!("must refuse before looking up the data root")).is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    #[test]
    fn configured_billing_accepts_the_native_clock_before_checking_credentials() {
        let connection = WireConnection::product_origin("https://product.invalid", "test").unwrap();
        let attempted_data_lookup = Cell::new(false);
        let result = configured_billing_with_clock(connection, BootClock::default(), || {
            attempted_data_lookup.set(true);
            None // Stop before touching the filesystem or credentials.
        });
        assert!(attempted_data_lookup.get(), "a working native clock must pass the billing clock gate");
        assert!(result.is_none(), "a clock alone cannot authorize a charge");
    }

    #[test]
    fn configured_origin_uses_the_desktop_keyring_namespace() {
        let wire = WireConnection::product_origin("https://Product.Example:00443/", "1").unwrap();
        assert_eq!(wire.base_url(), "https://product.example");
        for invalid in ["https://product.example:0", "https://product.example:/", "https://product.example/.", "https://pr%6fduct.example"] {
            assert!(WireConnection::product_origin(invalid, "1").is_err(), "{invalid}");
        }
    }

    #[test]
    fn runtime_data_directory_cannot_select_another_keyring_lock() {
        // Use a child to inject environment values without mutating this test process's env.
        const CHILD: &str = "INTCAD_TEST_ACCOUNT_PATH_CHILD";
        let root = AccountPaths::for_current_user();
        if let Some(expected) = std::env::var_os(CHILD) {
            assert_eq!(root.as_ref().map(|paths| paths.root().as_os_str()).unwrap_or_default(), expected);
            return;
        }
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        assert!(root.is_some(), "the test user has an OS account directory");
        let other = std::env::temp_dir().join("intcad-test-untrusted-account-location");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "account::tests::runtime_data_directory_cannot_select_another_keyring_lock"])
            .env(CHILD, root.as_ref().map(|paths| paths.root().as_os_str()).unwrap_or_default())
            .env("HOME", &other)
            .env("XDG_DATA_HOME", &other)
            .env("APPDATA", &other)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[test]
    fn cli_requires_both_desktop_lock_and_ledger() {
        let root = std::env::temp_dir().join(format!("intcad-existing-store-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        assert!(AccountPaths::lock_existing(&root).is_none());
        let ledger = root.join("account");
        let lock_path = root.join("account-keyring.lock");
        assert!(!ledger.exists() && !lock_path.exists());
        std::fs::create_dir(&ledger).unwrap();
        assert!(AccountPaths::lock_existing(&root).is_none());
        assert!(!lock_path.exists());
        File::create(&lock_path).unwrap();
        let (opened, lock) = AccountPaths::lock_existing(&root).unwrap();
        assert_eq!(opened, ledger.canonicalize().unwrap());
        assert!(AccountPaths::lock_existing(&root).is_none(), "the CLI must acquire the shared lock");
        drop(lock);
        std::fs::remove_file(lock_path).unwrap();
        std::fs::remove_dir(ledger).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn programmatic_login_never_starts_a_flow() {
        assert_eq!(programmatic_login().unwrap_err().code, RefusalCode::AuthRequired);
    }

    #[test]
    fn explicit_login_without_configuration_is_refused_before_any_request() {
        assert_eq!(begin_explicit_device_login("").unwrap_err().code, RefusalCode::AuthRequired);
    }

    /// A process-local store that claims persistence. It never touches the OS keychain and lets
    /// the startup gate be exercised without a credential or a live service.
    #[derive(Clone, Default)]
    struct PersistentMemoryStore {
        entries: Arc<Mutex<BTreeMap<String, String>>>,
    }

    impl PersistentMemoryStore {
        fn set_raw(&self, key: &str, value: &str) {
            self.entries.lock().expect("test store lock").insert(key.to_owned(), value.to_owned());
        }
    }

    impl CredentialStore for PersistentMemoryStore {
        fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
            Ok(self
                .entries
                .lock()
                .map_err(|_| CredentialError {
                    code: solidcraft_engine::entitlement::CredentialErrorCode::Unavailable,
                    message: "test store lock".into(),
                })?
                .get(key)
                .cloned())
        }

        fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
            self.entries
                .lock()
                .map_err(|_| CredentialError {
                    code: solidcraft_engine::entitlement::CredentialErrorCode::Unavailable,
                    message: "test store lock".into(),
                })?
                .insert(key.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
            self.entries
                .lock()
                .map_err(|_| CredentialError {
                    code: solidcraft_engine::entitlement::CredentialErrorCode::Unavailable,
                    message: "test store lock".into(),
                })?
                .remove(key);
            Ok(())
        }

        fn persistent(&self) -> bool {
            true
        }
    }

    /// The exchange is a local test double only. It never reaches a provider or the network.
    struct TestExchange;

    impl ProductSessionExchange for TestExchange {
        fn exchange(&self, provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
            Ok(ProductSession::from_trusted_exchange(provider.account_id(), Secret::new("product_session_test_only"), None))
        }
    }

    fn test_scope() -> AuthenticatedHostScope {
        AuthenticatedHostScope::new(
            AccountId::new("test-account"),
            InstallationId::new("test-installation"),
            HostSessionBinding::new("test-session-binding"),
        )
    }

    fn active_product_session() -> CredentialSession {
        let store = PersistentMemoryStore::default();
        product_session_in(store)
    }

    fn product_session_in(store: PersistentMemoryStore) -> CredentialSession {
        let mut session = CredentialSession::new(Box::new(store));
        session.complete_provider_login("acct_cli", "provider_token_test_only").expect("test provider login");
        session.activate_product_session(&TestExchange).expect("test product exchange");
        CredentialHandoff::installation_credential(&mut session, "install_cli", Secret::new("installation_test_only"))
            .expect("test installation binding");
        session
    }

    struct Offline;

    impl Transport for Offline {
        fn send(&self, _: &WireRequest) -> Result<WireResponse, TransportFailure> {
            panic!("local account probes must not send a request");
        }
    }

    fn local_account(session: CredentialSession) -> McpAccount {
        McpAccount::from_session(SessionBilling::new(
            session,
            WireConnection::new("https://product.invalid", "test").unwrap(),
            Box::new(Offline),
            PathBuf::new(),
            BootClock::default(),
        ))
    }

    #[test]
    fn headless_mcp_without_a_secure_store_has_no_host_scope() {
        assert!(local_account(no_persistent_session()).live.is_none(), "a process-local session without secure storage cannot mint a host scope");
    }

    #[test]
    fn headless_mcp_provider_only_login_has_no_host_scope() {
        let session = CredentialSession::new(Box::new(PersistentMemoryStore::default()));
        session.complete_provider_login("acct_cli", "provider_token_test_only").expect("test provider login");
        assert!(local_account(session).live.is_none(), "a WorkOS provider token is not an Intcad product session");
    }

    #[test]
    fn headless_mcp_pending_logout_has_no_host_scope() {
        let store = PersistentMemoryStore::default();
        let session = product_session_in(store.clone());
        store.set_raw("auth/logout_pending", "sign_out");
        assert!(local_account(session).live.is_none(), "an unconfirmed sign-out keeps every controlled path closed");
    }

    #[test]
    fn headless_mcp_with_an_active_product_session_exposes_a_provable_host_scope() {
        let account = local_account(active_product_session());
        let scope = account.live.expect("live authority").0.scope().expect("the secure product session yields a scope");
        assert!(scope.is_provable());
        assert_eq!(scope.account().as_str(), "acct_cli");
        assert_eq!(scope.installation().as_str(), "install_cli");
        assert!(!scope.session_binding().as_str().is_empty());
    }

    #[test]
    fn an_empty_or_unprovable_authority_is_never_installed() {
        let (billing, _counts) = counting_billing();
        let unprovable = AuthenticatedHostScope::new(AccountId::new(""), InstallationId::new("install"), HostSessionBinding::new("session"));
        assert_eq!(
            McpAccount::authorized(unprovable, Box::new(billing), verified_mcp_budget(), Duration::from_secs(60)).err().map(|r| r.code),
            Some(RefusalCode::AuthUnavailable)
        );
        let (billing, _counts) = counting_billing();
        assert_eq!(
            McpAccount::authorized(test_scope(), Box::new(billing), verified_mcp_budget(), Duration::ZERO).err().map(|r| r.code),
            Some(RefusalCode::UnsafeToStart)
        );
    }

    #[test]
    fn default_headless_mcp_refuses_a_charged_tool_before_dispatch() {
        let (backend, calls) = counting_backend();
        let account = local_account(no_persistent_session());
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless, BudgetTable::unverified_default());
        let result = call(&mut server, "measure", json!({}));
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE");
        assert_eq!(calls.get(), 0, "no backend dispatch without a host authority");
    }

    #[test]
    fn injected_host_scope_admits_and_settles_one_logical_operation() {
        let (backend, calls) = counting_backend();
        let (billing, counts) = counting_billing();
        let account = McpAccount::authorized(test_scope(), Box::new(billing), verified_mcp_budget(), Duration::from_secs(60))
            .expect("the local host scope is provable");
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless, BudgetTable::unverified_default());
        let result = call(&mut server, "measure", json!({}));
        assert_ne!(result["isError"], true, "{result}");
        assert_eq!(calls.get(), 1, "the charged call reaches the backend once");
        assert_eq!(counts.admits(), 1);
        assert_eq!(counts.commits(), 1);
    }

    #[test]
    fn one_host_scope_mints_a_distinct_operation_for_each_charged_call() {
        let (backend, calls) = counting_backend();
        let (billing, counts) = counting_billing();
        let account = McpAccount::authorized(test_scope(), Box::new(billing), verified_mcp_budget(), Duration::from_secs(60))
            .expect("the local host scope is provable");
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless, BudgetTable::unverified_default());
        for _ in 0..2 {
            let result = call(&mut server, "measure", json!({}));
            assert_ne!(result["isError"], true, "{result}");
        }
        assert_eq!(calls.get(), 2);
        assert_eq!(counts.admits(), 2, "each external tool call is one logical operation");
        assert_eq!(counts.commits(), 2);
    }

    #[test]
    fn injected_host_scope_with_an_unverified_budget_refuses_before_dispatch() {
        let (backend, calls) = counting_backend();
        let (billing, counts) = counting_billing();
        let account = McpAccount::authorized(test_scope(), Box::new(billing), BudgetTable::unverified_default(), Duration::from_secs(60))
            .expect("the scope itself is provable");
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless, BudgetTable::unverified_default());
        let result = call(&mut server, "measure", json!({}));
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "UNSAFE_TO_START");
        assert_eq!(calls.get(), 0);
        assert_eq!(counts.admits(), 0);
    }

    #[test]
    fn live_authority_with_no_measured_budget_refuses_before_network_or_dispatch() {
        let account = local_account(active_product_session());
        assert!(account.live.is_some());
        let (backend, calls) = counting_backend();
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless, BudgetTable::unverified_default());
        let result = call(&mut server, "measure", json!({}));
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "UNSAFE_TO_START");
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn gui_bridged_mcp_without_live_authority_refuses_before_dispatch() {
        let (backend, calls) = counting_backend();
        let (billing, counts) = counting_billing();
        let account = McpAccount::authorized(test_scope(), Box::new(billing), verified_mcp_budget(), Duration::from_secs(60))
            .expect("the local test scope is provable");
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::GuiBridge, BudgetTable::unverified_default());
        let result = call(&mut server, "measure", json!({}));
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE");
        assert_eq!(calls.get(), 0, "transport to a reachable app is not proven authority");
        assert_eq!(counts.admits(), 0, "a GUI bridge call is never charged by the CLI locally");
    }

    fn call(server: &mut Server, name: &str, arguments: Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        });
        let line = server.handle_line(&request.to_string()).expect("a tools/call reply");
        let reply: Value = serde_json::from_str(&line).expect("valid JSON-RPC");
        reply["result"].clone()
    }

    struct CountingBackend {
        headless: Headless,
        calls: Rc<Cell<usize>>,
    }

    impl Backend for CountingBackend {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.calls.set(self.calls.get().saturating_add(1));
            Backend::call(&mut self.headless, method, params)
        }

        fn has_ui(&self) -> bool {
            false
        }

        fn describe(&self) -> String {
            "CLI account composition test backend".into()
        }
    }

    fn counting_backend() -> (CountingBackend, Rc<Cell<usize>>) {
        let calls = Rc::new(Cell::new(0));
        (CountingBackend { headless: Headless::default(), calls: Rc::clone(&calls) }, calls)
    }

    #[derive(Clone, Default)]
    struct CountingBilling {
        admits: Arc<Mutex<usize>>,
        commits: Arc<Mutex<usize>>,
    }

    impl CountingBilling {
        fn admits(&self) -> usize {
            *self.admits.lock().expect("test billing lock")
        }

        fn commits(&self) -> usize {
            *self.commits.lock().expect("test billing lock")
        }
    }

    impl BillingPort for CountingBilling {
        fn admit(&mut self, operation: &OperationId, capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
            let mut admits = self.admits.lock().expect("test billing lock");
            *admits = admits.saturating_add(1);
            Ok(Grant::from_server(GrantId::new(format!("grant_test_{}", operation.as_str())), capability, MonotonicInstant::from_millis(60_000)))
        }

        fn commit(&mut self, _operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
            let mut commits = self.commits.lock().expect("test billing lock");
            *commits = commits.saturating_add(1);
            Ok(result)
        }

        fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
            Ok(())
        }
    }

    fn counting_billing() -> (CountingBilling, CountingBilling) {
        let billing = CountingBilling::default();
        (billing.clone(), billing)
    }

    fn verified_mcp_budget() -> BudgetTable {
        let mut budget = BudgetTable::unverified_default();
        budget.set_verified(Capability::Mcp, EntryCategory::HeadlessMcp, Duration::from_secs(10));
        budget
    }
}
