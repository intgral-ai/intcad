//! Unit tests for the Tool/Turn/Task state machines, budget, cancel, stale, partial/unknown,
//! repair and terminal truth. These use fake ports only: the real engine adapter is in `ui-egui`.

use serde_json::{Value, json};

use crate::contract::*;
use crate::controller::{TaskController, ToolDecision, TurnAction};
use crate::ports::*;
use crate::skill::*;

const DOC: &str = "doc-1";

fn profile() -> ValidationProfileRef {
    ValidationProfileRef { profile_id: "native-basic".into(), version: "1.0.0".into(), digest: "sha256:host".into() }
}

fn budget(repairs: u32) -> Budget {
    Budget::finite(8, 32, repairs, 60_000)
}

fn spec(skill: &str, repairs: u32) -> TaskSpec {
    TaskSpec {
        skill_id: skill.into(),
        input: "make the plate 40 mm wide".into(),
        document: DocumentRef { project_id: "p".into(), document_id: DOC.into(), version: 1 },
        selection: Vec::new(),
        requirements: None,
        budget: budget(repairs),
    }
}

fn permission() -> PermissionContext {
    PermissionContext { principal: "test".into(), approval_mode: ApprovalMode::Ask, allow_list: Vec::new(), turn_valid: true }
}

fn registry(repairs: u32) -> SkillRegistry {
    builtins(profile(), budget(repairs)).unwrap()
}

struct FakeTools {
    document: DocumentRef,
    snapshots: u32,
    checks_pass: bool,
    inspect_ok: bool,
}

impl FakeTools {
    fn new() -> Self {
        Self {
            document: DocumentRef { project_id: "p".into(), document_id: DOC.into(), version: 1 },
            snapshots: 0,
            checks_pass: true,
            inspect_ok: true,
        }
    }

    fn did_write(&mut self) {
        self.document.version += 1;
    }
}

impl ToolPort for FakeTools {
    fn inspect(&mut self, _document: &DocumentRef) -> Result<String, PortError> {
        if self.inspect_ok { Ok("plate, 1 feature, parameter width".into()) } else { Err(PortError::new("inspect failed")) }
    }

    fn current_document(&mut self) -> Result<DocumentRef, PortError> {
        Ok(self.document.clone())
    }

    fn capture_snapshot(&mut self, document: &DocumentRef, purpose: SnapshotPurpose) -> Result<SnapshotRef, PortError> {
        self.snapshots += 1;
        Ok(SnapshotRef {
            id: format!("snap-{}", self.snapshots),
            document: document.clone(),
            digest: format!("sha256:snap-{}", self.snapshots),
            purpose,
        })
    }

    fn run_checks(&mut self, _document: &DocumentRef, required: &[String]) -> Result<Vec<CheckResult>, PortError> {
        Ok(required.iter().map(|id| CheckResult { check_id: id.clone(), passed: self.checks_pass, detail: "host exact check".into() }).collect())
    }
}

struct FakeValidation {
    pass: bool,
    required: Vec<String>,
    calls: u32,
}

impl FakeValidation {
    fn new(pass: bool) -> Self {
        Self {
            pass,
            required: vec!["geometry.exact".into(), "input.integrity".into(), "native.rebuild".into(), "parameters.required".into()],
            calls: 0,
        }
    }
}

impl ValidationPort for FakeValidation {
    fn validate(&mut self, snapshot: &SnapshotRef, profile: &ValidationProfileRef) -> Result<ValidationReport, PortError> {
        self.calls += 1;
        Ok(ValidationReport {
            run_id: format!("validation-{}", self.calls),
            profile: profile.clone(),
            snapshot: snapshot.clone(),
            document: snapshot.document.clone(),
            passed: self.pass,
            checks: self
                .required
                .iter()
                .map(|id| CheckResult { check_id: id.clone(), passed: self.pass, detail: "trusted validation".into() })
                .collect(),
        })
    }
}

