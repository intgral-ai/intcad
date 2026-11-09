//! Tool-boundary and turn-boundary tests on the labelled mock host.
//! These prove the task state machine; they are not CAD evidence.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use cad_rig_harness::controller::TaskController;
use cad_rig_harness::mock_host::{MockHost, PermissionMode, ValidationMode, WriteMode};
use cad_rig_harness::rig_runner::{build_agent, run_plain, run_task};
use cad_rig_harness::scenario;
use rig::test_utils::{MockCompletionModel, MockTurn};

use cad_rig_harness::types::{
    Budget, CadHost, CadOp, DocumentStamp, PermissionDecision, TaskSpec, TaskTerminal, WriteOutcome, WritePermit, WriteRequest,
};

fn controller() -> (Arc<MockHost>, TaskController) {
    let host = MockHost::l_bracket();
    let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("start");
    (host, controller)
}

fn controller_with_budget(budget: Budget) -> (Arc<MockHost>, TaskController) {
    let host = MockHost::l_bracket();
    let mut spec = TaskSpec::l_bracket();
    spec.budget = budget;
    let controller = TaskController::start(spec, host.clone()).expect("start");
    (host, controller)
}

fn thickness_ops() -> Vec<CadOp> {
    vec![CadOp::change_parameter("thickness", "8 mm")]
}

#[test]
fn host_dispatch_requires_permit_version_and_registration() {
    let (host, controller) = controller();
    controller.inspect().expect("inspect");
    controller.capture_snapshot().expect("baseline snapshot");
    let applied = controller.apply(thickness_ops());
    assert!(matches!(applied, WriteOutcome::Applied { .. }), "{applied:?}");
    assert_eq!(host.revision(), 2);

    // An ACP-style / external caller with no controller permit is refused.
    let bypass = controller.external_write_without_permit(thickness_ops());
    assert!(matches!(bypass, WriteOutcome::Denied { .. }), "{bypass:?}");

    // A fabricated permit the host never registered is refused too.
    let fabricated = WritePermit { task_id: "x".into(), run_id: "y".into(), nonce: "z".into() };
    let request = WriteRequest {
        write_id: "fabricated".into(),
        task_id: "x".into(),
        run_id: "y".into(),
        expected: DocumentStamp::new("doc-l-bracket", 2),
        ops: thickness_ops(),
        permit: Some(fabricated),
    };
    let out = host.dispatch_write(request).expect("host");
    assert!(matches!(out, WriteOutcome::Denied { .. }), "{out:?}");

    // A registered permit still cannot write an old revision.
    let permit = WritePermit { task_id: "t".into(), run_id: "r".into(), nonce: "n".into() };
    host.register_permit(&permit).expect("register");
    let request = WriteRequest {
        write_id: "stale".into(),
        task_id: "t".into(),
        run_id: "r".into(),
        expected: DocumentStamp::new("doc-l-bracket", 1),
        ops: thickness_ops(),
        permit: Some(permit),
    };
    let out = host.dispatch_write(request).expect("host");
    assert!(matches!(out, WriteOutcome::Stale { .. }), "{out:?}");
}

#[test]
fn host_dispatch_is_idempotent_for_one_write_id() {
    let (host, _controller) = controller();
    let permit = WritePermit { task_id: "t".into(), run_id: "r".into(), nonce: "n".into() };
    host.register_permit(&permit).expect("register");
    let request = WriteRequest {
        write_id: "same-write".into(),
        task_id: "t".into(),
        run_id: "r".into(),
        expected: DocumentStamp::new("doc-l-bracket", 1),
        ops: thickness_ops(),
        permit: Some(permit),
    };
    let first = host.dispatch_write(request.clone()).expect("host");
    assert!(matches!(first, WriteOutcome::Applied { .. }), "{first:?}");
    let second = host.dispatch_write(request).expect("host");
    match second {
        WriteOutcome::Applied { replayed, after, .. } => {
            assert!(replayed);
            assert_eq!(after.revision, 2);
        }
        other => panic!("second dispatch was not a replay: {other:?}"),
    }
    assert_eq!(host.revision(), 2, "a duplicate request must not write twice");
}

#[test]
fn permission_deny_blocks_the_write_then_allow_lets_it_through() {
    let (host, controller) = controller();
    host.set_permission_mode(PermissionMode::Ask(VecDeque::from([
        PermissionDecision::Deny("user said no".into()),
        PermissionDecision::Allow,
    ])));
    controller.inspect().expect("inspect");
    controller.capture_snapshot().expect("snapshot");
    let denied = controller.apply(thickness_ops());
    assert!(matches!(denied, WriteOutcome::Denied { .. }), "{denied:?}");
    assert_eq!(host.revision(), 1, "denied write changed nothing");
    let allowed = controller.apply(thickness_ops());
    assert!(matches!(allowed, WriteOutcome::Applied { .. }), "{allowed:?}");
    assert_eq!(host.revision(), 2);
    assert_eq!(host.permission_requests().len(), 2);
}

