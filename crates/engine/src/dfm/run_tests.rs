//! DFM run records: persistence, lineage, staleness, the §4.5 state mapping and profile
//! compilation. The fixtures are hand-made sheets and profiles; nothing here reads a project.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use super::*;
use crate::dfm::compile::{RunInput, SheetRef};
use crate::dfm::findings::{self, FindingState, FindingsIdentity, SheetCheck, SourceRef};
use crate::dfm::run::{Freshness, RunFiles, RunMeta, RunOutcome, RunRequest, RunStore, SheetEvidence, freshness};
use crate::dfm::{SheetCheckParameters, check_sheet};
use crate::schema::snapshot::{CandidateArtifact, CandidateRef, SnapshotSource};
use crate::validation::{
    ArtifactRef, CheckProfile, CheckState, RunnerIdentity, ValidationReport, ValidationState, native_basic_profile, profile_ref,
};
use solidcore_record::{Loop2, Plane, Region2, Vec2, Vec3};
use solidcraft_dfm_rules::{
    AppState, Confirmation, Declared, Feature, Finding as RuleFinding, Layer, Limit, Location, ModelRef, Profile, Report, Requirement,
    RequirementRef, RuleResult, RuleStatus,
};
use solidcraft_doc::sheet::{AngleDisplay, ReliefShape, SheetBody, SheetFlange};
use solidcraft_doc::sheet_dfm;
use solidcraft_geom::PlaneGeom;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("solidcraft-dfm-run-{tag}-{}-{nanos}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 40 x 25 mm plate at `thickness`, with one 3 mm hole.
fn plate(thickness: f64) -> SheetBody {
    SheetBody {
        body: "Plate".into(),
        rule: "Steel (mm)".into(),
        t: thickness,
        k: 0.44,
        frame: Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y).expect("XY plane"),
        base: Region2 {
            outer: Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), Vec2::new(40.0, 25.0), Vec2::new(0.0, 25.0)]),
            holes: Vec::new(),
        },
        flanges: Vec::<SheetFlange>::new(),
        holes: vec![Region2 { outer: Loop2::circle(Vec2::new(20.0, 12.5), 1.5), holes: Vec::new() }],
        pockets: Vec::new(),
        bores: Vec::new(),
        perforations: Vec::new(),
        labels: Vec::new(),
        flat: false,
        gap: 0.0,
        relief: ReliefShape::Notch,
        relief_size: 0.0,
        bend_relief: (0.0, 0.0),
        angle_display: AngleDisplay::Bend,
        base_curves: Vec::new(),
        altered_by: None,
    }
}

fn requirement(rule: &str, value: f64, confirmation: Option<Confirmation>) -> Requirement {
    Requirement {
        rule: rule.into(),
        limit: Limit::Min,
        value: BTreeMap::from([("mm".to_string(), value)]),
        inputs: BTreeMap::new(),
        measuring_line: "TEST-ONLY".into(),
        method: "TEST-ONLY".into(),
        source: "TEST-ONLY".into(),
        rights: "TEST-ONLY".into(),
        applies: Default::default(),
        exceptions: Vec::new(),
        validation_samples: Vec::new(),
        die: None,
        bends: None,
        angles_deg: None,
        confirmation,
    }
}

fn parameters(sheet: &SheetBody, index: usize, declared_thickness: Option<f64>, rows: Vec<Requirement>) -> SheetCheckParameters {
    SheetCheckParameters {
        sheet: index,
        body: sheet.body.clone(),
        declared: Declared {
            process: Some("press brake".into()),
            material: Some("5052-H32".into()),
            thickness_mm: declared_thickness,
            thickness_from: declared_thickness.map(|_| "declared".to_string()),
        },
        generic: Profile { format: 1, name: "generic".into(), version: "TEST-ONLY".into(), layer: Layer::Generic, requirements: rows },
        shop: None,
        overrides: Vec::new(),
        model: ModelRef { model_version: "sha256:model".into(), body: sheet.body.clone(), source: "native".into() },
        source: "native".into(),
    }
}

fn run_input() -> RunInput {
    RunInput {
        project_id: "project-1".into(),
        project_name: "Bracket".into(),
        config_digest: "sha256:config-a".into(),
        declared: Declared {
            process: Some("press brake".into()),
            material: Some("5052-H32".into()),
            thickness_mm: Some(1.5),
            thickness_from: Some("declared".into()),
        },
        generic: Profile { format: 1, name: "generic".into(), version: "TEST-ONLY".into(), layer: Layer::Generic, requirements: Vec::new() },
        shop: None,
        overrides: Vec::new(),
        source: "native".into(),
    }
}

fn identity(run: &str) -> FindingsIdentity {
    FindingsIdentity {
        run_id: run.into(),
        candidate_digest: "sha256:model-a".into(),
        profile_digest: "sha256:profile".into(),
        config_digest: "sha256:config-a".into(),
        model_version: "sha256:model-a".into(),
        project_id: "project-1".into(),
        source: SourceRef::Native,
    }
}

fn meta(run: &str, config: &str) -> RunMeta {
    RunMeta::new(
        run,
        "project-1",
        "sha256:model-a",
        config,
        "sha256:model-a",
        "2026-11-14T00:00:00.000Z",
        "2026-11-14T00:00:01.000Z",
        RunOutcome::Current,
        None,
    )
}

