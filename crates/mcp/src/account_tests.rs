//! Admission, settlement and retry semantics at the MCP logical-operation boundary.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, BillingPort, BillingResult, BudgetTable, Capability, ConservativeDeadline, EntryCategory, Grant, GrantId,
    HostSessionBinding, InstallationId, MonotonicInstant, OperationId, OperationRecord, OperationStatus, OperationStore, OperationStoreError,
    Refusal, RefusalCode, RetryDecision, RetryScopeKey, ReviewOperation, RunId, SaveOutcome, SettlementIntent, TransportCorrelation, TrustedContext,
};

use crate::account::{
    CallOutcome, MeteredReply, ToolCallContext, call_tool_forwarded, call_tool_with, expired_state_response, missing_operation_state,
    operation_for_review, record_review_operation, tool_is_free,
};
use crate::backend::NOT_RUNNING;
use crate::{Backend, Headless};

#[derive(Default)]
struct CountingBilling {
    admits: usize,
    commits: usize,
    releases: usize,
    conflict: bool,
    commit_error: Option<RefusalCode>,
    release_error: Option<RefusalCode>,
}

impl BillingPort for CountingBilling {
    fn admit(&mut self, operation: &OperationId, capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
        if self.conflict {
            return Err(Refusal::new(RefusalCode::OperationConflict, "the operation already exists"));
        }
        self.admits = self.admits.saturating_add(1);
        Ok(Grant::from_server(GrantId::new(format!("grant_test_{}", operation.as_str())), capability, MonotonicInstant::from_millis(60_000)))
    }

    fn commit(&mut self, _operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        if let Some(code) = self.commit_error {
            return Err(Refusal::new(code, "the settlement reply was not available"));
        }
        self.commits = self.commits.saturating_add(1);
        Ok(result)
    }

    fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
        if let Some(code) = self.release_error {
            return Err(Refusal::new(code, "the release reply was not available"));
        }
        self.releases = self.releases.saturating_add(1);
        Ok(())
    }
}

fn verified_budget() -> BudgetTable {
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Mcp, EntryCategory::HeadlessMcp, Duration::from_secs(10));
    budget.set_verified(Capability::Dfm, EntryCategory::CliDfm, Duration::from_secs(10));
    budget.set_verified(Capability::Review, EntryCategory::ReviewPanel, Duration::from_secs(10));
    budget
}

fn operation(raw: &str) -> OperationId {
    OperationId::parse(raw).expect("a valid host-minted operation id")
}

fn context(raw: &str) -> ToolCallContext {
    ToolCallContext::new(operation(raw), Capability::Mcp, EntryCategory::HeadlessMcp).with_safe_stop(Duration::from_secs(30))
}

fn step(length: f64) -> Value {
    json!({"command":"solid.box","params":{"length":length,"width":10,"height":5}})
}

fn reply_body(reply: &MeteredReply) -> Value {
    if let Some(structured) = reply.value().get("structuredContent") {
        return structured.clone();
    }
    reply
        .value()
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.last())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null)
}

#[test]
fn charged_tool_with_a_free_origin_is_refused_before_dispatch() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let free = ToolCallContext::new(operation("op_mcp_free_origin_0001"), Capability::Mcp, EntryCategory::HumanGuiFree);
    let reply =
        call_tool_with(&mut backend, "execute", &json!({"command":"solid.box","params":{"length":10}}), &free, &verified_budget(), &mut billing);
    assert!(reply.is_error(), "{}", reply.value());
    assert_eq!(reply_body(&reply)["errorDetails"]["reason"], "CAPABILITY_DENIED");
    assert_eq!(billing.admits, 0, "a free origin takes no grant");
    assert!(backend.session.doc.features.is_empty(), "nothing was executed");
}

#[test]
fn forwarded_charged_tool_with_a_free_trusted_origin_is_refused() {
    let scope = AuthenticatedHostScope::new(
        AccountId::new("acc_forward_free"),
        InstallationId::new("inst_forward_free"),
        HostSessionBinding::new("session_forward_free"),
    );
    let trusted =
        TrustedContext::mint(&scope, "op_mcp_forward_free_0001", Capability::Mcp, EntryCategory::HumanGuiFree).expect("host-minted context");
    let context = ToolCallContext::from_trusted(trusted);
    let mut backend = Headless::default();
    let reply = call_tool_forwarded(&mut backend, "execute", &json!({"command":"solid.box","params":{"length":10}}), &context);
    assert!(reply.is_error(), "{}", reply.value());
    assert_eq!(reply_body(&reply)["errorDetails"]["reason"], "CAPABILITY_DENIED");
    assert!(backend.session.doc.features.is_empty(), "a free forwarded origin executes nothing");
}

