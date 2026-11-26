//! Task 8 trust-boundary evidence: forwarded host provenance, wire refusal and logical-operation
//! counting for the Mastra/ACP bridge lane.
//!
//! All billing doubles here live only in this test module. No production path constructs a grant,
//! and no test sends a real provider request.

#![cfg(unix)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    AccountId, AccountService, Admission, AuthenticatedHostScope, Availability, BillingPort, BillingResult, BudgetTable, Capability,
    ConservativeDeadline, EntryCategory, Grant, GrantId, HostSessionBinding, InstallationId, MonotonicClock, MonotonicInstant, OperationId,
    OperationStatus, Refusal, RefusalCode, UnsettledOperation,
};

use super::host_bridge::{HostBridge, HostProvenance};
use crate::control::{ControlRequest, Reissue, admit_control_request, admit_once};

fn host_scope() -> AuthenticatedHostScope {
    // Synthetic host identity from the public authenticated constructor, inside a test module.
    AuthenticatedHostScope::new(
        AccountId::new("test-account"),
        InstallationId::new("test-installation"),
        HostSessionBinding::new("test-session-binding"),
    )
}

fn provenance() -> HostProvenance {
    HostProvenance::new(host_scope())
}

/// A local billing double at the host seam. It records admissions and never ships.
#[derive(Clone, Default)]
struct CountingBilling {
    admits: Arc<AtomicUsize>,
    commits: Arc<AtomicUsize>,
}

impl CountingBilling {
    fn admits(&self) -> usize {
        self.admits.load(Ordering::Relaxed)
    }

    fn commits(&self) -> usize {
        self.commits.load(Ordering::Relaxed)
    }
}

impl BillingPort for CountingBilling {
    fn admit(&mut self, operation: &OperationId, capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
        self.admits.fetch_add(1, Ordering::Relaxed);
        Ok(Grant::from_server(GrantId::new(format!("grant_test_{}", operation.as_str())), capability, MonotonicInstant::from_millis(60_000)))
    }

    fn commit(&mut self, _operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        self.commits.fetch_add(1, Ordering::Relaxed);
        Ok(result)
    }

    fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
        Ok(())
    }
}

fn bridge_budget() -> BudgetTable {
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Mcp, EntryCategory::GuiBridgeMcp, Duration::from_secs(30));
    budget
}

fn authorized_bridge() -> (HostBridge, std::sync::mpsc::Receiver<ControlRequest>, Arc<AtomicBool>) {
    let (tools, rx) = std::sync::mpsc::channel();
    let bridge = HostBridge::open(tools, Some(provenance())).expect("the bridge opens");
    let active = Arc::new(AtomicBool::new(true));
    bridge.begin_turn(Arc::clone(&active));
    (bridge, rx, active)
}

fn write_wire(child: &mut std::os::unix::net::UnixStream, message: &Value) {
    use std::io::Write;
    let mut writer = child.try_clone().expect("clone");
    writer.write_all(message.to_string().as_bytes()).expect("write");
    writer.write_all(b"\n").expect("newline");
    writer.flush().expect("flush");
}

fn read_wire(child: &mut std::os::unix::net::UnixStream) -> Value {
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(child);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read");
    serde_json::from_str(&line).expect("a JSON-RPC reply")
}

fn wire(child: &mut std::os::unix::net::UnixStream, message: Value) -> Value {
    write_wire(child, &message);
    read_wire(child)
}

fn child(bridge: &mut HostBridge) -> std::os::unix::net::UnixStream {
    bridge.take_child().expect("the child end")
}

fn tools_call(id: u64, name: &str, arguments: Value, params_extra: Value) -> Value {
    let mut params = json!({"name": name, "arguments": arguments});
    if let Some(extra) = params_extra.as_object() {
        for (key, value) in extra {
            params[key] = value.clone();
        }
    }
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params})
}

fn reply_to(req: ControlRequest, ok: bool, result: Value) {
    let _ = req.reply.send(if ok { json!({"ok":true,"result":result}) } else { json!({"ok":false,"error":"test refusal"}) });
}

#[test]
fn bridge_without_authenticated_provenance_refuses_charged_tools_before_the_queue() {
    for app in [crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default()), {
        let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        crate::control::test_support::TestAuthority::new("test-account", "test-installation", "").install(&mut app);
        app
    }] {
        let (tools, requests) = std::sync::mpsc::channel();
        let check = app.commit_scope.as_ref().and_then(|scope| app.commit_account.as_ref()?.bridge_scope_check(scope));
        let mut bridge = HostBridge::open_checked(tools, check).expect("the bridge opens without account authority");
        bridge.begin_turn(Arc::new(AtomicBool::new(true)));
        let mut child = child(&mut bridge);
        let reply = wire(&mut child, tools_call(1, "execute", json!({"command":"parameters.list"}), Value::Null));
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert_eq!(reply["result"]["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE", "{reply}");
        assert!(requests.try_recv().is_err(), "a charged call without authority never reaches Control");
    }
}

#[test]
fn external_acp_cannot_forge_trusted_context() {
    let (mut bridge, rx, _active) = authorized_bridge();
    let mut child = child(&mut bridge);
    let reply = wire(
        &mut child,
        tools_call(
            1,
            "execute",
            json!({"command":"solid.box","params":{},"trusted":{"entry":"human_gui_free","operation":"op_forged_0001"}}),
            json!({"entry":"human_gui_free","operationId":"op_forged_0001"}),
        ),
    );
    let result = &reply["result"];
    assert_eq!(result["isError"], true, "{reply}");
    assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "CAPABILITY_DENIED", "{reply}");
    assert_eq!(result["structuredContent"]["errorDetails"]["code"], "OP_REFUSED", "{reply}");
    assert!(rx.try_recv().is_err(), "a forged call never reaches the control queue");
    assert_eq!(bridge.minted_operations(), 0, "a refused call mints no operation");
}