#[test]
fn stale_document_cannot_be_a_success() {
    let (host, controller) = controller();
    host.set_write_mode(WriteMode::StaleOnce);
    controller.inspect().expect("inspect");
    controller.capture_snapshot().expect("snapshot");
    let out = controller.apply(thickness_ops());
    assert!(matches!(out, WriteOutcome::Stale { .. }), "{out:?}");
    assert_eq!(controller.terminal().map(|(terminal, _)| terminal), Some(TaskTerminal::Error));
    assert!(!controller.result().validated());
    assert!(host.applied_writes().is_empty());
}

#[test]
fn undo_restores_the_document_and_invalidates_validation_inputs() {
    let (host, controller) = controller();
    controller.inspect().expect("inspect");
    controller.capture_snapshot().expect("snapshot");
    let applied = controller.apply(thickness_ops());
    assert!(matches!(applied, WriteOutcome::Applied { .. }), "{applied:?}");
    assert_eq!(host.thickness(), 8.0);
    let undone = controller.undo();
    assert!(matches!(undone, WriteOutcome::Applied { .. }), "{undone:?}");
    assert_eq!(host.thickness(), 5.0, "undo restored the parameter");
    // The old final snapshot cannot be reused for validation.
    assert!(controller.validate().is_err());
    assert!(!controller.result().validated());
}

#[test]
fn unknown_write_is_not_replayed_and_reconciles_once() {
    let (host, controller) = controller();
    host.set_write_mode(WriteMode::UnknownOnce);
    controller.inspect().expect("inspect");
    controller.capture_snapshot().expect("snapshot");
    let lost = controller.apply(thickness_ops());
    assert!(matches!(lost, WriteOutcome::Unknown { .. }), "{lost:?}");
    assert_eq!(controller.terminal().map(|(terminal, _)| terminal), Some(TaskTerminal::Unknown));
    assert_eq!(host.revision(), 2, "the mock did apply the lost write");

    let second = controller.apply(thickness_ops());
    assert!(matches!(second, WriteOutcome::Denied { .. }), "unknown writes must not be replayed: {second:?}");
    assert!(controller.validate().is_err(), "validation is refused while the write is unknown");

    let reconciled = controller.reconcile().expect("reconcile");
    assert!(matches!(reconciled, cad_rig_harness::types::ReconcileOutcome::Applied { .. }), "{reconciled:?}");
    controller.capture_snapshot().expect("final snapshot");
    let report = controller.validate().expect("validate");
    assert!(report.passed(), "{report:?}");
    assert_eq!(controller.result().terminal, TaskTerminal::ValidatedSuccess);
    assert_eq!(host.revision(), 2);
    assert_eq!(host.applied_writes().len(), 1);
}

#[test]
fn partial_write_stays_unknown_and_cannot_be_validated_into_success() {
    let (host, controller) = controller_with_budget(Budget { repair_rounds: 0, ..TaskSpec::l_bracket().budget });
    host.set_write_mode(WriteMode::PartialOnce);
    controller.inspect().expect("inspect");
    controller.capture_snapshot().expect("snapshot");
    let lost = controller.apply(vec![CadOp::change_parameter("thickness", "7 mm"), CadOp::change_parameter("thickness", "8 mm")]);
    assert!(matches!(lost, WriteOutcome::Unknown { .. }), "{lost:?}");
    assert_eq!(host.thickness(), 7.0, "only the first op was applied");
    let reconciled = controller.reconcile().expect("reconcile");
    assert!(matches!(reconciled, cad_rig_harness::types::ReconcileOutcome::Applied { .. }), "{reconciled:?}");
    controller.capture_snapshot().expect("final snapshot");
    let report = controller.validate().expect("validate");
    assert_eq!(report.state, "quality_failed");
    assert_eq!(controller.result().terminal, TaskTerminal::QualityFailed);
    assert!(!controller.result().validated());
}

#[test]
fn turn_end_without_host_validation_is_never_success() {
    let (_host, controller) = controller_with_budget(Budget { repair_rounds: 0, ..TaskSpec::l_bracket().budget });
    let decision = controller.turn_end(false);
    assert!(matches!(decision, cad_rig_harness::controller::TurnDecision::Stop(_)), "{decision:?}");
    assert_eq!(controller.result().terminal, TaskTerminal::Incomplete);
    assert!(!controller.result().validated());
}

