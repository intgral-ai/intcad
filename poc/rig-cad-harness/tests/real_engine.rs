//! Real-engine acceptance: the same TaskController and Rig interaction, with
//! the solidcraft engine and ValidationPort (no mock host, no UI).
//!
//! This file intentionally has one test: the engine's candidate store is a
//! process-global host setting in this build, so real-engine cases run
//! sequentially.

use std::time::Duration;

use cad_rig_harness::controller::TaskController;
use cad_rig_harness::engine_host::{EngineCadHost, EnginePermission};
use cad_rig_harness::rig_runner::{build_agent, run_task};
use cad_rig_harness::scenario;
use cad_rig_harness::types::{
    CadHost, CadOp, DocumentStamp, InspectView, TaskEvent, TaskResult, TaskSpec, TaskTerminal, WriteOutcome, WritePermit, WriteRequest,
};
use serde_json::json;

fn unique_root(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("cad-rig-engine-{}-{nanos}-{name}", std::process::id()))
}

fn parameter(view: &InspectView, name: &str) -> Option<f64> {
    view.parameters
        .get("parameters")?
        .as_array()?
        .iter()
        .find(|parameter| parameter.get("name").and_then(|value| value.as_str()) == Some(name))?
        .get("value")?
        .as_f64()
}

fn last_report(result: &TaskResult) -> Option<(String, Vec<cad_rig_harness::types::CheckResult>)> {
    result.events.iter().rev().find_map(|event| match event {
        TaskEvent::ValidationReport { state, checks } => Some((state.clone(), checks.clone())),
        _ => None,
    })
}

