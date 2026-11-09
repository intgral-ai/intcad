//! The host-owned CAD task controller.
//!
//! It is a policy/lifecycle layer beside the native agent loop: it observes the host's existing
//! tool boundary, requires snapshots and validation itself, and is the only producer of
//! `ValidatedSuccess`. It does not prompt a model, own a provider/session/tool registry or execute
//! a second tool loop.

use std::sync::Arc;

use serde_json::Value;

use crate::contract::{
    ApprovalMode, ArtifactRef, CheckResult, DocumentRef, PermissionContext, RunId, SnapshotPurpose, SnapshotRef, TaskEvent, TaskId, TaskResult,
    TaskSpec, TaskState, TaskTerminal, ToolOutcome, ValidationReport,
};
use crate::ports::{Clock, TaskStore, ToolPort, ValidationPort};
use crate::skill::{Skill, SkillRegistry, ToolKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskError(pub String);

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TaskError {}

impl TaskError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// What the host may do with one model tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolDecision {
    Allow,
    Deny(String),
}

/// What the host should do after the model's turn ended.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnAction {
    /// Ask the same native backend for a bounded repair turn.
    Repair { prompt: String },
    /// The task is over; only this result may be shown as its terminal.
    Terminal(TaskResult),
}

/// The task controller. Every transition and every terminal is explicit and event-recorded.
pub struct TaskController {
    task_id: TaskId,
    run_id: RunId,
    spec: TaskSpec,
    skill: Skill,
    permission: PermissionContext,
    state: TaskState,
    store: Box<dyn TaskStore>,
    clock: Arc<dyn Clock>,
    started_ms: u64,
    start_document: DocumentRef,
    inspected: bool,
    pre_snapshot: Option<SnapshotRef>,
    validation_snapshot: Option<SnapshotRef>,
    validation: Option<ValidationReport>,
    validation_invalidated: bool,
    after_validation_write: bool,
    has_written_ever: bool,
    writes_admitted: u32,
    writes_completed: u32,
    writes_failed: u32,
    writes_pending: u32,
    /// The tool names of admitted writes whose results have not arrived. Results are matched by
    /// name so a read result cannot consume a write's pending slot if a host delivers out of order.
    pending_write_tools: Vec<String>,
    writes_unknown: u32,
    repair_round: u32,
    model_calls: u32,
    tool_calls: u32,
    budget_hit: Option<String>,
    cancelled: bool,
    terminal: Option<TaskResult>,
}

impl TaskController {
    /// Freeze the task input and look up the skill. Call [`TaskController::begin`] next, from the
    /// host thread that owns the CAD session, to inspect and record the plan.
    pub fn start(
        registry: &SkillRegistry,
        spec: TaskSpec,
        permission: PermissionContext,
        clock: Arc<dyn Clock>,
        store: Box<dyn TaskStore>,
    ) -> Result<Self, TaskError> {
        let skill = registry.get(&spec.skill_id).cloned().ok_or_else(|| TaskError::new(format!("unknown skill `{}`", spec.skill_id)))?;
        if spec.document.document_id.trim().is_empty() {
            return Err(TaskError::new("the task has no document"));
        }
        if spec.budget.model_calls == 0 || spec.budget.tool_calls == 0 || spec.budget.wall_time_ms == 0 {
            return Err(TaskError::new("the task budget is not finite"));
        }
        if permission.approval_mode == ApprovalMode::Deny {
            return Err(TaskError::new("the host permission context forbids the task"));
        }
        let now = clock.now_ms();
        let task_id = TaskId::new(format!("task-{}", spec.document.document_id));
        let run_id = RunId::new(format!("run-{}", now));
        let mut controller = Self {
            task_id: task_id.clone(),
            run_id: run_id.clone(),
            start_document: spec.document.clone(),
            spec,
            skill,
            permission,
            state: TaskState::Created,
            store,
            clock,
            started_ms: now,
            inspected: false,
            pre_snapshot: None,
            validation_snapshot: None,
            validation: None,
            validation_invalidated: false,
            after_validation_write: false,
            has_written_ever: false,
            writes_admitted: 0,
            writes_completed: 0,
            writes_failed: 0,
            writes_pending: 0,
            pending_write_tools: Vec::new(),
            writes_unknown: 0,
            repair_round: 0,
            model_calls: 0,
            tool_calls: 0,
            budget_hit: None,
            cancelled: false,
            terminal: None,
        };
        controller.record(TaskEvent::Started { task_id, run_id, spec: controller.spec.clone() });
        Ok(controller)
    }