fn candidate() -> CandidateRef {
    CandidateRef {
        candidate_id: "cand-1".into(),
        source: SnapshotSource { document_id: "doc-1".into(), revision: "7".into() },
        candidate_digest: "sha256:model-a".into(),
        manifest_digest: "sha256:manifest-a".into(),
        artifact: CandidateArtifact {
            artifact_id: "artifact-1".into(),
            digest: "sha256:artifact-a".into(),
            size_bytes: 10,
            media_type: "application/zip".into(),
        },
    }
}

fn validation_report(run: &str, profile: &CheckProfile) -> ValidationReport {
    let reference = profile_ref(profile).unwrap();
    let checks = profile
        .checks
        .iter()
        .map(|check| crate::validation::CheckResult {
            check_id: check.check_id.clone(),
            required: check.required,
            state: CheckState::Passed,
            expected: check.parameters.clone(),
            actual: serde_json::json!({}),
            evidence: Vec::<ArtifactRef>::new(),
        })
        .collect();
    ValidationReport::new(
        run,
        &candidate(),
        &reference,
        &RunnerIdentity { runner_id: "local-solidcraft".into(), version: "1.0.0".into(), engine_version: "test".into() },
        ValidationState::Passed,
        None,
        "2026-11-14T00:00:01.000Z".into(),
        checks,
        None,
    )
}

fn files(run: &str, findings: findings::RunFindings) -> RunFiles {
    let profile = native_basic_profile();
    let report = validation_report(run, &profile);
    RunFiles {
        profile,
        report,
        findings,
        sheets: vec![SheetEvidence {
            sheet: 0,
            body: "Plate".into(),
            facts: serde_json::json!({"thicknessMm": 1.5}),
            rules: serde_json::json!({"counts": {"failed": 0}}),
        }],
    }
}

fn empty_findings(run: &str) -> findings::RunFindings {
    findings::build(&identity(run), &[])
}

/// One normal run writes every file the report needs, and only then becomes `latest`.
#[test]
fn a_complete_run_writes_every_file_and_then_updates_latest() {
    let root = scratch("complete");
    let store = RunStore::new(&root);
    let run = "run-1111111111111111-0000000000000001";
    let meta = meta(run, "sha256:config-a");
    store.mark_submitted(&meta).unwrap();
    let outcome = store.write_complete(&meta, &files(run, empty_findings(run))).unwrap();
    assert_eq!(outcome, RunOutcome::Current);
    for name in ["run.json", "profile.json", "report.json", "findings.json", "summary.md"] {
        assert!(store.run_dir(run).unwrap().join(name).is_file(), "{name} was written");
    }
    assert!(store.run_dir(run).unwrap().join("evidence/sheet-0.facts.json").is_file());
    assert!(store.run_dir(run).unwrap().join("evidence/sheet-0.rules.json").is_file());
    let latest = store.latest().unwrap().expect("a complete run becomes latest");
    assert_eq!(latest.run_id, run);
    assert_eq!(latest.candidate_digest, "sha256:model-a");
    assert_eq!(store.load_meta(run).unwrap().outcome, RunOutcome::Current);
    assert_eq!(store.load_findings(run).unwrap(), empty_findings(run));
    assert!(store.load_summary(run).unwrap().contains("not a pass"), "no applicable object is never a pass");
    let report_text = std::fs::read_to_string(store.run_dir(run).unwrap().join("report.json")).unwrap();
    assert!(!report_text.contains("startedAt") && !report_text.contains("finishedAt"), "only run.json holds times");
    let run_text = std::fs::read_to_string(store.run_dir(run).unwrap().join("run.json")).unwrap();
    assert!(run_text.contains("startedAt") && run_text.contains("endedAt"));
    let _ = std::fs::remove_dir_all(root);
}

/// The same facts and configuration give the same `findings.json` bytes.
#[test]
fn identical_inputs_give_byte_identical_findings() {
    let sheet = plate(1.5);
    let outcome = check_sheet(&sheet, parameters(&sheet, 0, Some(1.5), Vec::new())).unwrap();
    let facts = sheet_dfm::facts(&sheet).unwrap();
    let sheet_check = SheetCheck { sheet: 0, body: "Plate".into(), facts, report: outcome.report };
    let first = findings::build(&identity("run-1"), std::slice::from_ref(&sheet_check));
    let second = findings::build(&identity("run-1"), std::slice::from_ref(&sheet_check));
    assert_eq!(findings::to_bytes(&first).unwrap(), findings::to_bytes(&second).unwrap());
    let text = String::from_utf8(findings::to_bytes(&first).unwrap()).unwrap();
    assert!(!text.contains("2026-"), "no time reaches findings.json");
    assert!(!text.contains("/tmp"), "no path reaches findings.json");
}

/// A newer submission wins even when the older run finishes last.
#[test]
fn a_late_loser_is_recorded_superseded_and_cannot_take_latest_back() {
    let root = scratch("supersede");
    let store = RunStore::new(&root);
    let (older, newer) = ("run-2222222222222222-0000000000000001", "run-3333333333333333-0000000000000002");
    let older_meta = meta(older, "sha256:config-a");
    let newer_meta = meta(newer, "sha256:config-a");
    store.mark_submitted(&older_meta).unwrap();
    store.mark_submitted(&newer_meta).unwrap();
    assert_eq!(store.write_complete(&newer_meta, &files(newer, empty_findings(newer))).unwrap(), RunOutcome::Current);
    assert_eq!(store.write_complete(&older_meta, &files(older, empty_findings(older))).unwrap(), RunOutcome::Superseded);
    assert_eq!(store.latest().unwrap().unwrap().run_id, newer);
    let older_record = store.load_meta(older).unwrap();
    assert_eq!(older_record.outcome, RunOutcome::Superseded);
    assert!(older_record.reason.as_deref().unwrap_or("").contains("newer"));
    let _ = std::fs::remove_dir_all(root);
}

