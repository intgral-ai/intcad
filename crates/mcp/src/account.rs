//! Defensive logical-operation identity and credit accounting for MCP tools.
//!
//! One external MCP tool call is one logical operation. A `batch` is admitted once as a whole,
//! however many commands it sends to the engine, and a forwarded DFM/Review request reuses that
//! outer identity instead of buying a second admission. The host supplies the context: JSON-RPC
//! request ids, tool-call ids and run ids are data and never authority.

use std::time::Duration;

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    Admission, BillingPort, BillingResult, BudgetTable, Capability, ConservativeDeadline, EntryCategory, MonotonicInstant, OperationId,
    OperationStore, OperationStoreError, Refusal, RefusalCode, ReviewOperation, RunId, UpperBound,
};

use crate::backend::Backend;

/// Host-minted identity for one external MCP tool call.
///
/// `operation` comes from the host's authenticated path, never from the wire. The context carries
/// no grant; only [`BillingPort::admit`] can return one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallContext {
    operation: OperationId,
    capability: Capability,
    entry: EntryCategory,
    remaining: Option<Duration>,
    deadline: Option<ConservativeDeadline>,
    now: Option<MonotonicInstant>,
    admission: Option<Admission>,
    nested: bool,
}

impl ToolCallContext {
    /// Identity supplied by the host after it minted (or recovered) the logical operation.
    pub fn new(operation: OperationId, capability: Capability, entry: EntryCategory) -> Self {
        Self { operation, capability, entry, remaining: None, deadline: None, now: None, admission: None, nested: false }
    }

    /// The remaining authorization measured by the host for this request.
    pub fn with_safe_stop(mut self, remaining: Duration) -> Self {
        self.remaining = Some(remaining);
        self
    }

    /// The host's conservative deadline, used only to prove a late settlement is past expiry.
    pub fn with_deadline(mut self, deadline: ConservativeDeadline) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// The host's monotonic reading for the settlement decision.
    pub fn with_now(mut self, now: MonotonicInstant) -> Self {
        self.now = Some(now);
        self
    }

    /// A continuation inside the same outer operation. It never admits or settles again.
    ///
    /// This is sealed to the crate's tests until Task 8/9 bind the parent's admission and
    /// deadline into a production forwarding context. A production context cannot set
    /// `nested`, so an external caller cannot use it to skip admission.
    #[cfg(test)]
    fn nested_context(&self, operation: OperationId) -> Self {
        Self {
            operation,
            capability: self.capability,
            entry: self.entry,
            remaining: self.remaining,
            deadline: self.deadline.clone(),
            now: self.now,
            admission: self.admission.clone(),
            nested: true,
        }
    }

    /// A nested continuation is valid only with a real parent admission and an authorization
    /// window that is still active. Without both, a forwarded call is refused until Task 9.
    fn has_active_parent(&self) -> bool {
        let admitted = self.admission.as_ref().is_some_and(|admission| matches!(admission, Admission::NewlyAdmitted { .. }));
        let remaining = self.remaining.is_some_and(|remaining| !remaining.is_zero());
        let deadline_active = match (self.deadline.as_ref(), self.now) {
            (Some(deadline), Some(now)) => deadline.remaining(now).is_some(),
            _ => false,
        };
        admitted && remaining && deadline_active
    }

    pub fn operation(&self) -> &OperationId {
        &self.operation
    }

    pub fn capability(&self) -> Capability {
        self.capability
    }

    pub fn entry(&self) -> EntryCategory {
        self.entry
    }
}

/// How one metered call settled, or why it never ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    /// A protocol, notification, render or catalog helper; it maps to no Intcad logical command.
    Free,
    Succeeded,
    PartialRetained,
    FailedNoEffect,
    /// The call may have run; its outcome is not provable. Never retry it.
    Unknown,
    /// Attendance refused before dispatch.
    Refused(RefusalCode),
    /// The operation already exists; an original may have run. Never dispatch again.
    AlreadyExists,
}

/// A tool reply plus the logical operation it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeteredReply {
    value: Value,
    operation: OperationId,
    outcome: CallOutcome,
    context: ToolCallContext,
}

impl MeteredReply {
    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn into_value(self) -> Value {
        self.value
    }

    pub fn operation_id(&self) -> &OperationId {
        &self.operation
    }

    pub fn outcome(&self) -> &CallOutcome {
        &self.outcome
    }

    pub fn is_error(&self) -> bool {
        self.value.get("isError") == Some(&Value::Bool(true))
    }

    /// A continuation of this call: a batch step, a bridged tool or a forwarded DFM request.
    /// Test-only until Task 8/9 bind a production parent admission and active deadline.
    #[cfg(test)]
    pub(crate) fn nested_context(&self) -> ToolCallContext {
        self.context.nested_context(self.operation.clone())
    }