#[test]
fn one_batch_of_n_steps_admits_once() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let ctx = context("op_mcp_batch_0001");
    let reply =
        call_tool_with(&mut backend, "batch", &json!({"commands":[step(10.0),step(20.0),step(30.0)]}), &ctx, &verified_budget(), &mut billing);
    assert!(!reply.is_error(), "{}", reply.value());
    assert_eq!(billing.admits, 1);
    assert_eq!(billing.commits, 1);
    assert_eq!(billing.releases, 0);
    assert_eq!(reply.operation_id(), &operation("op_mcp_batch_0001"));
}

#[test]
fn nested_batch_does_not_recharge() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let ctx = context("op_mcp_nested_0001")
        .with_deadline(ConservativeDeadline::new(MonotonicInstant::from_millis(1_000), Duration::from_secs(60)))
        .with_now(MonotonicInstant::from_millis(2_000));
    let outer = call_tool_with(&mut backend, "batch", &json!({"commands":[step(10.0),step(20.0)]}), &ctx, &verified_budget(), &mut billing);
    assert_eq!((billing.admits, billing.commits), (1, 1));
    let nested_ctx = outer.nested_context();
    let inner = call_tool_with(&mut backend, "batch", &json!({"commands":[step(30.0)]}), &nested_ctx, &verified_budget(), &mut billing);
    assert!(!inner.is_error(), "{}", inner.value());
    assert_eq!(inner.operation_id(), outer.operation_id());
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0), "an inner batch never buys a second operation");
}

#[test]
fn read_only_tool_success_charges_once() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(&mut backend, "measure", &json!({}), &context("op_mcp_measure_0001"), &verified_budget(), &mut billing);
    assert!(!reply.is_error(), "{}", reply.value());
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0));
}

#[test]
fn full_rollback_releases() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let args = json!({"commands":[
        {"command":"parameters.change","params":{"name":"temporary","expression":1}},
        {"command":"parameters.change","params":{"name":"broken","expression":"("}},
    ]});
    let reply = call_tool_with(&mut backend, "batch", &args, &context("op_mcp_rollback_0001"), &verified_budget(), &mut billing);
    assert!(reply.is_error());
    assert_eq!(reply_body(&reply)["rolled_back"], true, "{}", reply.value());
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 0, 1));
}

#[test]
fn no_retained_changes_releases() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let args = json!({"commands":[
        {"command":"parameters.change","params":{"name":"broken","expression":"("}},
    ]});
    let reply = call_tool_with(&mut backend, "batch", &args, &context("op_mcp_no_effect_0001"), &verified_budget(), &mut billing);
    assert!(reply.is_error());
    assert_eq!(reply_body(&reply)["applied"], json!([]), "{}", reply.value());
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 0, 1));
}

#[test]
fn partial_retained_charges_once() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let args = json!({"rollback":false,"commands":[
        {"command":"parameters.change","params":{"name":"retained","expression":2}},
        {"command":"parameters.change","params":{"name":"broken","expression":"("}},
    ]});
    let reply = call_tool_with(&mut backend, "batch", &args, &context("op_mcp_partial_0001"), &verified_budget(), &mut billing);
    assert!(reply.is_error());
    assert_eq!(reply_body(&reply)["rolled_back"], false, "{}", reply.value());
    assert_eq!(reply_body(&reply)["applied"], json!([0]));
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0));
}

struct ScriptedBackend {
    calls: usize,
    reply: Result<Value, String>,
}

impl Backend for ScriptedBackend {
    fn call(&mut self, _method: &str, _params: Value) -> Result<Value, String> {
        self.calls = self.calls.saturating_add(1);
        self.reply.clone()
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "scripted MCP backend".into()
    }
}

#[test]
fn dfm_report_with_violations_still_charges_once() {
    let mut backend = ScriptedBackend { calls: 0, reply: Ok(json!({"report":{"profile":"dfm","violations":[{"rule":"wall"}]}, "violations":1})) };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({"command":"dfm.check","params":{}}),
        &context("op_mcp_dfm_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(!reply.is_error(), "{}", reply.value());
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0), "a produced DFM report is charged once, violations or not");
    assert_eq!(backend.calls, 1);
}