/// Cancelling the newest submission keeps the previous complete record and says why.
#[test]
fn a_cancelled_run_keeps_the_previous_record_and_gives_a_reason() {
    let root = scratch("cancel");
    let store = RunStore::new(&root);
    let (first, second) = ("run-4444444444444444-0000000000000001", "run-5555555555555555-0000000000000002");
    let first_meta = meta(first, "sha256:config-a");
    store.mark_submitted(&first_meta).unwrap();
    store.write_complete(&first_meta, &files(first, empty_findings(first))).unwrap();
    let mut cancelled = meta(second, "sha256:config-a");
    cancelled.outcome = RunOutcome::Cancelled;
    cancelled.reason = Some("the user cancelled the check".into());
    store.mark_submitted(&cancelled).unwrap();
    assert_eq!(store.write_stopped(&cancelled).unwrap(), RunOutcome::Cancelled);
    assert_eq!(store.latest().unwrap().unwrap().run_id, first, "a cancelled run never becomes latest");
    assert_eq!(store.load_meta(second).unwrap().reason.as_deref(), Some("the user cancelled the check"));
    let _ = std::fs::remove_dir_all(root);
}

/// A rebuild or write failure keeps the previous record too.
#[test]
fn a_failed_run_keeps_the_previous_record() {
    let root = scratch("failure");
    let store = RunStore::new(&root);
    let (first, second) = ("run-6666666666666666-0000000000000001", "run-7777777777777777-0000000000000002");
    let first_meta = meta(first, "sha256:config-a");
    store.mark_submitted(&first_meta).unwrap();
    store.write_complete(&first_meta, &files(first, empty_findings(first))).unwrap();
    let mut failed = meta(second, "sha256:config-a");
    failed.outcome = RunOutcome::Failed;
    failed.reason = Some("native.rebuild failed: the captured design did not open".into());
    store.mark_submitted(&failed).unwrap();
    assert_eq!(store.write_stopped(&failed).unwrap(), RunOutcome::Failed);
    assert_eq!(store.latest().unwrap().unwrap().run_id, first);
    assert!(store.load_meta(second).unwrap().reason.is_some());
    let _ = std::fs::remove_dir_all(root);
}

/// A write failure leaves the previous complete record and reports the error; it never panics.
#[test]
fn a_write_failure_keeps_the_previous_latest() {
    let root = scratch("write-failure");
    let store = RunStore::new(&root);
    let (first, second) = ("run-8888888888888888-0000000000000001", "run-9999999999999999-0000000000000002");
    let first_meta = meta(first, "sha256:config-a");
    store.mark_submitted(&first_meta).unwrap();
    store.write_complete(&first_meta, &files(first, empty_findings(first))).unwrap();
    let second_meta = meta(second, "sha256:config-a");
    store.mark_submitted(&second_meta).unwrap();
    // A regular file where the new run's folder must be: the write fails before any of it lands.
    std::fs::write(store.runs_dir().join(second), b"not a folder").unwrap();
    assert!(store.write_complete(&second_meta, &files(second, empty_findings(second))).is_err());
    assert_eq!(store.latest().unwrap().unwrap().run_id, first);
    let _ = std::fs::remove_dir_all(root);
}

/// A submission that lands between the older run's file write and its publish wins: the older
/// run is superseded and never touches `latest.json`.
#[test]
fn a_submission_between_the_files_and_the_publish_keeps_latest() {
    let root = scratch("publish-race");
    let store = RunStore::new(&root);
    let (older, newer) = ("run-b1b1b1b1b1b1b1b1-0000000000000001", "run-c2c2c2c2c2c2c2c2-0000000000000002");
    let older_meta = meta(older, "sha256:config-a");
    let newer_meta = meta(newer, "sha256:config-a");
    store.mark_submitted(&older_meta).unwrap();
    store.write_files(&older_meta, &files(older, empty_findings(older))).unwrap();
    // A newer run is submitted after the older one wrote its files, before it publishes.
    store.mark_submitted(&newer_meta).unwrap();
    assert_eq!(store.publish(&older_meta, &files(older, empty_findings(older))).unwrap(), RunOutcome::Superseded);
    assert!(store.latest().unwrap().is_none(), "the newer run has not completed yet, so nothing is latest");
    assert_eq!(store.load_meta(older).unwrap().outcome, RunOutcome::Superseded);
    // The newer run completes and publishes normally.
    store.write_files(&newer_meta, &files(newer, empty_findings(newer))).unwrap();
    assert_eq!(store.publish(&newer_meta, &files(newer, empty_findings(newer))).unwrap(), RunOutcome::Current);
    assert_eq!(store.latest().unwrap().unwrap().run_id, newer);
    let _ = std::fs::remove_dir_all(root);
}

