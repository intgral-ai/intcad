//! Native Chat adapter for the UI-independent task controller.
//!
//! It is not a second agent loop: [`NativeTaskAdapter`] only forwards lifecycle events from the
//! existing `ChatBackend`/control/gate boundary and prompts that same backend. The real engine
//! ports live in the UI-independent `solidcraft-agent-task-engine` crate, so a headless test/CLI
//! uses exactly the same controller and CAD/validation implementation.

pub use solidcraft_agent_task_engine::{EngineTaskPorts, EngineTaskTools, EngineTaskValidation};

use serde_json::Value;

use solidcraft_agent_task as task;

use crate::agent_chat::backend::{ChatBackend, PromptPart};

/// A thin adapter over the existing native backend. It owns no session, provider, loop or tool
/// registry; each call is one lifecycle event from the host's existing boundary.
#[derive(Default)]
pub struct NativeTaskAdapter {
    controller: Option<task::TaskController>,
}

impl NativeTaskAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(
        &mut self,
        registry: &task::skill::SkillRegistry,
        spec: task::TaskSpec,
        permission: task::PermissionContext,
        store: Box<dyn task::TaskStore>,
        tools: &mut dyn task::ToolPort,
    ) -> Result<(), String> {
        let mut controller =
            task::TaskController::start(registry, spec, permission, std::sync::Arc::new(task::SystemClock), store).map_err(|e| e.to_string())?;
        controller.begin(tools).map_err(|e| e.to_string())?;
        self.controller = Some(controller);
        Ok(())
    }

    pub fn on_tool_request(&mut self, tool: &str, arguments: &Value, tools: &mut dyn task::ToolPort) -> task::ToolDecision {
        match self.controller.as_mut() {
            Some(controller) => controller.on_tool_request(tool, arguments, tools),
            None => task::ToolDecision::Deny("no task is running".into()),
        }
    }

    pub fn on_tool_result(&mut self, tool: &str, outcome: task::ToolOutcome, detail: &str) {
        if let Some(controller) = self.controller.as_mut() {
            controller.on_tool_result(tool, outcome, detail);
        }
    }

    pub fn on_turn_end(&mut self, tools: &mut dyn task::ToolPort, validation: &mut dyn task::ValidationPort) -> task::TurnAction {
        match self.controller.as_mut() {
            Some(controller) => controller.on_turn_end(tools, validation),
            None => task::TurnAction::Terminal(task::TaskResult {
                task_id: task::TaskId::new("task-none"),
                run_id: task::RunId::new("run-none"),
                terminal: task::TaskTerminal::Incomplete,
                reason: "no task is running".into(),
                artifacts: Vec::new(),
                required_checks: Vec::new(),
                pending_writes: 0,
                unknown_writes: 0,
            }),
        }
    }

    pub fn on_cancel(&mut self, reason: &str) {
        if let Some(controller) = self.controller.as_mut() {
            controller.on_cancel(reason);
        }
    }

    pub fn set_turn_valid(&mut self, valid: bool) {
        if let Some(controller) = self.controller.as_mut() {
            controller.set_turn_valid(valid);
        }
    }

    pub fn result(&self) -> Option<&task::TaskResult> {
        self.controller.as_ref().and_then(task::TaskController::result)
    }

    /// Prompt the same native backend. This is the only place the adapter touches a model loop.
    pub fn prompt(&mut self, backend: &mut dyn ChatBackend, text: &str) -> Result<(), String> {
        let controller = self.controller.as_mut().ok_or_else(|| "no task is running".to_string())?;
        controller.on_turn_start().map_err(|e| e.to_string())?;
        backend.prompt(&[PromptPart::Text(text.into())]);
        Ok(())
    }

    pub fn events(&self) -> Vec<task::TaskEvent> {
        self.controller.as_ref().map(task::TaskController::events).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny scripted backend: it records the prompt and emits one end. No provider, no loop.
    #[derive(Default)]
    struct Scripted {
        prompts: Vec<String>,
    }

    impl ChatBackend for Scripted {
        fn poll(&mut self) -> Vec<crate::agent_chat::backend::Event> {
            Vec::new()
        }
        fn prompt(&mut self, parts: &[PromptPart]) {
            self.prompts.extend(parts.iter().filter_map(|p| match p {
                PromptPart::Text(text) => Some(text.clone()),
                _ => None,
            }));
        }
        fn cancel(&mut self) {}
        fn new_session(&mut self) {}
        fn answer_permission(&mut self, _key: u64, _option: Option<&str>) {}
        fn authenticate(&mut self, _method: &str) {}
        fn shutdown(&mut self, _wait: bool) {}
    }

    #[test]
    fn the_adapter_prompts_the_same_native_backend_and_denies_without_a_task() {
        let mut backend = Scripted::default();
        let mut adapter = NativeTaskAdapter::new();
        let mut tools = FakeTools;
        assert!(adapter.prompt(&mut backend, "hello").is_err(), "no task is running");
        let _ = &mut tools;
        assert_eq!(adapter.on_tool_request("execute", &serde_json::json!({}), &mut tools), task::ToolDecision::Deny("no task is running".into()));
    }

    #[test]
    fn a_running_task_prompts_the_same_backend() {
        let profile = task::ValidationProfileRef { profile_id: "native-basic".into(), version: "1.0.0".into(), digest: "sha256:x".into() };
        let registry = task::skill::builtins(profile, task::Budget::finite(4, 8, 0, 60_000)).unwrap();
        let spec = task::TaskSpec {
            skill_id: task::TEXT_TO_CAD_ID.into(),
            input: "bracket".into(),
            document: task::DocumentRef { project_id: "p".into(), document_id: "d".into(), version: 1 },
            selection: Vec::new(),
            requirements: None,
            budget: task::Budget::finite(4, 8, 0, 60_000),
        };
        let permission =
            task::PermissionContext { principal: "test".into(), approval_mode: task::ApprovalMode::Ask, allow_list: Vec::new(), turn_valid: true };
        let mut adapter = NativeTaskAdapter::new();
        let mut tools = FakeTools;
        adapter.begin(&registry, spec, permission, Box::new(task::InMemoryStore::new()), &mut tools).unwrap();
        let mut backend = Scripted::default();
        adapter.prompt(&mut backend, "make the bracket").unwrap();
        assert_eq!(backend.prompts, ["make the bracket"]);
    }

    struct FakeTools;
    impl task::ToolPort for FakeTools {
        fn inspect(&mut self, _document: &task::DocumentRef) -> Result<String, task::PortError> {
            Ok(String::new())
        }
        fn current_document(&mut self) -> Result<task::DocumentRef, task::PortError> {
            Ok(task::DocumentRef { project_id: "p".into(), document_id: "d".into(), version: 1 })
        }
        fn capture_snapshot(&mut self, _document: &task::DocumentRef, purpose: task::SnapshotPurpose) -> Result<task::SnapshotRef, task::PortError> {
            Ok(task::SnapshotRef {
                id: "s".into(),
                document: task::DocumentRef { project_id: "p".into(), document_id: "d".into(), version: 1 },
                digest: "d".into(),
                purpose,
            })
        }
        fn run_checks(&mut self, _document: &task::DocumentRef, _required: &[String]) -> Result<Vec<task::CheckResult>, task::PortError> {
            Ok(Vec::new())
        }
    }
}