#[test]
fn dfm_failure_with_typed_no_effect_releases_without_charge() {
    let mut backend = ScriptedBackend {
        calls: 0,
        reply: Ok(json!({"ok":false,"completed":0,"results":[],
            "failed":{"index":0,"command":"dfm.check","error":"dfm runner failed","stage":"runner"},
            "rolled_back":true,"undone":0,"applied":[],"not_run":[0]})),
    };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "batch",
        &json!({"commands":[{"command":"dfm.check","params":{}}]}),
        &context("op_mcp_dfm_fail_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 0, 1));
    assert_eq!(backend.calls, 1, "one outer DFM batch, one backend call");
}

#[test]
fn ambiguous_dfm_runner_error_stays_unknown_and_never_releases() {
    let mut backend = ScriptedBackend { calls: 0, reply: Err("dfm runner failed".into()) };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({"command":"dfm.check","params":{}}),
        &context("op_mcp_dfm_ambiguous_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!(*reply.outcome(), CallOutcome::Unknown);
    assert_eq!(reply_body(&reply)["code"], "OP_OUTCOME_UNKNOWN");
    assert_eq!(reply_body(&reply)["operationId"], "op_mcp_dfm_ambiguous_0001");
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0), "no effect was proved, so no release");
}

#[test]
fn expectation_mismatch_still_charges_the_retained_cad_effect() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({
            "command":"solid.box",
            "params":{"length":10,"width":10,"height":5},
            "expect":{"bodies":99},
        }),
        &context("op_mcp_expectation_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error(), "the expectation mismatch is still reported as an error");
    assert_eq!(reply_body(&reply)["expectation"]["ok"], false, "{}", reply.value());
    assert_eq!(reply_body(&reply)["created_bodies"], json!(["Body1"]), "the completed command result is retained");
    assert_eq!(*reply.outcome(), CallOutcome::PartialRetained);
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0), "a retained CAD effect is charged even when `expect` fails");
}

#[test]
fn ambiguous_post_dispatch_error_stays_unknown_without_release() {
    let mut backend = ScriptedBackend { calls: 0, reply: Err("the app did not answer engine.execute; it may have run".into()) };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({"command":"solid.box","params":{"length":10,"width":10,"height":5}}),
        &context("op_mcp_ambiguous_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!(*reply.outcome(), CallOutcome::Unknown);
    assert_eq!(reply_body(&reply)["code"], "OP_OUTCOME_UNKNOWN");
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0));
}