    fn refused(context: &ToolCallContext, error: Refusal) -> Self {
        let operation = context.operation.clone();
        let body = json!({
            "error": error.message,
            "errorDetails": {"code": "OP_REFUSED", "reason": refusal_wire(error.code)},
            "operationId": operation.as_str(),
        });
        Self { value: error_result(body), operation, outcome: CallOutcome::Refused(error.code), context: context.clone() }
    }

    fn already_exists(context: &ToolCallContext) -> Self {
        let operation = context.operation.clone();
        let body = json!({
            "error": "this logical operation already exists; it was not sent again",
            "errorDetails": {"code": "OP_ALREADY_EXISTS"},
            "operationId": operation.as_str(),
        });
        Self { value: error_result(body), operation, outcome: CallOutcome::AlreadyExists, context: context.clone() }
    }

    /// A retry of an operation that already completed. It keeps the original id and never
    /// dispatches the backend again.
    #[cfg(test)]
    pub(crate) fn reused(context: &ToolCallContext) -> Self {
        let operation = context.operation.clone();
        let body = json!({
            "error": "this logical operation was already applied; inspect the design instead of sending it again",
            "errorDetails": {"code": "OP_SUCCEEDED", "retry": "never"},
            "operationId": operation.as_str(),
        });
        Self { value: error_result(body), operation, outcome: CallOutcome::Succeeded, context: context.clone() }
    }

    fn unknown(context: &ToolCallContext, message: &str) -> Self {
        let operation = context.operation.clone();
        let body = json!({
            "error": message,
            "errorDetails": {"code": "OP_OUTCOME_UNKNOWN", "retry": "never"},
            "operationId": operation.as_str(),
        });
        Self { value: error_result(body), operation, outcome: CallOutcome::Unknown, context: context.clone() }
    }
}

/// Protocol, notification, render, catalog and diagnostics helpers. These map to no billed
/// Intcad logical operation; Mastra's own prompt/plan tools never reach this module at all.
pub fn tool_is_free(name: &str) -> bool {
    matches!(
        name,
        "pending" | "screenshot" | "compare_picture" | "running" | "list_commands" | "describe_command" | "skill_reference" | "crash_reports"
    )
}

/// Run one MCP tool under a host operation context and settle it exactly once.
///
/// A charged call needs a classified entry, a verified safe-stop bound, enough remaining
/// authorization and a server-issued grant. Missing context is refused before dispatch.
pub fn call_tool_with(
    backend: &mut dyn Backend,
    name: &str,
    args: &Value,
    context: &ToolCallContext,
    budget: &BudgetTable,
    billing: &mut dyn BillingPort,
) -> MeteredReply {
    let operation = context.operation.clone();
    if tool_is_free(name) || context.entry.is_free() {
        let value = crate::tools::call_tool(backend, name, args);
        return MeteredReply { value, operation, outcome: CallOutcome::Free, context: context.clone() };
    }
    if context.nested {
        if !context.has_active_parent() {
            return MeteredReply::refused(
                context,
                Refusal::new(RefusalCode::UnsafeToStart, "a forwarded MCP call has no active parent operation or deadline"),
            );
        }
        let value = crate::tools::call_tool(backend, name, args);
        let outcome = classify_reply(&value);
        return MeteredReply { value, operation, outcome, context: context.clone() };
    }
    if let Err(refusal) = check_preflight(context, budget) {
        return MeteredReply::refused(context, refusal);
    }
    let admission = match &context.admission {
        Some(admission) => admission.clone(),
        None => match billing.admit(&operation, context.capability, context.entry) {
            Ok(grant) => Admission::NewlyAdmitted { operation: operation.clone(), grant },
            Err(refusal) if refusal.code == RefusalCode::OperationConflict => {
                return MeteredReply::already_exists(context);
            }
            Err(refusal) => return MeteredReply::refused(context, refusal),
        },
    };
    let result_value = crate::tools::call_tool(backend, name, args);
    let outcome = classify_reply(&result_value);
    let settle = settle(billing, &operation, &outcome);
    let mut reply_context = context.clone();
    reply_context.admission = Some(admission);
    let mut reply = MeteredReply { value: result_value, operation: operation.clone(), outcome, context: reply_context };
    if let Err(error) = settle {
        let disposition = match (context.deadline.as_ref(), context.now) {
            (Some(deadline), Some(now)) => missing_operation_state(Some(deadline), now, error.code),
            _ => MissingOperationState::Unknown,
        };
        match disposition {
            MissingOperationState::ExpiredNoState => {
                let body = json!({
                    "error": "the authorization deadline passed and the settlement is closed",
                    "errorDetails": {"code": "OP_STATE_UNAVAILABLE", "retry": "never"},
                    "operationId": operation.as_str(),
                });
                reply.value = error_result(body);
                reply.outcome = CallOutcome::FailedNoEffect;
            }
            MissingOperationState::Unknown => {
                let unknown = MeteredReply::unknown(context, "the settlement reply was lost; the outcome is unknown");
                reply.value = unknown.value;
                reply.outcome = CallOutcome::Unknown;
            }
        }
    }
    if *reply.outcome() == CallOutcome::Unknown {
        decorate_unknown(&mut reply.value, &reply.operation);
    }
    reply
}

