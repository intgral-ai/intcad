//! Rig agent construction and run helpers.
//!
//! One agent builder, one model, one Rig tool registry. Plain chat is the same
//! agent with no task hook attached; it is not a second loop/provider/session.

use rig::agent::{Agent, AgentBuilder};
use rig::test_utils::MockCompletionModel;

use crate::controller::TaskController;
use crate::hooks::TaskControlHook;
use crate::tools::{CadApplyTool, CadCaptureSnapshotTool, CadInspectTool, CadUndoTool, CadValidateTool};
use crate::types::{TaskResult, TaskTerminal};

pub const PREAMBLE: &str = "You are the Text-to-CAD skill. Use the cad_* tools. \
The host owns document versions, permissions, snapshots, undo and validation. \
Never claim success until cad_validate returns passed. If validation fails, repair within the budget.";

pub fn build_agent(model: MockCompletionModel, controller: &TaskController) -> Agent {
    AgentBuilder::new(model)
        .preamble(PREAMBLE)
        .tool(CadInspectTool { controller: controller.clone() })
        .tool(CadCaptureSnapshotTool { controller: controller.clone() })
        .tool(CadApplyTool { controller: controller.clone() })
        .tool(CadValidateTool { controller: controller.clone() })
        .tool(CadUndoTool { controller: controller.clone() })
        .build()
}

/// Run a CAD task through Rig. The task hook is the only extension point.
pub async fn run_task(agent: &Agent, controller: &TaskController, prompt: &str, max_turns: usize) -> TaskResult {
    let result = agent
        .prompt(prompt)
        .max_turns(max_turns)
        .add_hook(TaskControlHook::new(controller.clone()))
        .run()
        .await;
    if let Err(error) = &result {
        // `on_run_settled` normally records the truthful terminal; this keeps
        // a pre-turn provider failure from being mistaken for success.
        controller.settle_error(&error.to_string());
    }
    controller.result()
}

/// Plain chat: the same agent, no CAD task hook. The CAD policy is not global.
pub async fn run_plain(agent: &Agent, prompt: &str) -> Result<String, String> {
    agent.prompt(prompt).max_turns(2).await.map(|response| response.output()).map_err(|error| error.to_string())
}

pub fn terminal_is(result: &TaskResult, terminal: TaskTerminal) -> bool {
    result.terminal == terminal
}
