//! #67: the host-owned Text-to-CAD task runtime for the native Chat.
//!
//! One Windows-thread object per skill turn. It freezes the document, selection, connection,
//! skill version and budget, then observes the chat's existing tool boundary: no second agent
//! loop, provider, session, command catalog or write queue. The controller alone may produce a
//! terminal, and only after the host's real snapshot and exact validation pass.
//!
//! The Beta acceptance case is two-stage: the create turn validates an L-shaped bracket
//! (1800 mm² L profile) 5 mm thick with four 2 mm holes; after a person changes thickness to
//! 8 mm, an explicit host revalidation captures the new revision and validates it with the 8 mm
//! profile. The profiles live in `solidcraft-agent-task-engine`; this module only chooses when
//! to run them.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use solidcraft_agent_task as task;
use solidcraft_agent_task_engine::{LiveTask, Revalidation, l_bracket_profile};

use crate::SolveApp;

/// The Beta prompt contract. It names the geometry the host will validate, not a plan the model
/// may replace with its own definition of success.
pub const BETA_CONTRACT: &str = "Acceptance contract (two-stage): (1) create or keep an L-shaped bracket with a 1800 mm² L profile, \
     5 mm thick, with four 2 mm mounting holes; the host validates this 5 mm snapshot. (2) after a person changes the thickness \
     parameter to 8 mm and rebuilds, the host revalidates the new revision with the 8 mm profile. Keep the four holes parametric. \
     The host checks the exact parameter, the four 2 mm holes and the resulting volume on an immutable snapshot; your own \
     message is not evidence of success.";

/// The thickness the create turn must produce.
pub const CREATE_THICKNESS_MM: f64 = 5.0;
/// The thickness the manual change must produce.
pub const TARGET_THICKNESS_MM: f64 = 8.0;

fn budget() -> task::Budget {
    task::Budget::finite(4, 16, 1, 120_000)
}

/// What the host froze before the first model call of this skill turn.
#[derive(Clone, Debug)]
pub struct FrozenTurn {
    pub skill_id: String,
    pub skill_version: String,
    pub document: task::DocumentRef,
    pub selection: Vec<String>,
    pub model: String,
    pub connection: String,
    pub budget: task::Budget,
}

/// The live task plus the host identities the UI and receipts quote.
pub struct Runtime {
    task: LiveTask,
    frozen: FrozenTurn,
    revalidation_profile: solidcraft_engine::validation::CheckProfile,
    result_snapshots: Vec<task::SnapshotRef>,
}

impl Runtime {
    fn frozen_label(&self) -> String {
        format!("{} v{} · {} · {}", self.frozen.skill_id, self.frozen.skill_version, self.frozen.model, self.frozen.connection)
    }
}

/// Extend a skill's host note with the Beta acceptance contract. Ordinary Chat and Anatomy are
/// unchanged.
pub fn prompt_note(skill: super::skill::Skill, note: String) -> String {
    if skill == super::skill::Skill::TextToCad { format!("{note}\n[{BETA_CONTRACT}]") } else { note }
}

fn paths() -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let candidates = solidcraft_engine::cmd::candidate_store_root().ok_or("this build has no candidate store")?;
    let store = std::env::var_os("SOLIDCRAFT_VALIDATION_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| solidcraft_engine::recovery::data_dir().map(|dir| dir.join("validation")))
        .ok_or("this build has no validation store")?
        .join("agent-task");
    Ok((candidates, store))
}