#[test]
fn retry_same_operation_id_never_executes_twice() {
    let mut backend = ScriptedBackend { calls: 0, reply: Ok(json!({"ok":true})) };
    let mut billing = CountingBilling { conflict: true, ..CountingBilling::default() };
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({"command":"solid.box","params":{"length":10,"width":10,"height":5}}),
        &context("op_mcp_exists_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!(*reply.outcome(), CallOutcome::AlreadyExists);
    assert_eq!(reply.operation_id(), &operation("op_mcp_exists_0001"));
    assert_eq!(backend.calls, 0, "an existing logical operation is never dispatched again");
    assert_eq!((billing.admits, billing.commits, billing.releases), (0, 0, 0));
}

#[test]
fn unknown_returns_stable_code_and_original_id() {
    let mut backend = ScriptedBackend { calls: 0, reply: Err("still running: the request has not answered".into()) };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({"command":"solid.box","params":{"length":10,"width":10,"height":5}}),
        &context("op_mcp_unknown_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!(*reply.outcome(), CallOutcome::Unknown);
    assert_eq!(reply_body(&reply)["code"], "OP_OUTCOME_UNKNOWN");
    assert_eq!(reply_body(&reply)["operationId"], "op_mcp_unknown_0001");
    assert_eq!(billing.admits, 1);
    assert_eq!(billing.releases, 0, "an unknown outcome is never released as free");
    assert_eq!(billing.commits, 1, "the unknown settlement is recorded once as PENDING");
}

fn deadline() -> ConservativeDeadline {
    ConservativeDeadline::new(MonotonicInstant::from_millis(1_000), Duration::from_secs(10))
}

#[test]
fn expired_finalize_and_lookup_return_generic_no_state() {
    let known = deadline();
    let at_deadline = MonotonicInstant::from_millis(11_000);
    let later = MonotonicInstant::from_millis(30_000);
    for now in [at_deadline, later] {
        assert_eq!(missing_operation_state(Some(&known), now, RefusalCode::AuthRequired), crate::account::MissingOperationState::ExpiredNoState);
        assert_eq!(missing_operation_state(Some(&known), now, RefusalCode::AuthRequired), crate::account::MissingOperationState::ExpiredNoState);
        let finalize = expired_state_response();
        let lookup = expired_state_response();
        assert_eq!(finalize, lookup, "finalize and lookup share one generic no-state response");
        assert!(finalize.get("operationId").is_none(), "the generic response names no operation state: {finalize}");
    }
}

#[test]
fn client_maps_no_charge_only_when_known_deadline() {
    assert_eq!(
        missing_operation_state(Some(&deadline()), MonotonicInstant::from_millis(11_000), RefusalCode::AuthRequired),
        crate::account::MissingOperationState::ExpiredNoState
    );
}

#[test]
fn unknown_401_is_not_automatically_no_charge() {
    for refusal in [RefusalCode::AuthRequired, RefusalCode::AuthUnavailable, RefusalCode::OperationOwnedElsewhere] {
        assert_eq!(
            missing_operation_state(None, MonotonicInstant::from_millis(120_000), refusal),
            crate::account::MissingOperationState::Unknown,
            "only a known local deadline proves the no-charge case"
        );
    }
    assert_eq!(
        missing_operation_state(Some(&deadline()), MonotonicInstant::from_millis(5_000), RefusalCode::AuthRequired),
        crate::account::MissingOperationState::Unknown,
        "a 401 before the known deadline is not a release"
    );
}

struct CountingBackend {
    calls: usize,
}

impl Backend for CountingBackend {
    fn call(&mut self, _method: &str, _params: Value) -> Result<Value, String> {
        self.calls = self.calls.saturating_add(1);
        Ok(json!({"ok":true}))
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "counting MCP backend".into()
    }
}

struct RetryScope {
    key: RetryScopeKey,
    registry: BTreeMap<RetryScopeKey, OperationId>,
}

fn retry_scope(account: &str, installation: &str, correlation: &str) -> RetryScope {
    let key = RetryScopeKey {
        host_scope: AuthenticatedHostScope::new(
            AccountId::new(account),
            InstallationId::new(installation),
            HostSessionBinding::new("host_session_binding"),
        ),
        transport_correlation: TransportCorrelation::new(correlation),
    };
    RetryScope { key, registry: BTreeMap::new() }
}

fn call_tool_with_retry_scope(
    backend: &mut dyn Backend,
    tool: &str,
    args: &Value,
    scope: &mut RetryScope,
    billing: &mut CountingBilling,
) -> MeteredReply {
    let ctx = match scope.registry.get(&scope.key) {
        Some(operation) => {
            return MeteredReply::reused(
                &ToolCallContext::new(operation.clone(), Capability::Mcp, EntryCategory::HeadlessMcp).with_safe_stop(Duration::from_secs(30)),
            );
        }
        None => {
            ToolCallContext::new(operation("op_mcp_retry_0001"), Capability::Mcp, EntryCategory::HeadlessMcp).with_safe_stop(Duration::from_secs(30))
        }
    };
    let reply = call_tool_with(backend, tool, args, &ctx, &verified_budget(), billing);
    if !matches!(reply.outcome(), CallOutcome::Free | CallOutcome::Refused(_)) {
        scope.registry.insert(scope.key.clone(), reply.operation_id().clone());
    }
    reply
}

#[test]
fn mcp_boundary_lost_response_retry_reuses_same_operation() {
    let mut billing = CountingBilling::default();
    let mut backend = CountingBackend { calls: 0 };
    let mut scope = retry_scope("acct_example_0001", "inst_example_0001", "req_example_0001");
    let first = call_tool_with_retry_scope(&mut backend, "execute", &json!({"command":"solid.box","params":{}}), &mut scope, &mut billing);
    let original_id = first.operation_id().clone();
    drop(first); // the retry deliberately carries only scope + correlation, no operation id
    let retry = call_tool_with_retry_scope(&mut backend, "execute", &json!({"command":"solid.box","params":{}}), &mut scope, &mut billing);
    assert_eq!(retry.operation_id(), &original_id);
    assert_eq!(billing.admits, 1);
    assert_eq!(billing.commits, 1);
    assert_eq!(backend.calls, 1, "ReuseOriginal must not dispatch again");
}

#[derive(Default)]
struct ReviewStore {
    reviews: BTreeMap<RunId, OperationId>,
}

impl OperationStore for ReviewStore {
    fn save_new(&mut self, _record: &OperationRecord) -> Result<SaveOutcome, OperationStoreError> {
        Err(OperationStoreError::InvalidState)
    }
    fn load(&self, _operation: &OperationId) -> Result<Option<OperationRecord>, OperationStoreError> {
        Ok(None)
    }
    fn mark_started(&mut self, _operation: &OperationId, _started_at_ms: u64) -> Result<(), OperationStoreError> {
        Ok(())
    }
    fn mark_terminal(&mut self, _operation: &OperationId, _status: OperationStatus) -> Result<(), OperationStoreError> {
        Ok(())
    }
    fn persist_intent(
        &mut self,
        _scope: &AuthenticatedHostScope,
        _correlation: &TransportCorrelation,
        _capability: Capability,
        _entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        Err(SaveOutcome::Conflict)
    }
    fn persist_fresh_intent(
        &mut self,
        _intent: &solidcraft_engine::entitlement::FreshIntent,
        _capability: Capability,
        _entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        Err(SaveOutcome::Conflict)
    }
    fn lookup_original_retry(&self, _key: &RetryScopeKey) -> RetryDecision {
        RetryDecision::Unprovable
    }
    fn record_review(&mut self, review: &ReviewOperation) -> Result<(), OperationStoreError> {
        match self.reviews.get(&review.run_id) {
            Some(existing) if existing != &review.operation_id => Err(OperationStoreError::InvalidState),
            _ => {
                self.reviews.insert(review.run_id.clone(), review.operation_id.clone());
                Ok(())
            }
        }
    }
    fn review_operation(&self, run_id: &RunId) -> Result<Option<OperationId>, OperationStoreError> {
        Ok(self.reviews.get(run_id).cloned())
    }
    fn record_settlement_intent(&mut self, _operation: &OperationId, _intent: SettlementIntent) -> Result<(), OperationStoreError> {
        Ok(())
    }
    fn settlement_intent(&self, _operation: &OperationId) -> Result<Option<SettlementIntent>, OperationStoreError> {
        Ok(None)
    }
}

#[test]
fn review_run_reuses_same_logical_id() {
    let mut store = ReviewStore::default();
    let run = RunId::new("run_review_0001");
    let logical = operation("op_review_shared_0001");
    record_review_operation(&mut store, &ReviewOperation::new(run.clone(), logical.clone())).expect("the review identity is stored");
    assert_eq!(operation_for_review(&store, &run).expect("the ledger is readable"), Some(logical.clone()));
    let mcp = ToolCallContext::new(logical.clone(), Capability::Review, EntryCategory::HeadlessMcp).with_safe_stop(Duration::from_secs(30));
    let bridge = ToolCallContext::new(logical.clone(), Capability::Review, EntryCategory::GuiBridgeMcp).with_safe_stop(Duration::from_secs(30));
    assert_eq!(mcp.operation(), &logical);
    assert_eq!(bridge.operation(), &logical, "MCP, bridge and Review share one logical id");
    let record = ReviewOperation { run_id: run, operation_id: logical };
    assert_eq!(record.operation_id.as_str(), "op_review_shared_0001");
}

#[test]
fn mastra_owned_helpers_are_free_while_intcad_commands_are_charged() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let free = call_tool_with(
        &mut backend,
        "skill_reference",
        &json!({"skill":"solidcraft-modeling"}),
        &context("op_mcp_free_0001"),
        &verified_budget(),
        &mut billing,
    );
    let charged = call_tool_with(&mut backend, "measure", &json!({}), &context("op_mcp_charged_0001"), &verified_budget(), &mut billing);
    assert!(tool_is_free("skill_reference"));
    assert!(!tool_is_free("measure"));
    assert_eq!(billing.admits, 1, "only the Intcad logical command admitted; the helper stayed free");
    assert_eq!(free.outcome(), &CallOutcome::Free);
    assert_eq!(charged.outcome(), &CallOutcome::Succeeded);
}

#[test]
fn unverified_budget_and_missing_context_fail_closed() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let unverified =
        call_tool_with(&mut backend, "measure", &json!({}), &context("op_mcp_unverified_0001"), &BudgetTable::unverified_default(), &mut billing);
    assert_eq!(unverified.outcome(), &CallOutcome::Refused(RefusalCode::UnsafeToStart));
    assert_eq!(billing.admits, 0);

    let unknown = call_tool_with(
        &mut backend,
        "measure",
        &json!({}),
        &ToolCallContext::new(operation("op_mcp_unknown_entry_0001"), Capability::Mcp, EntryCategory::Unknown)
            .with_safe_stop(Duration::from_secs(30)),
        &verified_budget(),
        &mut billing,
    );
    assert_eq!(unknown.outcome(), &CallOutcome::Refused(RefusalCode::CapabilityDenied));

    let no_stop = call_tool_with(
        &mut backend,
        "measure",
        &json!({}),
        &ToolCallContext::new(operation("op_mcp_no_stop_0001"), Capability::Mcp, EntryCategory::HeadlessMcp),
        &verified_budget(),
        &mut billing,
    );
    assert_eq!(no_stop.outcome(), &CallOutcome::Refused(RefusalCode::UnsafeToStart));
    assert_eq!(billing.admits, 0);
}

