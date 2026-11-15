//! The explicit, persistable task contract shared by every host (Chat panel, CLI, CI, scheduler).
//!
//! These are plain serde values: a future host can persist the event stream and resume from it.
//! Phase 0 keeps them in memory; nothing here is derived from model prose.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A stable task identity, allocated by the host.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

/// One execution of a task (a repair gets a new run under the same task).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub String);

impl RunId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

/// The exact document revision a turn was frozen against. The host owns the numbers; the model
/// never supplies or changes them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentRef {
    pub project_id: String,
    pub document_id: String,
    pub version: u64,
}

/// How a host wants CAD writes approved. The task controller only narrows what the existing gate
/// already allows; it is not a second permission system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Ask,
    Allow,
    Deny,
}

/// The turn's permission context, frozen before the first model call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionContext {
    pub principal: String,
    pub approval_mode: ApprovalMode,
    pub allow_list: Vec<String>,
    /// The host's trusted cancellation/expiry flag for this turn.
    pub turn_valid: bool,
}

/// Finite task resources. `0` is a hard zero, never "unlimited".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Budget {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub repair_rounds: u32,
    pub wall_time_ms: u64,
}

impl Budget {
    pub fn finite(model_calls: u32, tool_calls: u32, repair_rounds: u32, wall_time_ms: u64) -> Self {
        Self { model_calls, tool_calls, repair_rounds, wall_time_ms }
    }
}

/// A host-owned, versioned requirements identity (DQ-10/#13 in Phase 0).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequirementsRef {
    pub id: String,
    pub version: String,
    pub digest: String,
}

/// A host-owned, versioned validation profile identity. Model text cannot widen it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationProfileRef {
    pub profile_id: String,
    pub version: String,
    pub digest: String,
}

/// The whole task input, frozen by the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSpec {
    pub skill_id: String,
    pub input: String,
    pub document: DocumentRef,
    pub selection: Vec<String>,
    pub requirements: Option<RequirementsRef>,
    pub budget: Budget,
}

/// A task lifecycle state. Terminal states never transition again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Created,
    Inspecting,
    Planning,
    Applying,
    Checking,
    Validating,
    Repairing,
    Succeeded,
    Failed,
    Incomplete,
    Cancelled,
    Unknown,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Incomplete | Self::Cancelled | Self::Unknown)
    }
}

/// What the host reports to a user. Only the host produces `ValidatedSuccess`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTerminal {
    ValidatedSuccess,
    QualityFailed,
    NeedsRepair,
    Incomplete,
    Cancelled,
    Error,
    Unknown,
}

/// A tool result as the host observed it. `Partial` and `Unknown` are never success and are never
/// replayed blindly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Completed,
    Failed,
    Partial,
    Unknown,
}

/// Why a snapshot was captured. The pre-apply checkpoint and the validation input are different
/// immutable artifacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotPurpose {
    PreApply,
    Validation,
}

/// A host-owned immutable snapshot reference. `id` and `digest` come from the snapshot store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotRef {
    pub id: String,
    pub document: DocumentRef,
    pub digest: String,
    pub purpose: SnapshotPurpose,
}

/// One headless, deterministic check result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckResult {
    pub check_id: String,
    pub passed: bool,
    pub detail: String,
}

/// A trusted validation report from the host's ValidationPort. No LLM produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationReport {
    pub run_id: String,
    pub profile: ValidationProfileRef,
    pub snapshot: SnapshotRef,
    pub document: DocumentRef,
    pub passed: bool,
    pub checks: Vec<CheckResult>,
}

/// An artifact the task produced (pre-apply snapshot, validation snapshot, report).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRef {
    pub kind: String,
    pub id: String,
    pub digest: Option<String>,
}

/// The host's final answer. `terminal` is the only value a UI may present as success.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskResult {
    pub task_id: TaskId,
    pub run_id: RunId,
    pub terminal: TaskTerminal,
    pub reason: String,
    pub artifacts: Vec<ArtifactRef>,
    pub required_checks: Vec<String>,
    pub pending_writes: u32,
    pub unknown_writes: u32,
}

/// One explicit event. A future host persists this stream; Phase 0 keeps it in memory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    Started { task_id: TaskId, run_id: RunId, spec: TaskSpec },
    State { state: TaskState },
    Inspected { document: DocumentRef, summary: String },
    Plan { steps: Vec<String> },
    ToolRequested { tool: String, arguments: Value },
    ToolDecided { tool: String, allowed: bool, reason: String },
    ToolResult { tool: String, outcome: ToolOutcome, detail: String },
    Snapshot { snapshot: SnapshotRef },
    CheckReport { document: DocumentRef, checks: Vec<CheckResult> },
    ValidationReport { report: ValidationReport },
    ValidationInvalidated { reason: String },
    Repair { round: u32, reason: String },
    Cancelled { reason: String },
    Terminal { result: TaskResult },
}