/// Freeze the turn and open the real task over the one live session. The caller refuses the turn
/// when this fails: an unverified Text-to-CAD turn must not look like a verified one.
pub fn begin(app: &mut SolveApp, binding: &super::skill::Binding) -> Result<(), String> {
    let (candidates, validation_store) = paths()?;
    let create_profile = l_bracket_profile(CREATE_THICKNESS_MM)?;
    let revalidation_profile = l_bracket_profile(TARGET_THICKNESS_MM)?;
    let frozen = FrozenTurn {
        skill_id: binding.skill.id().into(),
        skill_version: binding.version.clone(),
        document: task::DocumentRef { project_id: "local".into(), document_id: binding.document_id.clone(), version: binding.revision },
        selection: binding.selection.clone(),
        model: binding.model.clone(),
        connection: binding.connection.clone(),
        budget: budget(),
    };
    let spec = task::TaskSpec {
        skill_id: task::TEXT_TO_CAD_ID.into(),
        input: BETA_CONTRACT.into(),
        document: frozen.document.clone(),
        selection: frozen.selection.clone(),
        requirements: None,
        budget: frozen.budget.clone(),
    };
    let permission =
        task::PermissionContext { principal: "agent-chat".into(), approval_mode: task::ApprovalMode::Ask, allow_list: Vec::new(), turn_valid: true };
    let mut live = LiveTask::start(&mut app.session, candidates, validation_store, create_profile, spec, permission)?;
    live.on_turn_start()?;
    app.agent_chat.task = Some(Runtime { task: live, frozen, revalidation_profile, result_snapshots: Vec::new() });
    Ok(())
}

/// The turn a request belongs to is this chat's running turn. Deferred user control requests and
/// requests from a replaced turn are not narrowed by this task.
fn owns(app: &SolveApp, turn: Option<&Arc<AtomicBool>>) -> bool {
    let Some(request_turn) = turn else { return false };
    app.agent_chat.turn.as_ref().is_some_and(|live| Arc::ptr_eq(request_turn, &live.active))
}

fn cad_method(method: &str) -> bool {
    matches!(method, "engine.execute" | "command" | "engine.batch" | "engine.command" | "engine.script")
        || method.starts_with("document.")
        || method.starts_with("inspect.")
}

/// Before the existing gate/control dispatch: ask the controller whether this request may run.
/// `Some(reason)` is returned to the agent as an error; nothing reaches the gate or the document.
pub fn before_tool(app: &mut SolveApp, method: &str, params: &Value, turn: Option<&Arc<AtomicBool>>) -> Option<String> {
    if !cad_method(method) || !owns(app, turn) {
        return None;
    }
    let (chat, session) = (&mut app.agent_chat, &mut app.session);
    let runtime = chat.task.as_mut()?;
    match runtime.task.on_tool_request(session, method, params) {
        task::ToolDecision::Allow => None,
        task::ToolDecision::Deny(reason) => Some(reason),
    }
}

/// After the gate executed a request: report the host-observed outcome. A completed write also
/// gets a fresh immutable result snapshot, recorded as task evidence.
pub fn after_tool(app: &mut SolveApp, method: &str, params: &Value, reply: &Value, turn: Option<&Arc<AtomicBool>>) {
    if !cad_method(method) || !owns(app, turn) {
        return;
    }
    let (chat, session) = (&mut app.agent_chat, &mut app.session);
    let Some(runtime) = chat.task.as_mut() else { return };
    let completed = reply.get("ok").and_then(Value::as_bool) == Some(true);
    let outcome = if completed { task::ToolOutcome::Completed } else { task::ToolOutcome::Failed };
    let detail = reply
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| if completed { "the host executed the request".into() } else { "the host refused the request".into() });
    runtime.task.on_tool_result(method, outcome, &detail);
    let is_write = runtime.task.classify(method, params) == task::ToolKind::Write;
    if completed && is_write {
        match runtime.task.capture_result(session) {
            Ok(snapshot) => runtime.result_snapshots.push(snapshot),
            // The write landed but its evidence did not. The controller still validates the live
            // revision at turn end; the host does not pretend a snapshot was captured.
            Err(error) => log::warn!("the task result snapshot was not captured: {error}"),
        }
    }
}

/// Cancel the task before the backend is asked to stop. The controller then reports Cancelled or
/// Unknown at turn end; it never reports success.
pub fn cancel(app: &mut SolveApp, reason: &str) {
    if let Some(runtime) = app.agent_chat.task.as_mut() {
        runtime.task.on_cancel(reason);
    }
}

fn latest_skill_terminal(app: &mut SolveApp, text: String) {
    if let Some(super::Item::Skill { terminal, .. }) = app.agent_chat.items.iter_mut().rev().find(|item| matches!(item, super::Item::Skill { .. })) {
        *terminal = Some(text);
    }
}