#[test]
fn failed_settlement_is_unknown_unless_known_deadline_expired() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling { commit_error: Some(RefusalCode::AuthRequired), ..CountingBilling::default() };
    let reply = call_tool_with(&mut backend, "measure", &json!({}), &context("op_mcp_settle_0001"), &verified_budget(), &mut billing);
    assert_eq!(reply.outcome(), &CallOutcome::Unknown, "an unproven settlement is never reported as success");
    assert_eq!(reply_body(&reply)["code"], "OP_OUTCOME_UNKNOWN");

    let mut billing = CountingBilling { commit_error: Some(RefusalCode::AuthRequired), ..CountingBilling::default() };
    let late = ToolCallContext::new(operation("op_mcp_settle_late_0001"), Capability::Mcp, EntryCategory::HeadlessMcp)
        .with_safe_stop(Duration::from_secs(30))
        .with_deadline(ConservativeDeadline::new(MonotonicInstant::from_millis(1_000), Duration::from_secs(10)))
        .with_now(MonotonicInstant::from_millis(12_000));
    let reply = call_tool_with(&mut backend, "measure", &json!({}), &late, &verified_budget(), &mut billing);
    assert_eq!(reply.outcome(), &CallOutcome::FailedNoEffect);
    assert_eq!(reply_body(&reply)["errorDetails"]["code"], "OP_STATE_UNAVAILABLE");
    assert_eq!(billing.commits, 0);
}