fn controller(repairs: u32) -> (SkillRegistry, TaskController, FakeTools, FakeValidation) {
    let registry = registry(repairs);
    let mut controller = TaskController::start(
        &registry,
        spec(TEXT_TO_CAD_ID, repairs),
        permission(),
        std::sync::Arc::new(SystemClock),
        Box::new(InMemoryStore::new()),
    )
    .unwrap();
    let mut tools = FakeTools::new();
    controller.begin(&mut tools).unwrap();
    (registry, controller, tools, FakeValidation::new(true))
}

fn write(controller: &mut TaskController, tools: &mut FakeTools, validation: &mut FakeValidation, tool: &str, args: Value) -> TurnAction {
    assert_eq!(controller.on_tool_request(tool, &args, tools), ToolDecision::Allow);
    tools.did_write();
    controller.on_tool_result(tool, ToolOutcome::Completed, "applied");
    controller.on_turn_end(tools, validation)
}

#[test]
fn happy_path_is_validated_only_by_the_host() {
    let (_, mut controller, mut tools, mut validation) = controller(1);
    let action = write(
        &mut controller,
        &mut tools,
        &mut validation,
        "execute",
        json!({"command": "parameters.change", "params": {"name": "width", "expression": "40 mm"}}),
    );
    let TurnAction::Terminal(result) = action else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess, "{result:?}");
    assert_eq!(controller.state(), TaskState::Succeeded);
    assert!(result.artifacts.iter().any(|a| a.kind == "pre_apply_snapshot"));
    assert!(result.artifacts.iter().any(|a| a.kind == "validation_snapshot"));
    assert!(result.artifacts.iter().any(|a| a.kind == "validation_report"));
    assert_eq!(validation.calls, 1);
    let events = controller.events();
    assert!(events.iter().any(|e| matches!(e, TaskEvent::Inspected { .. })));
    assert!(events.iter().any(|e| matches!(e, TaskEvent::Snapshot { snapshot } if snapshot.purpose == SnapshotPurpose::PreApply)));
    assert!(events.iter().any(|e| matches!(e, TaskEvent::Snapshot { snapshot } if snapshot.purpose == SnapshotPurpose::Validation)));
    assert!(events.iter().any(|e| matches!(e, TaskEvent::Terminal { result } if result.terminal == TaskTerminal::ValidatedSuccess)));
}

#[test]
fn early_model_end_cannot_be_success() {
    let (_, mut controller, mut tools, mut validation) = controller(0);
    let action = controller.on_turn_end(&mut tools, &mut validation);
    let TurnAction::Terminal(result) = action else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::Incomplete);
    assert_ne!(result.terminal, TaskTerminal::ValidatedSuccess);
    assert!(result.reason.contains("without a completed CAD write"));
    assert_eq!(validation.calls, 0, "no write means no validation is claimed");
}

#[test]
fn a_write_before_inspect_is_denied() {
    let registry = registry(0);
    let mut controller =
        TaskController::start(&registry, spec(TEXT_TO_CAD_ID, 0), permission(), std::sync::Arc::new(SystemClock), Box::new(InMemoryStore::new()))
            .unwrap();
    let mut tools = FakeTools::new();
    let decision = controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools);
    assert!(matches!(decision, ToolDecision::Deny(reason) if reason.contains("inspected")));
    controller.begin(&mut tools).unwrap();
    assert_eq!(controller.on_tool_request("inspect_design", &json!({}), &mut tools), ToolDecision::Allow);
}

#[test]
fn a_failed_validation_requests_a_bounded_repair_then_stops() {
    let (_, mut controller, mut tools, mut validation) = controller(1);
    validation.pass = false;
    let action = write(&mut controller, &mut tools, &mut validation, "execute", json!({"command": "solid.box", "params": {}}));
    let TurnAction::Repair { prompt } = action else { panic!("expected repair, got {action:?}") };
    assert!(prompt.contains("repair round 1"));
    assert_eq!(controller.state(), TaskState::Repairing);
    // The repair turn applies another write but validation still fails; the budget is exhausted.
    let action = write(&mut controller, &mut tools, &mut validation, "execute", json!({"command": "solid.box", "params": {}}));
    let TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
    assert_eq!(result.terminal, TaskTerminal::QualityFailed);
    assert!(result.reason.contains("repair budget"));
}

