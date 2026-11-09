//! The product scenario and its scripted fake-provider turns.
//!
//! The fake provider is Rig's deterministic `MockCompletionModel`; it is
//! labelled as such everywhere. It is used to drive the same task contract and
//! to inject faults. The real CAD path is `EngineCadHost`, exercised by the
//! engine integration test.

use rig::test_utils::{MockCompletionModel, MockTurn};
use serde_json::json;

pub const PROMPT: &str = "L-shaped bracket, 5 mm thick, four mounting holes. Change the thickness to 8 mm and keep the four holes parametric.";

fn call(id: &str, tool: &str, args: serde_json::Value) -> MockTurn {
    MockTurn::tool_call(id, tool, args)
}

fn apply_thickness(id: &str, thickness: &str) -> MockTurn {
    call(
        id,
        "cad_apply",
        json!({
            "reason": "set bracket thickness",
            "ops": [{
                "command": "parameters.change",
                "params": {"name": "thickness", "expression": format!("{thickness} mm")}
            }]
        }),
    )
}

/// Happy path: inspect, baseline snapshot, guarded write, final snapshot,
/// host validation, then a final text turn. The final text is accepted only
/// because the controller already has a passing host report.
pub fn happy_path_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({"purpose": "baseline"})),
        apply_thickness("t3", "8"),
        call("t4", "cad_capture_snapshot", json!({"purpose": "final"})),
        call("t5", "cad_validate", json!({"profile_id": "l-bracket-task", "version": "1.0.0"})),
        MockTurn::text("Done: thickness 8 mm, four holes preserved, host validation passed."),
    ]
}

/// The model stops early, after the write but before the final snapshot or
/// validation. The turn-end hook must not accept that as task success.
pub fn early_stop_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "8"),
        MockTurn::text("Done already."),
        MockTurn::text("I said done."),
    ]
}

/// The model captures the final snapshot but never calls validation, then
/// claims success in prose.
pub fn missing_validation_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "8"),
        call("t4", "cad_capture_snapshot", json!({})),
        MockTurn::text("Trust me, it is correct."),
        MockTurn::text("Really, it is done."),
    ]
}

/// First validation fails; one bounded repair write, then validation passes.
pub fn repair_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "7"),
        call("t4", "cad_capture_snapshot", json!({})),
        call("t5", "cad_validate", json!({})),
        apply_thickness("t6", "8"),
        call("t7", "cad_capture_snapshot", json!({})),
        call("t8", "cad_validate", json!({})),
        MockTurn::text("Repaired and validated: thickness 8 mm."),
    ]
}

/// Validation fails and the model keeps claiming success; the repair budget
/// is zero, so the task must end quality_failed.
pub fn quality_failed_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "7"),
        call("t4", "cad_capture_snapshot", json!({})),
        call("t5", "cad_validate", json!({})),
        MockTurn::text("Done."),
        MockTurn::text("Done, really."),
    ]
}

/// The model writes without inspecting first; the pre-dispatch gate skips it.
pub fn write_without_inspect_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_capture_snapshot", json!({})),
        apply_thickness("t2", "8"),
        MockTurn::text("Done."),
        MockTurn::text("Still done."),
    ]
}

pub fn happy_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(happy_path_turns())
}

pub fn early_stop_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(early_stop_turns())
}

pub fn missing_validation_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(missing_validation_turns())
}

pub fn repair_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(repair_turns())
}

pub fn quality_failed_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(quality_failed_turns())
}

pub fn write_without_inspect_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(write_without_inspect_turns())
}

pub fn unknown_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "8"),
        MockTurn::text("Done."),
    ]
}

pub fn stale_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "8"),
        MockTurn::text("Done."),
    ]
}

pub fn denied_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "8"),
        MockTurn::text("Done."),
        MockTurn::text("Still done."),
    ]
}