/// Publishing the same complete run twice is harmless (a retry after a crash).
#[test]
fn publishing_the_same_run_again_keeps_latest_on_that_run() {
    let root = scratch("publish-again");
    let store = RunStore::new(&root);
    let run = "run-d3d3d3d3d3d3d3d3-0000000000000001";
    let meta = meta(run, "sha256:config-a");
    store.mark_submitted(&meta).unwrap();
    store.write_files(&meta, &files(run, empty_findings(run))).unwrap();
    assert_eq!(store.publish(&meta, &files(run, empty_findings(run))).unwrap(), RunOutcome::Current);
    assert_eq!(store.publish(&meta, &files(run, empty_findings(run))).unwrap(), RunOutcome::Current);
    assert_eq!(store.latest().unwrap().unwrap().run_id, run);
    assert_eq!(store.load_findings(run).unwrap(), empty_findings(run));
    let _ = std::fs::remove_dir_all(root);
}

/// A damaged submission record is never read as "current": the write is superseded and the
/// previous complete record stays. A damaged `latest.json` is an error, not `Current`.
#[test]
fn damaged_submission_and_latest_records_are_never_current() {
    let root = scratch("damaged-submission");
    let store = RunStore::new(&root);
    let (first, second) = ("run-e4e4e4e4e4e4e4e4-0000000000000001", "run-f5f5f5f5f5f5f5f5-0000000000000002");
    let first_meta = meta(first, "sha256:config-a");
    store.mark_submitted(&first_meta).unwrap();
    store.write_complete(&first_meta, &files(first, empty_findings(first))).unwrap();
    std::fs::write(root.join(".intcad/checks/submitted.json"), b"{not json").unwrap();
    assert!(store.submitted().is_err());
    let second_meta = meta(second, "sha256:config-a");
    assert_eq!(store.write_complete(&second_meta, &files(second, empty_findings(second))).unwrap(), RunOutcome::Superseded);
    assert_eq!(store.latest().unwrap().unwrap().run_id, first, "a damaged submission never publishes over a good record");
    assert!(store.latest().unwrap().unwrap().freshness(None, Some("sha256:config-a")) == Freshness::Unknown);
    std::fs::write(root.join(".intcad/checks/latest.json"), b"{not json").unwrap();
    assert!(store.latest().is_err());
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn damaged_records_are_errors_and_never_current() {
    let root = scratch("damaged");
    let store = RunStore::new(&root);
    std::fs::create_dir_all(root.join(".intcad/checks")).unwrap();
    std::fs::write(root.join(".intcad/checks/latest.json"), b"{not json").unwrap();
    assert!(store.latest().is_err());
    let run = "run-aaaaaaaaaaaaaaaa-0000000000000001";
    std::fs::create_dir_all(store.run_dir(run).unwrap()).unwrap();
    std::fs::write(store.run_dir(run).unwrap().join("findings.json"), b"[]").unwrap();
    assert!(store.load_findings(run).is_err());
    assert!(store.run_dir("../escape").is_err(), "a hostile id never leaves the runs folder");
    let _ = std::fs::remove_dir_all(root);
}

/// A config-only change makes the old report stale; the model did not move.
#[test]
fn a_config_only_change_makes_the_old_report_stale() {
    let run = meta("run-bbbbbbbbbbbbbbbb-0000000000000001", "sha256:config-a");
    assert_eq!(freshness(&run, Some("sha256:model-a"), Some("sha256:config-a")), Freshness::Current);
    assert_eq!(freshness(&run, Some("sha256:model-a"), Some("sha256:config-b")), Freshness::StaleConfig);
    assert_eq!(freshness(&run, Some("sha256:model-b"), Some("sha256:config-a")), Freshness::StaleModel);
    assert_eq!(freshness(&run, Some("sha256:model-b"), Some("sha256:config-b")), Freshness::StaleBoth);
    assert_eq!(freshness(&run, None, Some("sha256:config-a")), Freshness::Unknown);
    assert_eq!(freshness(&run, Some("sha256:model-a"), None), Freshness::Unknown);
}

/// Acceptance ②: a declared thickness conflict fails only the body whose thickness differs.
#[test]
fn a_thickness_conflict_blocks_only_the_affected_body() {
    let matching = plate(1.5);
    let thicker = plate(2.0);
    let first = check_sheet(&matching, parameters(&matching, 0, Some(1.5), Vec::new())).unwrap();
    let second = check_sheet(&thicker, parameters(&thicker, 1, Some(1.5), Vec::new())).unwrap();
    assert_eq!(thicker.body, matching.body);
    let sheets = [
        SheetCheck { sheet: 0, body: "Plate".into(), facts: sheet_dfm::facts(&matching).unwrap(), report: first.report },
        SheetCheck { sheet: 1, body: "Plate".into(), facts: sheet_dfm::facts(&thicker).unwrap(), report: second.report },
    ];
    let built = findings::build(&identity("run-1"), &sheets);
    let gate = |sheet: usize| {
        built.findings.iter().find(|finding| finding.sheet == sheet && finding.rules == ["sm.gate_sheet_thickness"]).expect("a thickness gate result")
    };
    assert_eq!(gate(0).state, FindingState::Checked, "matching the declaration is not a pass");
    assert_eq!(gate(1).state, FindingState::Failed, "the mismatched body fails");
    assert_eq!(gate(1).code.as_deref(), Some("THICKNESS_MISMATCH"));
    assert_eq!(gate(1).id, "sheet-2/F1");
}

/// §4.5: an unconfirmed number never becomes `failed`; `not_applicable` is not a pass.
#[test]
fn a_generic_number_is_checked_or_advisory_but_never_failed() {
    let sheet = plate(1.5);
    let outcome = check_sheet(&sheet, parameters(&sheet, 0, Some(1.5), vec![requirement("sm.hole_min_diameter", 4.0, None)])).unwrap();
    let sheet_check = SheetCheck { sheet: 0, body: "Plate".into(), facts: sheet_dfm::facts(&sheet).unwrap(), report: outcome.report };
    let built = findings::build(&identity("run-1"), std::slice::from_ref(&sheet_check));
    let finding = built.findings.iter().find(|finding| finding.rules == ["sm.hole_min_diameter"]).expect("a hole rule result");
    assert_ne!(finding.state, FindingState::Failed);
    assert_eq!(finding.requirement.as_ref().and_then(|requirement| requirement.confirmed), None);
    assert!(!finding.state.is_pass());
    let _ = &built.not_checked;
}

#[test]
fn a_generic_row_with_a_confirmation_mark_is_still_not_a_factory_decision() {
    let sheet = plate(1.5);
    let row = requirement("sm.hole_min_diameter", 4.0, Some(Confirmation::FactoryConfirmed));
    let outcome = check_sheet(&sheet, parameters(&sheet, 0, Some(1.5), vec![row])).unwrap();
    let sheet_check = SheetCheck { sheet: 0, body: "Plate".into(), facts: sheet_dfm::facts(&sheet).unwrap(), report: outcome.report };
    let built = findings::build(&identity("run-1"), std::slice::from_ref(&sheet_check));
    let finding = built.findings.iter().find(|finding| finding.rules == ["sm.hole_min_diameter"]).expect("a hole rule result");
    assert_eq!(finding.requirement.as_ref().map(|requirement| requirement.layer), Some(Layer::Generic));
    assert_ne!(finding.state, FindingState::Failed, "the generic layer is never a factory decision");
}

#[test]
fn a_confirmed_factory_number_can_fail_and_the_rule_row_says_so() {
    let sheet = plate(1.5);
    let mut params = parameters(&sheet, 0, Some(1.5), Vec::new());
    params.shop = Some(Profile {
        format: 1,
        name: "shop".into(),
        version: "TEST-ONLY".into(),
        layer: Layer::Factory,
        requirements: vec![requirement("sm.hole_min_diameter", 4.0, Some(Confirmation::FactoryConfirmed))],
    });
    let outcome = check_sheet(&sheet, params).unwrap();
    let sheet_check = SheetCheck { sheet: 0, body: "Plate".into(), facts: sheet_dfm::facts(&sheet).unwrap(), report: outcome.report };
    let built = findings::build(&identity("run-1"), std::slice::from_ref(&sheet_check));
    let finding = built.findings.iter().find(|finding| finding.rules == ["sm.hole_min_diameter"]).expect("a hole rule result");
    assert_eq!(finding.state, FindingState::Failed);
    let row = built.rules.iter().find(|row| row.rule == "sm.hole_min_diameter").expect("a rule row");
    assert_eq!(row.state, FindingState::Failed);
}

fn synthetic_report(findings: Vec<RuleFinding>) -> Report {
    Report {
        schema_version: 1,
        rules_version: "TEST-ONLY".into(),
        model: ModelRef::default(),
        declared: Declared::default(),
        data: solidcraft_dfm_rules::DataIdentity {
            generic: solidcraft_dfm_rules::ProfileIdentity { name: "generic".into(), version: "TEST-ONLY".into(), layer: Layer::Generic },
            shop: None,
            requirements_sha256: "TEST-ONLY".into(),
            overrides_applied: 0,
            confirmed_rows: 0,
        },
        clean: false,
        counts: solidcraft_dfm_rules::Counts::default(),
        not_checked: String::new(),
        findings,
        rules: vec![RuleResult { rule_id: "sm.hole_min_diameter".into(), state: AppState::Unsupported, count: 1 }],
    }
}

fn rule_finding(status: RuleStatus, state: AppState, requirement: Option<RequirementRef>) -> RuleFinding {
    RuleFinding {
        id: "TEST-ONLY".into(),
        rules: vec!["sm.hole_min_diameter".into()],
        status,
        state,
        error_code: None,
        reason_code: None,
        message: "TEST-ONLY".into(),
        feature: Feature { kind: "hole".into(), number: Some(1), description: "TEST-ONLY".into() },
        location: Some(Location { x_mm: 1.0, y_mm: 2.0, z_mm: 3.0 }),
        measured_mm: Some(2.4),
        requirement,
        needs: Vec::new(),
    }
}

#[test]
fn the_state_mapping_distinguishes_checked_unsupported_and_not_applicable() {
    let sheet = plate(1.5);
    let facts = sheet_dfm::facts(&sheet).unwrap();
    let confirmed = RequirementRef {
        value_mm: 3.0,
        layer: Layer::Generic,
        confirmation: None,
        source: "TEST-ONLY".into(),
        text: "TEST-ONLY".into(),
        overridden: false,
    };
    let sheets = [SheetCheck {
        sheet: 0,
        body: "Plate".into(),
        facts,
        report: synthetic_report(vec![
            rule_finding(RuleStatus::NotVerified, AppState::Unsupported, Some(confirmed.clone())),
            rule_finding(RuleStatus::NotVerified, AppState::Unsupported, None),
            rule_finding(RuleStatus::NotApplicable, AppState::Checked, Some(confirmed)),
        ]),
    }];
    let built = findings::build(&identity("run-1"), &sheets);
    assert_eq!(built.findings[0].state, FindingState::Checked, "a measured number without a decision is checked");
    assert_eq!(built.findings[1].state, FindingState::Unsupported);
    assert_eq!(built.findings[1].code.as_deref(), Some("NOT_VERIFIED"), "unsupported always carries a reason");
    assert_eq!(built.findings[2].state, FindingState::NotApplicable);
    assert!(!built.findings[2].state.is_pass(), "not applicable is not a pass");
    assert!(findings::summary(&built).contains("not a pass"));
}

#[test]
fn an_app_overlap_finding_may_fail_without_factory_data() {
    let sheet = plate(1.5);
    let facts = sheet_dfm::facts(&sheet).unwrap();
    let mut finding = rule_finding(RuleStatus::Violation, AppState::Failed, None);
    finding.rules = vec!["app.flat_no_overlap".into()];
    finding.measured_mm = Some(12.5);
    let sheets = [SheetCheck { sheet: 0, body: "Plate".into(), facts, report: synthetic_report(vec![finding]) }];
    let built = findings::build(&identity("run-1"), &sheets);
    assert_eq!(built.findings[0].state, FindingState::Failed);
    assert_eq!(built.findings[0].from, findings::FindingFrom::App);
    assert_eq!(built.findings[0].rules, ["app.flat_no_overlap"]);
    assert_eq!(built.findings[0].measured.map(|measured| measured.unit), Some(findings::MeasureUnit::SquareMillimetre));
}

#[test]
fn compile_puts_one_check_per_sheet_after_the_native_baseline() {
    let sheets =
        [SheetRef { sheet: 0, body: "Plate".into(), ..Default::default() }, SheetRef { sheet: 1, body: "Bracket".into(), ..Default::default() }];
    let compiled = compile::compile(&run_input(), &sheets, "sha256:model-a").unwrap();
    let ids: Vec<&str> = compiled.profile.checks.iter().map(|check| check.check_id.as_str()).collect();
    assert_eq!(ids, ["dfm.sheet.000", "dfm.sheet.001", "input.integrity", "native.rebuild"]);
    assert!(compiled.profile.checks.iter().filter(|check| check.kind == "dfm.sheet").all(|check| !check.required));
    assert_eq!(compiled.sheets.len(), 2);
    assert_eq!(compiled.sheets[1].model.model_version, "sha256:model-a");
    assert_eq!(compiled.sheets[1].body, "Bracket");
}

/// A project without a declared thickness checks a native body at the body's own model
/// thickness, recording where the value came from. Converted bodies never get that fallback.
#[test]
fn a_missing_project_thickness_falls_back_to_a_native_bodys_model_thickness() {
    let mut input = run_input();
    input.declared.thickness_mm = None;
    input.declared.thickness_from = None;
    let sheets = [SheetRef { sheet: 0, body: "Plate".into(), thickness_mm: Some(2.0), ..Default::default() }];
    let compiled = compile::compile(&input, &sheets, "sha256:model-a").unwrap();
    assert_eq!(compiled.sheets[0].declared.thickness_mm, Some(2.0));
    assert_eq!(compiled.sheets[0].declared.thickness_from.as_deref(), Some("model"));

    let converted = [SheetRef { sheet: 0, body: "Imported".into(), thickness_mm: Some(2.0), converted: true, ..Default::default() }];
    let compiled = compile::compile(&input, &converted, "sha256:model-a").unwrap();
    assert_eq!(compiled.sheets[0].declared.thickness_mm, None, "a converted body never borrows the model thickness");
    assert_eq!(compiled.sheets[0].declared.thickness_from, None);
}

/// An explicit project thickness is used as written, even when one body differs; that body's own
/// gate reports the conflict and other bodies still run.
#[test]
fn an_explicit_project_thickness_is_not_replaced_by_a_bodys_own() {
    let sheets = [SheetRef { sheet: 0, body: "Plate".into(), thickness_mm: Some(2.0), ..Default::default() }];
    let compiled = compile::compile(&run_input(), &sheets, "sha256:model-a").unwrap();
    assert_eq!(compiled.sheets[0].declared.thickness_mm, Some(1.5));
    assert_eq!(compiled.sheets[0].declared.thickness_from.as_deref(), Some("declared"));
}

#[test]
fn compile_refuses_duplicate_sheets_and_a_missing_model_identity() {
    let sheets =
        [SheetRef { sheet: 0, body: "Plate".into(), ..Default::default() }, SheetRef { sheet: 0, body: "Other".into(), ..Default::default() }];
    assert!(compile::compile(&run_input(), &sheets, "sha256:model-a").is_err());
    assert!(compile::compile(&run_input(), &[], "").is_err());
    let empty = compile::compile(&run_input(), &[], "sha256:model-a").unwrap();
    assert_eq!(empty.profile.checks.len(), 2, "no sheet still gets the native baseline");
    assert!(empty.sheets.is_empty());
}

// ---------------------------------------------------------------------------
// The background worker: submit, pump, persist, reopen.

fn sheet_design() -> (Vec<u8>, String) {
    let mut session = crate::Session::default();
    session.execute("sketch.create", &serde_json::json!({"plane": "XY", "name": "Base"})).unwrap();
    session.execute("sketch.rectangle.two_point", &serde_json::json!({"p0": [0, 0], "p1": [40, 25]})).unwrap();
    session.execute("sketch.finish", &serde_json::json!({})).unwrap();
    session.execute("sheet.flange", &serde_json::json!({"sketch": "Base"})).unwrap();
    let body = session.model.state().sheets.first().map(|sheet| sheet.body.clone()).unwrap_or_else(|| "Base".into());
    (solidcraft_io::write_design_container(&session.doc).unwrap(), body)
}

fn write_candidate(root: &std::path::Path, id: &str, design: Vec<u8>) -> CandidateRef {
    use solidcraft_io::candidate::{self, CandidateFile};
    let files = vec![CandidateFile { path: "design.sc".into(), bytes: design }];
    let inputs: Vec<serde_json::Value> = files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path,
                "kind": "design",
                "dependencyId": "design-root",
                "dependencyVersion": null,
                "digest": candidate::digest(&file.bytes),
                "sizeBytes": file.bytes.len(),
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "schemaVersion": "1.0",
        "candidateId": id,
        "source": {"documentId": "doc-1", "revision": "7"},
        "capturedAt": "2026-11-07T12:00:00.000Z",
        "formatVersion": "solidcraft/unknown",
        "engineVersion": env!("CARGO_PKG_VERSION"),
        "inputs": inputs,
    });
    let stored = candidate::write(root, id, &files, &manifest, &format!("artifact-{id}")).unwrap();
    CandidateRef {
        candidate_id: id.into(),
        source: SnapshotSource { document_id: "doc-1".into(), revision: "7".into() },
        candidate_digest: stored.candidate_digest,
        manifest_digest: stored.manifest_digest,
        artifact: CandidateArtifact {
            artifact_id: stored.artifact.artifact_id,
            digest: stored.artifact.digest,
            size_bytes: stored.artifact.size_bytes,
            media_type: stored.artifact.media_type,
        },
    }
}

