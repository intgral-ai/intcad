//! Track B-B: an isolated Rig integration spike for the same CAD Task
//! Contract as the native harness.
//!
//! The CAD host is the authority: Rig owns the model/tool loop, but every
//! write needs a controller-minted permit, the pinned document version, a
//! snapshot and post-write validation. The controller's turn-end hook refuses
//! task success without a trusted host report.
//!
//! `mock_host` is labelled MOCK and is only for the state machine and fault
//! injection. `engine_host` runs the real solidcraft engine.

pub mod controller;
pub mod engine_host;
pub mod hooks;
pub mod mock_host;
pub mod rig_runner;
pub mod scenario;
pub mod tools;
pub mod types;

pub use controller::{TaskController, ToolGate, TurnDecision};
pub use types::*;
