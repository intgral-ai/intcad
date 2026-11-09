//! Headless acceptance run for the native AI harness spike.
//!
//! It creates the CTO acceptance model — an L-shaped bracket 5 mm thick with four 5 mm mounting
//! holes — then runs the real `TaskController` over the real engine/snapshot/validation ports to
//! change the thickness to 8 mm. No egui, no model, no Node.
//!
//! `cargo run -p solidcraft-agent-task-engine --example headless_bracket`

use std::path::PathBuf;

use serde_json::{Value, json};

use solidcraft_agent_task as task;
use solidcraft_agent_task_engine::EngineTaskPorts;
use solidcraft_engine::Session;
use solidcraft_engine::validation::{MeasurementRequirement, ParameterRequirement, task_profile};

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("solidcraft-headless-bracket-{name}-{}-{nanos}", std::process::id()))
}

fn run(session: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    session.execute(id, &params).map_err(|e| format!("{id}: {e}"))
}

fn bracket(thickness: &str) -> Result<Session, String> {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "thickness", "expression": thickness}))?;
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Profile"}))?;
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [60, 0], [60, 40], [30, 40], [30, 20], [0, 20]], "closed": true}))?;
    run(&mut s, "sketch.finish", json!({}))?;
    run(&mut s, "solid.extrude", json!({"distance": "thickness", "body_name": "Bracket"}))?;
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Mounting holes"}))?;
    for (x, y) in [(10.0, 10.0), (20.0, 10.0), (40.0, 10.0), (50.0, 10.0)] {
        run(&mut s, "sketch.point", json!({"point": [x, y]}))?;
    }
    run(&mut s, "sketch.finish", json!({}))?;
    run(&mut s, "solid.hole", json!({"sketch": "Mounting holes", "diameter": 5}))?;
    Ok(s)
}

fn expected_volume(thickness: f64) -> f64 {
    (1800.0 - 4.0 * std::f64::consts::PI * 2.5 * 2.5) * thickness
}

fn measured_volume(session: &mut Session) -> Result<f64, String> {
    let measured = run(session, "inspect.measure", json!({}))?;
    measured.pointer("/bodies/0/volume_mm3").and_then(Value::as_f64).ok_or_else(|| "the bracket has no closed volume".into())
}

fn main() -> Result<(), String> {
    let root = scratch("run");
    let candidates = root.join("candidates");
    let validation_store = root.join("validation");
    std::fs::create_dir_all(&candidates).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&validation_store).map_err(|e| e.to_string())?;
    solidcraft_engine::cmd::set_candidate_store(Some(candidates.clone()));

    let mut session = bracket("5 mm")?;
    let before_volume = measured_volume(&mut session)?;
    let stamp = session.document_stamp();
    let undo_before = session.undo.len();
    let document = task::DocumentRef { project_id: "local".into(), document_id: stamp.document_id, version: stamp.revision };
    let profile = task_profile(
        "bracket-check",
        "1.0.0",
        Some(vec![ParameterRequirement { name: "thickness".into(), value: 8.0, abs_tolerance: 1e-6 }]),
        Some(vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/bodies/0/volume_mm3".into(),
            expected: expected_volume(8.0),
            abs_tolerance: 0.05,
            rel_tolerance: 1e-4,
        }]),
    )
    .map_err(|e| e.to_string())?;
    let mut ports = EngineTaskPorts::open(session, candidates, validation_store, profile)?;
    let registry = task::skill::builtins(ports.profile().clone(), task::Budget::finite(4, 16, 1, 120_000)).map_err(|e| e.to_string())?;
    let mut controller = task::TaskController::start(
        &registry,
        task::TaskSpec {
            skill_id: task::TEXT_TO_CAD_ID.into(),
            input: "Create an L-shaped bracket 5 mm thick with four mounting holes; change the thickness to 8 mm".into(),
            document,
            selection: Vec::new(),
            requirements: None,
            budget: task::Budget::finite(4, 16, 1, 120_000),
        },
        task::PermissionContext { principal: "headless".into(), approval_mode: task::ApprovalMode::Ask, allow_list: Vec::new(), turn_valid: true },
        std::sync::Arc::new(task::SystemClock),
        Box::new(task::InMemoryStore::new()),
    )
    .map_err(|e| e.to_string())?;
    ports.with_parts(|tools, _| controller.begin(tools)).map_err(|e| e.to_string())?;

    let inspect = ports.with_parts(|tools, _| controller.on_tool_request("inspect_design", &json!({}), tools));
    if inspect != task::ToolDecision::Allow {
        return Err(format!("inspect was denied: {inspect:?}"));
    }
    let change = json!({"name": "thickness", "expression": "8 mm"});
    let write = ports.with_parts(|tools, _| controller.on_tool_request("execute", &json!({"command": "parameters.change", "params": change}), tools));
    if write != task::ToolDecision::Allow {
        return Err(format!("the write was denied: {write:?}"));
    }
    let expected = controller.expected_document();
    ports.apply_expected("parameters.change", &change, &expected)?;
    controller.on_tool_result("execute", task::ToolOutcome::Completed, "thickness changed to 8 mm");
    let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
    let task::TurnAction::Terminal(result) = action else { return Err(format!("expected a terminal, got {action:?}")) };

    let after_measured = ports.execute("inspect.measure", &json!({}))?;
    let after_volume = after_measured.pointer("/bodies/0/volume_mm3").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let mounting_holes = ports
        .session()
        .doc
        .features
        .iter()
        .find_map(|feature| match &feature.kind {
            solidcraft_engine::doc::FeatureKind::Hole { points: Some(points), .. } => Some(points.ids.len()),
            _ => None,
        })
        .unwrap_or(0);
    let thickness_after = ports.session().doc.param("thickness").map(|p| p.expr.clone());
    let undo_after_write = ports.session().undo.len();
    let final_document = controller.final_document().cloned();
    final_document.map(|document| ports.apply_expected("edit.undo", &json!({}), &document)).transpose()?;
    let thickness_after_undo = ports.session().doc.param("thickness").map(|p| p.expr.clone());
    let undo_after_undo = ports.session().undo.len();
    let report = controller.events().iter().find_map(|event| match event {
        task::TaskEvent::ValidationReport { report } => Some(report.clone()),
        _ => None,
    });
    let snapshots: Vec<Value> = controller
        .events()
        .iter()
        .filter_map(|event| match event {
            task::TaskEvent::Snapshot { snapshot } => {
                Some(json!({"id": snapshot.id, "purpose": snapshot.purpose, "document": snapshot.document, "digest": snapshot.digest}))
            }
            _ => None,
        })
        .collect();
    let output = json!({
        "terminal": result.terminal,
        "reason": result.reason,
        "thickness": thickness_after,
        "before_volume_mm3": before_volume,
        "after_volume_mm3": after_volume,
        "expected_after_volume_mm3": expected_volume(8.0),
        "mounting_holes": mounting_holes,
        "thickness_after_undo": thickness_after_undo,
        "undo_after_undo": undo_after_undo,
        "snapshots": snapshots,
        "validation": report.map(|r| json!({
            "run_id": r.run_id,
            "passed": r.passed,
            "profile": r.profile,
            "checks": r.checks,
        })),
        "artifacts": result.artifacts,
        "undo_before": undo_before,
        "undo_after": undo_after_write,
    });
    println!("{}", serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?);
    Ok(())
}