fn terminal_label(terminal: task::TaskTerminal) -> String {
    let key = match terminal {
        task::TaskTerminal::ValidatedSuccess => "ui.agent_chat_task_validated",
        task::TaskTerminal::QualityFailed => "ui.agent_chat_task_quality_failed",
        task::TaskTerminal::NeedsRepair => "ui.agent_chat_task_incomplete",
        task::TaskTerminal::Incomplete => "ui.agent_chat_task_incomplete",
        task::TaskTerminal::Cancelled => "ui.agent_chat_task_cancelled",
        task::TaskTerminal::Error => "ui.agent_chat_task_error",
        task::TaskTerminal::Unknown => "ui.agent_chat_task_unknown",
    };
    crate::i18n::tr(key).to_string()
}

fn result_identity(result: &task::TaskResult) -> String {
    let snapshot = result
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "validation_snapshot")
        .or_else(|| result.artifacts.iter().find(|artifact| artifact.kind == "pre_apply_snapshot"));
    let report = result.artifacts.iter().find(|artifact| artifact.kind == "validation_report");
    let snapshot = snapshot.map(|artifact| artifact.id.as_str()).unwrap_or("-");
    let report = report.map(|artifact| artifact.id.as_str()).unwrap_or("-");
    crate::i18n::format("ui.agent_chat_task_identity", &[("snapshot", snapshot), ("report", report)])
}

fn show_terminal(app: &mut SolveApp, result: &task::TaskResult) {
    let frozen = app.agent_chat.task.as_ref().map(Runtime::frozen_label).unwrap_or_default();
    let text = format!("{frozen} · {} · {}", terminal_label(result.terminal), result_identity(result));
    latest_skill_terminal(app, text);
}

fn repair_prompt(prompt: String, round: u32) -> String {
    format!("{prompt}\n[{BETA_CONTRACT}]\nThe host will re-check the exact snapshot after this repair round ({round}).")
}

/// Send the bounded repair prompt to the same native backend. This starts no second loop and
/// creates no second provider/session; it is the next turn of the existing conversation.
fn send_repair(app: &mut SolveApp, prompt: String) {
    let Some(active) = app.agent_chat.turn.as_ref().map(|turn| Arc::clone(&turn.active)) else {
        // The backend already ended the turn; start a fresh host turn.
        let active = Arc::new(AtomicBool::new(true));
        let stamp = app.session.document_stamp();
        app.agent_chat.turn = Some(super::Turn {
            active: Arc::clone(&active),
            start_doc: app.session.doc.clone(),
            start_undo: app.session.undo.len(),
            changes: 0,
            requests: 0,
            limit_told: false,
            tools_available: app.agent_chat.tools_available.unwrap_or(app.agent_chat.tools_port.is_some()),
            tool_called: false,
            replied: false,
            edit_requested: false,
            label: "repair".into(),
        });
        app.agent_chat.state = Some(super::State::Busy);
        app.agent_chat.stopped = false;
        app.agent_chat.steering = false;
        app.agent_chat.ended = None;
        if let Some(backend) = app.agent_chat.backend.as_mut() {
            backend.set_document(&stamp.document_id, stamp.revision);
            backend.set_turn_control(Arc::clone(&active));
            backend.prompt(&[super::backend::PromptPart::Text(prompt)]);
        }
        return;
    };
    let stamp = app.session.document_stamp();
    app.agent_chat.state = Some(super::State::Busy);
    app.agent_chat.ended = None;
    if let Some(backend) = app.agent_chat.backend.as_mut() {
        backend.set_document(&stamp.document_id, stamp.revision);
        backend.set_turn_control(active);
        backend.prompt(&[super::backend::PromptPart::Text(prompt)]);
    }
}

