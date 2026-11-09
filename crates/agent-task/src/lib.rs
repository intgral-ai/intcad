//! Native AI harness Phase 0 task contract.
//!
//! UI-independent: nothing here knows egui, a window, a model provider, an agent loop or a tool
//! registry. A host adapter supplies [`ports::ToolPort`], [`ports::ValidationPort`] and
//! [`controller::TaskController::on_tool_request`] from its existing control/gate boundary.
//!
//! The controller is a host-side policy/lifecycle layer. It never prompts a model on its own and
//! never decides *how* to model; it decides whether the host may report success.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod contract;
pub mod controller;
pub mod ports;
pub mod skill;

#[cfg(test)]
mod tests;

pub use contract::{
    ApprovalMode, ArtifactRef, Budget, CheckResult, DocumentRef, PermissionContext, RequirementsRef, RunId, SnapshotPurpose, SnapshotRef, TaskEvent,
    TaskId, TaskResult, TaskSpec, TaskState, TaskTerminal, ToolOutcome, ValidationProfileRef, ValidationReport,
};
pub use controller::{TaskController, TaskError, ToolDecision, TurnAction};
pub use ports::{Clock, InMemoryStore, ManualClock, MemoryPort, NoMemory, PortError, SystemClock, TaskStore, ToolPort, ValidationPort};
pub use skill::{ANATOMY_ID, Skill, SkillError, SkillRegistry, StagePolicy, TEXT_TO_CAD_ID, ToolKind};