#[tokio::test(flavor = "current_thread")]
async fn early_stop_is_incomplete_even_after_the_write() {
    let (host, controller) = controller();
    let agent = build_agent(scenario::early_stop_model(), &controller);
    let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    assert_eq!(result.terminal, TaskTerminal::Incomplete);
    assert!(!result.validated());
    assert_eq!(host.revision(), 2, "the write did happen, but it was not validated");
}

#[tokio::test(flavor = "current_thread")]
async fn missing_validation_prose_cannot_succeed() {
    let (_host, controller) = controller();
    let agent = build_agent(scenario::missing_validation_model(), &controller);
    let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    assert_eq!(result.terminal, TaskTerminal::Incomplete);
    assert!(!result.validated());
}

#[tokio::test(flavor = "current_thread")]
async fn bounded_repair_succeeds_and_is_recorded() {
    let (host, controller) = controller();
    host.set_validation_mode(ValidationMode::FailOnce);
    let agent = build_agent(scenario::repair_model(), &controller);
    let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess);
    assert!(result.events.iter().any(|event| matches!(event, cad_rig_harness::types::TaskEvent::Repair { .. })));
    assert_eq!(host.revision(), 3, "the failed 7 mm write and the repaired 8 mm write");
}

#[tokio::test(flavor = "current_thread")]
async fn quality_failure_with_zero_repair_budget_is_terminal() {
    let (host, controller) = controller_with_budget(Budget { repair_rounds: 0, ..TaskSpec::l_bracket().budget });
    host.set_validation_mode(ValidationMode::FailAlways);
    let agent = build_agent(scenario::quality_failed_model(), &controller);
    let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    assert_eq!(result.terminal, TaskTerminal::QualityFailed);
    assert!(!result.validated());
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_during_write_blocks_the_write_and_success() {
    let (host, controller) = controller();
    host.set_write_mode(WriteMode::CancelDuringWrite);
    let agent = build_agent(scenario::happy_model(), &controller);
    let cancel_controller = controller.clone();
    let cancel_host = host.clone();
    let handle = std::thread::spawn(move || {
        assert!(cancel_host.wait_until_write_started(Duration::from_secs(5)), "write never started");
        cancel_controller.cancel("test pressed Stop");
    });
    let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    handle.join().expect("cancel thread");
    assert_eq!(result.terminal, TaskTerminal::Cancelled);
    assert!(!result.validated());
    assert_eq!(host.revision(), 1, "cancelled write applied nothing");
    assert!(host.applied_writes().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn wall_clock_timeout_is_not_success() {
    let (host, controller) = controller_with_budget(Budget { wall_time_ms: 0, ..TaskSpec::l_bracket().budget });
    let agent = build_agent(scenario::happy_model(), &controller);
    let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    assert_eq!(result.terminal, TaskTerminal::Error);
    assert!(!result.validated());
    assert_eq!(host.revision(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn plain_chat_has_no_cad_policy_and_no_tool_calls() {
    let (host, controller) = controller();
    let agent = build_agent(scenario::plain_chat_model(), &controller);
    let text = run_plain(&agent, "Hello, just chat.").await.expect("plain chat");
    assert!(text.contains("ordinary chat"), "{text}");
    let events = controller.events();
    assert!(events.iter().all(|event| !matches!(event, cad_rig_harness::types::TaskEvent::ToolRequested { .. })));
    assert!(controller.terminal().is_none());
    assert_eq!(host.revision(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn a_write_after_success_is_denied_and_cannot_invalidate_the_terminal() {
    let (host, controller) = controller();
    let agent = build_agent(scenario::happy_model(), &controller);
    let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess);
    let after = controller.apply_after_success_for_test(vec![CadOp::change_parameter("thickness", "6 mm")]);
    assert!(matches!(after, WriteOutcome::Denied { .. }), "{after:?}");
    assert_eq!(controller.result().terminal, TaskTerminal::ValidatedSuccess);
    assert_eq!(host.revision(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn one_agent_model_and_tool_registry_serve_task_and_plain_chat() {
    let (host, controller) = controller();
    let turns = scenario::happy_path_turns().into_iter().chain([MockTurn::text("plain answer from the same agent")]);
    let model = MockCompletionModel::from_turns(turns);
    let agent = build_agent(model, &controller);
    let task = run_task(&agent, &controller, scenario::PROMPT, 24).await;
    assert_eq!(task.terminal, TaskTerminal::ValidatedSuccess);
    let events_after_task = controller.events().len();
    let text = run_plain(&agent, "Hello, just chat.").await.expect("plain chat");
    assert_eq!(text, "plain answer from the same agent");
    assert_eq!(controller.events().len(), events_after_task, "plain chat added no CAD task/tool events");
    assert_eq!(controller.result().terminal, TaskTerminal::ValidatedSuccess);
    assert_eq!(host.revision(), 2);
}