    /// Inspect the frozen design and record the explicit plan. No model is involved.
    pub fn begin(&mut self, tools: &mut dyn ToolPort) -> Result<(), TaskError> {
        if self.state != TaskState::Created {
            return Err(TaskError::new("the task is already begun"));
        }
        self.transition(TaskState::Inspecting);
        match tools.inspect(&self.start_document) {
            Ok(summary) => {
                self.inspected = true;
                self.record(TaskEvent::Inspected { document: self.start_document.clone(), summary });
            }
            Err(error) => {
                let result = self.finish(TaskTerminal::Error, format!("the design could not be inspected: {error}"));
                let _ = result;
                return Err(TaskError::new("the design could not be inspected"));
            }
        }
        self.transition(TaskState::Planning);
        self.record(TaskEvent::Plan {
            steps: vec![
                "inspect the frozen design".into(),
                "capture an immutable pre-apply snapshot".into(),
                "apply guarded CAD writes through the host gate".into(),
                "run exact host checks".into(),
                "run trusted validation on the final snapshot".into(),
                "report only the host result".into(),
            ],
        });
        Ok(())
    }

    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn state(&self) -> TaskState {
        self.state
    }

    pub fn result(&self) -> Option<&TaskResult> {
        self.terminal.as_ref()
    }

    /// The exact document the host may write against now: the pre-apply snapshot's revision, or
    /// the frozen start revision before the first write. The host passes this to its guarded
    /// executor; it is never supplied by the model.
    pub fn expected_document(&self) -> DocumentRef {
        self.pre_snapshot.as_ref().map(|snapshot| snapshot.document.clone()).unwrap_or_else(|| self.start_document.clone())
    }

    /// The final validated revision, when a validation snapshot was captured.
    pub fn final_document(&self) -> Option<&DocumentRef> {
        self.validation_snapshot.as_ref().map(|snapshot| &snapshot.document)
    }

    pub fn events(&self) -> Vec<TaskEvent> {
        self.store.events()
    }

    pub fn skill(&self) -> &Skill {
        &self.skill
    }

    /// The host's trusted turn flag changed (expiry or revocation). No new write may be admitted.
    pub fn set_turn_valid(&mut self, valid: bool) {
        self.permission.turn_valid = valid;
    }

    /// A new model turn starts: budget accounting before the host prompts the same backend.
    pub fn on_turn_start(&mut self) -> Result<(), TaskError> {
        if self.terminal.is_some() || self.cancelled {
            return Err(TaskError::new("the task is over"));
        }
        self.model_calls = self.model_calls.saturating_add(1);
        if self.model_calls > self.spec.budget.model_calls {
            self.budget_hit = Some(format!("the task used more than {} model calls", self.spec.budget.model_calls));
            return Err(TaskError::new("the model-call budget is exhausted"));
        }
        Ok(())
    }

    /// The host's tool boundary calls this before it admits a request. A denied request never
    /// reaches the existing gate, the control queue, the document or undo.
    pub fn on_tool_request(&mut self, tool: &str, arguments: &Value, tools: &mut dyn ToolPort) -> ToolDecision {
        self.record(TaskEvent::ToolRequested { tool: tool.into(), arguments: arguments.clone() });
        if let Some(reason) = self.blocked() {
            return self.deny(tool, reason);
        }
        self.tool_calls = self.tool_calls.saturating_add(1);
        if self.tool_calls > self.spec.budget.tool_calls {
            self.budget_hit = Some(format!("the task used more than {} tool calls", self.spec.budget.tool_calls));
            return self.deny(tool, "the tool-call budget is exhausted".into());
        }
        let kind = self.skill.classify(tool, arguments);
        match kind {
            ToolKind::Forbidden => self.deny(tool, format!("`{tool}` is not allowed by the {} skill", self.skill.id)),
            ToolKind::Read => {
                if is_inspect(tool, arguments) {
                    self.inspected = true;
                    if !self.state.is_terminal() && self.state != TaskState::Validating {
                        self.transition(TaskState::Inspecting);
                    }
                }
                self.allow(tool)
            }
            ToolKind::Write => self.write_request(tool, tools),
        }
    }