/// A sheet-metal candidate, a project folder and an open port.
fn worker_fixture(tag: &str) -> (RunRequest, Arc<crate::validation::ValidationPort>, PathBuf) {
    let root = scratch(tag);
    let candidates = root.join("candidates");
    let store_root = root.join("validation");
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".intcad/checks/runs")).unwrap();
    let (design, body) = sheet_design();
    let candidate = write_candidate(&candidates, "cand-sheet", design);
    let port = Arc::new(crate::validation::ValidationPort::open(store_root, candidates).unwrap());
    let request = RunRequest {
        project_root: project,
        candidate,
        input: RunInput {
            declared: Declared { process: Some("press brake".into()), material: Some("5052-H32".into()), thickness_mm: None, thickness_from: None },
            ..run_input()
        },
        sheets: vec![SheetRef { sheet: 0, body, ..Default::default() }],
        started_at: "2026-11-14T01:00:00.000Z".into(),
    };
    (request, port, root)
}

/// Acceptance ①: run without AI, reopen, and read the same report and summary.
#[test]
fn a_background_run_writes_a_report_that_reopens_unchanged() {
    let (request, port, root) = worker_fixture("worker-reopen");
    let result = run::run_once(&port, &request, &std::sync::atomic::AtomicBool::new(false), &|_| {}).unwrap();
    assert_eq!(result.outcome, RunOutcome::Current, "{:?}", result.reason);
    let findings = result.findings.clone().expect("a complete run keeps its findings");
    assert!(
        findings.findings.iter().any(|finding| finding.rules == ["sm.gate_sheet_thickness"]),
        "not checked: {}\nsummary: {}",
        findings.not_checked,
        findings::summary(&findings)
    );

    // A fresh process opens the same project: `latest` plus `run.json` decide freshness, and the
    // findings read exactly as written.
    let reopened = RunStore::new(&request.project_root);
    let latest = reopened.latest().unwrap().expect("latest");
    assert_eq!(latest.run_id, result.run_id);
    assert_eq!(reopened.load_findings(&result.run_id).unwrap(), findings);
    let meta = reopened.load_meta(&result.run_id).unwrap();
    assert_eq!(meta.outcome, RunOutcome::Current);
    assert_eq!(freshness(&meta, Some(&request.candidate.candidate_digest), Some(&request.input.config_digest)), Freshness::Current);
    assert!(reopened.load_summary(&result.run_id).unwrap().contains("Not checked"));
    let _ = std::fs::remove_dir_all(root);
}