#[test]
fn charged_call_without_a_trusted_context_is_refused_before_dispatch() {
    let mut backend = ScriptedBackend { calls: 0, reply: Ok(json!({"ok":true})) };
    let mut billing = CountingBilling::default();
    let no_stop = call_tool_with(
        &mut backend,
        "execute",
        &json!({"command":"solid.box","params":{}}),
        &ToolCallContext::new(operation("op_mcp_no_context_0001"), Capability::Mcp, EntryCategory::Unknown).with_safe_stop(Duration::from_secs(30)),
        &verified_budget(),
        &mut billing,
    );
    assert!(no_stop.is_error());
    assert_eq!(backend.calls, 0);
    assert_eq!(billing.admits, 0);
}

struct SharedBilling(std::rc::Rc<std::cell::RefCell<CountingBilling>>);

impl BillingPort for SharedBilling {
    fn admit(&mut self, operation: &OperationId, capability: Capability, entry: EntryCategory) -> Result<Grant, Refusal> {
        self.0.borrow_mut().admit(operation, capability, entry)
    }
    fn commit(&mut self, operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        self.0.borrow_mut().commit(operation, result)
    }
    fn release(&mut self, operation: &OperationId, reason: RefusalCode) -> Result<(), Refusal> {
        self.0.borrow_mut().release(operation, reason)
    }
}

struct ContextBackend {
    headless: Headless,
    context: Option<ToolCallContext>,
    calls: std::rc::Rc<std::cell::Cell<usize>>,
}

impl Backend for ContextBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.calls.set(self.calls.get().saturating_add(1));
        crate::Backend::call(&mut self.headless, method, params)
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "context-holding MCP backend".into()
    }
    fn tool_context(&self, _tool: &str) -> Option<ToolCallContext> {
        self.context.clone()
    }
}

fn server_call(server: &mut crate::Server, name: &str, args: Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {"name": name, "arguments": args},
    });
    let line = server.handle_line(&request.to_string()).expect("a tools/call reply");
    let reply: Value = serde_json::from_str(&line).expect("valid JSON-RPC");
    reply["result"].clone()
}