    fn write_request(&mut self, tool: &str, tools: &mut dyn ToolPort) -> ToolDecision {
        if self.skill.stage_policy.require_inspect_before_write && !self.inspected {
            return self.deny(tool, "the design must be inspected before the first write".into());
        }
        if self.validation_snapshot.is_some() || self.validation.is_some() {
            self.after_validation_write = true;
            self.invalidated("a model write was requested after validation");
            return self.deny(tool, "no model write is allowed after validation".into());
        }
        if self.pre_snapshot.is_none() && self.skill.stage_policy.require_snapshot_before_apply {
            let current = match tools.current_document() {
                Ok(current) => current,
                Err(error) => return self.deny(tool, format!("the current document could not be read: {error}")),
            };
            if !self.has_written_ever && current != self.start_document {
                self.transition(TaskState::Incomplete);
                return self.deny(tool, "the document changed before the first write (stale turn)".into());
            }
            match tools.capture_snapshot(&current, SnapshotPurpose::PreApply) {
                Ok(snapshot) if snapshot.document == current => {
                    self.record(TaskEvent::Snapshot { snapshot: snapshot.clone() });
                    self.pre_snapshot = Some(snapshot);
                }
                // Capture reads the live session: if the document moved between the version check
                // and the capture, this snapshot cannot authorize the write.
                Ok(_) => return self.deny(tool, "the pre-apply snapshot is not bound to the checked revision".into()),
                Err(error) => return self.deny(tool, format!("the pre-apply snapshot failed: {error}")),
            }
        }
        self.writes_admitted = self.writes_admitted.saturating_add(1);
        self.writes_pending = self.writes_pending.saturating_add(1);
        self.pending_write_tools.push(tool.to_string());
        self.has_written_ever = true;
        self.transition(TaskState::Applying);
        self.allow(tool)
    }

    /// The host observed the tool's real outcome. Unknown and partial results are never success.
    pub fn on_tool_result(&mut self, tool: &str, outcome: ToolOutcome, detail: &str) {
        // Match the pending write by the tool that requested it, not by re-classifying an empty
        // argument object: a read result must not consume a write's slot.
        let pending_write = self.pending_write_tools.iter().position(|name| name == tool);
        if let Some(index) = pending_write {
            self.pending_write_tools.remove(index);
            self.writes_pending = self.writes_pending.saturating_sub(1);
            match outcome {
                ToolOutcome::Completed => {
                    self.writes_completed = self.writes_completed.saturating_add(1);
                    if !self.state.is_terminal() {
                        self.transition(TaskState::Checking);
                    }
                }
                ToolOutcome::Failed => self.writes_failed = self.writes_failed.saturating_add(1),
                ToolOutcome::Partial => {
                    // A partial write is not success and is not retried blindly; the repair
                    // prompt must state a new explicit plan.
                    self.writes_failed = self.writes_failed.saturating_add(1);
                }
                ToolOutcome::Unknown => {
                    self.writes_unknown = self.writes_unknown.saturating_add(1);
                    self.transition(TaskState::Unknown);
                }
            }
        }
        self.record(TaskEvent::ToolResult { tool: tool.into(), outcome, detail: detail.into() });
    }