#[test]
fn external_wire_fields_do_not_choose_entry_or_operation() {
    let (mut bridge, rx, _active) = authorized_bridge();
    let mut child = child(&mut bridge);
    let call = tools_call(
        1,
        "execute",
        json!({"command":"engine.commands","params":{}}),
        json!({"entry":"human_gui_free","operationId":"op_forged_0001","account":"attacker"}),
    );
    write_wire(&mut child, &call);
    let req = rx.recv_timeout(Duration::from_secs(5)).expect("the host queue receives the call");
    assert_eq!(req.entry(), EntryCategory::GuiBridgeMcp, "wire fields cannot choose a free entry");
    let operation = req.operation().expect("the host minted the operation");
    assert_ne!(operation.as_str(), "op_forged_0001", "wire fields cannot choose the operation");
    let trusted = req.trusted().expect("the forwarded call carries host provenance");
    assert_eq!(trusted.operation(), operation);
    assert_eq!(trusted.capability(), Capability::Mcp);
    assert_eq!(trusted.entry(), EntryCategory::GuiBridgeMcp);
    reply_to(req, true, json!({}));
    let reply = read_wire(&mut child);
    assert_eq!(reply["result"]["isError"], Value::Null, "{reply}");
    assert_eq!(bridge.minted_operations(), 1);
}

#[test]
fn bridge_preserves_id_across_substeps_within_one_logical_call() {
    let (bridge, _rx, _active) = authorized_bridge();
    let call = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    let first = call.substep("validate");
    let second = call.substep("apply");
    assert_eq!(first.operation(), second.operation());
    assert_eq!(first.trusted(), second.trusted());
    assert!(first.is_host_bound() && second.is_host_bound());
}

#[test]
fn bridge_different_logical_calls_get_distinct_ids() {
    let (bridge, _rx, _active) = authorized_bridge();
    let first = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    let second = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    assert_ne!(first.operation_id(), second.operation_id());
}

#[test]
fn mastra_one_turn_two_intcad_tools_mints_two_operations() {
    let (bridge, _rx, _active) = authorized_bridge();
    let first = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    let second = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    assert_ne!(first.operation_id(), second.operation_id());

    let mut billing = CountingBilling::default();
    let budget = bridge_budget();
    let (one, _one_rx) =
        ControlRequest::trusted_bridge_forwarded("engine.execute", json!({"command":"engine.commands"}), first.trusted().clone(), None, None);
    let (two, _two_rx) =
        ControlRequest::trusted_bridge_forwarded("engine.execute", json!({"command":"engine.commands"}), second.trusted().clone(), None, None);
    assert!(matches!(admit_control_request(&one, &budget, &mut billing), Admission::NewlyAdmitted { .. }));
    assert!(matches!(admit_control_request(&two, &budget, &mut billing), Admission::NewlyAdmitted { .. }));
    assert_eq!(billing.admits(), 2, "two logical tool calls admit twice, not once per turn");
    assert_eq!(billing.commits(), 0, "commit/settlement remains the host commit gate (Task 9)");
}

#[test]
fn forwarded_call_is_admitted_once_across_reissue() {
    let (bridge, _rx, _active) = authorized_bridge();
    let call = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    let billing = CountingBilling::default();
    let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    app.install_billing(Box::new(billing.clone()), bridge_budget());
    app.install_review_store(crate::dfm_review::tests::test_operation_store());
    crate::control::test_support::TestAuthority::from_scope(call.trusted().scope().clone())
        .install_with_bridge_billing(&mut app, Some(Box::new(billing.clone())));
    let (mut req, _rx) =
        ControlRequest::trusted_bridge_forwarded("engine.execute", json!({"command":"engine.commands"}), call.trusted().clone(), None, None);
    let first = admit_once(&mut app, &mut req).expect("the forwarded host call is admitted");
    assert!(matches!(first, Admission::NewlyAdmitted { .. }));
    assert!(matches!(admit_once(&mut app, &mut req), Ok(Admission::NewlyAdmitted { .. })), "the stored admission is reused");
    let (mut reissued, _rx) = crate::control::reissue(Reissue::from_request(&req), "engine.execute", json!({"command":"engine.commands"}));
    assert!(matches!(admit_once(&mut app, &mut reissued), Ok(Admission::NewlyAdmitted { .. })), "a reissued request keeps the first admission");
    assert_eq!(billing.admits(), 1, "one logical call is admitted once, not once per seam");
}

/// Advance at dispatch, not after a wall-clock sleep. The next authorization read is at commit.
struct BridgeResultClock {
    dispatched: Arc<AtomicBool>,
    result_ms: u64,
}

impl MonotonicClock for BridgeResultClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_millis(if self.dispatched.load(Ordering::Relaxed) { self.result_ms } else { 0 })
    }

    fn wall_now(&self) -> std::time::SystemTime {
        std::time::SystemTime::UNIX_EPOCH
    }
}

#[derive(Default)]
struct BridgeAccounts {
    refuse_next_begin: bool,
    begun: Vec<OperationId>,
    finalized: Vec<(OperationId, BillingResult)>,
}

