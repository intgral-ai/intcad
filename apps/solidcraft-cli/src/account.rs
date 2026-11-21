//! Headless account composition.
//!
//! A normal program call never starts a device sign-in: it returns the stable `AUTH_REQUIRED`
//! refusal and no request is sent. Only an explicit command may call
//! [`begin_explicit_device_login`], and even then the caller displays the URL and code; this
//! program never opens a browser. The fallback session is process-local and is never persisted.
//!
//! MCP composition is defensive. The CLI builds one [`McpAccount`] from the same credential
//! session the account commands use, and `serve_mcp` consults it for every backend route.
//! **Transport is not authority:** a reachable app control port, a valid launch token, a TCP
//! connection, a header or a request field never authorizes a charged MCP tool.
//!
//! - Every charged call is denied before dispatch in production. A documented server-verified
//!   product-session exchange exists, but the shipped CLI configures no product service origin
//!   and no explicit provider sign-in command, and there is no reviewed production
//!   [`BillingPort`], durable operation/intent lifecycle or verified MCP safe-stop budget. No
//!   reviewed authenticated protocol proves that a bridged app may settle a charged call. The
//!   shipped CLI passes a non-persistent session, so it has no host scope; `Server::new` keeps
//!   its default refusal.
//! - A GUI-bridged call may use the app for uncharged tool transport, but charged calls remain
//!   closed; the CLI never bills a remote call locally.
//! - `McpAccount::authorized` is a local test-only static-window composition seam. It proves
//!   the backend wiring with an in-memory billing double; it is not a live lifecycle and must
//!   not be used to enable production. Static/synthetic host scopes and test grants never
//!   authorize production.
//!
//! The remaining live interface is explicit. Headless metering still needs a configured product
//! service origin plus an explicit provider sign-in that reaches the reviewed
//! `ServerVerifiedProductSession`, a reviewed [`BillingPort`] over the native account adapter, a
//! durable operation/intent store, a verified MCP safe-stop budget and a live authority
//! revalidation at dispatch/commit (not a fixed window captured at startup). GUI bridging
//! additionally needs a reviewed authenticated protocol that proves the app's authority and
//! carries the app-verified entry, operation and deadline without letting the CLI mint either,
//! plus same-id durable settlement. Until then the shipped CLI stays fail-closed.
//!
//! No token, account label or session binding is read from MCP wire arguments. A host scope
//! candidate, when present, comes only from [`CredentialSession::authenticated_scope`] after the
//! same secure-store, logout-marker and product-session gates the desktop runtime uses; it is
//! never authority by itself.

use std::time::Duration;

use solidcraft_account_native::{
    CredentialSession, DeviceCodeFlow, DeviceCodeMode, DeviceCodePrompt, MemorySession, NoPersistentStore, UreqDeviceCodeTransport,
    WorkOsPublicClient,
};
use solidcraft_engine::entitlement::{
    AuthenticatedHostScope, BillingPort, BudgetTable, Capability, EntryCategory, OperationId, Refusal, RefusalCode, TrustedContext,
};
use solidcraft_engine::io::vfs::PathScope;
use solidcraft_mcp::{Backend, Server, ToolCallContext};

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
pub struct McpAuthority {
    scope: AuthenticatedHostScope,
    billing: Box<dyn BillingPort>,
    budget: BudgetTable,
    remaining: Duration,
}

/// The MCP-side account decision for the process.
///
/// It is built once before `serve_mcp`; a request cannot add, replace or widen it.
pub struct McpAccount {
    scope: Option<AuthenticatedHostScope>,
    authority: Option<McpAuthority>,
}

impl McpAccount {
    /// Read the host's authenticated scope from one credential session.
    ///
    /// All four facts must hold together: a usable secure store, no durable logout marker, an
    /// active product session (never a provider-only login), and a provable account/installation/
    /// session binding. A missing one leaves the scope `None`; the MCP server then keeps its
    /// default refusal. This mirrors the desktop startup gate without creating a second policy.
    pub fn from_session(credentials: CredentialSession) -> Self {
        let secure_store = credentials.is_persistent();
        let logout_pending = credentials.logout_pending().unwrap_or(true);
        let product_session_active = credentials.product_session_active();
        let scope = (secure_store && !logout_pending && product_session_active)
            .then(|| credentials.authenticated_scope())
            .flatten()
            .filter(AuthenticatedHostScope::is_provable);
        Self { scope, authority: None }
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
        Ok(Self { scope: Some(scope.clone()), authority: Some(McpAuthority { scope, billing, budget, remaining }) })
    }
}

