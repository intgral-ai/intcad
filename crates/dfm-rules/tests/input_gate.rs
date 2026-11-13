//! The input gate against INTdfm's `tests/unit/test_gate.py` cases.
//!
//! Each refusal code has one case, plus the one pass. The expected codes, messages and measured
//! solids come from `gate.py`; the messages quoted here are the fixed text that test file pins.

use solidcraft_dfm_rules::input_gate::{
    Census, INPUT_GATE, Outcome, ReaderFacts, SolidFacts, Stop, from_outcome, from_scan, invalid_faults, loose, many, no_shape,
};
use solidcraft_dfm_rules::{AppState, Finding, RuleStatus};

fn census() -> Census {
    Census { products: 1, links: 0, solids: 1, brep_faces: 7, faceted_breps: 0, mesh_entities: 0 }
}

fn solid() -> SolidFacts {
    SolidFacts { volume_mm3: 1494.1, faces: 12, triangle_faces: 0, shells_closed: true, analyzer_valid: true, bop_valid: true }
}

fn reader() -> ReaderFacts {
    ReaderFacts {
        read_done: true,
        roots: 1,
        transferred: 1,
        shapes: 1,
        shape_null: false,
        solids: 1,
        shells: 1,
        faces: 12,
        loose_shells: 0,
        loose_faces: 0,
        loose_edges: 0,
        length_units: vec!["millimetre".into()],
        transfer_fails: 0,
        solid: Some(solid()),
        log: Vec::new(),
    }
}

fn from_reader(reader: &ReaderFacts, census: &Census) -> Finding {
    from_outcome(census, Outcome::Read(Some(reader)))
}

fn assert_refused(finding: &Finding, code: &str, solids: Option<f64>) {
    assert_eq!(finding.status, RuleStatus::NotVerified, "{code}");
    assert_eq!(finding.state, AppState::Unsupported, "{code}");
    assert_eq!(finding.rules, vec![INPUT_GATE.to_string()], "{code}");
    assert_eq!(finding.reason_code.as_deref(), Some(code));
    assert_eq!(finding.error_code, None);
    assert_eq!(finding.id, format!("{INPUT_GATE}/{code}"));
    assert_eq!(finding.measured_mm, solids, "{code}");
    assert_eq!(finding.needs.len(), 1, "{code}");
    assert!(!finding.needs[0].trim().is_empty(), "{code}");
}

#[test]
fn not_a_step_file() {
    let finding = from_scan(None).expect("a refusal");
    assert_refused(&finding, "INPUT_NOT_STEP", None);
    assert!(finding.message.contains("ISO-10303-21"));
}

#[test]
fn faceted_brep_is_mesh_only() {
    let refused = census();
    let refused = Census { faceted_breps: 1, links: 1, solids: 2, ..refused };
    assert_refused(&from_scan(Some(&refused)).expect("a refusal"), "INPUT_MESH_ONLY", None);
}

#[test]
fn mesh_without_brep_faces_is_mesh_only() {
    let refused = Census { mesh_entities: 1, brep_faces: 0, ..census() };
    assert_refused(&from_scan(Some(&refused)).expect("a refusal"), "INPUT_MESH_ONLY", None);
}

#[test]
fn assembly_is_refused_before_multi_solid() {
    let refused = Census { links: 1, products: 2, solids: 3, ..census() };
    let finding = from_scan(Some(&refused)).expect("a refusal");
    assert_refused(&finding, "INPUT_ASSEMBLY", Some(3.0));
    assert!(finding.message.contains("2 products"));
    assert!(finding.message.contains("1 assembly link"));
    assert!(finding.message.contains("3 solid definitions"));
}

#[test]
fn two_solids_is_multi_solid() {
    let refused = Census { solids: 2, ..census() };
    let finding = from_scan(Some(&refused)).expect("a refusal");
    assert_refused(&finding, "INPUT_MULTI_SOLID", Some(2.0));
    assert!(finding.message.contains("holds 2 solids"));
}

#[test]
fn the_scan_lets_one_solid_through() {
    assert!(from_scan(Some(&census())).is_none());
}

#[test]
fn a_failed_read_is_reader_refused() {
    let reader = ReaderFacts { read_done: false, ..reader() };
    let finding = from_reader(&reader, &census());
    assert_refused(&finding, "INPUT_READER_REFUSED", None);
    assert!(finding.message.contains("the read failed"));
}

#[test]
fn transfer_failures_are_reader_refused() {
    let reader = ReaderFacts { transfer_fails: 2, ..reader() };
    let finding = from_reader(&reader, &census());
    assert_refused(&finding, "INPUT_READER_REFUSED", None);
    assert!(finding.message.contains("2 transfer failures"));
}

#[test]
fn a_shape_with_no_solid_is_no_solid() {
    let reader = ReaderFacts { solids: 0, solid: None, ..reader() };
    let finding = from_reader(&reader, &census());
    assert_refused(&finding, "INPUT_NO_SOLID", Some(0.0));
    assert!(finding.message.contains("0 loose shells and 0 loose faces"));
    assert!(finding.message.contains("where the file names 1 solid"));
}