#[derive(Clone)]
struct BridgeAccount(Arc<Mutex<BridgeAccounts>>, solidcraft_engine::entitlement::BridgeJournal);

impl AccountService for BridgeAccount {
    fn bridge_generation_matches(&self, scope: &AuthenticatedHostScope) -> bool {
        scope == &host_scope()
    }

    fn bridge_worker_account(&self) -> Option<Box<dyn AccountService + Send>> {
        Some(Box::new(self.clone()))
    }

    fn bridge_journal(&self) -> Option<solidcraft_engine::entitlement::BridgeJournal> {
        Some(self.1.clone())
    }

    fn begin_covered_bridge(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        required: Duration,
        _deadline: Option<&ConservativeDeadline>,
    ) -> Result<Admission, Refusal> {
        assert_eq!(required, Duration::from_secs(280));
        self.begin(scope, operation, Capability::Mcp, EntryCategory::GuiBridgeMcp)
    }

    fn status(&self) -> Result<Availability, Refusal> {
        Ok(Availability { online: true, version_supported: true, model_available: true, balance: 100, held: 0, reasons: Vec::new() })
    }

    fn authorize(&mut self, _: &AuthenticatedHostScope, _: Capability) -> Result<Grant, Refusal> {
        panic!("standalone authorization is not operation admission");
    }

    fn begin(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<Admission, Refusal> {
        assert_eq!(scope, &host_scope());
        assert_eq!((capability, entry), (Capability::Mcp, EntryCategory::GuiBridgeMcp));
        let mut state = self.0.lock().unwrap();
        if std::mem::take(&mut state.refuse_next_begin) {
            return Ok(Admission::Refused(Refusal::new(RefusalCode::CreditsExhausted, "fixture rejected the hold")));
        }
        if state.begun.contains(operation) {
            return Ok(Admission::AlreadyExists { status: OperationStatus::Held });
        }
        state.begun.push(operation.clone());
        let grant =
            Grant::from_server(GrantId::new(format!("grant_test_{}", operation.as_str())), capability, MonotonicInstant::from_millis(300_000));
        Ok(Admission::NewlyAdmitted { operation: operation.clone(), grant })
    }

    fn finalize(&mut self, scope: &AuthenticatedHostScope, operation: &OperationId, result: BillingResult) -> Result<OperationStatus, Refusal> {
        assert_eq!(scope, &host_scope());
        let mut account = self.0.lock().unwrap();
        assert!(account.begun.contains(operation), "settlement uses the admitted operation");
        account.finalized.push((operation.clone(), result));
        Ok(OperationStatus::Charged)
    }

    fn operation(&self, scope: &AuthenticatedHostScope, operation: &OperationId) -> Result<OperationStatus, Refusal> {
        assert_eq!(scope, &host_scope());
        let account = self.0.lock().unwrap();
        if !account.begun.contains(operation) {
            return Err(Refusal::new(RefusalCode::AuthRequired, "there is no settlement credential before begin"));
        }
        Ok(if account.finalized.iter().any(|(id, _)| id == operation) { OperationStatus::Charged } else { OperationStatus::Held })
    }
}

#[test]
fn a_refused_bridge_begin_can_retry_only_the_original_id() {
    let scope = host_scope();
    let call = provenance().begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").unwrap();
    let operation = call.operation_id();
    let request =
        || ControlRequest::trusted_bridge_forwarded("engine.execute", json!({"command":"parameters.list"}), call.trusted().clone(), None, None);
    let journal = crate::dfm_review::tests::test_bridge_journal();
    let account = Arc::new(Mutex::new(BridgeAccounts { refuse_next_begin: true, ..Default::default() }));
    let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    app.install_commit_authority(
        scope,
        Box::new(BridgeAccount(account.clone(), journal.clone())),
        Box::new(BridgeResultClock { dispatched: Arc::default(), result_ms: 0 }),
    );
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Mcp, EntryCategory::GuiBridgeMcp, Duration::from_secs(280));
    app.install_budget(budget);
    let (req, reply) = request();
    crate::control::queue_bridge_admission(&mut app, req);
    assert_eq!(reply.recv_timeout(Duration::from_secs(5)).unwrap()["errorDetails"]["code"], "CreditsExhausted");
    drop(app.agent_chat.bridge_admission.take());
    assert_eq!(journal().unwrap().load(operation).unwrap().unwrap().status, OperationStatus::Held);
    let (req, reply) = request();
    crate::control::queue_bridge_admission(&mut app, req);
    let req = app.agent_chat.bridge_admission.take().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
    app.run_control(&egui::Context::default(), req);
    assert_eq!(reply.recv_timeout(Duration::from_secs(5)).unwrap()["ok"], true);
    let (req, reply) = request();
    crate::control::queue_bridge_admission(&mut app, req);
    assert_eq!(reply.recv_timeout(Duration::from_secs(5)).unwrap()["errorDetails"]["code"], "OperationFinalized");
    let account = account.lock().unwrap();
    assert_eq!(account.begun.as_slice(), std::slice::from_ref(operation));
    assert_eq!(account.finalized.as_slice(), &[(operation.clone(), BillingResult::Succeeded)]);
}