    /// The model's turn ended. The host, not the model, decides the terminal.
    pub fn on_turn_end(&mut self, tools: &mut dyn ToolPort, validation: &mut dyn ValidationPort) -> TurnAction {
        if let Some(result) = self.terminal.clone() {
            return TurnAction::Terminal(result);
        }
        if self.cancelled {
            // A write admitted before Stop may have landed: its outcome is unknown, not a clean
            // cancellation. Report that truth; never call it success.
            let terminal = if self.writes_unknown > 0 || self.writes_pending > 0 { TaskTerminal::Unknown } else { TaskTerminal::Cancelled };
            return TurnAction::Terminal(self.finish(terminal, "the task was cancelled; no result is success".into()));
        }
        if self.writes_unknown > 0 {
            return TurnAction::Terminal(self.finish(TaskTerminal::Unknown, "a tool outcome is unknown; it is reported, never replayed".into()));
        }
        if self.writes_pending > 0 {
            return TurnAction::Terminal(
                self.finish(TaskTerminal::Unknown, "a tool result was lost before the turn ended; it is not replayed".into()),
            );
        }
        if let Some(reason) = self.budget_hit.clone() {
            return TurnAction::Terminal(self.finish(TaskTerminal::Incomplete, reason));
        }
        let elapsed = self.clock.now_ms().saturating_sub(self.started_ms);
        if elapsed > self.spec.budget.wall_time_ms {
            return TurnAction::Terminal(self.finish(TaskTerminal::Error, "the task reached its wall-time budget".into()));
        }
        let current = match tools.current_document() {
            Ok(current) => current,
            Err(error) => return TurnAction::Terminal(self.finish(TaskTerminal::Error, format!("the final document could not be read: {error}"))),
        };
        if !self.has_written_ever && current != self.start_document {
            return TurnAction::Terminal(
                self.finish(TaskTerminal::Incomplete, "the document changed without a task write; the turn is stale".into()),
            );
        }
        if self.writes_failed > 0 || self.writes_completed == 0 {
            let reason = if self.writes_failed > 0 {
                "a CAD write failed; the design does not meet the requested change".to_string()
            } else {
                "the model ended without a completed CAD write".to_string()
            };
            return self.repair_or_finish(reason, TaskTerminal::Incomplete);
        }
        if self.after_validation_write || self.validation_invalidated {
            return self.repair_or_finish("the design changed after validation".into(), TaskTerminal::QualityFailed);
        }
        let snapshot = match self.validation_snapshot.clone() {
            Some(snapshot) if snapshot.document == current => snapshot,
            _ => match tools.capture_snapshot(&current, SnapshotPurpose::Validation) {
                Ok(snapshot) if snapshot.document == current => {
                    self.record(TaskEvent::Snapshot { snapshot: snapshot.clone() });
                    self.validation_snapshot = Some(snapshot.clone());
                    snapshot
                }
                Ok(_) => {
                    return TurnAction::Terminal(
                        self.finish(TaskTerminal::Error, "the validation snapshot is not bound to the final revision".into()),
                    );
                }
                Err(error) => {
                    return TurnAction::Terminal(self.finish(TaskTerminal::Error, format!("the validation snapshot failed: {error}")));
                }
            },
        };
        let checks = match tools.run_checks(&current, &self.skill.required_checks) {
            Ok(checks) => checks,
            Err(error) => return TurnAction::Terminal(self.finish(TaskTerminal::Error, format!("the exact checks failed to run: {error}"))),
        };
        self.record(TaskEvent::CheckReport { document: current.clone(), checks: checks.clone() });
        if checks.iter().any(|check| !check.passed) {
            return self.repair_or_finish("a required exact check failed".into(), TaskTerminal::QualityFailed);
        }
        self.transition(TaskState::Validating);
        let report = match validation.validate(&snapshot, &self.skill.validation_profile) {
            Ok(report) => report,
            Err(error) => return TurnAction::Terminal(self.finish(TaskTerminal::Error, format!("validation could not run: {error}"))),
        };
        self.record(TaskEvent::ValidationReport { report: report.clone() });
        self.validation = Some(report.clone());
        if let Some(reason) = self.success_blocker(&report, &current, &checks) {
            return self.repair_or_finish(reason, TaskTerminal::QualityFailed);
        }
        TurnAction::Terminal(self.finish(TaskTerminal::ValidatedSuccess, "the host validated the final design snapshot".into()))
    }

    /// Cancel is authoritative. A backend that never reports a turn end still gets a truthful
    /// terminal here.
    pub fn on_cancel(&mut self, reason: &str) {
        if self.terminal.is_some() {
            return;
        }
        self.cancelled = true;
        self.transition(TaskState::Cancelled);
        self.record(TaskEvent::Cancelled { reason: reason.into() });
    }

