//! Rig tools. Every CAD action is a thin call into the host-side
//! [`TaskController`]; no tool holds an engine or bypasses the host.

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::controller::TaskController;
use crate::types::{CadOp, WriteOutcome};

#[derive(Debug, thiserror::Error)]
pub enum CadToolError {
    #[error("host: {0}")]
    Host(String),
    #[error("policy: {0}")]
    Policy(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotArgs {
    #[serde(default)]
    pub purpose: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyArgs {
    pub ops: Vec<CadOp>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateArgs {
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

pub struct CadInspectTool {
    pub controller: TaskController,
}

impl Tool for CadInspectTool {
    const NAME: &'static str = "cad_inspect";
    type Error = CadToolError;
    type Args = EmptyArgs;
    type Output = Value;

    fn description(&self) -> String {
        "Read the live CAD document: revision, bodies, measured volume, hole count and parameters. Read-only.".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    async fn call(&self, _context: &mut ToolContext, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.controller.inspect().map(|view| json!({"status": "ok", "view": view})).map_err(|error| CadToolError::Host(error.to_string()))
    }
}

pub struct CadCaptureSnapshotTool {
    pub controller: TaskController,
}

impl Tool for CadCaptureSnapshotTool {
    const NAME: &'static str = "cad_capture_snapshot";
    type Error = CadToolError;
    type Args = SnapshotArgs;
    type Output = Value;

    fn description(&self) -> String {
        "Capture an immutable host snapshot of the current document. The host decides whether this is the pre-write baseline or the final validation input.".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"purpose": {"type": "string", "description": "Optional hint; the host decides."}},
            "additionalProperties": false
        })
    }

    async fn call(&self, _context: &mut ToolContext, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.controller
            .capture_snapshot()
            .map(|snapshot| json!({"status": "ok", "snapshot": snapshot}))
            .map_err(|error| CadToolError::Host(error.to_string()))
    }
}

pub struct CadApplyTool {
    pub controller: TaskController,
}

impl Tool for CadApplyTool {
    const NAME: &'static str = "cad_apply";
    type Error = CadToolError;
    type Args = ApplyArgs;
    type Output = Value;

    fn description(&self) -> String {
        "Apply parametric CAD operations through the host. The controller pins the expected document version and issues the only write permit; the model cannot supply either.".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ops": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "command": {"type": "string"},
                            "params": {"type": "object"}
                        },
                        "required": ["command", "params"],
                        "additionalProperties": false
                    }
                },
                "reason": {"type": "string"}
            },
            "required": ["ops"],
            "additionalProperties": false
        })
    }

    async fn call(&self, _context: &mut ToolContext, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let outcome = self.controller.apply(args.ops);
        Ok(write_outcome_json(&outcome))
    }
}

pub struct CadValidateTool {
    pub controller: TaskController,
}

impl Tool for CadValidateTool {
    const NAME: &'static str = "cad_validate";
    type Error = CadToolError;
    type Args = ValidateArgs;
    type Output = Value;

    fn description(&self) -> String {
        "Run the host's deterministic validation profile on the exact final snapshot. Only the host report can make the task successful.".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "profile_id": {"type": "string"},
                "version": {"type": "string"}
            },
            "additionalProperties": false
        })
    }

    async fn call(&self, _context: &mut ToolContext, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.controller
            .validate()
            .map(|report| json!({"status": if report.passed() { "passed" } else { &report.state }, "report": report}))
            .map_err(|error| CadToolError::Policy(error.to_string()))
    }
}

pub struct CadUndoTool {
    pub controller: TaskController,
}

impl Tool for CadUndoTool {
    const NAME: &'static str = "cad_undo";
    type Error = CadToolError;
    type Args = EmptyArgs;
    type Output = Value;

    fn description(&self) -> String {
        "Undo this task's last write through the host transaction. Validation is invalidated.".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    async fn call(&self, _context: &mut ToolContext, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(write_outcome_json(&self.controller.undo()))
    }
}

fn write_outcome_json(outcome: &WriteOutcome) -> Value {
    match outcome {
        WriteOutcome::Applied { before, after, applied_ops, replayed, .. } => json!({
            "status": "applied",
            "before": before,
            "after": after,
            "applied_ops": applied_ops,
            "replayed": replayed
        }),
        WriteOutcome::Denied { reason } => json!({"status": "denied", "reason": reason}),
        WriteOutcome::Stale { expected, actual } => json!({"status": "stale", "expected": expected, "actual": actual}),
        WriteOutcome::Failed { reason, unchanged } => json!({"status": "failed", "reason": reason, "unchanged": unchanged}),
        WriteOutcome::Unknown { write_id, message, .. } => json!({"status": "unknown", "write_id": write_id, "message": message}),
        WriteOutcome::Cancelled { reason } => json!({"status": "cancelled", "reason": reason}),
    }
}