#[test]
fn bridge_registers_and_settles_the_minted_id_only_before_the_deadline() {
    use crate::control::{Outcome, queue_bridge_admission};
    use solidcraft_engine::entitlement::SettlementIntent;

    for result_ms in [299_999, 300_000, 300_001] {
        let (mut bridge, requests, _active) = authorized_bridge();
        let mut child = child(&mut bridge);
        let args = json!({"command":"solid.box","params":{"length":10,"width":10,"height":10}});
        write_wire(&mut child, &tools_call(1, "execute", args, Value::Null));
        let req = requests.recv_timeout(Duration::from_secs(5)).expect("the bridge forwards the paid call");
        let operation = req.operation().expect("the bridge minted an id").clone();
        let account = Arc::new(Mutex::new(BridgeAccounts::default()));
        let scope = host_scope();
        let journal = crate::dfm_review::tests::test_bridge_journal();
        let worker_journal = {
            let journal = journal.clone();
            let window = std::thread::current().id();
            Arc::new(move || {
                assert_ne!(std::thread::current().id(), window, "the window must not open the bridge journal");
                journal()
            })
        };
        let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        app.install_commit_authority(
            scope.clone(),
            Box::new(BridgeAccount(Arc::clone(&account), worker_journal)),
            Box::new(BridgeResultClock { dispatched: Arc::clone(&req.work_started), result_ms }),
        );
        let mut budget = BudgetTable::unverified_default();
        budget.set_verified(Capability::Mcp, EntryCategory::GuiBridgeMcp, Duration::from_secs(280));
        app.install_budget(budget);

        queue_bridge_admission(&mut app, req);
        let mut req = app.agent_chat.bridge_admission.take().unwrap().recv_timeout(Duration::from_secs(5)).expect("the worker admits the request");
        let admission = req.admission.clone().unwrap();
        assert!(matches!(admission, Admission::NewlyAdmitted { operation: id, .. } if id == operation));
        let store = journal().unwrap();
        let record = store.load(&operation).unwrap().expect("durable operation intent");
        assert_eq!(record.account, *scope.account());
        assert_eq!((record.capability, record.entry, record.status), (Capability::Mcp, EntryCategory::GuiBridgeMcp, OperationStatus::Pending));
        assert_eq!(store.unsettled_operations(Some(&scope)).unwrap(), [UnsettledOperation::Owned(record)]);

        let Outcome::Done(value) = crate::control::handle(&mut app, &egui::Context::default(), &mut req) else { panic!("expected a tool reply") };
        assert!(req.work_started.load(Ordering::Relaxed));
        assert_eq!(app.session.doc.features.len(), 1, "the authorized tool dispatched");
        req.reply.send(value.clone()).unwrap();
        let forwarded = read_wire(&mut child);
        let account = account.lock().unwrap();
        assert_eq!(account.begun.as_slice(), std::slice::from_ref(&operation));
        let store = journal().unwrap();
        assert_eq!(store.settlement_intent(&operation).unwrap(), Some(SettlementIntent::Commit(BillingResult::Succeeded)));
        if result_ms < 300_000 {
            assert_eq!(value["ok"], true, "{value}");
            assert_eq!(forwarded["result"]["isError"], Value::Null, "{forwarded}");
            assert_eq!(account.finalized, [(operation, BillingResult::Succeeded)]);
            assert_eq!(store.load(&account.begun[0]).unwrap().unwrap().status, OperationStatus::Charged);
        } else {
            assert_eq!(value["errorDetails"]["code"], "UnsafeToStart", "{value}");
            assert_eq!(forwarded["result"]["isError"], true, "{forwarded}");
            assert!(account.finalized.is_empty(), "an expired result is neither charged nor released");
            assert_eq!(store.load(&operation).unwrap().unwrap().status, OperationStatus::Pending, "restart owns the unsettled intent");
        }
    }
}

#[test]
fn host_agent_parameter_edit_is_admitted_and_settled_once() {
    use crate::control::{Outcome, queue_bridge_admission};

    const NAME: &str = "int1481_acceptance_marker";
    let (mut bridge, requests, _active) = authorized_bridge();
    let mut child = child(&mut bridge);
    let accounts = Arc::new(Mutex::new(BridgeAccounts::default()));
    let journal = crate::dfm_review::tests::test_bridge_journal();
    let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    app.install_commit_authority(
        host_scope(),
        Box::new(BridgeAccount(Arc::clone(&accounts), journal.clone())),
        Box::new(BridgeResultClock { dispatched: Arc::default(), result_ms: 0 }),
    );
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Mcp, EntryCategory::GuiBridgeMcp, Duration::from_secs(280));
    app.install_budget(budget);

    let before = app.session.document_stamp();
    let undo_before = app.session.undo.len();
    assert!(app.session.doc.param(NAME).is_none());
    write_wire(&mut child, &tools_call(1, "set_parameter", json!({"name": NAME, "expression": "13 mm"}), Value::Null));
    let request = requests.recv_timeout(Duration::from_secs(5)).expect("the tool reaches the host");
    let operation = request.operation().expect("the host minted an operation").clone();
    queue_bridge_admission(&mut app, request);
    let mut admitted = app.agent_chat.bridge_admission.take().unwrap().recv_timeout(Duration::from_secs(5)).expect("host admission");
    let Outcome::Done(reply) = crate::control::handle(&mut app, &egui::Context::default(), &mut admitted) else {
        panic!("test parameter should complete");
    };
    assert_eq!(reply["ok"], true, "{reply}");
    admitted.reply.send(reply).unwrap();
    let tool_reply = read_wire(&mut child);
    assert_ne!(tool_reply["result"]["isError"], true, "{tool_reply}");

    let after = app.session.document_stamp();
    assert_eq!(after.document_id, before.document_id);
    // Revisions are globally allocated identifiers, not per-session counters: a parallel test
    // can consume the next number. One command shows as a changed stamp and one new undo step.
    assert_ne!(after.revision, before.revision);
    assert_eq!(app.session.undo.len(), undo_before + 1, "one undo step");
    let readback = app.session.execute("parameters.list", &json!({"filter":"user"})).expect("independent readback");
    let matches: Vec<_> = readback["parameters"].as_array().unwrap().iter().filter(|param| param["name"] == NAME).collect();
    assert_eq!(matches.len(), 1, "{readback}");
    assert_eq!(matches[0]["expression"], "13 mm");
    assert_eq!(matches[0]["value"], 13.0);
    assert!(requests.try_recv().is_err(), "no second desktop command");
    assert_eq!(bridge.minted_operations(), 1);
    let accounts = accounts.lock().unwrap();
    assert_eq!(accounts.begun, std::slice::from_ref(&operation));
    assert_eq!(accounts.finalized, [(operation.clone(), BillingResult::Succeeded)]);
    assert_eq!(journal().unwrap().load(&operation).unwrap().unwrap().status, OperationStatus::Charged);
}