#[test]
fn a_repair_can_succeed_with_the_same_native_backend() {
    let (_, mut controller, mut tools, mut validation) = controller(1);
    validation.pass = false;
    let action = write(&mut controller, &mut tools, &mut validation, "execute", json!({"command": "solid.box", "params": {}}));
    assert!(matches!(action, TurnAction::Repair { .. }));
    validation.pass = true;
    let action = write(&mut controller, &mut tools, &mut validation, "execute", json!({"command": "parameters.change", "params": {}}));
    let TurnAction::Terminal(result) = action else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess);
    assert_eq!(controller.events().iter().filter(|e| matches!(e, TaskEvent::Repair { .. })).count(), 1);
}

#[test]
fn cancel_before_a_write_denies_it_and_is_never_success() {
    let (_, mut controller, mut tools, mut validation) = controller(2);
    controller.on_cancel("the user stopped the message");
    let decision = controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools);
    assert!(matches!(decision, ToolDecision::Deny(_)));
    assert_eq!(tools.snapshots, 0, "no snapshot and no write after cancel");
    let TurnAction::Terminal(result) = controller.on_turn_end(&mut tools, &mut validation) else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::Cancelled);
}

#[test]
fn a_stale_document_before_the_first_write_is_refused() {
    let (_, mut controller, mut tools, mut validation) = controller(0);
    controller.on_tool_request("inspect_design", &json!({}), &mut tools);
    tools.document.version += 5; // someone else changed the document
    let decision = controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools);
    assert!(matches!(decision, ToolDecision::Deny(reason) if reason.contains("stale")));
    assert_eq!(tools.snapshots, 0);
    let TurnAction::Terminal(result) = controller.on_turn_end(&mut tools, &mut validation) else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::Incomplete);
}

#[test]
fn an_unknown_tool_result_is_terminal_and_not_replayed() {
    let (_, mut controller, mut tools, mut validation) = controller(2);
    assert_eq!(controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools), ToolDecision::Allow);
    controller.on_tool_result("execute", ToolOutcome::Unknown, "the bridge reply was lost");
    let TurnAction::Terminal(result) = controller.on_turn_end(&mut tools, &mut validation) else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::Unknown);
    assert_eq!(result.unknown_writes, 1);
    // A later write is denied: no blind replay.
    assert!(matches!(controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools), ToolDecision::Deny(_)));
}

#[test]
fn a_partial_write_is_not_success_and_gets_an_explicit_repair() {
    let (_, mut controller, mut tools, mut validation) = controller(1);
    assert_eq!(controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools), ToolDecision::Allow);
    controller.on_tool_result("execute", ToolOutcome::Partial, "half the feature landed");
    let action = controller.on_turn_end(&mut tools, &mut validation);
    assert!(matches!(action, TurnAction::Repair { .. }), "partial is repaired with a new plan, not replayed");
}

#[test]
fn validation_must_use_the_host_owned_profile_and_bind_the_final_version() {
    let (_, mut controller, mut tools, mut validation) = controller(0);
    // A ValidationPort that returns a report for another profile must not produce success.
    struct WrongProfile;
    impl ValidationPort for WrongProfile {
        fn validate(&mut self, snapshot: &SnapshotRef, profile: &ValidationProfileRef) -> Result<ValidationReport, PortError> {
            let mut wrong = profile.clone();
            wrong.digest = "sha256:other".into();
            Ok(ValidationReport {
                run_id: "wrong".into(),
                profile: wrong,
                snapshot: snapshot.clone(),
                document: snapshot.document.clone(),
                passed: true,
                checks: vec![
                    CheckResult { check_id: "geometry.exact".into(), passed: true, detail: String::new() },
                    CheckResult { check_id: "input.integrity".into(), passed: true, detail: String::new() },
                    CheckResult { check_id: "native.rebuild".into(), passed: true, detail: String::new() },
                    CheckResult { check_id: "parameters.required".into(), passed: true, detail: String::new() },
                ],
            })
        }
    }
    assert_eq!(controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools), ToolDecision::Allow);
    tools.did_write();
    controller.on_tool_result("execute", ToolOutcome::Completed, "applied");
    let action = controller.on_turn_end(&mut tools, &mut WrongProfile);
    let TurnAction::Terminal(result) = action else { panic!("expected terminal") };
    assert_ne!(result.terminal, TaskTerminal::ValidatedSuccess);
    assert!(result.reason.contains("profile"));
    let _ = &mut validation;
}