pub fn partial_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        call(
            "t3",
            "cad_apply",
            json!({
                "reason": "partial write fault injection",
                "ops": [
                    {"command": "parameters.change", "params": {"name": "thickness", "expression": "7 mm"}},
                    {"command": "parameters.change", "params": {"name": "thickness", "expression": "8 mm"}}
                ]
            }),
        ),
        MockTurn::text("Done."),
    ]
}

pub fn unknown_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(unknown_turns())
}

pub fn partial_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(partial_turns())
}

pub fn stale_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(stale_turns())
}

pub fn denied_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(denied_turns())
}

pub fn plain_chat_model() -> MockCompletionModel {
    MockCompletionModel::from_turns([MockTurn::text("Hello. This is ordinary chat, with no CAD task policy.")])
}

/// The real parametric L-bracket document used by `EngineCadHost`. It is a
/// command script, not a mesh: parameters drive the L profile, the extrusion
/// width and the four through-hole cuts.
pub fn l_bracket_script() -> serde_json::Value {
    json!({"commands": [
        {"command": "parameters.change", "params": {"name": "width", "expression": "60 mm"}},
        {"command": "parameters.change", "params": {"name": "length", "expression": "40 mm"}},
        {"command": "parameters.change", "params": {"name": "height", "expression": "30 mm"}},
        {"command": "parameters.change", "params": {"name": "thickness", "expression": "5 mm"}},
        {"command": "parameters.change", "params": {"name": "hole_diameter", "expression": "4 mm"}},
        {"command": "sketch.create", "params": {"plane": "XZ", "name": "Profile"}},
        {"command": "sketch.line", "params": {
            "points": [[0, 0], ["length", 0], ["length", "thickness"], ["thickness", "thickness"], ["thickness", "height"], [0, "height"]],
            "closed": true,
            "ids": ["a", "b", "c", "d", "e", "f"]
        }},
        {"command": "sketch.finish", "params": {}},
        {"command": "solid.extrude", "params": {"distance": "width", "direction": "negative", "body_name": "Bracket"}},
        {"command": "sketch.create", "params": {"plane": "XY", "name": "Holes"}},
        {"command": "sketch.circle.center", "params": {"center": [10, -15], "diameter": "hole_diameter"}},
        {"command": "sketch.circle.center", "params": {"center": [30, -15], "diameter": "hole_diameter"}},
        {"command": "sketch.circle.center", "params": {"center": [10, -45], "diameter": "hole_diameter"}},
        {"command": "sketch.circle.center", "params": {"center": [30, -45], "diameter": "hole_diameter"}},
        {"command": "sketch.finish", "params": {}},
        {"command": "solid.extrude", "params": {"distance": "thickness", "direction": "negative", "operation": "cut"}}
    ]})
}

/// Exact L-profile volume for the script above, minus four through holes.
pub fn expected_l_bracket_volume(thickness_mm: f64, hole_diameter_mm: f64) -> f64 {
    let length = 40.0;
    let height = 30.0;
    let width = 60.0;
    let area = length * thickness_mm + (height - thickness_mm) * thickness_mm;
    let holes = 4.0 * std::f64::consts::PI * (hole_diameter_mm / 2.0).powi(2) * thickness_mm;
    area * width - holes
}

/// A write, an undo, then a fresh guarded write and validation. The undo must
/// clear the prior snapshot and validation state.
pub fn undo_turns() -> Vec<MockTurn> {
    vec![
        call("t1", "cad_inspect", json!({})),
        call("t2", "cad_capture_snapshot", json!({})),
        apply_thickness("t3", "8"),
        call("t4", "cad_undo", json!({})),
        call("t5", "cad_capture_snapshot", json!({})),
        apply_thickness("t6", "8"),
        call("t7", "cad_capture_snapshot", json!({})),
        call("t8", "cad_validate", json!({})),
        MockTurn::text("Undo and re-apply validated."),
    ]
}

pub fn undo_model() -> MockCompletionModel {
    MockCompletionModel::from_turns(undo_turns())
}
