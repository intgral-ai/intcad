//! The CAD Task Contract used by this spike: the same shape as
//! `plan/2026-11-09-native-ai-harness-phase0-design.md` §3, reduced to the
//! fields the PoC actually exercises. Nothing here is a Rig type: the task,
//! document, permission and terminal vocabulary is host-owned.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A document identity and its actual design revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentStamp {
    pub document_id: String,
    pub revision: u64,
}

impl DocumentStamp {
    pub fn new(document_id: impl Into<String>, revision: u64) -> Self {
        Self { document_id: document_id.into(), revision }
    }
}

/// One command and its JSON parameters. The host, not the model, decides
/// whether it may run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CadOp {
    pub command: String,
    pub params: Value,
}

impl CadOp {
    pub fn change_parameter(name: &str, expression: &str) -> Self {
        Self {
            command: "parameters.change".into(),
            params: serde_json::json!({"name": name, "expression": expression}),
        }
    }
}

/// A host-permissioned right to write to one task/run. Only the task
/// controller mints these, and the host refuses a write without one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritePermit {
    pub task_id: String,
    pub run_id: String,
    pub nonce: String,
}

impl WritePermit {
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.task_id, self.run_id, self.nonce)
    }
}

/// What the user/agent may do in a turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalMode {
    /// Ask the host's approval port for every write.
    Ask,
    /// Only an allow-list may write without asking.
    AllowList(Vec<String>),
    /// No write may run.
    DenyAll,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionContext {
    pub principal: String,
    pub mode: ApprovalMode,
    /// False once the turn is cancelled, stale or finished.
    pub turn_valid: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub task_id: String,
    pub run_id: String,
    pub tool: String,
    pub command: String,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionDecision {
    Allow,
    Deny(String),
}

/// A write the host has been asked to apply.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteRequest {
    pub write_id: String,
    pub task_id: String,
    pub run_id: String,
    pub expected: DocumentStamp,
    pub ops: Vec<CadOp>,
    /// `None` is an external (for example ACP-bridge) caller that did not come
    /// through the task controller. The host must refuse it.
    pub permit: Option<WritePermit>,
}

/// A request to undo the task's last write.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UndoRequest {
    pub write_id: String,
    pub task_id: String,
    pub run_id: String,
    pub expected: DocumentStamp,
    pub permit: Option<WritePermit>,
}

/// The host's answer to a write. `Unknown` means the host cannot prove
/// whether the ops were applied; it must never be retried blindly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WriteOutcome {
    Applied { write_id: String, before: DocumentStamp, after: DocumentStamp, applied_ops: usize, replayed: bool },
    Denied { reason: String },
    Stale { expected: DocumentStamp, actual: DocumentStamp },
    /// The command failed before changing anything.
    Failed { reason: String, unchanged: bool },
    /// The ops may have been applied but the result was lost.
    Unknown { write_id: String, expected: DocumentStamp, message: String },
    Cancelled { reason: String },
}

/// A write whose outcome was lost, kept so it can be reconciled once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingWrite {
    pub write_id: String,
    pub expected: DocumentStamp,
    pub ops: Vec<CadOp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ReconcileOutcome {
    /// The write was applied; `after` is the current stamp.
    Applied { write_id: String, after: DocumentStamp },
    /// The write was not applied; the document still has `current`.
    NotApplied { write_id: String, current: DocumentStamp },
    /// Still unknown; do not write and do not claim success.
    Unknown { write_id: String, message: String },
}

/// An immutable validation input produced by the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRef {
    pub candidate_id: String,
    pub source: DocumentStamp,
    pub candidate_digest: String,
    pub manifest_digest: String,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub size_bytes: u64,
    pub media_type: String,
}

/// The host-owned identity of a validation profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationProfile {
    pub profile_id: String,
    pub version: String,
    pub digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_id: String,
    pub required: bool,
    pub state: String,
    pub expected: Value,
    pub actual: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub run_id: String,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub profile_id: String,
    pub profile_version: String,
    /// `passed`, `quality_failed`, `infrastructure_error`, `cancelled`,
    /// `unknown`.
    pub state: String,
    pub checks: Vec<CheckResult>,
    pub error: Option<String>,
}

impl ValidationReport {
    pub fn passed(&self) -> bool {
        self.state == "passed" && self.checks.iter().filter(|c| c.required).all(|c| c.state == "passed")
    }
}

