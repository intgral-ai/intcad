//! The profile data layer against INTdfm's recordings and bundled gauge tables.
//!
//! The digest constants below were produced by CPython from the same committed shop profiles:
//! `json.dumps(requirements, sort_keys=True, separators=(",", ":"), allow_nan=False)` then SHA-256.
//! Both shop profiles use float literals, so the typed rows reproduce them exactly.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use solidcraft_dfm_rules::profile_data::{ApplyGap, applying, effective, gauge_systems_from_json, gauges_within, nearest_gauge, requirements_sha256};
use solidcraft_dfm_rules::{Applies, Limit, Profile, Requirement};

#[derive(Deserialize)]
struct Fixture {
    generic: Profile,
    shop: Option<Profile>,
}

fn fixture(name: &str) -> Fixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/intdfm").join(format!("{name}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn shop(name: &str) -> Profile {
    fixture(name).shop.unwrap_or_else(|| panic!("{name} has a shop"))
}

#[test]
fn sha256_matches_python_for_the_factory_recording() {
    let shop = shop("hole_min_diameter.factory.below");
    assert_eq!(requirements_sha256(&shop.requirements).expect("finite"), "ecccd58c53d318b8fc31847cb3d1c6e25909d892ee92f41c2f858a0b4201e6f4");
}

#[test]
fn sha256_matches_python_for_the_6061_recording() {
    let shop = shop("shop_not_applicable");
    assert_eq!(requirements_sha256(&shop.requirements).expect("finite"), "08496715242764abe7fb8dd50b7bade31f2cc7d594c29941bda823fd3d841783");
}

#[test]
fn sha256_changes_when_a_row_changes() {
    let mut shop = shop("shop_not_applicable");
    let before = requirements_sha256(&shop.requirements).expect("finite");
    let row = shop.requirements.first_mut().expect("one row");
    row.value.insert("mm".into(), 2.0);
    assert_ne!(requirements_sha256(&shop.requirements).expect("finite"), before);
}

#[test]
fn effective_keeps_the_generic_layer_when_no_shop_is_given() {
    let generic = fixture("hole_min_diameter.above").generic;
    assert_eq!(effective(&generic, None), generic.requirements);
}

#[test]
fn effective_replaces_the_shop_rules_and_appends_the_shop_rows() {
    let generic = fixture("hole_min_diameter.above").generic;
    let shop = shop("hole_min_diameter.factory.below");
    let rows = effective(&generic, Some(&shop));
    assert_eq!(rows.len(), generic.requirements.len());
    assert_eq!(rows.iter().filter(|row| row.rule == "sm.hole_min_diameter").count(), 1);
    assert_eq!(rows.last().map(|row| row.rule.as_str()), Some("sm.hole_min_diameter"));
    assert_eq!(rows.last(), shop.requirements.first());
}

#[test]
fn applying_finds_the_material_and_reports_a_missing_temper() {
    let shop = shop("shop_not_applicable");

    let (rows, gap) = applying(&shop, "press brake", "6061-T6", 1.5);
    assert_eq!(gap, None);
    assert_eq!(rows.len(), 1);

    // The shop lists 6061 only with a temper, as 6061-T6.
    let (rows, gap) = applying(&shop, "press brake", "6061", 1.5);
    assert!(rows.is_empty());
    assert_eq!(gap, Some(ApplyGap::Temper));

    let (rows, gap) = applying(&shop, "press brake", "5052-H32", 1.5);
    assert!(rows.is_empty());
    assert_eq!(gap, Some(ApplyGap::Material));
}