    /// Cancel and produce the terminal at once (for a host whose loop silently stopped).
    pub fn cancel_now(&mut self, reason: &str) -> TaskResult {
        self.on_cancel(reason);
        let terminal = if self.writes_unknown > 0 { TaskTerminal::Unknown } else { TaskTerminal::Cancelled };
        self.finish(terminal, "the task was cancelled; no result is success".into())
    }

    /// A host/user may reconcile one unknown write; it is never replayed automatically.
    pub fn reconcile_unknown(&mut self, outcome: ToolOutcome, detail: &str) {
        if self.writes_unknown == 0 || self.terminal.is_some() {
            return;
        }
        self.writes_unknown = self.writes_unknown.saturating_sub(1);
        match outcome {
            ToolOutcome::Completed => self.writes_completed = self.writes_completed.saturating_add(1),
            ToolOutcome::Failed | ToolOutcome::Partial => self.writes_failed = self.writes_failed.saturating_add(1),
            ToolOutcome::Unknown => self.writes_unknown = self.writes_unknown.saturating_add(1),
        }
        self.record(TaskEvent::ToolResult { tool: "reconciled".into(), outcome, detail: detail.into() });
        if !self.state.is_terminal() {
            self.transition(TaskState::Checking);
        }
    }

    fn success_blocker(&self, report: &ValidationReport, current: &DocumentRef, checks: &[CheckResult]) -> Option<String> {
        if !self.permission.turn_valid {
            return Some("the turn permission expired".into());
        }
        if !report.passed {
            return Some("trusted validation did not pass".into());
        }
        if report.profile != self.skill.validation_profile {
            return Some("validation used a different profile than the host-owned one".into());
        }
        if report.document != *current || report.snapshot.document != *current {
            return Some("validation is not bound to the final document revision".into());
        }
        if report.snapshot.id != self.validation_snapshot.as_ref().map(|s| s.id.clone()).unwrap_or_default() {
            return Some("validation is not bound to the host's validation snapshot".into());
        }
        if checks.iter().any(|check| !check.passed) {
            return Some("a required exact check failed".into());
        }
        for required in &self.skill.required_checks {
            match report.checks.iter().find(|check| &check.check_id == required) {
                Some(check) if check.passed => {}
                _ => return Some(format!("the required validation check `{required}` did not pass")),
            }
        }
        if self.writes_admitted == 0 || self.writes_completed != self.writes_admitted {
            return Some("not every expected CAD write completed".into());
        }
        if self.writes_failed > 0 {
            return Some("a CAD write failed".into());
        }
        if self.writes_pending > 0 || self.writes_unknown > 0 {
            return Some("a CAD write is pending or unknown".into());
        }
        if self.after_validation_write || self.validation_invalidated {
            return Some("the design changed after validation".into());
        }
        if self.cancelled {
            return Some("the task was cancelled".into());
        }
        None
    }

    fn repair_or_finish(&mut self, reason: String, terminal: TaskTerminal) -> TurnAction {
        if self.repair_round < self.spec.budget.repair_rounds {
            self.begin_repair(&reason);
            return TurnAction::Repair { prompt: repair_prompt(&self.skill.id, self.repair_round, &reason) };
        }
        TurnAction::Terminal(self.finish(terminal, format!("{reason}; the repair budget is exhausted")))
    }

    fn begin_repair(&mut self, reason: &str) {
        self.repair_round = self.repair_round.saturating_add(1);
        self.state = TaskState::Repairing;
        self.record(TaskEvent::State { state: TaskState::Repairing });
        self.record(TaskEvent::Repair { round: self.repair_round, reason: reason.into() });
        self.record(TaskEvent::Plan {
            steps: vec![
                format!("repair round {}: state a new explicit plan", self.repair_round),
                "keep the frozen document identity".into(),
                "capture a fresh pre-apply snapshot".into(),
                "apply only guarded writes".into(),
                "run exact checks and trusted validation again".into(),
            ],
        });
        // Each repair is a new plan and a new validation input; prior per-round write counts do
        // not make a repaired design pass.
        self.pre_snapshot = None;
        self.validation_snapshot = None;
        self.validation = None;
        self.validation_invalidated = false;
        self.after_validation_write = false;
        self.writes_admitted = 0;
        self.writes_completed = 0;
        self.writes_failed = 0;
        self.writes_pending = 0;
        self.pending_write_tools.clear();
        self.writes_unknown = 0;
    }