#[test]
fn packaged_acp_edit_is_admitted_and_committed_once() {
    use super::acp_client::AcpBackend;
    use super::backend::{ChatBackend as _, Event, PromptPart, StartContext};
    use crate::control::Outcome;
    use std::time::Instant;

    // This child speaks the same stdio protocol as the bundled SDK. The SDK itself is
    // covered by agent-smoke; the app, MCP dispatch, CAD command and journal here are real.
    // It sits in a package-shaped folder, so the launcher validation runs for real too.
    let directory = std::env::temp_dir().join(format!("intcad-acp-paid-fixture-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let work = directory.as_path();
    let root = work.join("mastra-agent");
    for folder in ["agent", "node_modules/@mastra/code-sdk", "node_modules/@agentclientprotocol/sdk"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    std::fs::write(root.join("agent.json"), b"{}").unwrap();
    std::fs::write(root.join("node_modules/@mastra/code-sdk/package.json"), b"{}").unwrap();
    std::fs::write(root.join("node_modules/@agentclientprotocol/sdk/package.json"), b"{}").unwrap();
    std::fs::write(root.join("agent/main.mjs"), b"").unwrap();
    std::fs::write(
        root.join("node"),
        r#"#!/bin/sh
        IFS= read -r initialize || exit 1
        printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"agentInfo":{"name":"ACP CAD contract fixture"}}}'
        IFS= read -r session || exit 1
        printf '%s\n' "$session" > session.json
        printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"fixture-session"}}'
        IFS= read -r prompt || exit 1
        printf '%s\n' "$prompt" > prompt.json
        printf '%s\n' '{"jsonrpc":"2.0","id":71,"method":"_intcad/cad/tool","params":{"sessionId":"fixture-session","turn":"3","name":"set_parameter","arguments":{"name":"int1481_acceptance_marker","expression":"13 mm"}}}'
        IFS= read -r reply || exit 1
        printf '%s\n' "$reply" > reply.json
        printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
        while IFS= read -r unused; do :; done
        "#,
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(root.join("node"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let argv = [root.join("node").to_string_lossy().into_owned(), root.join("agent/main.mjs").to_string_lossy().into_owned()];
    let (tools, requests) = std::sync::mpsc::channel();
    let ctx = StartContext { cwd: work.to_owned(), bridge: None, tools: Some(tools), wake: Box::new(|| {}) };
    let mut backend = AcpBackend::start_packaged(&argv, ctx, work).unwrap();
    backend.new_session();
    let ready = Instant::now() + Duration::from_secs(5);
    while !backend.poll().iter().any(|event| matches!(event, Event::Ready { .. })) {
        assert!(Instant::now() < ready, "packaged ACP session did not open");
        std::thread::sleep(Duration::from_millis(2));
    }

    let accounts = Arc::new(Mutex::new(BridgeAccounts::default()));
    let journal = crate::dfm_review::tests::test_bridge_journal();
    let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    app.install_commit_authority(
        host_scope(),
        Box::new(BridgeAccount(Arc::clone(&accounts), journal.clone())),
        Box::new(BridgeResultClock { dispatched: Arc::default(), result_ms: 0 }),
    );
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Mcp, EntryCategory::GuiBridgeMcp, Duration::from_secs(280));
    app.install_budget(budget);

    let before = app.session.document_stamp();
    let undo_before = app.session.undo.len();
    assert!(app.session.doc.param("int1481_acceptance_marker").is_none());
    backend.set_document(&before.document_id, before.revision);
    backend.set_turn_control(Arc::new(AtomicBool::new(true)));
    backend.set_bridge_scope_check(Some(Box::new(|| Some(host_scope()))));
    backend.prompt(&[PromptPart::Text("Set the test parameter".into())]);
    let deadline = Instant::now() + Duration::from_secs(5);
    let request = loop {
        backend.poll();
        if let Ok(request) = requests.try_recv() {
            break request;
        }
        assert!(Instant::now() < deadline, "no call through the desktop queue");
        std::thread::sleep(Duration::from_millis(2));
    };
    let operation = request.operation().expect("the desktop minted an operation").clone();
    assert_eq!(request.params["expectedDocument"], crate::execution_guard::stamp_json(&before));
    crate::control::queue_bridge_admission(&mut app, request);
    let mut admitted = app.agent_chat.bridge_admission.take().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
    let Outcome::Done(result) = crate::control::handle(&mut app, &egui::Context::default(), &mut admitted) else {
        panic!("the CAD edit should complete");
    };
    assert_eq!(result["ok"], true, "{result}");
    admitted.reply.send(result).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !backend.poll().iter().any(|event| matches!(event, Event::TurnEnded { .. })) {
        assert!(Instant::now() < deadline, "the desktop never delivered the MCP reply");
        std::thread::sleep(Duration::from_millis(2));
    }
    backend.shutdown(true);

    let from_child = |name: &str| serde_json::from_str::<Value>(&std::fs::read_to_string(work.join(name)).unwrap()).unwrap();
    let session = from_child("session.json");
    assert_eq!(session["params"]["mcpServers"], json!([]));
    assert!(session["params"]["_meta"]["intcad"]["cadBridge"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == "set_parameter"));
    assert_eq!(from_child("prompt.json")["params"]["_meta"]["intcad"]["cadTurn"], "3");
    let reply = from_child("reply.json");
    assert_eq!(reply["id"], 71);
    assert!(reply["result"].is_object() && reply["result"]["isError"] != true, "{reply}");
    let after = app.session.document_stamp();
    assert_eq!(after.document_id, before.document_id);
    // Revisions are globally allocated identifiers, not per-session counters: a parallel test
    // can consume the next number. One command shows as a changed stamp and one new undo step.
    assert_ne!(after.revision, before.revision);
    assert_eq!(app.session.undo.len(), undo_before + 1, "one undo step");
    let readback = app.session.execute("parameters.list", &json!({"filter":"user"})).unwrap();
    let matches: Vec<_> = readback["parameters"].as_array().unwrap().iter().filter(|param| param["name"] == "int1481_acceptance_marker").collect();
    assert_eq!(matches.len(), 1, "{readback}");
    assert_eq!(matches[0]["value"], 13.0);
    assert_eq!(matches[0]["expression"], "13 mm");
    assert!(requests.try_recv().is_err(), "no second desktop dispatch");
    assert_eq!(journal().unwrap().load(&operation).unwrap().unwrap().status, OperationStatus::Charged);
    let accounts = accounts.lock().unwrap();
    assert_eq!(accounts.begun, std::slice::from_ref(&operation));
    assert_eq!(accounts.finalized, [(operation, BillingResult::Succeeded)]);
    for file in ["session.json", "prompt.json", "reply.json"] {
        std::fs::remove_file(work.join(file)).unwrap();
    }
    std::fs::remove_dir_all(work).unwrap();
}

#[test]
fn acp_worker_does_not_replace_an_explicit_document_precondition() {
    let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    app.session.execute("parameters.add", &json!({"name":"width","expression":"7 mm"})).unwrap();
    let current = app.session.document_stamp();
    let previous = solidcraft_engine::DocumentStamp { document_id: current.document_id.clone(), revision: current.revision - 1 };
    let expected = crate::execution_guard::stamp_json(&previous);
    let arguments = json!({
        "command":"parameters.change",
        "params":{"name":"width","expression":"13 mm"},
        "expectedDocument":expected,
    });
    let (tools, requests) = std::sync::mpsc::channel();
    std::thread::scope(|worker| {
        let reply = worker.spawn(move || {
            super::host_bridge::serve_acp_cad(
                tools,
                Some(Box::new(|| Some(host_scope()))),
                Arc::new(AtomicBool::new(true)),
                current,
                std::env::temp_dir(),
                "execute".into(),
                arguments,
            )
        });
        let mut request = requests.recv_timeout(Duration::from_secs(5)).expect("desktop request");
        let observed = request.params["expectedDocument"].clone();
        let refusal = crate::execution_guard::admit(&mut app, &mut request).err();
        let response = refusal
            .as_ref()
            .map_or_else(|| json!({"ok":false,"error":"test stopped the incorrectly admitted request"}), crate::execution_guard::refusal_json);
        request.reply.send(response).unwrap();
        let (outcome, _effect) = reply.join().unwrap();
        outcome.unwrap();
        assert_eq!(observed, expected, "the desktop must not advance a caller's condition to the current revision");
        assert!(matches!(refusal, Some(crate::execution_guard::GuardRefusal::Stale { .. })));
    });
    assert_eq!(app.session.doc.param("width").unwrap().expr, "7 mm");
}

#[test]
fn mastra_pure_chat_creates_no_billing_operation() {
    let (bridge, rx, _active) = authorized_bridge();
    // A chat turn that uses no Intcad tool sends no tools/call to the bridge.
    assert_eq!(bridge.minted_operations(), 0);
    assert!(rx.try_recv().is_err());
    let billing = CountingBilling::default();
    assert_eq!(billing.admits(), 0);
}

#[test]
fn mastra_own_tool_is_free_intcad_tool_bills() {
    let (mut bridge, rx, _active) = authorized_bridge();
    let mut child = child(&mut bridge);
    let free_call = tools_call(1, "list_commands", json!({"compact":true}), Value::Null);
    write_wire(&mut child, &free_call);
    let free_req = rx.recv_timeout(Duration::from_secs(5)).expect("the free helper reaches the control queue");
    assert!(free_req.trusted().is_none(), "a free helper carries no billed operation context");
    assert_eq!(free_req.helper.as_deref(), Some("list_commands"), "the originating helper is recorded");
    assert!(free_req.operation().is_none(), "a free helper mints no billable operation");
    reply_to(free_req, true, json!({"commands":[]}));
    let free = read_wire(&mut child);
    assert_eq!(free["result"]["isError"], Value::Null, "{free}");
    assert_eq!(bridge.minted_operations(), 0, "a free helper mints no billed operation");

    let charged_call = tools_call(2, "execute", json!({"command":"engine.commands"}), Value::Null);
    write_wire(&mut child, &charged_call);
    let charged_req = rx.recv_timeout(Duration::from_secs(5)).expect("the charged tool reaches the control queue");
    let trusted = charged_req.trusted().expect("the charged call is host-bound");
    assert_eq!(trusted.entry(), EntryCategory::GuiBridgeMcp);
    reply_to(charged_req, true, json!({}));
    let charged = read_wire(&mut child);
    assert_eq!(charged["result"]["isError"], Value::Null, "{charged}");
    assert_eq!(bridge.minted_operations(), 1, "one Intcad tool call mints one billed operation");
}

#[test]
fn free_helpers_use_no_credits_and_only_narrow_host_methods() {
    let (helper, _rx) = ControlRequest::trusted_bridge_helper("engine.commands", json!({}), "list_commands");
    let mut billing = CountingBilling::default();
    assert!(matches!(admit_control_request(&helper, &bridge_budget(), &mut billing), Admission::Allowed));
    assert_eq!(billing.admits(), 0, "a published helper consumes no credits");
    assert!(helper.operation().is_none(), "a published helper mints no operation");

    let (generic, _rx) = ControlRequest::trusted_bridge_helper("engine.execute", json!({"command":"solid.box","params":{"length":10}}), "screenshot");
    let mut billing = CountingBilling::default();
    let admission = admit_control_request(&generic, &bridge_budget(), &mut billing);
    assert!(matches!(admission, Admission::Refused(refusal) if refusal.code == RefusalCode::CapabilityDenied));
    assert_eq!(billing.admits(), 0, "a generic command cannot be made free by the screenshot helper");
}

#[test]
fn forwarded_expectation_substep_is_refused_before_work() {
    let (mut bridge, rx, _active) = authorized_bridge();
    let mut child = child(&mut bridge);
    let call = tools_call(1, "execute", json!({"command":"solid.box","params":{"length":10},"expect":{"bodies":1}}), Value::Null);
    let reply = wire(&mut child, call);
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    assert_eq!(reply["result"]["structuredContent"]["errorDetails"]["reason"], "UNSAFE_TO_START", "{reply}");
    assert!(rx.try_recv().is_err(), "the expectation substep is refused before the control queue");
    assert_eq!(bridge.minted_operations(), 0, "nothing was minted or sent");
}

#[test]
fn unsupported_multi_substep_tool_is_refused_before_work() {
    let (mut bridge, rx, _active) = authorized_bridge();
    let mut child = child(&mut bridge);
    let default_call = tools_call(1, "inspect_design", json!({}), Value::Null);
    let refused = wire(&mut child, default_call);
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert_eq!(refused["result"]["structuredContent"]["errorDetails"]["reason"], "UNSAFE_TO_START", "{refused}");
    assert!(rx.try_recv().is_err(), "the multi-substep call is refused before the control queue");
    assert_eq!(bridge.minted_operations(), 0, "nothing was minted or sent");

    let single_call = tools_call(2, "inspect_design", json!({"sketches":false,"joints":false}), Value::Null);
    write_wire(&mut child, &single_call);
    let request = rx.recv_timeout(Duration::from_secs(5)).expect("the single-read inspection reaches control");
    assert!(request.trusted().is_some(), "the allowed inspection is a charged host call");
    reply_to(request, true, json!({"bodies":[]}));
    let reply = read_wire(&mut child);
    assert_eq!(reply["result"]["isError"], Value::Null, "{reply}");
    assert_eq!(bridge.minted_operations(), 1);
}

#[test]
fn screenshot_camera_and_named_view_helpers_are_exact_and_free() {
    let camera = ControlRequest::trusted_bridge_helper(
        "engine.execute",
        json!({"command":"view.apply","params":{"camera":{"target":[0,0,0],"yaw":0.4,"pitch":0.2,"distance":100}}}),
        "screenshot",
    )
    .0;
    let named = ControlRequest::trusted_bridge_helper("engine.execute", json!({"command":"view.apply","params":{"view":"Photo"}}), "screenshot").0;
    let standard = ControlRequest::trusted_bridge_helper("ui.view", json!({"view":"top"}), "screenshot").0;
    for request in [camera, named, standard] {
        let mut billing = CountingBilling::default();
        assert!(matches!(admit_control_request(&request, &bridge_budget(), &mut billing), Admission::Allowed));
        assert_eq!(billing.admits(), 0, "a screenshot camera/view helper consumes no credits");
        assert!(request.operation().is_none(), "a screenshot camera/view helper mints no operation");
    }
}

#[test]
fn screenshot_helper_never_allows_generic_commands_or_extra_fields() {
    let extra = ControlRequest::trusted_bridge_helper(
        "engine.execute",
        json!({"command":"view.apply","params":{"camera":{"target":[0,0,0],"yaw":0,"pitch":0,"distance":100},"command":"solid.box"}}),
        "screenshot",
    )
    .0;
    let mutation = ControlRequest::trusted_bridge_helper("engine.execute", json!({"command":"solid.box","params":{"length":10}}), "screenshot").0;
    let other_tool =
        ControlRequest::trusted_bridge_helper("engine.execute", json!({"command":"view.apply","params":{"view":"Photo"}}), "list_commands").0;
    for request in [extra, mutation, other_tool] {
        let mut billing = CountingBilling::default();
        let admission = admit_control_request(&request, &bridge_budget(), &mut billing);
        assert!(matches!(admission, Admission::Refused(refusal) if refusal.code == RefusalCode::CapabilityDenied));
        assert_eq!(billing.admits(), 0, "no non-screenshot command is free");
    }
}

#[test]
fn multiple_tool_calls_and_a_resource_read_keep_context_per_request() {
    let (mut bridge, rx, _active) = authorized_bridge();
    let mut child = child(&mut bridge);
    let first_call = tools_call(1, "execute", json!({"command":"engine.commands"}), Value::Null);
    write_wire(&mut child, &first_call);
    let first = rx.recv_timeout(Duration::from_secs(5)).expect("the first charged call");
    let first_operation = first.operation().expect("the first operation").clone();
    assert!(first.trusted().is_some());
    reply_to(first, true, json!({}));
    let first_reply = read_wire(&mut child);
    assert_eq!(first_reply["result"]["isError"], Value::Null, "{first_reply}");

    let second_call = tools_call(2, "execute", json!({"command":"engine.commands"}), Value::Null);
    write_wire(&mut child, &second_call);
    let second = rx.recv_timeout(Duration::from_secs(5)).expect("the second charged call");
    let second_operation = second.operation().expect("the second operation").clone();
    assert_ne!(first_operation, second_operation, "two tool calls are two operations");
    reply_to(second, true, json!({}));
    let second_reply = read_wire(&mut child);
    assert_eq!(second_reply["result"]["isError"], Value::Null, "{second_reply}");

    let resource = json!({"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"solidcraft://design"}});
    write_wire(&mut child, &resource);
    let resource_request = rx.recv_timeout(Duration::from_secs(5)).expect("the resource read reaches the control queue");
    assert_eq!(resource_request.helper.as_deref(), Some("@resource"), "a resource read is a host helper");
    assert!(resource_request.trusted().is_none(), "a resource read carries no tool context");
    assert!(resource_request.operation().is_none(), "a resource read mints no operation");
    reply_to(resource_request, true, json!({"bodies":[]}));
    let resource_reply = read_wire(&mut child);
    assert_eq!(resource_reply["result"]["contents"][0]["uri"], "solidcraft://design", "{resource_reply}");

    let third_call = tools_call(4, "execute", json!({"command":"engine.commands"}), Value::Null);
    write_wire(&mut child, &third_call);
    let third = rx.recv_timeout(Duration::from_secs(5)).expect("the call after the resource read");
    let third_operation = third.operation().expect("the third operation").clone();
    assert_ne!(third_operation, first_operation);
    assert_ne!(third_operation, second_operation);
    reply_to(third, true, json!({}));
    let third_reply = read_wire(&mut child);
    assert_eq!(third_reply["result"]["isError"], Value::Null, "{third_reply}");
    assert_eq!(bridge.minted_operations(), 3, "only the three charged tool calls minted operations");
}

#[test]
fn nested_batch_one_charge() {
    let (bridge, _rx, _active) = authorized_bridge();
    let call = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "batch").expect("host mint");
    let validate = call.substep("validate");
    let apply = call.substep("apply");
    assert_eq!(validate.operation(), apply.operation());
    let (req, _rx) = ControlRequest::trusted_bridge_forwarded("engine.batch", json!({"commands":[]}), call.trusted().clone(), None, None);
    let mut billing = CountingBilling::default();
    assert!(matches!(admit_control_request(&req, &bridge_budget(), &mut billing), Admission::NewlyAdmitted { .. }));
    assert_eq!(billing.admits(), 1, "one batch is one admission regardless of internal substeps");
    assert_eq!(billing.commits(), 0);
}

#[test]
fn expired_forwarded_authority_is_refused_before_dispatch() {
    let expired = ConservativeDeadline::new(MonotonicInstant::from_millis(1_000), Duration::from_secs(10));
    let provenance = provenance().with_deadline(expired, MonotonicInstant::from_millis(11_000));
    let call = provenance.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    let (req, _rx) = ControlRequest::trusted_bridge_forwarded(
        "engine.execute",
        json!({"command":"engine.commands"}),
        call.trusted().clone(),
        Some(ConservativeDeadline::new(MonotonicInstant::from_millis(1_000), Duration::from_secs(10))),
        Some(MonotonicInstant::from_millis(11_000)),
    );
    // Minting an expired window is possible, but admission refuses it and no grant is taken.
    let mut billing = CountingBilling::default();
    let admission = admit_control_request(&req, &bridge_budget(), &mut billing);
    assert!(matches!(admission, Admission::Refused(refusal) if refusal.code == RefusalCode::UnsafeToStart));
    assert_eq!(billing.admits(), 0);
}

#[test]
fn a_forwarded_context_with_another_entry_is_refused_before_dispatch() {
    let (bridge, _rx, _active) = authorized_bridge();
    let call = bridge.begin_logical_call(EntryCategory::GuiBridgeMcp, "execute").expect("host mint");
    let (mut req, _rx) = ControlRequest::trusted_control("engine.execute", json!({"command":"engine.commands"}));
    req.trusted = Some(call.trusted().clone());
    let mut billing = CountingBilling::default();
    assert!(matches!(admit_control_request(&req, &bridge_budget(), &mut billing), Admission::Refused(_)));
    assert_eq!(billing.admits(), 0);
}
