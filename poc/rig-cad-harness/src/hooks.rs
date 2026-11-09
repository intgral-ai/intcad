//! The only Rig integration seam: an `AgentHook` that delegates CAD policy to
//! the task controller. It is not a loop; Rig still owns model calls, tool
//! dispatch and history.

use rig::agent::{
    AgentHook, DispatchAction, DispatchEvent, HookContext, ModelTurnAction, ModelTurnFinished, OutcomeAction, OutcomeEvent,
};
use rig::message::AssistantContent;
use serde_json::Value;

use crate::controller::{TaskController, ToolGate, TurnDecision};

#[derive(Clone)]
pub struct TaskControlHook {
    pub controller: TaskController,
}

impl TaskControlHook {
    pub fn new(controller: TaskController) -> Self {
        Self { controller }
    }
}

impl AgentHook for TaskControlHook {
    async fn on_dispatch(&self, _ctx: &HookContext, event: DispatchEvent<'_>) -> DispatchAction {
        let Some(tool) = event.tool_name() else {
            return DispatchAction::proceed();
        };
        let args = event.tool_args().and_then(|args| serde_json::from_str::<Value>(args).ok()).unwrap_or(Value::Null);
        match self.controller.tool_dispatch(tool, &args) {
            ToolGate::Allow => DispatchAction::proceed(),
            ToolGate::Skip(reason) => DispatchAction::skip(reason),
            ToolGate::Stop(reason) => DispatchAction::stop(reason),
        }
    }

    async fn on_outcome(&self, _ctx: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        let Some(tool) = event.tool_name() else {
            return OutcomeAction::proceed();
        };
        if let Some(result) = event.tool_result()
            && let Some(error) = result.error()
        {
            self.controller.note_tool_error(tool, error.message());
        }
        if let Some((terminal, reason)) = self.controller.terminal()
            && terminal != crate::types::TaskTerminal::ValidatedSuccess
        {
            return OutcomeAction::stop(reason);
        }
        OutcomeAction::proceed()
    }

    async fn on_model_turn_finished(&self, _ctx: &HookContext, event: ModelTurnFinished<'_>) -> ModelTurnAction {
        let has_tool_calls = event.content.iter().any(|content| matches!(content, AssistantContent::ToolCall(_)));
        match self.controller.turn_end(has_tool_calls) {
            TurnDecision::Continue => ModelTurnAction::continue_run(),
            TurnDecision::Retry(feedback) => ModelTurnAction::retry_with_feedback(feedback),
            TurnDecision::Stop(reason) => ModelTurnAction::stop(reason),
        }
    }

    async fn on_run_settled(&self, _ctx: &HookContext, event: rig::agent::RunSettled<'_>) {
        // Rig's `Stop` is a run stop, not a task terminal. If the run ended
        // without the controller deciding, record a truthful non-success.
        let _ = event;
        if self.controller.terminal().is_none() {
            self.controller.settle_incomplete("the Rig run ended without a host terminal");
        }
    }
}