/// Compose the MCP server from the host route and account decision.
///
/// Free helpers and protocol reads keep working through every route because the server's free
/// catalog answers them without an operation. A GUI-bridged charged call is refused before
/// dispatch: a reachable app is transport, not proven authority. The headless authority branch
/// is reachable only from the `cfg(test)` local seam; production always closes charged calls.
pub fn compose_mcp_server(backend: Box<dyn Backend>, path_scope: Option<PathScope>, account: McpAccount, route: McpRoute) -> Server {
    let McpAccount { scope, authority } = account;
    match (route, scope, authority) {
        (McpRoute::Headless, Some(scope), Some(authority)) if scope.is_provable() => {
            // The account and authority are built together by `McpAccount::authorized`; the
            // authority's own scope is the one the backend mints from.
            let backend: Box<dyn Backend> =
                Box::new(HostScopedBackend { inner: backend, scope: authority.scope.clone(), remaining: authority.remaining });
            Server::new(backend).with_path_scope(path_scope).with_billing(authority.billing, authority.budget)
        }
        // Transport is not authority. The app's control port may be local and token-bearing,
        // but the CLI has no reviewed authenticated protocol that proves the app may settle a
        // charged call. Keep the pre-dispatch refusal.
        (McpRoute::GuiBridge, _, _) => Server::new(backend).with_path_scope(path_scope),
        // A CLI account scope without a live authority keeps the headless default refusal.
        _ => Server::new(backend).with_path_scope(path_scope),
    }
}

/// One in-process MCP backend that mints a host operation per charged tool call.
///
/// The server calls [`Backend::tool_context`] once before dispatch. The operation, capability,
/// entry and account/session binding come from the injected host scope; tool arguments are only
/// data. A free helper returns `None`, so the server never bills it.
struct HostScopedBackend {
    inner: Box<dyn Backend>,
    scope: AuthenticatedHostScope,
    remaining: Duration,
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
        Some(ToolCallContext::from_trusted(trusted).with_safe_stop(self.remaining))
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
    use solidcraft_account_native::{ProductExchangeError, ProductSession, ProductSessionExchange, ProviderSession, Secret};
    use solidcraft_engine::entitlement::{
        AccountId, BillingResult, CredentialError, CredentialStore, Grant, GrantId, HostSessionBinding, InstallationId, MonotonicInstant,
    };
    use solidcraft_mcp::Headless;

    use super::*;

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
        let session = CredentialSession::new(Box::new(store.clone()));
        session.complete_provider_login("acct_cli", "provider_token_test_only").expect("test provider login");
        session.activate_product_session(&TestExchange).expect("test product exchange");
        session
            .bind_scope(&AuthenticatedHostScope::new(
                AccountId::new("acct_cli"),
                InstallationId::new("install_cli"),
                HostSessionBinding::new("test-binding"),
            ))
            .expect("test installation binding");
        session
    }

    #[test]
    fn headless_mcp_without_a_secure_store_has_no_host_scope() {
        let account = McpAccount::from_session(no_persistent_session());
        assert!(account.scope.is_none(), "a process-local session without secure storage cannot mint a host scope");
    }

    #[test]
    fn headless_mcp_provider_only_login_has_no_host_scope() {
        let session = CredentialSession::new(Box::new(PersistentMemoryStore::default()));
        session.complete_provider_login("acct_cli", "provider_token_test_only").expect("test provider login");
        let account = McpAccount::from_session(session);
        assert!(account.scope.is_none(), "a WorkOS provider token is not an Intcad product session");
    }

    #[test]
    fn headless_mcp_pending_logout_has_no_host_scope() {
        let store = PersistentMemoryStore::default();
        let session = CredentialSession::new(Box::new(store.clone()));
        session.complete_provider_login("acct_cli", "provider_token_test_only").expect("test provider login");
        session.activate_product_session(&TestExchange).expect("test product exchange");
        session
            .bind_scope(&AuthenticatedHostScope::new(
                AccountId::new("acct_cli"),
                InstallationId::new("install_cli"),
                HostSessionBinding::new("test-binding"),
            ))
            .expect("test installation binding");
        store.set_raw("auth/logout_pending", "sign_out");
        let account = McpAccount::from_session(session);
        assert!(account.scope.is_none(), "an unconfirmed sign-out keeps every controlled path closed");
    }

    #[test]
    fn headless_mcp_with_an_active_product_session_exposes_a_provable_host_scope() {
        let account = McpAccount::from_session(active_product_session());
        let scope = account.scope.as_ref().expect("the secure product session yields a scope");
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
        let account = McpAccount::from_session(no_persistent_session());
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless);
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
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless);
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
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless);
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
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless);
        let result = call(&mut server, "measure", json!({}));
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "UNSAFE_TO_START");
        assert_eq!(calls.get(), 0);
        assert_eq!(counts.admits(), 0);
    }

    #[test]
    fn a_provable_scope_alone_never_enables_a_charged_headless_call() {
        let account = McpAccount::from_session(active_product_session());
        assert!(account.scope.is_some(), "the secure product session yields the host scope candidate");
        let (backend, calls) = counting_backend();
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::Headless);
        let result = call(&mut server, "measure", json!({}));
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE");
        assert_eq!(calls.get(), 0, "a host scope is not authority without a reviewed live settlement path");
    }

    #[test]
    fn gui_bridged_mcp_without_live_authority_refuses_before_dispatch() {
        let (backend, calls) = counting_backend();
        let (billing, counts) = counting_billing();
        let account = McpAccount::authorized(test_scope(), Box::new(billing), verified_mcp_budget(), Duration::from_secs(60))
            .expect("the local test scope is provable");
        let mut server = compose_mcp_server(Box::new(backend), None, account, McpRoute::GuiBridge);
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