#[test]
fn a_reader_that_lost_solids_is_count_changed() {
    let reader = ReaderFacts { solids: 2, solid: None, loose_shells: 1, ..reader() };
    let finding = from_reader(&reader, &census());
    assert_refused(&finding, "INPUT_SOLID_COUNT_CHANGED", Some(2.0));
    assert!(finding.message.contains("names 1 solid but the reader returned 2"));
}

#[test]
fn a_solid_plus_a_loose_face_is_loose_surfaces() {
    let reader = ReaderFacts { loose_faces: 1, length_units: Vec::new(), ..reader() };
    let finding = from_reader(&reader, &census());
    assert_refused(&finding, "INPUT_LOOSE_SURFACES", Some(1.0));
    assert!(finding.message.contains("0 loose shells and 1 loose face"));
}

#[test]
fn a_file_without_units_is_no_unit() {
    let reader = ReaderFacts { length_units: Vec::new(), ..reader() };
    let finding = from_reader(&reader, &census());
    assert_refused(&finding, "INPUT_NO_UNIT", None);
}

#[test]
fn more_than_half_triangles_is_mesh_only() {
    let reader = ReaderFacts { solid: Some(SolidFacts { faces: 12, triangle_faces: 7, ..solid() }), ..reader() };
    let finding = from_reader(&reader, &census());
    assert_refused(&finding, "INPUT_MESH_ONLY", None);
    assert!(finding.message.contains("7 of 12"));
}

#[test]
fn exactly_half_triangles_passes() {
    let reader = ReaderFacts { solid: Some(SolidFacts { faces: 12, triangle_faces: 6, ..solid() }), ..reader() };
    let finding = from_reader(&reader, &census());
    assert_eq!(finding.status, RuleStatus::Pass);
}

#[test]
fn an_invalid_solid_names_every_fault() {
    let invalid = SolidFacts { volume_mm3: -1.0, shells_closed: false, analyzer_valid: false, bop_valid: false, ..solid() };
    assert_eq!(
        invalid_faults(&invalid),
        vec!["its volume is not above 0", "a shell is not closed", "BRepCheck_Analyzer finds it invalid", "BRepAlgoAPI_Check finds it invalid",]
    );
    let reader = ReaderFacts { solid: Some(invalid), ..reader() };
    assert_refused(&from_reader(&reader, &census()), "INPUT_SOLID_INVALID", None);
}

#[test]
fn nan_volume_is_invalid() {
    let invalid = SolidFacts { volume_mm3: f64::NAN, ..solid() };
    assert_eq!(invalid_faults(&invalid), vec!["its volume is not above 0"]);
}

#[test]
fn an_engine_time_limit_has_its_fixed_need() {
    let finding = from_outcome(&census(), Outcome::Stopped(Stop::TimeLimit));
    assert_refused(&finding, "ENGINE_TIME_LIMIT", None);
    assert!(finding.message.contains("placeholder"));
    assert!(finding.needs[0].contains("INTDFM_ENGINE_TIME_LIMIT_S"));
}

#[test]
fn a_failed_engine_is_engine_failed() {
    let finding = from_outcome(&census(), Outcome::Stopped(Stop::Failed));
    assert_refused(&finding, "ENGINE_FAILED", None);
}

#[test]
fn no_reader_at_all_is_engine_failed() {
    let finding = from_outcome(&census(), Outcome::Read(None));
    assert_refused(&finding, "ENGINE_FAILED", None);
}

#[test]
fn one_valid_solid_passes_and_reports_its_volume() {
    let finding = from_reader(&reader(), &census());
    assert_eq!(finding.status, RuleStatus::Pass);
    assert_eq!(finding.state, AppState::Checked);
    assert_eq!(finding.reason_code, None);
    assert_eq!(finding.error_code, None);
    assert_eq!(finding.measured_mm, Some(1494.1));
    assert!(finding.needs.is_empty());
    assert!(finding.message.contains("Length unit in the file: millimetre"));
    assert!(finding.message.contains("only says the file can be read"));
}

#[test]
fn no_shape_is_checked_in_intdfm_order() {
    let empty = ReaderFacts { roots: 0, transferred: 0, shapes: 0, solids: 0, shells: 0, faces: 0, solid: None, ..reader() };
    assert_eq!(no_shape(&empty).as_deref(), Some("it found 0 roots to transfer"));

    let null = ReaderFacts { shape_null: true, ..reader() };
    assert_eq!(no_shape(&null).as_deref(), Some("the shape is null"));
}

#[test]
fn many_and_loose_match_intdfm() {
    assert_eq!(many(1, "solid"), "1 solid");
    assert_eq!(many(0, "solid"), "0 solids");
    assert_eq!(many(2, "assembly link"), "2 assembly links");
    let reader = ReaderFacts { loose_shells: 1, loose_faces: 0, ..reader() };
    assert_eq!(loose(&reader), "1 loose shell and 0 loose faces");
}
