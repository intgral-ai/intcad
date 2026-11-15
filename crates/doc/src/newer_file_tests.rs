//! Designs a newer build saved: Intcad refuses them, as 0.1.0 did. It never opens one, never
//! re-saves at the newer number and never drops what it cannot read. A *newer container* around a
//! design this build reads keeps its unknown parts (`crate::newer_file`, used by the io layer).

use serde_json::{Value, json};

use crate::format::{DESIGN, FORMAT, Newer, format_string, says_newer};
use crate::{Document, FeatureKind, Model, PlaneRef};

/// A real design: a sketched disc and its extrusion, and a parameter.
fn design() -> Document {
    use solidcraft_sketch::Sketch;
    let mut d = Document::new("Plate");
    d.set_param("depth", "5 mm", Some("mm"), None).unwrap();
    let mut sketch = Sketch::new();
    sketch.add_circle(solidcore_record::Vec2::new(0.0, 0.0), 10.0, None, None).unwrap();
    let base = d.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch }, Some("Base")).unwrap();
    d.add_feature(extrude(base), Some("Pad")).unwrap();
    d
}

fn extrude(sketch: u64) -> FeatureKind {
    FeatureKind::Extrude {
        thin: None,
        sketch: crate::SketchId(sketch),
        profiles: crate::ProfileSel::All,
        extent: serde_json::from_value(json!({"distance": "depth"})).unwrap(),
        operation: crate::Operation::NewBody,
        targets: vec![],
    }
}

fn saved(d: &Document) -> Value {
    serde_json::from_str(&d.to_json()).unwrap()
}

/// `(body name, volume)` of everything the design builds, and the error of each feature.
fn built(d: &Document) -> (Vec<(String, f64)>, Vec<Option<String>>) {
    let mut model = Model::default();
    model.evaluate(d);
    let bodies = model.state().bodies.iter().map(|b| (b.name.clone(), b.body.volume_at(0.01).unwrap())).collect();
    (bodies, model.results.iter().map(|r| r.error.clone()).collect())
}

fn newer() -> String {
    format_string(FORMAT + 1)
}

/// Intcad's rule (0.1.0's): a design a newer build saved is refused before anything in it is
/// read, so it is never opened, never re-saved at that build's number and never loses a field.
#[test]
fn a_newer_format_is_refused_and_never_opened() {
    let mut file = saved(&design());
    file["format"] = json!(newer());
    // A field this build doesn't know must not read as damage: the refusal precedes it.
    file["features"].as_array_mut().unwrap().push(json!({"id": 900, "name": "Lattice1", "type": "gyroid_lattice", "cell": 2.5}));
    let text = file.to_string();
    assert!(says_newer(&text));
    let why = Document::from_json(&text).unwrap_err().to_string();
    assert_eq!(
        why,
        format!("this design was saved by a newer Intcad (file format {}; this version reads formats up to {FORMAT}): update Intcad to open it", FORMAT + 1)
    );
    assert!(!why.contains("unknown variant"), "the feature was never decoded: {why}");
    // Refused again, the same: nothing was upgraded or written back.
    assert_eq!(Document::from_json(&text).unwrap_err().to_string(), why);
}

/// A change this build doesn't have, at a number it writes, is a newer build's too: refused with
/// that change named, before any check judges fields the change may have added.
#[test]
fn an_unknown_pending_change_is_refused() {
    let mut file = saved(&design());
    file["format_pending"] = json!(["a-later-change"]);
    let why = Document::from_json(&file.to_string()).unwrap_err().to_string();
    assert_eq!(
        why,
        format!(
            "this design was saved by a newer Intcad (file format {FORMAT} with `a-later-change`, which this version doesn't know): update Intcad to open it"
        )
    );
}

/// 0.1.0 wrote `solidcraft/51` with five changes pending. The merge folds those five, byte for
/// byte, into `v51/` of the current build (which writes 55): a 0.1.0 design is older, not newer.
#[test]
fn a_0_1_0_design_at_format_51_opens_and_round_trips() {
    let pending = [
        "bridged-counterbore",
        "fillet-turn",
        "hole-direction-own-faces",
        "revolve-axis-origin-expressions",
        "transferred-profile-frame",
    ];
    let original = design();
    let mut file = saved(&original);
    file["format"] = json!("solidcraft/51");
    file["format_pending"] = json!(pending);
    let text = file.to_string();
    assert!(!says_newer(&text), "0.1.0 is older than this build, not newer");
    let opened = Document::from_json(&text).expect("a 0.1.0 design opens");
    assert!(opened.newer_file.is_none(), "nothing of a 0.1.0 file is a newer build's");
    assert_eq!(opened.format, format_string(FORMAT));
    assert_eq!(opened.params, original.params);
    assert_eq!(built(&opened), built(&original), "the same bodies build");
    // Saving writes this build's format, with no pending list, and reads back unchanged.
    let again = saved(&opened);
    assert_eq!(again["format"], json!(format_string(FORMAT)));
    assert!(again.get("format_pending").is_none());
    assert_eq!(Document::from_json(&opened.to_json()).unwrap(), opened);
}

/// A design this build writes stays at this build's format with every field it had: nothing is
/// downgraded on save, and there is no newer-file report to carry.
#[test]
fn a_design_this_build_writes_round_trips_at_its_own_format() {
    let original = design();
    let opened = Document::from_json(&original.to_json()).unwrap();
    assert!(opened.newer_file.is_none());
    assert_eq!(DESIGN.newer(FORMAT, None), None);
    let again = saved(&opened);
    assert_eq!(again["format"], json!(format_string(FORMAT)));
    assert!(again.get("format_pending").is_none());
    assert_eq!(again["params"], saved(&original)["params"]);
    assert_eq!(built(&opened), built(&original));
    assert_eq!(Document::from_json(&opened.to_json()).unwrap(), opened);
}

/// Damage in a file of this build's own format is still damage, never a newer build's.
#[test]
fn damage_and_foreign_tags_are_still_refused() {
    let mut file = saved(&design());
    file["features"][1]["type"] = json!("gyroid_lattice");
    let why = Document::from_json(&file.to_string()).unwrap_err().to_string();
    assert!(why.contains("design file") && !why.contains("newer Intcad"), "a newer report must not hide damage: {why}");
    for (text, says) in [
        // Read first for its resource budgets, a text that isn't JSON is refused there.
        ("not json", "HDR resource admission"),
        ("{\"format\":\"solidcraft/0\",\"name\":\"x\"}", "not a Intcad design (format 0)"),
        ("{\"format\":\"someone-else/9\"}", "not a Intcad design (format `someone-else/9`)"),
    ] {
        let why = Document::from_json(text).unwrap_err().to_string();
        assert!(why.contains(says), "{text}: {why}");
    }
}

/// The report a **newer container** uses (the io layer still keeps unknown parts beside a design
/// it reads): its notice names the container, and a design this build knows keeps everything.
#[test]
fn a_newer_container_report_names_it_and_keeps_its_note() {
    let newer = Newer { format: FORMAT, reads_up_to: FORMAT, unknown_changes: vec!["solidcraft-container/999".into()] };
    let report = crate::newer_file::newer_file(&newer);
    assert!(report.notice().contains("solidcraft-container/999"), "{}", report.notice());
    assert!(report.not_understood.is_empty());
    assert!(crate::newer_file::not_kept(&design()).is_empty());
}
