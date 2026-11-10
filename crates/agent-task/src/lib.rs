//! Native AI harness task contract (Beta, #67).
//!
//! UI-independent: nothing here knows egui, a window, a model provider, an agent loop or a tool
//! registry. A host adapter supplies [`ports::ToolPort`], [`ports::ValidationPort`] and its own
//! lifecycle controller from its existing control/gate boundary.
//!
//! This is the first stacked PR: contract, skills and ports. The controller is the next PR.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod contract;
pub mod ports;
pub mod skill;

#[cfg(test)]
mod tests;

pub use contract::{
    ApprovalMode, ArtifactRef, Budget, CheckResult, DocumentRef, PermissionContext, RequirementsRef, RunId, SnapshotPurpose, SnapshotRef, TaskEvent,
    TaskId, TaskResult, TaskSpec, TaskState, TaskTerminal, ToolOutcome, ValidationProfileRef, ValidationReport,
};
pub use ports::{Clock, InMemoryStore, ManualClock, MemoryPort, NoMemory, PortError, SystemClock, TaskStore, ToolPort, ValidationPort};
pub use skill::{ANATOMY_ID, Skill, SkillError, SkillRegistry, StagePolicy, TEXT_TO_CAD_ID, ToolKind};