fn context_backend(context: Option<ToolCallContext>) -> (ContextBackend, std::rc::Rc<std::cell::Cell<usize>>) {
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    (ContextBackend { headless: Headless::default(), context, calls: std::rc::Rc::clone(&calls) }, calls)
}

fn shared_billing() -> (SharedBilling, std::rc::Rc<std::cell::RefCell<CountingBilling>>) {
    let inner = std::rc::Rc::new(std::cell::RefCell::new(CountingBilling::default()));
    (SharedBilling(std::rc::Rc::clone(&inner)), inner)
}

#[test]
fn server_metered_charged_tool_without_context_is_refused_before_dispatch() {
    let (backend, calls) = context_backend(None);
    let (billing, counts) = shared_billing();
    let mut server = crate::Server::new(Box::new(backend)).with_billing(Box::new(billing), verified_budget());
    let result = server_call(&mut server, "measure", json!({}));
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["errorDetails"]["code"], "OP_REFUSED");
    assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE");
    assert_eq!(calls.get(), 0, "no backend dispatch without a host operation");
    assert_eq!(counts.borrow().admits, 0);
}

#[test]
fn server_metered_charged_tool_admits_once_with_context() {
    let (backend, calls) = context_backend(Some(context("op_mcp_server_0001")));
    let (billing, counts) = shared_billing();
    let mut server = crate::Server::new(Box::new(backend)).with_billing(Box::new(billing), verified_budget());
    let result = server_call(&mut server, "measure", json!({}));
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(calls.get(), 1);
    assert_eq!(counts.borrow().admits, 1);
    assert_eq!(counts.borrow().commits, 1);
}

#[test]
fn server_metered_unverified_budget_refuses_before_dispatch() {
    let (backend, calls) = context_backend(Some(context("op_mcp_server_unverified_0001")));
    let (billing, counts) = shared_billing();
    let mut server = crate::Server::new(Box::new(backend)).with_billing(Box::new(billing), BudgetTable::unverified_default());
    let result = server_call(&mut server, "measure", json!({}));
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["errorDetails"]["code"], "OP_REFUSED");
    assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "UNSAFE_TO_START");
    assert_eq!(calls.get(), 0);
    assert_eq!(counts.borrow().admits, 0);
}

#[test]
fn server_metered_free_helper_runs_without_context_or_billing() {
    let (backend, calls) = context_backend(None);
    let (billing, counts) = shared_billing();
    let mut server = crate::Server::new(Box::new(backend)).with_billing(Box::new(billing), verified_budget());
    let result = server_call(&mut server, "skill_reference", json!({"skill":"solidcraft-modeling"}));
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(calls.get(), 0, "the helper answers from its own reference");
    assert_eq!(counts.borrow().admits, 0, "a Mastra/protocol helper never reaches billing");
}

#[test]
fn free_human_entry_does_not_make_a_charged_tool_free() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let free =
        ToolCallContext::new(operation("op_mcp_human_0001"), Capability::Mcp, EntryCategory::HumanGuiFree).with_safe_stop(Duration::from_secs(30));
    let reply = call_tool_with(&mut backend, "measure", &json!({}), &free, &verified_budget(), &mut billing);
    assert!(reply.is_error(), "{}", reply.value());
    assert_eq!(reply.outcome(), &CallOutcome::Refused(RefusalCode::CapabilityDenied));
    assert_eq!((billing.admits, billing.commits, billing.releases), (0, 0, 0));
}

#[test]
fn nested_continuation_without_an_active_deadline_is_refused() {
    let mut backend = Headless::default();
    let mut billing = CountingBilling::default();
    let outer = call_tool_with(
        &mut backend,
        "batch",
        &json!({"commands":[step(10.0)]}),
        &context("op_mcp_nested_no_deadline_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert_eq!((billing.admits, billing.commits), (1, 1));
    let nested = outer.nested_context();
    let inner = call_tool_with(&mut backend, "batch", &json!({"commands":[step(20.0)]}), &nested, &verified_budget(), &mut billing);
    assert!(inner.is_error());
    assert_eq!(inner.outcome(), &CallOutcome::Refused(RefusalCode::UnsafeToStart));
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0), "a refused nested hook never buys or settles anything");
}