fn check_preflight(context: &ToolCallContext, budget: &BudgetTable) -> Result<(), Refusal> {
    if context.entry.is_free() {
        return Ok(());
    }
    if context.entry == EntryCategory::Unknown {
        return Err(Refusal::new(RefusalCode::CapabilityDenied, "the MCP request entry is unclassified"));
    }
    let Some(remaining) = context.remaining else {
        return Err(Refusal::new(RefusalCode::UnsafeToStart, "this controlled MCP call has no measured remaining authorization"));
    };
    match budget.bound(context.capability, context.entry) {
        UpperBound::Unverified => Err(Refusal::new(RefusalCode::UnsafeToStart, "this controlled MCP path has no verified safe-stop bound")),
        UpperBound::Verified(bound) if remaining < bound => {
            Err(Refusal::new(RefusalCode::UnsafeToStart, "the remaining authorization is shorter than the safe-stop bound"))
        }
        UpperBound::Verified(_) => Ok(()),
    }
}

fn settle(billing: &mut dyn BillingPort, operation: &OperationId, outcome: &CallOutcome) -> Result<(), Refusal> {
    match outcome {
        CallOutcome::Succeeded => billing.commit(operation, BillingResult::Succeeded).map(|_| ()),
        CallOutcome::PartialRetained => billing.commit(operation, BillingResult::PartialRetained).map(|_| ()),
        CallOutcome::Unknown => billing.commit(operation, BillingResult::Unknown).map(|_| ()),
        CallOutcome::FailedNoEffect => billing.release(operation, RefusalCode::UnsafeToStart),
        CallOutcome::Free | CallOutcome::Refused(_) | CallOutcome::AlreadyExists => Ok(()),
    }
}

/// A known local deadline and a missing settlement answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingOperationState {
    /// The host's own conservative deadline passed. The contract says no charge, and finalize and
    /// lookup answer with the same generic no-state refusal.
    ExpiredNoState,
    /// There is no proof of a local deadline: a 401 or lost reply is not automatically free.
    Unknown,
}

/// Resolve a finalize/lookup refusal without inventing operation state.
///
/// Only a known local deadline that has actually passed maps to no charge. Any other 401,
/// transport failure or missing mapping stays unknown; it is never treated as released.
pub fn missing_operation_state(deadline: Option<&ConservativeDeadline>, now: MonotonicInstant, _refusal: RefusalCode) -> MissingOperationState {
    match deadline {
        Some(deadline) if deadline.remaining(now).is_none() => MissingOperationState::ExpiredNoState,
        _ => MissingOperationState::Unknown,
    }
}

/// The same run keeps one logical operation across MCP, the bridge and a Review tool.
pub fn record_review_operation(store: &mut dyn OperationStore, review: &ReviewOperation) -> Result<(), Refusal> {
    store.record_review(review).map_err(store_refusal)
}

pub fn operation_for_review(store: &dyn OperationStore, run: &RunId) -> Result<Option<OperationId>, Refusal> {
    store.review_operation(run).map_err(store_refusal)
}

fn store_refusal(error: OperationStoreError) -> Refusal {
    Refusal::new(RefusalCode::AuthUnavailable, format!("the operation ledger is unavailable: {error:?}"))
}