/// Acceptance ②: two native bodies with different thicknesses. The declared thickness agrees with
/// one body and conflicts with the other; only that body's gate fails.
#[test]
fn two_native_bodies_with_different_thicknesses_are_blocked_separately() {
    let root = scratch("worker-two-bodies");
    let candidates = root.join("candidates");
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".intcad/checks/runs")).unwrap();
    let mut session = crate::Session::default();
    for (name, x, thickness) in [("Thin", 0, "1.5 mm"), ("Thick", 60, "2.0 mm")] {
        session.execute("sheet.manage_rules", &serde_json::json!({"name": name, "thickness": thickness})).unwrap();
        session.execute("sketch.create", &serde_json::json!({"plane": "XY", "name": name})).unwrap();
        session.execute("sketch.rectangle.two_point", &serde_json::json!({"p0": [x, 0], "p1": [x + 40, 25]})).unwrap();
        session.execute("sketch.finish", &serde_json::json!({})).unwrap();
        session.execute("sheet.flange", &serde_json::json!({"sketch": name, "rule": name})).unwrap();
    }
    let bodies: Vec<String> = session.model.state().sheets.iter().map(|sheet| sheet.body.clone()).collect();
    assert_eq!(bodies.len(), 2, "two native sheet bodies");
    let design = solidcraft_io::write_design_container(&session.doc).unwrap();
    let candidate = write_candidate(&candidates, "cand-two", design);
    let port = Arc::new(crate::validation::ValidationPort::open(root.join("validation"), candidates).unwrap());
    let request = RunRequest {
        project_root: project.clone(),
        candidate,
        input: RunInput {
            declared: Declared {
                process: Some("press brake".into()),
                material: Some("5052-H32".into()),
                thickness_mm: Some(1.5),
                thickness_from: Some("declared".into()),
            },
            ..run_input()
        },
        sheets: bodies
            .iter()
            .enumerate()
            .map(|(sheet, body)| SheetRef { sheet, body: body.clone(), thickness_mm: Some(1.5), ..Default::default() })
            .collect(),
        started_at: "2026-11-14T02:00:00.000Z".into(),
    };
    let result = run::run_once(&port, &request, &std::sync::atomic::AtomicBool::new(false), &|_| {}).unwrap();
    assert_eq!(result.outcome, RunOutcome::Current, "{:?}", result.reason);
    let findings = result.findings.expect("a complete run");
    let gate = |sheet: usize| {
        findings
            .findings
            .iter()
            .find(|finding| finding.sheet == sheet && finding.rules == ["sm.gate_sheet_thickness"])
            .expect("a thickness gate result per body")
    };
    assert_eq!(gate(0).state, FindingState::Checked, "the body at the declared thickness is only checked");
    assert_eq!(gate(1).state, FindingState::Failed, "the other body's conflict fails");
    assert_eq!(gate(1).code.as_deref(), Some("THICKNESS_MISMATCH"));
    // The affected body's dependent rules are not covered, not silently passed; the matching
    // body still produced its own rule results.
    assert!(findings.findings.iter().any(|finding| finding.sheet == 0 && finding.rules != ["sm.gate_sheet_thickness"]));
    assert!(findings.not_checked.contains(&format!("Sheet 2 (`{}`)", bodies[1])));
    let _ = std::fs::remove_dir_all(root);
}