fn all_reports(result: &TaskResult) -> Vec<(String, Vec<cad_rig_harness::types::CheckResult>)> {
    result
        .events
        .iter()
        .filter_map(|event| match event {
            TaskEvent::ValidationReport { state, checks } => Some((state.clone(), checks.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn real_engine_l_bracket_acceptance() {
    let base = unique_root("acceptance");
    std::fs::create_dir_all(&base).expect("test root");

    // 1. Happy path: 5 mm / four holes -> guarded 8 mm -> real validation.
    {
        let host = EngineCadHost::l_bracket(base.join("happy")).expect("engine host");
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        let before = controller.inspect().expect("inspect before");
        assert_eq!(parameter(&before, "thickness"), Some(5.0));
        assert_eq!(parameter(&before, "hole_diameter"), Some(4.0));
        assert_eq!(before.hole_count, Some(4));
        let expected_before = scenario::expected_l_bracket_volume(5.0, 4.0);
        assert!((before.volume_mm3.expect("volume") - expected_before).abs() < expected_before * 0.01, "{before:?}");

        let agent = build_agent(scenario::happy_model(), &controller);
        let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
        assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess, "{result:?}");
        assert!(result.validated());
        assert!(result.pending_unknown_writes.is_empty());
        assert!(result.artifacts.iter().any(|artifact| artifact.starts_with("snapshot:")), "{:?}", result.artifacts);
        assert!(result.artifacts.iter().any(|artifact| artifact.starts_with("report:")), "{:?}", result.artifacts);

        let (state, checks) = last_report(&result).expect("validation report");
        assert_eq!(state, "passed");
        let ids: Vec<_> = checks.iter().map(|check| check.check_id.as_str()).collect();
        for required in ["input.integrity", "native.rebuild", "parameters.required", "geometry.exact"] {
            assert!(ids.contains(&required), "missing {required}: {ids:?}");
        }
        assert!(checks.iter().all(|check| check.state == "passed"), "{checks:?}");
        let geometry = checks.iter().find(|check| check.check_id == "geometry.exact").expect("geometry check");
        let actual = geometry.actual.get("measurements").and_then(|value| value.as_array()).and_then(|value| value.first()).and_then(|value| value.get("actual")).and_then(|value| value.as_f64()).expect("measured volume");
        let expected_after = scenario::expected_l_bracket_volume(8.0, 4.0);
        assert!((actual - expected_after).abs() < expected_after * 0.01, "measured {actual}, expected {expected_after}");

        let after = host.inspect().expect("inspect after");
        assert_eq!(parameter(&after, "thickness"), Some(8.0));
        assert_eq!(parameter(&after, "hole_diameter"), Some(4.0), "hole diameter stayed parametric");
        assert_eq!(after.hole_count, Some(4), "four mounting holes remain");
        assert!((after.volume_mm3.expect("volume") - expected_after).abs() < expected_after * 0.01, "{after:?}");
        println!("HAPPY before={before:?}\nHAPPY after={after:?}");
    }

    // 2. Real validation failure -> one bounded repair -> pass.
    {
        let host = EngineCadHost::l_bracket(base.join("repair")).expect("engine host");
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        let agent = build_agent(scenario::repair_model(), &controller);
        let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
        assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess, "{result:?}");
        let reports = all_reports(&result);
        assert_eq!(reports.len(), 2, "{reports:?}");
        assert_eq!(reports[0].0, "quality_failed");
        assert_eq!(reports[1].0, "passed");
        assert!(result.events.iter().any(|event| matches!(event, TaskEvent::Repair { .. })));
        let after = host.inspect().expect("inspect");
        assert_eq!(parameter(&after, "thickness"), Some(8.0));
        assert_eq!(after.hole_count, Some(4));
        println!("REPAIR after={after:?}");
    }

    // 3. Real undo restores the design, then a fresh write validates.
    {
        let host = EngineCadHost::l_bracket(base.join("undo")).expect("engine host");
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        let agent = build_agent(scenario::undo_model(), &controller);
        let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
        assert_eq!(result.terminal, TaskTerminal::ValidatedSuccess, "{result:?}");
        assert!(result.events.iter().any(|event| matches!(event, TaskEvent::Undone { .. })));
        let after = host.inspect().expect("inspect");
        assert_eq!(parameter(&after, "thickness"), Some(8.0));
        assert_eq!(after.hole_count, Some(4));
        println!("UNDO after={after:?}");
    }

    // 4. Early stop and missing validation on the real engine: never success.
    {
        let host = EngineCadHost::l_bracket(base.join("early")).expect("engine host");
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        let agent = build_agent(scenario::early_stop_model(), &controller);
        let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
        assert_eq!(result.terminal, TaskTerminal::Incomplete, "{result:?}");
        assert!(!result.validated());
        let after = host.inspect().expect("inspect");
        assert_eq!(parameter(&after, "thickness"), Some(8.0), "the write happened but was not validated");
        assert!(result.events.iter().all(|event| !matches!(event, TaskEvent::Terminal { terminal: TaskTerminal::ValidatedSuccess, .. })));
    }
    {
        let host = EngineCadHost::l_bracket(base.join("missing")).expect("engine host");
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        let agent = build_agent(scenario::missing_validation_model(), &controller);
        let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
        assert_eq!(result.terminal, TaskTerminal::Incomplete, "{result:?}");
        assert!(!result.validated());
    }

    // 5. Real quality failure with no repair budget is terminal.
    {
        let host = EngineCadHost::l_bracket(base.join("quality")).expect("engine host");
        let mut spec = TaskSpec::l_bracket();
        spec.budget.repair_rounds = 0;
        let controller = TaskController::start(spec, host.clone()).expect("controller");
        let agent = build_agent(scenario::quality_failed_model(), &controller);
        let result = run_task(&agent, &controller, scenario::PROMPT, 24).await;
        assert_eq!(result.terminal, TaskTerminal::QualityFailed, "{result:?}");
        assert!(!result.validated());
    }

    // 6. Real stale boundary and external bypass refusal.
    {
        let host = EngineCadHost::l_bracket(base.join("stale")).expect("engine host");
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        let before = controller.inspect().expect("inspect");
        controller.capture_snapshot().expect("snapshot");
        let applied = controller.apply(vec![CadOp::change_parameter("thickness", "8 mm")]);
        assert!(matches!(applied, WriteOutcome::Applied { .. }), "{applied:?}");

        let bypass = controller.external_write_without_permit(vec![CadOp::change_parameter("thickness", "6 mm")]);
        assert!(matches!(bypass, WriteOutcome::Denied { .. }), "{bypass:?}");

        let permit = WritePermit { task_id: "stale-test".into(), run_id: "stale-test".into(), nonce: "n".into() };
        host.register_permit(&permit).expect("register");
        let stale = host
            .dispatch_write(WriteRequest {
                write_id: "stale-test".into(),
                task_id: "stale-test".into(),
                run_id: "stale-test".into(),
                expected: DocumentStamp::new(before.document.document_id.clone(), before.document.revision),
                ops: vec![CadOp::change_parameter("thickness", "6 mm")],
                permit: Some(permit),
            })
            .expect("host");
        assert!(matches!(stale, WriteOutcome::Stale { .. }), "{stale:?}");
        let after = host.inspect().expect("inspect");
        assert_eq!(parameter(&after, "thickness"), Some(8.0), "stale write changed nothing");
    }

    // 6b. The real host also refuses a permission-denied write.
    {
        let host = EngineCadHost::l_bracket(base.join("denied")).expect("engine host");
        host.set_permission(EnginePermission::DenyAll);
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        controller.inspect().expect("inspect");
        controller.capture_snapshot().expect("snapshot");
        let denied = controller.apply(vec![CadOp::change_parameter("thickness", "8 mm")]);
        assert!(matches!(denied, WriteOutcome::Denied { .. }), "{denied:?}");
        let after = host.inspect().expect("inspect");
        assert_eq!(parameter(&after, "thickness"), Some(5.0));
        assert_eq!(host.permission_requests().len(), 1);
    }

    // 7. Real in-flight cancel: the engine worker rolls the transaction back.
    {
        let host = EngineCadHost::l_bracket(base.join("cancel")).expect("engine host");
        let controller = TaskController::start(TaskSpec::l_bracket(), host.clone()).expect("controller");
        controller.inspect().expect("inspect");
        controller.capture_snapshot().expect("snapshot");
        let cancel_controller = controller.clone();
        let cancel_host = host.clone();
        let handle = std::thread::spawn(move || {
            assert!(cancel_host.wait_until_write_started(Duration::from_secs(5)), "write never started");
            std::thread::sleep(Duration::from_millis(20));
            cancel_controller.cancel("test pressed Stop");
        });
        let ops: Vec<CadOp> = (0..500)
            .map(|_| CadOp { command: "solid.box".into(), params: json!({"length": 1, "width": 1, "height": 1}) })
            .collect();
        let outcome = controller.apply(ops);
        handle.join().expect("cancel thread");
        assert!(matches!(outcome, WriteOutcome::Cancelled { .. }), "{outcome:?}");
        assert_eq!(controller.result().terminal, TaskTerminal::Cancelled);
        let after = host.inspect().expect("inspect");
        assert_eq!(parameter(&after, "thickness"), Some(5.0), "rollback restored the design");
        assert_eq!(after.body_count, 1, "rollback removed the uncommitted boxes");
        assert_eq!(after.hole_count, Some(4));
    }

    let _ = std::fs::remove_dir_all(&base);
}