/// What the model may read before writing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InspectView {
    pub document: DocumentStamp,
    pub body_count: usize,
    pub volume_mm3: Option<f64>,
    pub hole_count: Option<usize>,
    pub parameters: Value,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationRequest {
    pub task_id: String,
    pub run_id: String,
    pub snapshot: SnapshotRef,
    pub profile: ValidationProfile,
}

/// The host boundary for CAD: permission, expected version, snapshot,
/// validation, undo and unknown-write reconciliation. There is deliberately
/// no raw engine handle in this trait.
pub trait CadHost: Send + Sync {
    fn document(&self) -> Result<DocumentStamp, HostError>;
    fn inspect(&self) -> Result<InspectView, HostError>;
    fn capture_snapshot(&self, expected: &DocumentStamp) -> Result<SnapshotRef, HostError>;
    fn dispatch_write(&self, request: WriteRequest) -> Result<WriteOutcome, HostError>;
    fn undo(&self, request: UndoRequest) -> Result<WriteOutcome, HostError>;
    fn validate(&self, request: ValidationRequest) -> Result<ValidationReport, HostError>;
    fn reconcile(&self, pending: &PendingWrite) -> Result<ReconcileOutcome, HostError>;
    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision;
    /// Register the one permit the task controller may use. A host keeps only
    /// registered permits; a fabricated one is refused by `dispatch_write`.
    fn register_permit(&self, permit: &WritePermit) -> Result<(), HostError>;
    fn revoke_permit(&self, permit: &WritePermit) -> Result<(), HostError>;
    fn cancel(&self);
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("host unavailable: {0}")]
    Unavailable(String),
    #[error("host rejected the request: {0}")]
    Invalid(String),
    #[error("document changed: expected {expected:?}, actual {actual:?}")]
    Stale { expected: DocumentStamp, actual: DocumentStamp },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub repair_rounds: u32,
    pub wall_time_ms: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self { model_calls: 16, tool_calls: 32, repair_rounds: 1, wall_time_ms: 120_000 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub task_id: String,
    pub run_id: String,
    pub skill_id: String,
    pub input: String,
    pub document_ref: DocumentStamp,
    pub requirements_ref: ValidationProfile,
    pub budget: Budget,
}

impl TaskSpec {
    pub fn l_bracket() -> Self {
        Self {
            task_id: "task-l-bracket".into(),
            run_id: "run-l-bracket-1".into(),
            skill_id: "text-to-cad".into(),
            input: "L-shaped bracket, 5 mm thick, four mounting holes; then thickness 8 mm, keep hole parametric relations.".into(),
            document_ref: DocumentStamp::new("doc-l-bracket", 0),
            requirements_ref: ValidationProfile {
                profile_id: "l-bracket-task".into(),
                version: "1.0.0".into(),
                digest: None,
            },
            budget: Budget::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskTerminal {
    ValidatedSuccess,
    QualityFailed,
    NeedsRepair,
    Incomplete,
    Cancelled,
    Error,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TaskEvent {
    Started { task_id: String, run_id: String, document: DocumentStamp },
    Inspected { document: DocumentStamp },
    Snapshot { candidate_id: String, source: DocumentStamp, purpose: String },
    ToolRequested { tool: String, args: Value },
    ToolDecided { tool: String, allowed: bool, reason: String },
    WriteApplied { write_id: String, before: DocumentStamp, after: DocumentStamp, replayed: bool },
    WriteDenied { write_id: String, reason: String },
    WriteStale { write_id: String, expected: DocumentStamp, actual: DocumentStamp },
    WriteFailed { write_id: String, reason: String },
    WriteUnknown { write_id: String, message: String },
    WriteReconciled { write_id: String, outcome: String },
    ValidationStarted { candidate_id: String, profile_id: String, profile_version: String },
    ValidationReport { state: String, checks: Vec<CheckResult> },
    Repair { round: u32, reason: String },
    Undone { write_id: String, after: DocumentStamp },
    Cancelled { reason: String },
    Terminal { terminal: TaskTerminal, reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub run_id: String,
    pub terminal: TaskTerminal,
    pub reason: String,
    pub artifacts: Vec<String>,
    pub required_checks: Vec<String>,
    pub pending_unknown_writes: Vec<String>,
    pub events: Vec<TaskEvent>,
}

impl TaskResult {
    pub fn validated(&self) -> bool {
        self.terminal == TaskTerminal::ValidatedSuccess && self.pending_unknown_writes.is_empty()
    }
}