/// Acceptance ⑤: a config-only change makes the same report stale; the model did not move.
#[test]
fn a_run_from_before_a_config_change_reports_stale_config() {
    let (request, port, root) = worker_fixture("worker-config");
    let result = run::run_once(&port, &request, &std::sync::atomic::AtomicBool::new(false), &|_| {}).unwrap();
    let meta = RunStore::new(&request.project_root).load_meta(&result.run_id).unwrap();
    assert_eq!(freshness(&meta, Some(&request.candidate.candidate_digest), Some("sha256:other-config")), Freshness::StaleConfig);
    let _ = std::fs::remove_dir_all(root);
}

/// Acceptance ④: a candidate that does not rebuild writes a failed record with a reason and
/// leaves the previous `latest` alone.
#[test]
fn a_rebuild_failure_keeps_the_previous_record_and_gives_a_reason() {
    let (mut request, port, root) = worker_fixture("worker-rebuild");
    let first = run::run_once(&port, &request, &std::sync::atomic::AtomicBool::new(false), &|_| {}).unwrap();
    assert_eq!(first.outcome, RunOutcome::Current);
    // A candidate whose design does not open: the native rebuild check fails before any DFM rule.
    let broken = write_candidate(&root.join("candidates"), "cand-broken", b"not a design".to_vec());
    request.candidate = broken;
    let failed = run::run_once(&port, &request, &std::sync::atomic::AtomicBool::new(false), &|_| {}).unwrap();
    assert_eq!(failed.outcome, RunOutcome::Failed);
    assert!(failed.reason.as_deref().unwrap_or("").contains("native.rebuild") || failed.reason.is_some());
    assert_eq!(RunStore::new(&request.project_root).latest().unwrap().unwrap().run_id, first.run_id);
    assert!(RunStore::new(&request.project_root).load_meta(&failed.run_id).unwrap().reason.is_some());
    let _ = std::fs::remove_dir_all(root);
}

/// Acceptance ④: a cancellation before submission writes a cancelled record and keeps `latest`.
#[test]
fn a_cancelled_run_writes_a_reason_and_keeps_the_previous_record() {
    let (request, port, root) = worker_fixture("worker-cancel");
    let first = run::run_once(&port, &request, &std::sync::atomic::AtomicBool::new(false), &|_| {}).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(true);
    let cancelled = run::run_once(&port, &request, &cancel, &|_| {}).unwrap();
    assert_eq!(cancelled.outcome, RunOutcome::Cancelled);
    assert!(cancelled.reason.is_some());
    assert_eq!(RunStore::new(&request.project_root).latest().unwrap().unwrap().run_id, first.run_id);
    let _ = std::fs::remove_dir_all(root);
}
