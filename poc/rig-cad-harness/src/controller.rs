//! Host-side CAD task controller. This is a policy/state layer, not a second
//! agent loop: Rig owns the loop; this owns when a write may happen and
//! whether the task may be called successful.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::types::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolGate {
    Allow,
    Skip(String),
    Stop(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnDecision {
    Continue,
    Retry(String),
    Stop(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Created,
    Inspecting,
    Applying,
    Validating,
    Repairing,
    Terminal,
}

struct Core {
    spec: TaskSpec,
    permit: WritePermit,
    phase: Phase,
    document: DocumentStamp,
    inspected: bool,
    base_snapshot: Option<SnapshotRef>,
    final_snapshot: Option<SnapshotRef>,
    last_write: Option<DocumentStamp>,
    validated: Option<ValidationReport>,
    pending_unknown: Option<PendingWrite>,
    repair_rounds: u32,
    tool_calls: u32,
    model_calls: u32,
    terminal: Option<(TaskTerminal, String)>,
    events: Vec<TaskEvent>,
    artifacts: Vec<String>,
    write_seq: u64,
    undo_available: bool,
    /// A committed write is waiting for its final snapshot.
    final_pending: bool,
    cancelled: bool,
    started_at: Instant,
}

#[derive(Clone)]
pub struct TaskController {
    core: Arc<Mutex<Core>>,
    host: Arc<dyn CadHost>,
}

impl TaskController {
    /// Freeze the document and register the one permit this run may write with.
    pub fn start(mut spec: TaskSpec, host: Arc<dyn CadHost>) -> Result<Self, HostError> {
        let document = host.document()?;
        spec.document_ref = document.clone();
        let task_id = spec.task_id.clone();
        let run_id = spec.run_id.clone();
        let nonce = format!("{run_id}-{}", unix_nanos());
        let permit = WritePermit { task_id: task_id.clone(), run_id: run_id.clone(), nonce };
        host.register_permit(&permit)?;
        let core = Core {
            spec,
            permit,
            phase: Phase::Created,
            document: document.clone(),
            inspected: false,
            base_snapshot: None,
            final_snapshot: None,
            last_write: None,
            validated: None,
            pending_unknown: None,
            repair_rounds: 0,
            tool_calls: 0,
            model_calls: 0,
            terminal: None,
            events: vec![TaskEvent::Started { task_id, run_id, document }],
            artifacts: Vec::new(),
            write_seq: 0,
            undo_available: false,
            final_pending: false,
            cancelled: false,
            started_at: Instant::now(),
        };
        Ok(Self { core: Arc::new(Mutex::new(core)), host })
    }

    fn lock(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn host(&self) -> Arc<dyn CadHost> {
        Arc::clone(&self.host)
    }

    pub fn spec(&self) -> TaskSpec {
        self.lock().spec.clone()
    }

    pub fn document(&self) -> DocumentStamp {
        self.lock().document.clone()
    }

    pub fn terminal(&self) -> Option<(TaskTerminal, String)> {
        self.lock().terminal.clone()
    }

    pub fn result(&self) -> TaskResult {
        let core = self.lock();
        let (terminal, reason) = core.terminal.clone().unwrap_or((TaskTerminal::Incomplete, "task has not reached a terminal".into()));
        TaskResult {
            task_id: core.spec.task_id.clone(),
            run_id: core.spec.run_id.clone(),
            terminal,
            reason,
            artifacts: core.artifacts.clone(),
            required_checks: vec![core.spec.requirements_ref.profile_id.clone()],
            pending_unknown_writes: core.pending_unknown.iter().map(|p| p.write_id.clone()).collect(),
            events: core.events.clone(),
        }
    }

    pub fn events(&self) -> Vec<TaskEvent> {
        self.lock().events.clone()
    }

    pub fn cancel(&self, reason: &str) {
        self.host.cancel();
        let mut core = self.lock();
        core.cancelled = true;
        core.phase = Phase::Terminal;
        if core.terminal.is_none() {
            core.terminal = Some((TaskTerminal::Cancelled, reason.into()));
            core.events.push(TaskEvent::Cancelled { reason: reason.into() });
            core.events.push(TaskEvent::Terminal { terminal: TaskTerminal::Cancelled, reason: reason.into() });
        }
    }

    /// A pre-dispatch policy check. This runs before the tool body, so a
    /// forbidden call never reaches the host.
    pub fn tool_dispatch(&self, tool: &str, args: &Value) -> ToolGate {
        let mut core = self.lock();
        core.events.push(TaskEvent::ToolRequested { tool: tool.into(), args: args.clone() });
        if let Some((terminal, reason)) = &core.terminal {
            let reason = format!("task is {terminal:?}: {reason}");
            core.events.push(TaskEvent::ToolDecided { tool: tool.into(), allowed: false, reason: reason.clone() });
            return ToolGate::Stop(reason);
        }
        if core.cancelled || self.host.is_cancelled() {
            let reason = "the turn was cancelled".to_string();
            core.cancelled = true;
            core.terminal = Some((TaskTerminal::Cancelled, reason.clone()));
            core.events.push(TaskEvent::ToolDecided { tool: tool.into(), allowed: false, reason: reason.clone() });
            return ToolGate::Stop(reason);
        }
        if timed_out(&core) {
            let reason = "wall-clock budget exhausted".to_string();
            core.terminal = Some((TaskTerminal::Error, reason.clone()));
            core.events.push(TaskEvent::ToolDecided { tool: tool.into(), allowed: false, reason: reason.clone() });
            return ToolGate::Stop(reason);
        }
        core.tool_calls += 1;
        if core.tool_calls > core.spec.budget.tool_calls {
            let reason = format!("tool budget exhausted ({} calls)", core.spec.budget.tool_calls);
            core.terminal = Some((TaskTerminal::Error, reason.clone()));
            core.events.push(TaskEvent::ToolDecided { tool: tool.into(), allowed: false, reason: reason.clone() });
            return ToolGate::Stop(reason);
        }
        match tool {
            "cad_inspect" | "cad_capture_snapshot" => Self::decide(&mut core, tool, true, "read/host action".into()),
            "cad_apply" => {
                if !core.inspected {
                    return Self::decide(&mut core, tool, false, "inspect the design before the first write".into());
                }
                if let Some(pending) = &core.pending_unknown {
                    let write_id = pending.write_id.clone();
                    return Self::decide(&mut core, tool, false, format!("write {write_id} has an unknown outcome; reconcile it before any other write"));
                }
                if core.phase == Phase::Repairing && core.repair_rounds > core.spec.budget.repair_rounds {
                    let reason = "repair budget exhausted".to_string();
                    core.terminal = Some((TaskTerminal::QualityFailed, reason.clone()));
                    core.events.push(TaskEvent::ToolDecided { tool: tool.into(), allowed: false, reason: reason.clone() });
                    return ToolGate::Stop(reason);
                }
                if !snapshot_matches(core.base_snapshot.as_ref(), &core.document) {
                    return Self::decide(&mut core, tool, false, "capture a snapshot bound to the current document revision before writing".into());
                }
                Self::decide(&mut core, tool, true, "policy passed".into())
            }
            "cad_undo" => {
                if !core.undo_available {
                    return Self::decide(&mut core, tool, false, "there is no write from this task to undo".into());
                }
                Self::decide(&mut core, tool, true, "policy passed".into())
            }
            "cad_validate" => {
                if let Some(pending) = &core.pending_unknown {
                    let write_id = pending.write_id.clone();
                    return Self::decide(&mut core, tool, false, format!("write {write_id} has an unknown outcome; validation cannot pass"));
                }
                if !snapshot_matches(core.final_snapshot.as_ref(), &core.document) {
                    return Self::decide(&mut core, tool, false, "capture a final snapshot bound to the current document revision before validation".into());
                }
                Self::decide(&mut core, tool, true, "policy passed".into())
            }
            other => Self::decide(&mut core, other, false, format!("tool `{other}` is not allowed by this task")),
        }
    }

    fn decide(core: &mut Core, tool: &str, allowed: bool, reason: String) -> ToolGate {
        core.events.push(TaskEvent::ToolDecided { tool: tool.into(), allowed, reason: reason.clone() });
        if allowed { ToolGate::Allow } else { ToolGate::Skip(reason) }
    }

    pub fn inspect(&self) -> Result<InspectView, HostError> {
        let view = self.host.inspect()?;
        let mut core = self.lock();
        core.document = view.document.clone();
        core.inspected = true;
        core.phase = Phase::Inspecting;
        core.events.push(TaskEvent::Inspected { document: view.document.clone() });
        Ok(view)
    }

    /// Capture the baseline before a write, or the final snapshot after one.
    /// The host, not the model, chooses which from the controller state.
    pub fn capture_snapshot(&self) -> Result<SnapshotRef, HostError> {
        let (expected, purpose, as_final) = {
            let core = self.lock();
            if let Some((terminal, _)) = &core.terminal {
                return Err(HostError::Invalid(format!("task is {terminal:?}")));
            }
            let as_final = core.final_pending;
            (core.document.clone(), if as_final { "final" } else { "baseline" }, as_final)
        };
        let snapshot = self.host.capture_snapshot(&expected)?;
        let mut core = self.lock();
        if as_final {
            core.final_snapshot = Some(snapshot.clone());
            core.final_pending = false;
            core.phase = Phase::Validating;
        } else {
            core.base_snapshot = Some(snapshot.clone());
        }
        core.events.push(TaskEvent::Snapshot {
            candidate_id: snapshot.candidate_id.clone(),
            source: snapshot.source.clone(),
            purpose: purpose.into(),
        });
        Ok(snapshot)
    }

    /// Apply through the host with the controller's permit and pinned version.
    pub fn apply(&self, ops: Vec<CadOp>) -> WriteOutcome {
        let (expected, write_id, permit) = {
            let mut core = self.lock();
            if let Some((terminal, reason)) = core.terminal.clone() {
                return WriteOutcome::Denied { reason: format!("task is {terminal:?}: {reason}") };
            }
            if core.cancelled || self.host.is_cancelled() {
                core.terminal = Some((TaskTerminal::Cancelled, "cancelled before write".into()));
                return WriteOutcome::Cancelled { reason: "cancelled before write".into() };
            }
            if let Some(pending) = &core.pending_unknown {
                return WriteOutcome::Denied { reason: format!("unknown write {} is unresolved", pending.write_id) };
            }
            if !snapshot_matches(core.base_snapshot.as_ref(), &core.document) {
                return WriteOutcome::Denied { reason: "no baseline snapshot bound to the current revision".into() };
            }
            core.write_seq += 1;
            let write_id = format!("{}:write-{}", core.spec.run_id, core.write_seq);
            (core.document.clone(), write_id, core.permit.clone())
        };
        let request = WriteRequest {
            write_id: write_id.clone(),
            task_id: self.spec().task_id,
            run_id: self.spec().run_id,
            expected: expected.clone(),
            ops: ops.clone(),
            permit: Some(permit),
        };
        let outcome = match self.host.dispatch_write(request) {
            Ok(outcome) => outcome,
            Err(error) => WriteOutcome::Unknown {
                write_id: write_id.clone(),
                expected: expected.clone(),
                message: format!("host call failed: {error}"),
            },
        };
        // A failed or cancelled engine write may have rolled back and moved
        // the live revision. Re-read the host stamp; never trust a guessed one.
        let after_refresh = match &outcome {
            WriteOutcome::Failed { .. } | WriteOutcome::Cancelled { .. } => self.host.document().ok(),
            _ => None,
        };
        let mut core = self.lock();
        match &outcome {
            WriteOutcome::Applied { after, replayed, .. } => {
                core.document = after.clone();
                core.last_write = Some(after.clone());
                core.final_snapshot = None;
                core.base_snapshot = None;
                core.validated = None;
                core.phase = Phase::Applying;
                core.undo_available = true;
                core.final_pending = true;
                core.events.push(TaskEvent::WriteApplied {
                    write_id: write_id.clone(),
                    before: expected.clone(),
                    after: after.clone(),
                    replayed: *replayed,
                });
            }
            WriteOutcome::Denied { reason } => {
                core.events.push(TaskEvent::WriteDenied { write_id: write_id.clone(), reason: reason.clone() });
            }
            WriteOutcome::Stale { actual, .. } => {
                let reason = "document changed before the write".to_string();
                core.terminal = Some((TaskTerminal::Error, reason));
                core.events.push(TaskEvent::WriteStale { write_id: write_id.clone(), expected: expected.clone(), actual: actual.clone() });
            }
            WriteOutcome::Failed { reason, .. } => {
                if let Some(current) = &after_refresh {
                    core.document = current.clone();
                }
                core.events.push(TaskEvent::WriteFailed { write_id: write_id.clone(), reason: reason.clone() });
            }
            WriteOutcome::Unknown { message, .. } => {
                core.pending_unknown = Some(PendingWrite { write_id: write_id.clone(), expected: expected.clone(), ops });
                core.terminal = Some((TaskTerminal::Unknown, message.clone()));
                core.phase = Phase::Terminal;
                core.events.push(TaskEvent::WriteUnknown { write_id: write_id.clone(), message: message.clone() });
            }
            WriteOutcome::Cancelled { reason } => {
                if let Some(current) = &after_refresh {
                    core.document = current.clone();
                }
                core.cancelled = true;
                core.terminal = Some((TaskTerminal::Cancelled, reason.clone()));
                core.phase = Phase::Terminal;
                core.events.push(TaskEvent::Cancelled { reason: reason.clone() });
            }
        }
        outcome
    }

    pub fn undo(&self) -> WriteOutcome {
        let (expected, write_id, permit) = {
            let mut core = self.lock();
            if core.terminal.is_some() || core.cancelled || !core.undo_available {
                return WriteOutcome::Denied { reason: "undo is not available".into() };
            }
            core.write_seq += 1;
            let id = format!("{}:undo-{}", core.spec.run_id, core.write_seq);
            (core.document.clone(), id, core.permit.clone())
        };
        let request = UndoRequest {
            write_id: write_id.clone(),
            task_id: self.spec().task_id,
            run_id: self.spec().run_id,
            expected: expected.clone(),
            permit: Some(permit),
        };
        let outcome = match self.host.undo(request) {
            Ok(outcome) => outcome,
            Err(error) => WriteOutcome::Unknown {
                write_id: write_id.clone(),
                expected: expected.clone(),
                message: format!("undo host call failed: {error}"),
            },
        };
        let mut core = self.lock();
        if let WriteOutcome::Applied { after, .. } = &outcome {
            core.document = after.clone();
            core.last_write = Some(after.clone());
            core.validated = None;
            core.final_snapshot = None;
            core.base_snapshot = None;
            core.undo_available = false;
            core.final_pending = false;
            core.events.push(TaskEvent::Undone { write_id: write_id.clone(), after: after.clone() });
        }
        outcome
    }

    /// Validate the exact final snapshot with the host's deterministic port.
    pub fn validate(&self) -> Result<ValidationReport, HostError> {
        let (snapshot, profile) = {
            let core = self.lock();
            if core.terminal.is_some() || core.cancelled {
                return Err(HostError::Invalid("the task is not live".into()));
            }
            if let Some(pending) = &core.pending_unknown {
                return Err(HostError::Invalid(format!("unknown write {} is unresolved", pending.write_id)));
            }
            let Some(snapshot) = core.final_snapshot.clone().filter(|s| s.source == core.document) else {
                return Err(HostError::Invalid("no final snapshot bound to the current revision".into()));
            };
            (snapshot, core.spec.requirements_ref.clone())
        };
        let mut core = self.lock();
        core.events.push(TaskEvent::ValidationStarted {
            candidate_id: snapshot.candidate_id.clone(),
            profile_id: profile.profile_id.clone(),
            profile_version: profile.version.clone(),
        });
        drop(core);

        let report = self.host.validate(ValidationRequest {
            task_id: self.spec().task_id,
            run_id: self.spec().run_id,
            snapshot: snapshot.clone(),
            profile: profile.clone(),
        })?;

        let mut core = self.lock();
        core.events.push(TaskEvent::ValidationReport { state: report.state.clone(), checks: report.checks.clone() });
        core.artifacts.push(format!("snapshot:{}", snapshot.candidate_id));
        core.artifacts.push(format!("report:{}", report.run_id));
        if report.passed() && core.pending_unknown.is_none() {
            core.validated = Some(report.clone());
            core.phase = Phase::Terminal;
            core.terminal = Some((TaskTerminal::ValidatedSuccess, "host validation passed".into()));
            core.events.push(TaskEvent::Terminal {
                terminal: TaskTerminal::ValidatedSuccess,
                reason: "host validation passed".into(),
            });
        } else if report.state == "quality_failed" && core.repair_rounds < core.spec.budget.repair_rounds {
            core.repair_rounds += 1;
            core.phase = Phase::Repairing;
            // The failed final snapshot is still bound to the current revision;
            // it is a valid baseline for the one bounded repair write.
            core.base_snapshot = Some(snapshot);
            core.final_snapshot = None;
            core.final_pending = false;
            let round = core.repair_rounds;
            core.events.push(TaskEvent::Repair {
                round,
                reason: format!("validation: {}", report.state),
            });
        } else {
            let terminal = if report.state == "quality_failed" { TaskTerminal::QualityFailed } else { TaskTerminal::Error };
            core.phase = Phase::Terminal;
            core.terminal = Some((terminal.clone(), format!("validation: {}", report.state)));
            core.events.push(TaskEvent::Terminal { terminal, reason: format!("validation: {}", report.state) });
        }
        Ok(report)
    }

    /// Reconcile a lost write exactly once. It is never replayed.
    pub fn reconcile(&self) -> Result<ReconcileOutcome, HostError> {
        let pending = {
            let core = self.lock();
            core.pending_unknown.clone().ok_or_else(|| HostError::Invalid("no unknown write to reconcile".into()))?
        };
        let outcome = self.host.reconcile(&pending)?;
        let mut core = self.lock();
        match &outcome {
            ReconcileOutcome::Applied { after, .. } => {
                core.document = after.clone();
                core.last_write = Some(after.clone());
                core.pending_unknown = None;
                core.terminal = None;
                core.phase = Phase::Applying;
                core.final_pending = true;
                core.events.push(TaskEvent::WriteReconciled { write_id: pending.write_id.clone(), outcome: "applied".into() });
            }
            ReconcileOutcome::NotApplied { current, .. } => {
                core.document = current.clone();
                core.pending_unknown = None;
                core.terminal = None;
                core.phase = Phase::Inspecting;
                core.events.push(TaskEvent::WriteReconciled { write_id: pending.write_id.clone(), outcome: "not_applied".into() });
            }
            ReconcileOutcome::Unknown { message, .. } => {
                core.events.push(TaskEvent::WriteReconciled { write_id: pending.write_id.clone(), outcome: format!("unknown: {message}") });
            }
        }
        Ok(outcome)
    }

    /// Called at the end of every model turn. A tool-free turn cannot be
    /// accepted as task success unless the host validated the final snapshot.
    pub fn turn_end(&self, has_tool_calls: bool) -> TurnDecision {
        let mut core = self.lock();
        core.model_calls += 1;
        if let Some((terminal, reason)) = core.terminal.clone() {
            return if terminal == TaskTerminal::ValidatedSuccess { TurnDecision::Continue } else { TurnDecision::Stop(reason) };
        }
        if timed_out(&core) {
            let reason = "wall-clock budget exhausted before the task terminal".to_string();
            core.phase = Phase::Terminal;
            core.terminal = Some((TaskTerminal::Error, reason.clone()));
            core.events.push(TaskEvent::Terminal { terminal: TaskTerminal::Error, reason: reason.clone() });
            return TurnDecision::Stop(reason);
        }
        if has_tool_calls {
            return TurnDecision::Continue;
        }
        if core.repair_rounds < core.spec.budget.repair_rounds {
            core.repair_rounds += 1;
            let reason = "the host has not validated the final snapshot; capture the final snapshot and run cad_validate, or repair the design".to_string();
            let round = core.repair_rounds;
            core.events.push(TaskEvent::Repair { round, reason: reason.clone() });
            TurnDecision::Retry(reason)
        } else {
            let (terminal, reason) = if core.phase == Phase::Repairing {
                (TaskTerminal::QualityFailed, "host validation failed and the bounded repair budget is exhausted".to_string())
            } else {
                (TaskTerminal::Incomplete, "the model stopped before host validation; the task is incomplete".to_string())
            };
            core.phase = Phase::Terminal;
            core.terminal = Some((terminal.clone(), reason.clone()));
            core.events.push(TaskEvent::Terminal { terminal, reason: reason.clone() });
            TurnDecision::Stop(reason)
        }
    }

    /// A tool returned an error after it may have written.
    pub fn note_tool_error(&self, tool: &str, message: &str) {
        let mut core = self.lock();
        if tool == "cad_apply" || tool == "cad_undo" {
            let write_id = core.pending_unknown.as_ref().map(|p| p.write_id.clone()).unwrap_or_else(|| "tool-error".into());
            if core.pending_unknown.is_none() {
                core.pending_unknown = Some(PendingWrite {
                    write_id: write_id.clone(),
                    expected: core.document.clone(),
                    ops: Vec::new(),
                });
            }
            core.terminal = Some((TaskTerminal::Unknown, message.into()));
            core.events.push(TaskEvent::WriteUnknown { write_id, message: message.into() });
        }
    }

    pub fn is_validated(&self) -> bool {
        self.lock().validated.is_some()
    }

    /// The Rig run ended (error or max turns) without a host terminal. Never
    /// map that to success.
    pub fn settle_error(&self, reason: &str) {
        let mut core = self.lock();
        if core.terminal.is_none() {
            core.phase = Phase::Terminal;
            core.terminal = Some((TaskTerminal::Error, reason.into()));
            core.events.push(TaskEvent::Terminal { terminal: TaskTerminal::Error, reason: reason.into() });
        }
    }

    pub fn settle_incomplete(&self, reason: &str) {
        let mut core = self.lock();
        if core.terminal.is_none() {
            core.phase = Phase::Terminal;
            core.terminal = Some((TaskTerminal::Incomplete, reason.into()));
            core.events.push(TaskEvent::Terminal { terminal: TaskTerminal::Incomplete, reason: reason.into() });
        }
    }

    pub fn pending_unknown(&self) -> Option<PendingWrite> {
        self.lock().pending_unknown.clone()
    }

    /// Test/ACP analysis: a raw host write with no controller permit. The host
    /// must refuse it, proving there is no bypass at the CAD boundary.
    pub fn external_write_without_permit(&self, ops: Vec<CadOp>) -> WriteOutcome {
        let core = self.lock();
        let request = WriteRequest {
            write_id: format!("external:{}", unix_nanos()),
            task_id: "external-acp".into(),
            run_id: "external-acp".into(),
            expected: core.document.clone(),
            ops,
            permit: None,
        };
        drop(core);
        self.host.dispatch_write(request).unwrap_or_else(|error| WriteOutcome::Denied { reason: error.to_string() })
    }

    /// Test evidence: a post-success write invalidates the prior validation.
    pub fn apply_after_success_for_test(&self, ops: Vec<CadOp>) -> WriteOutcome {
        self.apply(ops)
    }

    /// Test evidence: the raw event stream is the task's audit log.
    pub fn push_event_for_test(&self, event: TaskEvent) {
        self.lock().events.push(event);
    }

    /// Serialized view for the headless CLI/report.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self.result()).unwrap_or_else(|error| json!({"error": error.to_string()}))
    }
}

fn snapshot_matches(snapshot: Option<&SnapshotRef>, document: &DocumentStamp) -> bool {
    snapshot.is_some_and(|s| s.source == *document)
}

fn unix_nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn timed_out(core: &Core) -> bool {
    (core.started_at.elapsed().as_millis() as u64) >= core.spec.budget.wall_time_ms
}