    fn blocked(&self) -> Option<String> {
        if self.terminal.is_some() {
            return Some("the task is over".into());
        }
        if self.cancelled {
            return Some("the task was cancelled".into());
        }
        if !self.permission.turn_valid {
            return Some("the turn permission expired".into());
        }
        None
    }

    fn allow(&mut self, tool: &str) -> ToolDecision {
        self.record(TaskEvent::ToolDecided { tool: tool.into(), allowed: true, reason: String::new() });
        ToolDecision::Allow
    }

    fn deny(&mut self, tool: &str, reason: String) -> ToolDecision {
        self.record(TaskEvent::ToolDecided { tool: tool.into(), allowed: false, reason: reason.clone() });
        ToolDecision::Deny(reason)
    }

    fn invalidated(&mut self, reason: &str) {
        self.validation_invalidated = true;
        self.record(TaskEvent::ValidationInvalidated { reason: reason.into() });
    }

    fn transition(&mut self, state: TaskState) {
        if self.state == state {
            return;
        }
        self.state = state;
        self.record(TaskEvent::State { state });
    }

    fn record(&mut self, event: TaskEvent) {
        self.store.record(event);
    }

    fn finish(&mut self, terminal: TaskTerminal, reason: String) -> TaskResult {
        let state = match terminal {
            TaskTerminal::ValidatedSuccess => TaskState::Succeeded,
            TaskTerminal::QualityFailed => TaskState::Failed,
            TaskTerminal::NeedsRepair | TaskTerminal::Incomplete => TaskState::Incomplete,
            TaskTerminal::Cancelled => TaskState::Cancelled,
            TaskTerminal::Error => TaskState::Failed,
            TaskTerminal::Unknown => TaskState::Unknown,
        };
        self.transition(state);
        let mut artifacts = Vec::new();
        if let Some(snapshot) = &self.pre_snapshot {
            artifacts.push(ArtifactRef { kind: "pre_apply_snapshot".into(), id: snapshot.id.clone(), digest: Some(snapshot.digest.clone()) });
        }
        if let Some(snapshot) = &self.validation_snapshot {
            artifacts.push(ArtifactRef { kind: "validation_snapshot".into(), id: snapshot.id.clone(), digest: Some(snapshot.digest.clone()) });
        }
        if let Some(report) = &self.validation {
            artifacts.push(ArtifactRef { kind: "validation_report".into(), id: report.run_id.clone(), digest: None });
        }
        let result = TaskResult {
            task_id: self.task_id.clone(),
            run_id: self.run_id.clone(),
            terminal,
            reason,
            artifacts,
            required_checks: self.skill.required_checks.clone(),
            pending_writes: self.writes_pending,
            unknown_writes: self.writes_unknown,
        };
        self.terminal = Some(result.clone());
        self.record(TaskEvent::Terminal { result: result.clone() });
        result
    }
}

fn is_inspect(tool: &str, arguments: &Value) -> bool {
    if matches!(tool, "inspect_design" | "measure" | "body_topology" | "list_commands" | "describe_command" | "check_parameters") {
        return true;
    }
    matches!(tool, "document.inspect" | "document.revision" | "engine.commands" | "engine.describe")
        || arguments.get("command").and_then(Value::as_str).is_some_and(|id| id.starts_with("inspect.") || id == "document.inspect")
}

fn repair_prompt(skill_id: &str, round: u32, reason: &str) -> String {
    format!(
        "[host validation] The {skill_id} task did not pass: {reason}. This is repair round {round}. \
         State a new explicit plan, inspect the current design if needed, then make the smallest guarded change. \
         Do not claim success; the host will validate the final design again."
    )
}
