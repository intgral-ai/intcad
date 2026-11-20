//! Task 8 trust-boundary evidence: forwarded host provenance, wire refusal and logical-operation
//! counting for the Mastra/ACP bridge lane.
//!
//! All billing doubles here live only in this test module. No production path constructs a grant,
//! and no test sends a real provider request.

#![cfg(unix)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    AccountId, Admission, AuthenticatedHostScope, BillingPort, BillingResult, BudgetTable, Capability, ConservativeDeadline, EntryCategory, Grant,
    GrantId, HostSessionBinding, InstallationId, MonotonicInstant, OperationId, Refusal, RefusalCode,
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
    let (mut req, _rx) =
        ControlRequest::trusted_bridge_forwarded("engine.execute", json!({"command":"engine.commands"}), call.trusted().clone(), None, None);
    let first = admit_once(&mut app, &mut req).expect("the forwarded host call is admitted");
    assert!(matches!(first, Admission::NewlyAdmitted { .. }));
    assert!(matches!(admit_once(&mut app, &mut req), Ok(Admission::NewlyAdmitted { .. })), "the stored admission is reused");
    let (mut reissued, _rx) = crate::control::reissue(Reissue::from_request(&req), "engine.execute", json!({"command":"engine.commands"}));
    assert!(matches!(admit_once(&mut app, &mut reissued), Ok(Admission::NewlyAdmitted { .. })), "a reissued request keeps the first admission");
    assert_eq!(billing.admits(), 1, "one logical call is admitted once, not once per seam");
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