#[test]
fn applying_narrows_process_then_thickness() {
    let text = r#"{
      "format": 1, "name": "TEST-ONLY-shop", "version": "TEST-ONLY-1", "layer": "factory",
      "requirements": [{
        "rule": "sm.hole_min_diameter", "limit": "min", "value": { "mm": 1.0 }, "inputs": {},
        "measuring_line": "TEST-ONLY", "method": "TEST-ONLY", "source": "TEST-ONLY",
        "rights": "TEST-ONLY",
        "applies": { "process": ["press brake"], "material": [], "thickness_mm": [1.0, 2.0] },
        "exceptions": [], "validation_samples": []
      }]
    }"#;
    let shop: Profile = serde_json::from_str(text).expect("a profile");

    let (rows, gap) = applying(&shop, "PRESS BRAKE", "anything", 1.5);
    assert_eq!(gap, None);
    assert_eq!(rows.len(), 1);

    let (rows, gap) = applying(&shop, "laser cut", "anything", 1.5);
    assert!(rows.is_empty());
    assert_eq!(gap, Some(ApplyGap::Process));

    let (rows, gap) = applying(&shop, "press brake", "anything", 2.5);
    assert!(rows.is_empty());
    assert_eq!(gap, Some(ApplyGap::Thickness));
    assert_eq!(ApplyGap::Thickness.as_str(), "thickness");
}

const TEST_GAUGES: &str = r#"{
  "systems": [
    { "system": "TEST-ONLY-a", "standard": "TEST-ONLY", "source": "TEST-ONLY", "rights": "TEST-ONLY",
      "gauges": [ {"gauge": 10, "mm": 1.5}, {"gauge": 11, "mm": 1.0}, {"gauge": 12, "mm": 0.75} ] },
    { "system": "TEST-ONLY-b", "standard": "TEST-ONLY", "source": "TEST-ONLY", "rights": "TEST-ONLY",
      "gauges": [ {"gauge": 10, "mm": 1.6}, {"gauge": 11, "mm": 1.1} ] }
  ]
}"#;

#[test]
fn gauge_systems_keep_the_files_order() {
    let systems = gauge_systems_from_json(TEST_GAUGES).expect("TEST-ONLY gauges parse");
    assert_eq!(systems.iter().map(|system| system.name.as_str()).collect::<Vec<_>>(), ["TEST-ONLY-a", "TEST-ONLY-b"]);
    assert_eq!(systems[0].gauges()[0].mm, 1.5);
}

#[test]
fn nearest_gauge_finds_the_closest_and_keeps_the_first_tie() {
    let systems = gauge_systems_from_json(TEST_GAUGES).expect("TEST-ONLY gauges parse");
    let nearest = nearest_gauge(&systems[0], 1.4).expect("rows");
    assert_eq!((nearest.number, nearest.mm), (10, 1.5));
    let tie = nearest_gauge(&systems[0], 0.875).expect("rows");
    assert_eq!(tie.number, 11);
}

#[test]
fn gauges_within_is_closest_first_and_keeps_the_files_order_on_a_tie() {
    let systems = gauge_systems_from_json(TEST_GAUGES).expect("TEST-ONLY gauges parse");
    let found = gauges_within(&systems, 1.55, 0.051).expect("rows");
    assert_eq!(found.iter().map(|gauge| gauge.number).collect::<Vec<_>>(), [10, 10]);
    assert_eq!(found[0].system, "TEST-ONLY-a");
    assert_eq!(found[1].system, "TEST-ONLY-b");
}

#[test]
fn a_gate_tolerance_names_the_closest_gauge() {
    let systems = gauge_systems_from_json(TEST_GAUGES).expect("TEST-ONLY gauges parse");
    let found = gauges_within(&systems, 1.5, 0.02).expect("rows");
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].system.as_str(), found[0].number, found[0].mm), ("TEST-ONLY-a", 10, 1.5));
}

fn one_row(value: f64) -> Requirement {
    Requirement {
        rule: "sm.hole_min_diameter".into(),
        limit: Limit::Min,
        value: BTreeMap::from([("mm".to_string(), value)]),
        inputs: BTreeMap::new(),
        measuring_line: "TEST-ONLY".into(),
        method: "TEST-ONLY".into(),
        source: "TEST-ONLY".into(),
        rights: "TEST-ONLY".into(),
        applies: Applies::default(),
        exceptions: Vec::new(),
        validation_samples: Vec::new(),
        die: None,
        bends: None,
        confirmation: None,
    }
}

#[test]
fn sha256_refuses_non_finite_like_allow_nan_false() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(requirements_sha256(&[one_row(value)]).is_err(), "{value}");
    }
}