fn classify_reply(value: &Value) -> CallOutcome {
    let is_error = value.get("isError") == Some(&Value::Bool(true));
    if !is_error {
        return CallOutcome::Succeeded;
    }
    let Some(body) = reply_body(value) else {
        let text = reply_text(value).unwrap_or_default();
        if text.starts_with("still running:") || text.contains("was lost while") {
            return CallOutcome::Unknown;
        }
        let no_effect = text.starts_with("not run:") || text.contains("was not sent") || text.starts_with("the app is not running");
        return if no_effect { CallOutcome::FailedNoEffect } else { CallOutcome::Unknown };
    };
    if body.get("pending") == Some(&Value::Bool(true)) || body.get("outcomeUnknown") == Some(&Value::Bool(true)) {
        return CallOutcome::Unknown;
    }
    // A typed proof that the request never reached the engine.
    if body.get("notSent") == Some(&Value::Bool(true)) {
        return CallOutcome::FailedNoEffect;
    }
    // A typed checkpoint result: restored means no retained change; restored:false is the
    // opposite and must be charged.
    if body.get("restored") == Some(&Value::Bool(true)) {
        return CallOutcome::FailedNoEffect;
    }
    if body.get("restored") == Some(&Value::Bool(false)) {
        return CallOutcome::PartialRetained;
    }
    // A batch reports its own applied steps. Full rollback and an empty applied list are
    // positive no-effect proof; a non-empty list is a retained effect.
    if body.get("rolled_back") == Some(&Value::Bool(true)) {
        return CallOutcome::FailedNoEffect;
    }
    if let Some(applied) = body.get("applied").and_then(Value::as_array) {
        return if applied.is_empty() { CallOutcome::FailedNoEffect } else { CallOutcome::PartialRetained };
    }
    // An expectation check runs after the command completed. The write and stamp are retained,
    // so a mismatch is still a charged partial-retained result, never a free failure.
    if body.get("expectation").is_some_and(|expectation| expectation.get("ok") != Some(&Value::Bool(true))) {
        return CallOutcome::PartialRetained;
    }
    // Other typed batch failures: a checkpoint failure or zero completed steps proves that no
    // step ran. Anything else may have retained a partial effect, and stays unknown.
    if body.get("failed").is_some() {
        if body.get("failed").and_then(|failed| failed.get("stage")).and_then(Value::as_str) == Some("checkpoint") {
            return CallOutcome::FailedNoEffect;
        }
        if body.get("completed").and_then(Value::as_u64) == Some(0) && body.get("results").and_then(Value::as_array).is_none_or(Vec::is_empty) {
            return CallOutcome::FailedNoEffect;
        }
    }
    CallOutcome::Unknown
}

fn reply_body(value: &Value) -> Option<Value> {
    if let Some(structured) = value.get("structuredContent") {
        return Some(structured.clone());
    }
    let text = value.get("content").and_then(Value::as_array).and_then(|content| content.last())?.get("text")?.as_str()?;
    serde_json::from_str(text).ok()
}

fn reply_text(value: &Value) -> Option<&str> {
    value.get("content").and_then(Value::as_array).and_then(|content| content.last())?.get("text")?.as_str()
}

fn decorate_unknown(value: &mut Value, operation: &OperationId) {
    if let Some(object) = value.as_object_mut() {
        object.insert("operationId".into(), json!(operation.as_str()));
        match object.get_mut("structuredContent").and_then(Value::as_object_mut) {
            Some(structured) => {
                structured.insert("code".into(), json!("OP_OUTCOME_UNKNOWN"));
                structured.insert("operationId".into(), json!(operation.as_str()));
            }
            None => {
                object.insert("structuredContent".into(), json!({"code":"OP_OUTCOME_UNKNOWN","operationId":operation.as_str()}));
            }
        }
    }
}

fn error_result(body: Value) -> Value {
    let text = serde_json::to_string_pretty(&body).unwrap_or_default();
    json!({"content":[{"type":"text","text":text}],"structuredContent":body,"isError":true})
}

fn refusal_wire(code: RefusalCode) -> &'static str {
    match code {
        RefusalCode::AuthRequired => "AUTH_REQUIRED",
        RefusalCode::EmailUnverified => "EMAIL_UNVERIFIED",
        RefusalCode::AccountDisabled => "ACCOUNT_DISABLED",
        RefusalCode::CapabilityDenied => "CAPABILITY_DENIED",
        RefusalCode::VersionUnsupported => "VERSION_UNSUPPORTED",
        RefusalCode::ProtocolUnsupported => "PROTOCOL_UNSUPPORTED",
        RefusalCode::UnsafeToStart => "UNSAFE_TO_START",
        RefusalCode::CreditsExhausted => "CREDITS_EXHAUSTED",
        RefusalCode::CreditsHeld => "CREDITS_HELD",
        RefusalCode::OperationConflict => "OPERATION_CONFLICT",
        RefusalCode::OperationFinalized => "OPERATION_FINALIZED",
        RefusalCode::OperationOwnedElsewhere => "OPERATION_OWNED_ELSEWHERE",
        RefusalCode::AuthUnavailable => "AUTH_UNAVAILABLE",
    }
}

/// A refusal raised before any tool dispatch (no context, unclassified entry, unverified budget).
pub(crate) fn refusal_result(error: Refusal) -> Value {
    error_result(json!({
        "error": error.message,
        "errorDetails": {"code": "OP_REFUSED", "reason": refusal_wire(error.code)},
    }))
}

/// The generic response for finalize/lookup after the known local deadline: it names no state
/// and carries no operation id, so a caller cannot infer a charge or a release from it.
pub fn expired_state_response() -> Value {
    json!({
        "error": {"code": "AUTH_REQUIRED", "message": "no operation state is available"},
        "retry": "never",
    })
}