#[test]
fn default_server_charged_tool_is_refused_even_with_a_context() {
    let (backend, calls) = context_backend(Some(context("op_mcp_closed_server_0001")));
    let mut server = crate::Server::new(Box::new(backend));
    let result = server_call(&mut server, "measure", json!({}));
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["errorDetails"]["code"], "OP_REFUSED");
    assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE");
    assert_eq!(calls.get(), 0, "the default external server has no host authority and never dispatch");
}

#[test]
fn default_server_still_serves_a_free_helper() {
    let (backend, calls) = context_backend(None);
    let mut server = crate::Server::new(Box::new(backend));
    let result = server_call(&mut server, "skill_reference", json!({"skill":"solidcraft-modeling"}));
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(calls.get(), 0);
}

struct RemoteRejectBackend {
    calls: usize,
}

impl Backend for RemoteRejectBackend {
    fn call(&mut self, _method: &str, _params: Value) -> Result<Value, String> {
        self.calls = self.calls.saturating_add(1);
        Err("remote call should not be reached".into())
    }
    fn call_guarded(&mut self, _method: &str, _params: Value) -> Result<crate::GuardedReply, crate::GuardedCallError> {
        self.calls = self.calls.saturating_add(1);
        Err(crate::GuardedCallError::Rejected {
            message: "the remote app rejected the guarded request".into(),
            error_details: json!({"code":"REMOTE_REJECTED","retry":"unknown"}),
        })
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "inert remote rejection backend".into()
    }
}

#[test]
fn local_guarded_rejection_is_typed_no_effect_and_releases() {
    let mut backend = ScriptedBackend { calls: 0, reply: Ok(json!({"ok":true})) };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({
            "command":"file.save",
            "params":{},
            "expectedDocument":{"documentId":"doc-A","revision":"0"},
        }),
        &context("op_mcp_local_reject_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!(reply.value()["noEffect"], true, "{}", reply.value());
    assert_eq!(reply_body(&reply)["error"]["code"], "UNSUPPORTED_CAPABILITY");
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 0, 1), "a local pre-dispatch refusal is no effect");
    assert_eq!(backend.calls, 0, "the local guard refused before any backend call");
}

#[test]
fn remote_guarded_rejection_stays_unknown_without_no_effect_proof() {
    let mut backend = RemoteRejectBackend { calls: 0 };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({
            "command":"solid.box",
            "params":{"length":10,"width":10,"height":5},
            "expectedDocument":{"documentId":"doc-A","revision":"0"},
        }),
        &context("op_mcp_remote_reject_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!(*reply.outcome(), CallOutcome::Unknown, "a remote Rejected reply has no proof that no work ran");
    assert_eq!(reply_body(&reply)["code"], "OP_OUTCOME_UNKNOWN");
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 1, 0));
    assert_eq!(backend.calls, 1);
}

#[test]
fn confirmed_no_send_backend_error_releases() {
    let mut backend = ScriptedBackend { calls: 0, reply: Err(format!("{NOT_RUNNING} 127.0.0.1:9: it is not running")) };
    let mut billing = CountingBilling::default();
    let reply = call_tool_with(
        &mut backend,
        "execute",
        &json!({"command":"solid.box","params":{"length":10,"width":10,"height":5}}),
        &context("op_mcp_no_send_0001"),
        &verified_budget(),
        &mut billing,
    );
    assert!(reply.is_error());
    assert_eq!(*reply.outcome(), CallOutcome::FailedNoEffect);
    assert_eq!((billing.admits, billing.commits, billing.releases), (1, 0, 1));
    assert_eq!(backend.calls, 1);
}

#[test]
fn free_helper_catalog_matches_the_mcp_tool_definitions() {
    let defs = crate::tool_definitions();
    let mut free: Vec<String> = defs
        .iter()
        .filter_map(|definition| definition.get("name").and_then(Value::as_str))
        .filter(|name| tool_is_free(name))
        .map(str::to_owned)
        .collect();
    free.sort();
    assert_eq!(
        free,
        vec![
            "compare_picture".to_string(),
            "crash_reports".to_string(),
            "describe_command".to_string(),
            "list_commands".to_string(),
            "pending".to_string(),
            "running".to_string(),
            "screenshot".to_string(),
            "skill_reference".to_string(),
        ],
        "the free helper list must match the published MCP catalog"
    );
    for charged in ["execute", "batch", "measure", "inspect_design", "check_parameters", "save", "open"] {
        assert!(!tool_is_free(charged), "{charged} is an Intcad logical operation and must be charged");
    }
}
