//! Contract, skill and port tests. The controller and its lifecycle tests are the next stacked PR.

use serde_json::json;

use super::*;

fn profile() -> ValidationProfileRef {
    ValidationProfileRef { profile_id: "text-to-cad".into(), version: "1.0.0".into(), digest: format!("sha256:{}", "a".repeat(64)) }
}

fn budget() -> Budget {
    Budget::finite(4, 16, 1, 120_000)
}

fn registry() -> SkillRegistry {
    skill::builtins(profile(), budget()).expect("the built-ins register")
}

#[test]
fn a_finite_budget_has_no_unlimited_spelling() {
    let budget = budget();
    assert_eq!((budget.model_calls, budget.tool_calls, budget.repair_rounds, budget.wall_time_ms), (4, 16, 1, 120_000));
    let zero = Budget::finite(0, 0, 0, 0);
    assert_eq!((zero.model_calls, zero.tool_calls, zero.repair_rounds, zero.wall_time_ms), (0, 0, 0, 0));
}

#[test]
fn the_spec_round_trips_with_camel_case_and_refuses_unknown_fields() {
    let spec = TaskSpec {
        skill_id: TEXT_TO_CAD_ID.into(),
        input: "change the bracket to 8 mm".into(),
        document: DocumentRef { project_id: "local".into(), document_id: "doc-1".into(), version: 7 },
        selection: vec!["Bracket".into()],
        requirements: None,
        budget: budget(),
    };
    let value = serde_json::to_value(&spec).expect("the spec serializes");
    assert_eq!(value["document"]["documentId"], "doc-1");
    assert_eq!(value["document"]["version"], 7);
    assert_eq!(value["budget"]["repairRounds"], 1);
    let back: TaskSpec = serde_json::from_value(value.clone()).expect("the spec reads back");
    assert_eq!(back, spec);

    let mut broken = value;
    broken["unexpected"] = json!(true);
    assert!(serde_json::from_value::<TaskSpec>(broken).is_err(), "a model or host cannot smuggle an unknown field");
}

#[test]
fn the_beta_profile_identity_is_host_owned_and_versioned() {
    let profile = profile();
    assert_eq!((profile.profile_id.as_str(), profile.version.as_str()), ("text-to-cad", "1.0.0"));
    let report = ValidationReport {
        run_id: "run-1".into(),
        profile: profile.clone(),
        snapshot: SnapshotRef {
            id: "cand-1".into(),
            document: DocumentRef { project_id: "local".into(), document_id: "doc-1".into(), version: 8 },
            digest: format!("sha256:{}", "b".repeat(64)),
            purpose: SnapshotPurpose::Validation,
        },
        document: DocumentRef { project_id: "local".into(), document_id: "doc-1".into(), version: 8 },
        passed: true,
        checks: vec![CheckResult { check_id: "native.rebuild".into(), passed: true, detail: "checked".into() }],
    };
    let value = serde_json::to_value(&report).expect("a report serializes");
    assert_eq!(value["snapshot"]["purpose"], "validation");
    assert_eq!(value["profile"]["profileId"], "text-to-cad");
    assert_eq!(value["passed"], true);
}

#[test]
fn the_builtins_are_fixed_and_an_id_version_cannot_be_replaced() {
    let mut registry = registry();
    assert_eq!(registry.ids(), vec![ANATOMY_ID, TEXT_TO_CAD_ID]);
    assert_eq!(registry.get(TEXT_TO_CAD_ID).map(|skill| skill.version.as_str()), Some("0.1.0"));
    assert!(registry.get(ANATOMY_ID).is_some_and(|skill| skill.write_tools.is_empty()));

    let changed = skill::text_to_cad(profile(), Budget::finite(1, 1, 0, 1));
    assert!(registry.register(changed).is_err(), "the same id/version is immutable");

    let mut next = skill::text_to_cad(profile(), budget());
    next.version = "0.2.0".into();
    assert!(registry.register(next).is_err(), "a new version cannot take an existing id");
}

#[test]
fn classification_accepts_only_the_native_read_and_write_areas() {
    let registry = registry();
    let skill = registry.get(TEXT_TO_CAD_ID).expect("the text-to-cad skill");
    assert_eq!(skill.classify("document.inspect", &json!({})), ToolKind::Read);
    assert_eq!(skill.classify("engine.commands", &json!({})), ToolKind::Read);
    assert_eq!(skill.classify("engine.execute", &json!({"command": "inspect.measure", "params": {}})), ToolKind::Read);
    assert_eq!(skill.classify("engine.execute", &json!({"command": "parameters.change", "params": {}})), ToolKind::Write);
    assert_eq!(skill.classify("execute", &json!({"command": "solid.extrude", "params": {}})), ToolKind::Write);
    assert_eq!(skill.classify("engine.batch", &json!({"commands": []})), ToolKind::Write);
    assert_eq!(skill.classify("engine.execute", &json!({"command": "file.save", "params": {}})), ToolKind::Forbidden);
    assert_eq!(skill.classify("engine.execute", &json!({"command": "app.quit", "params": {}})), ToolKind::Forbidden);
    assert_eq!(skill.classify("shell", &json!({"command": "rm -rf"})), ToolKind::Forbidden);
    assert!(skill.allows("inspect_design", &json!({})));
    assert!(!skill.allows("file.save", &json!({})));
}

#[test]
fn the_anatomy_descriptor_is_read_only_in_this_beta() {
    let registry = registry();
    let anatomy = registry.get(ANATOMY_ID).expect("the anatomy descriptor");
    assert!(anatomy.write_tools.is_empty());
    assert_eq!(anatomy.classify("execute", &json!({"command": "parameters.change", "params": {}})), ToolKind::Forbidden);
}

#[test]
fn an_unknown_skill_is_not_silently_created() {
    let registry = registry();
    assert!(registry.get("not-a-skill").is_none());
    assert!(skill::SkillRegistry::new().ids().is_empty());
}