/// The model's turn ended. The controller decides: a bounded repair on the same backend, or a
/// host terminal. The Skill item always carries the host's words, never the model's.
pub fn on_turn_end(app: &mut SolveApp) {
    let action = {
        let Some(runtime) = app.agent_chat.task.as_mut() else { return };
        runtime.task.on_turn_end(&mut app.session)
    };
    match action {
        task::TurnAction::Repair { prompt } => {
            let round = app.agent_chat.task.as_ref().and_then(|runtime| {
                runtime.task.events().iter().rev().find_map(|event| match event {
                    task::TaskEvent::Repair { round, .. } => Some(*round),
                    _ => None,
                })
            });
            let budget_failure =
                app.agent_chat.task.as_mut().and_then(|runtime| runtime.task.on_turn_start().err().map(|error| runtime.task.stop_incomplete(&error)));
            if let Some(result) = budget_failure {
                show_terminal(app, &result);
                return;
            }
            let text = crate::i18n::format("ui.agent_chat_task_repair", &[("round", &round.unwrap_or(0).to_string())]);
            latest_skill_terminal(app, text);
            send_repair(app, repair_prompt(prompt, round.unwrap_or(0)));
        }
        task::TurnAction::Terminal(result) => show_terminal(app, &result),
    }
}

/// Explicit host revalidation after a person changes a parameter by hand. It captures the new
/// revision and runs the same versioned profile. This is a control action, not an automatic
/// watcher: a manual parameter change alone does not start validation.
pub fn revalidate(app: &mut SolveApp) -> Result<Value, String> {
    let revalidation = {
        let Some(runtime) = app.agent_chat.task.as_mut() else {
            return Err("no Text-to-CAD task is bound to this chat".into());
        };
        let profile = runtime.revalidation_profile.clone();
        runtime.task.revalidate_with(&mut app.session, profile)?
    };
    let Revalidation { document, snapshot, report, passed } = revalidation;
    let terminal = if passed { "validated_success" } else { "quality_failed" };
    let value = json!({
        "terminal": terminal,
        "document": document,
        "snapshot": {"id": snapshot.id, "digest": snapshot.digest, "purpose": snapshot.purpose},
        "report": {"runId": report.run_id, "passed": report.passed, "profile": report.profile, "checks": report.checks},
    });
    let label = if passed { crate::i18n::tr("ui.agent_chat_task_validated") } else { crate::i18n::tr("ui.agent_chat_task_quality_failed") };
    let identity = crate::i18n::format("ui.agent_chat_task_identity", &[("snapshot", snapshot.id.as_str()), ("report", report.run_id.as_str())]);
    latest_skill_terminal(app, format!("{label} · {identity}"));
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_chat::backend::{ChatBackend, Event, PromptPart};
    use crate::agent_chat::{Item, State, Turn, skill};
    use crate::control::{ControlRequest, Outcome};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn bracket() -> solidcraft_engine::Session {
        let mut s = solidcraft_engine::Session::default();
        s.execute("parameters.add", &json!({"name": "thickness", "expression": "5 mm"})).unwrap();
        s.execute("sketch.create", &json!({"plane": "XY", "name": "Profile"})).unwrap();
        s.execute("sketch.line", &json!({"points": [[0, 0], [60, 0], [60, 40], [30, 40], [30, 20], [0, 20]], "closed": true})).unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute("solid.extrude", &json!({"distance": "thickness", "body_name": "Bracket"})).unwrap();
        s.execute("sketch.create", &json!({"plane": "XY", "name": "Holes"})).unwrap();
        for (x, y) in [(10.0, 10.0), (20.0, 10.0), (40.0, 10.0), (50.0, 10.0)] {
            s.execute("sketch.point", &json!({"point": [x, y]})).unwrap();
        }
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute("solid.hole", &json!({"sketch": "Holes", "diameter": 2})).unwrap();
        s
    }

    fn start(app: &mut SolveApp) -> (Arc<AtomicBool>, skill::Binding) {
        app.session = bracket();
        let active = Arc::new(AtomicBool::new(true));
        let stamp = app.session.document_stamp();
        app.agent_chat.turn = Some(Turn {
            active: Arc::clone(&active),
            start_doc: app.session.doc.clone(),
            start_undo: app.session.undo.len(),
            changes: 0,
            requests: 0,
            limit_told: false,
            tools_available: true,
            tool_called: false,
            replied: false,
            edit_requested: false,
            label: "task test".into(),
        });
        app.agent_chat.state = Some(State::Busy);
        let binding = skill::Binding::new(
            skill::Skill::TextToCad,
            stamp.document_id,
            stamp.revision,
            "model",
            "connection",
            Vec::new(),
            skill::SkillSupport::Unverified,
        );
        begin(app, &binding).unwrap();
        app.agent_chat.push(Item::Skill {
            title: binding.skill.title().into(),
            version: binding.version.clone(),
            document_id: binding.document_id.clone(),
            revision: binding.revision,
            model: binding.model.clone(),
            selection: 0,
            support: binding.support,
            terminal: None,
        });
        (active, binding)
    }

    fn call(app: &mut SolveApp, active: &Arc<AtomicBool>, method: &str, params: Value) -> Value {
        let (mut req, _receiver) = ControlRequest::new(method, params);
        req.chat_turn = Some(Arc::clone(active));
        match crate::control::handle(app, &egui::Context::default(), &req) {
            Outcome::Done(value) => value,
            Outcome::Screenshot { .. } => panic!("a CAD request did not answer with a value"),
        }
    }

    fn change(thickness: &str) -> (String, Value) {
        ("engine.execute".into(), json!({"command": "parameters.change", "params": {"name": "thickness", "expression": thickness}}))
    }

    #[test]
    fn control_refuses_a_cancelled_task_write_and_does_not_execute_it() {
        let _store = crate::control_job::test_support::CANDIDATE_STORE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = crate::scenario::Harness::new();
        let store = h.dir.join("candidates");
        std::fs::create_dir_all(&store).unwrap();
        solidcraft_engine::cmd::set_candidate_store(Some(store));
        let (active, _binding) = start(&mut h.app);
        cancel(&mut h.app, "the user stopped the message");
        let (method, params) = change("8 mm");
        let reply = call(&mut h.app, &active, &method, params);
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().is_some_and(|error| error.contains("cancelled")), "{reply}");
        assert_eq!(h.app.session.doc.param("thickness").map(|p| p.expr.as_str()), Some("5 mm"));
    }

    #[test]
    fn control_records_the_host_result_and_a_fresh_snapshot_before_validation() {
        let _store = crate::control_job::test_support::CANDIDATE_STORE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = crate::scenario::Harness::new();
        let store = h.dir.join("candidates");
        std::fs::create_dir_all(&store).unwrap();
        solidcraft_engine::cmd::set_candidate_store(Some(store));
        let (active, _binding) = start(&mut h.app);
        let inspect = call(&mut h.app, &active, "document.inspect", json!({"measure": false}));
        assert_eq!(inspect["ok"], true, "{inspect}");
        let (method, params) = change("5 mm");
        let reply = call(&mut h.app, &active, &method, params);
        assert_eq!(reply["ok"], true, "{reply}");
        let snapshots = h.app.agent_chat.task.as_ref().map(|runtime| runtime.result_snapshots.len()).unwrap_or(0);
        assert_eq!(snapshots, 1, "the completed write records one fresh result snapshot");
        on_turn_end(&mut h.app);
        let result = h.app.agent_chat.task.as_ref().and_then(|runtime| runtime.task.result()).cloned().expect("a host terminal");
        assert_eq!(result.terminal, task::TaskTerminal::ValidatedSuccess, "{result:?}");
        let terminal = h.app.agent_chat.items.iter().rev().find_map(|item| match item {
            Item::Skill { terminal, .. } => terminal.clone(),
            _ => None,
        });
        let snapshot = result.artifacts.iter().find(|artifact| artifact.kind == "validation_snapshot").map(|artifact| artifact.id.as_str());
        assert!(terminal.as_deref().is_some_and(|text| snapshot.is_some_and(|id| text.contains(id))), "{terminal:?}");
    }

    struct Recording {
        active: Arc<Mutex<Vec<String>>>,
    }

    impl ChatBackend for Recording {
        fn poll(&mut self) -> Vec<Event> {
            Vec::new()
        }
        fn prompt(&mut self, parts: &[PromptPart]) {
            let text = parts
                .iter()
                .filter_map(|part| match part {
                    PromptPart::Text(text) => Some(text.clone()),
                    PromptPart::Image { .. } | PromptPart::Document { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if let Ok(mut active) = self.active.lock() {
                active.push(text);
            }
        }
        fn cancel(&mut self) {}
        fn new_session(&mut self) {}
        fn answer_permission(&mut self, _key: u64, _option: Option<&str>) {}
        fn authenticate(&mut self, _method: &str) {}
        fn shutdown(&mut self, _wait: bool) {}
    }

    #[test]
    fn a_failed_validation_sends_one_bounded_repair_to_the_same_backend() {
        let _store = crate::control_job::test_support::CANDIDATE_STORE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = crate::scenario::Harness::new();
        let store = h.dir.join("candidates");
        std::fs::create_dir_all(&store).unwrap();
        solidcraft_engine::cmd::set_candidate_store(Some(store));
        let prompts = Arc::new(Mutex::new(Vec::new()));
        h.app.agent_chat.backend = Some(Box::new(Recording { active: Arc::clone(&prompts) }));
        let (active, _binding) = start(&mut h.app);
        let (method, params) = change("6 mm");
        let reply = call(&mut h.app, &active, &method, params);
        assert_eq!(reply["ok"], true, "{reply}");
        on_turn_end(&mut h.app);
        let sent = prompts.lock().map(|prompts| prompts.clone()).unwrap_or_default();
        assert_eq!(sent.len(), 1, "one repair prompt, no new loop: {sent:?}");
        assert!(sent[0].contains("repair round 1"), "{}", sent[0]);
        assert!(sent[0].contains(BETA_CONTRACT), "the repair prompt states the acceptance contract: {}", sent[0]);
        assert_eq!(h.app.agent_chat.turn.as_ref().map(|turn| turn.active.load(Ordering::Relaxed)), Some(true));
        let terminal = h.app.agent_chat.items.iter().rev().find_map(|item| match item {
            Item::Skill { terminal, .. } => terminal.clone(),
            _ => None,
        });
        assert!(terminal.as_deref().is_some_and(|text| text.contains("repair round 1")), "{terminal:?}");
        assert!(h.app.agent_chat.task.as_ref().and_then(|runtime| runtime.task.result()).is_none());
    }

    #[test]
    fn explicit_revalidation_binds_the_new_revision_and_does_not_invent_success() {
        let _store = crate::control_job::test_support::CANDIDATE_STORE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = crate::scenario::Harness::new();
        let store = h.dir.join("candidates");
        std::fs::create_dir_all(&store).unwrap();
        solidcraft_engine::cmd::set_candidate_store(Some(store));
        let (active, _binding) = start(&mut h.app);
        let (method, params) = change("5 mm");
        assert_eq!(call(&mut h.app, &active, &method, params)["ok"], true);
        on_turn_end(&mut h.app);
        let created = h.app.agent_chat.task.as_ref().and_then(|runtime| runtime.task.result()).cloned().expect("the create terminal");
        assert_eq!(created.terminal, task::TaskTerminal::ValidatedSuccess, "the create turn validates 5 mm: {created:?}");
        assert_eq!(h.app.session.doc.param("thickness").map(|p| p.expr.as_str()), Some("5 mm"));

        h.app.session.execute("parameters.change", &json!({"name": "thickness", "expression": "7 mm"})).unwrap();
        let failed = call(&mut h.app, &active, "agent.task.revalidate", json!({}));
        assert_eq!(failed["ok"], true, "{failed}");
        assert_eq!(failed["result"]["terminal"], "quality_failed", "{failed}");
        assert_eq!(failed["result"]["document"]["version"].as_u64(), Some(h.app.session.document_stamp().revision));
        h.app.session.execute("parameters.change", &json!({"name": "thickness", "expression": "8 mm"})).unwrap();
        let passed = call(&mut h.app, &active, "agent.task.revalidate", json!({}));
        assert_eq!(passed["ok"], true, "{passed}");
        assert_eq!(passed["result"]["terminal"], "validated_success", "{passed}");
        assert_eq!(passed["result"]["report"]["passed"], true);
    }
}