#[test]
fn a_read_result_cannot_consume_a_pending_write() {
    let (_, mut controller, mut tools, mut validation) = controller(0);
    assert_eq!(controller.on_tool_request("inspect_design", &json!({}), &mut tools), ToolDecision::Allow);
    assert_eq!(controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools), ToolDecision::Allow);
    // An out-of-order read result must not make the host think the write finished.
    controller.on_tool_result("measure", ToolOutcome::Completed, "a reading, not the write");
    tools.did_write();
    controller.on_tool_result("execute", ToolOutcome::Completed, "the write");
    let TurnAction::Terminal(result) = controller.on_turn_end(&mut tools, &mut validation) else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess, "{result:?}");
}

#[test]
fn cancel_after_a_write_leaves_the_outcome_unknown() {
    let (_, mut controller, mut tools, mut validation) = controller(0);
    assert_eq!(controller.on_tool_request("inspect_design", &json!({}), &mut tools), ToolDecision::Allow);
    assert_eq!(controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools), ToolDecision::Allow);
    controller.on_cancel("the user stopped the message before the write result arrived");
    let TurnAction::Terminal(result) = controller.on_turn_end(&mut tools, &mut validation) else { panic!("expected terminal") };
    assert_eq!(result.terminal, TaskTerminal::Unknown);
    assert_eq!(result.pending_writes, 1);
}

#[test]
fn a_pre_apply_snapshot_for_another_revision_is_refused() {
    struct WrongSnapshot(FakeTools);
    impl ToolPort for WrongSnapshot {
        fn inspect(&mut self, document: &DocumentRef) -> Result<String, PortError> {
            self.0.inspect(document)
        }
        fn current_document(&mut self) -> Result<DocumentRef, PortError> {
            self.0.current_document()
        }
        fn capture_snapshot(&mut self, document: &DocumentRef, purpose: SnapshotPurpose) -> Result<SnapshotRef, PortError> {
            let mut snapshot = self.0.capture_snapshot(document, purpose)?;
            snapshot.document.version = snapshot.document.version.saturating_add(1);
            Ok(snapshot)
        }
        fn run_checks(&mut self, document: &DocumentRef, required: &[String]) -> Result<Vec<CheckResult>, PortError> {
            self.0.run_checks(document, required)
        }
    }
    let (_, mut controller, fake, _) = controller(0);
    let mut tools = WrongSnapshot(fake);
    let decision = controller.on_tool_request("inspect_design", &json!({}), &mut tools);
    assert_eq!(decision, ToolDecision::Allow);
    let decision = controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools);
    assert!(matches!(decision, ToolDecision::Deny(reason) if reason.contains("bound")));
}

#[test]
fn a_write_after_a_reported_terminal_is_denied() {
    let (_, mut controller, mut tools, mut validation) = controller(0);
    let TurnAction::Terminal(result) = write(&mut controller, &mut tools, &mut validation, "execute", json!({"command": "solid.box", "params": {}}))
    else {
        panic!("expected terminal")
    };
    assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess);
    assert!(matches!(controller.on_tool_request("execute", &json!({"command": "solid.box", "params": {}}), &mut tools), ToolDecision::Deny(_)));
}
