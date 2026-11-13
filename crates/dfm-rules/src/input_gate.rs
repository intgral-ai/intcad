//! The input gate's policy on plain data: what a file's text and the reader's facts mean.
//!
//! This is the Rust port of `gate.py`'s `from_scan`, `from_outcome` and the helpers they use. It
//! is pure: it reads no file, runs no geometry engine and looks at no clock. The rows run in the
//! same fixed order as INTdfm's: what the text decides first, then the reader's facts, then the
//! solid's validity. A refusal is a whole-part `not_verified`.
//!
//! The gate's `measured` value is carried in [`Finding::measured_mm`], the crate's one numeric
//! measured slot. For this gate the value is a count of solids, not a length; the report's
//! quantity unit is `solids`, as in INTdfm. The per-unit identity is not part of this crate's
//! `Finding`, so it is documented here rather than invented in the type.

use serde::{Deserialize, Serialize};

use crate::{AppState, Feature, Finding, RuleStatus};

/// The input gate's rule id, from INTdfm's catalog.
pub const INPUT_GATE: &str = "sm.gate_input_solid";

/// The triangle-share limit. Placeholder: nothing has run on a shop's parts, public files peak at
/// 15% and meshes at 99 to 100%.
pub const TRIANGLE_SHARE_LIMIT: f64 = 0.5;

const MESH_NEEDS: &str = "A STEP export of the exact solid model, not a mesh.";
const FRESH_EXPORT: &str = "A fresh STEP export of the part.";

/// What a STEP file's text says, counted from its entities. `None` at the call site means the file
/// is not a STEP exchange file, which is INTdfm's `Scan.census is None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Census {
    /// `PRODUCT_DEFINITION` entities.
    #[serde(default)]
    pub products: usize,
    /// `NEXT_ASSEMBLY_USAGE_OCCURRENCE` entities.
    #[serde(default)]
    pub links: usize,
    /// Instances naming `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS` or `FACETED_BREP`, once each.
    #[serde(default)]
    pub solids: usize,
    /// `ADVANCED_FACE` plus `FACE_SURFACE` entities.
    #[serde(default)]
    pub brep_faces: usize,
    /// `FACETED_BREP` by its exact name; also counted in `solids` and `mesh_entities`.
    #[serde(default)]
    pub faceted_breps: usize,
    /// `FACETED_BREP`, or any name containing `TESSELLATED` or `TRIANGULATED`.
    #[serde(default)]
    pub mesh_entities: usize,
}

/// Why the geometry engine stopped before it produced facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    /// The engine's time limit was reached.
    TimeLimit,
    /// The engine stopped without a result.
    Failed,
}

/// What the reader measured on the single solid. Only the fields the input gate reads are here;
/// the rule layer's richer facts stay in [`crate::Facts`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SolidFacts {
    #[serde(default)]
    pub volume_mm3: f64,
    #[serde(default)]
    pub faces: usize,
    /// Planar, one wire, three straight edges.
    #[serde(default)]
    pub triangle_faces: usize,
    #[serde(default)]
    pub shells_closed: bool,
    #[serde(default)]
    pub analyzer_valid: bool,
    #[serde(default)]
    pub bop_valid: bool,
}

/// The reader's facts for one file. Every field the input gate reads is here, including the ones
/// that only decide whether a shape came back and the raw log lines the report keeps.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReaderFacts {
    #[serde(default)]
    pub read_done: bool,
    #[serde(default)]
    pub roots: usize,
    #[serde(default)]
    pub transferred: usize,
    #[serde(default)]
    pub shapes: usize,
    #[serde(default)]
    pub shape_null: bool,
    #[serde(default)]
    pub solids: usize,
    #[serde(default)]
    pub shells: usize,
    #[serde(default)]
    pub faces: usize,
    /// Shells outside any solid.
    #[serde(default)]
    pub loose_shells: usize,
    /// Faces outside any shell.
    #[serde(default)]
    pub loose_faces: usize,
    /// Edges outside any face.
    #[serde(default)]
    pub loose_edges: usize,
    /// Lower case, sorted, unique, blanks removed.
    #[serde(default)]
    pub length_units: Vec<String>,
    #[serde(default)]
    pub transfer_fails: usize,
    /// Only when exactly one solid was read.
    #[serde(default)]
    pub solid: Option<SolidFacts>,
    /// Raw `reader repaired: ... xN` and `reader load check: ... xN` lines.
    #[serde(default)]
    pub log: Vec<String>,
}

/// What the engine returned: the reader's facts, no reader, or a stopped engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome<'a> {
    /// `None` means a read was asked for and none came back.
    Read(Option<&'a ReaderFacts>),
    /// The engine stopped before it could return facts.
    Stopped(Stop),
}

/// The refusal the file's text decides, or `None` when the reader has to decide.
///
/// `census` is `None` exactly when the text is not a STEP exchange file.
pub fn from_scan(census: Option<&Census>) -> Option<Finding> {
    let Some(census) = census else {
        return Some(refused(
            "INPUT_NOT_STEP",
            "The file is not a STEP exchange file: it lacks the ISO-10303-21 start, a HEADER section, a DATA section or the END-ISO-10303-21 end.",
            "A STEP file of one part, exported from CAD.",
            None,
        ));
    };
    if census.faceted_breps > 0 {
        return Some(refused(
            "INPUT_MESH_ONLY",
            "The file holds a FACETED_BREP, a solid made only of flat polygons, which is an approximation and not exact geometry.",
            MESH_NEEDS,
            None,
        ));
    }
    if census.mesh_entities > 0 && census.brep_faces == 0 {
        return Some(refused(
            "INPUT_MESH_ONLY",
            "The file holds mesh entities and no B-rep faces, so it has no exact geometry to check.",
            MESH_NEEDS,
            None,
        ));
    }
    if census.links > 0 {
        return Some(refused(
            "INPUT_ASSEMBLY",
            format!(
                "The file is an assembly: {}, {}, {}. INTdfm checks one part and does not split files.",
                many(census.products, "product"),
                many(census.links, "assembly link"),
                many(census.solids, "solid definition")
            ),
            "A STEP export of the one part on its own.",
            Some(census.solids),
        ));
    }
    if census.solids >= 2 {
        return Some(refused(
            "INPUT_MULTI_SOLID",
            format!(
                "The file holds {} solids. INTdfm checks one solid and does not split files, and hardware bodies count as solids.",
                census.solids
            ),
            "A STEP export holding only the one bent part.",
            Some(census.solids),
        ));
    }
    None
}

/// The gate's finding for a file the scan let through.
pub fn from_outcome(census: &Census, outcome: Outcome<'_>) -> Finding {
    let reader = match outcome {
        Outcome::Stopped(stop) => return stopped(stop),
        Outcome::Read(reader) => reader,
    };
    let Some(reader) = reader else {
        // A read was asked for and none came back, so nothing is known.
        return stopped(Stop::Failed);
    };
    if let Some(signal) = no_shape(reader) {
        return refused("INPUT_READER_REFUSED", format!("The STEP reader could not get a shape from this file: {signal}."), FRESH_EXPORT, None);
    }
    if reader.solids == 0 {
        return refused(
            "INPUT_NO_SOLID",
            format!(
                "The reader found no solid: {}, where the file names {}. An open shell or a surface model has no inside to check.",
                loose(reader),
                many(census.solids, "solid")
            ),
            "A closed solid model of the part, exported again from CAD.",
            Some(0),
        );
    }
    let Some(solid) = reader.solid.as_ref() else {
        return count_changed(census, reader);
    };
    if reader.solids != census.solids {
        return count_changed(census, reader);
    }
    if reader.loose_shells > 0 || reader.loose_faces > 0 {
        return refused(
            "INPUT_LOOSE_SURFACES",
            format!("The file holds one solid plus {}, so it is more than one body.", loose(reader)),
            "A STEP export holding only the one solid.",
            Some(1),
        );
    }
    if reader.length_units.is_empty() {
        return refused(
            "INPUT_NO_UNIT",
            "The file does not state a length unit, so its sizes can't be trusted.",
            "A STEP export with its units set in CAD.",
            None,
        );
    }
    if solid.triangle_faces as f64 > TRIANGLE_SHARE_LIMIT * solid.faces as f64 {
        return refused(
            "INPUT_MESH_ONLY",
            format!("{} of {} faces are flat triangles, so this looks like a mesh saved as a solid.", solid.triangle_faces, solid.faces),
            MESH_NEEDS,
            None,
        );
    }
    let faults = invalid_faults(solid);
    if !faults.is_empty() {
        return refused(
            "INPUT_SOLID_INVALID",
            format!("The solid is not valid: {}.", faults.join(", ")),
            "A repaired solid, exported again from CAD.",
            None,
        );
    }
    pass(solid.volume_mm3, &reader.length_units)
}

fn count_changed(census: &Census, reader: &ReaderFacts) -> Finding {
    refused(
        "INPUT_SOLID_COUNT_CHANGED",
        format!("The file names {} but the reader returned {} after its own repairs.", many(census.solids, "solid"), reader.solids),
        "A fresh STEP export of the part as one solid.",
        Some(reader.solids),
    )
}

fn stopped(stop: Stop) -> Finding {
    let (code, message, needs) = match stop {
        Stop::TimeLimit => (
            "ENGINE_TIME_LIMIT",
            "The geometry engine did not finish within the time limit, so nothing is known about this part. The limit is a placeholder and depends on this machine.",
            "A longer limit through INTDFM_ENGINE_TIME_LIMIT_S, or a simpler export of the part.",
        ),
        Stop::Failed => {
            ("ENGINE_FAILED", "The geometry engine stopped on this file without a result, so nothing is known about this part.", FRESH_EXPORT)
        }
    };
    refused(code, message, needs, None)
}

/// Why the reader got no usable shape, or `None`.
pub fn no_shape(reader: &ReaderFacts) -> Option<String> {
    if !reader.read_done {
        return Some("the read failed".into());
    }
    if reader.roots == 0 {
        return Some("it found 0 roots to transfer".into());
    }
    if reader.transferred == 0 {
        return Some("0 roots transferred".into());
    }
    if reader.shapes == 0 {
        return Some("0 shapes came out".into());
    }
    if reader.shape_null {
        return Some("the shape is null".into());
    }
    if reader.solids == 0 && reader.shells == 0 && reader.faces == 0 {
        return Some("the shape is empty, with no solids, shells or faces".into());
    }
    if reader.transfer_fails > 0 {
        return Some(format!("the transfer reported {}", many(reader.transfer_fails, "transfer failure")));
    }
    None
}

/// The faults `BRepCheck_Analyzer` and `BRepAlgoAPI_Check` would name, in INTdfm's order.
///
/// Written as one-sided comparisons so NaN fails too, and infinity can't reach canonical JSON.
pub fn invalid_faults(solid: &SolidFacts) -> Vec<&'static str> {
    let mut faults = Vec::new();
    if !(solid.volume_mm3 > 0.0 && solid.volume_mm3 < f64::INFINITY) {
        faults.push("its volume is not above 0");
    }
    if !solid.shells_closed {
        faults.push("a shell is not closed");
    }
    if !solid.analyzer_valid {
        faults.push("BRepCheck_Analyzer finds it invalid");
    }
    if !solid.bop_valid {
        faults.push("BRepAlgoAPI_Check finds it invalid");
    }
    faults
}

/// `"{count} {noun}"` with INTdfm's plural rule: adding `s` unless the count is one.
pub fn many(count: usize, noun: &str) -> String {
    if count == 1 { format!("{count} {noun}") } else { format!("{count} {noun}s") }
}

/// The loose shells and faces phrase, e.g. `"1 loose shell and 0 loose faces"`.
pub fn loose(reader: &ReaderFacts) -> String {
    format!("{} and {}", many(reader.loose_shells, "loose shell"), many(reader.loose_faces, "loose face"))
}

fn refused(code: &str, message: impl Into<String>, need: &str, solids: Option<usize>) -> Finding {
    Finding {
        id: format!("{INPUT_GATE}/{code}"),
        rules: vec![INPUT_GATE.to_string()],
        status: RuleStatus::NotVerified,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(code.to_string()),
        message: message.into(),
        feature: whole_part(),
        location: None,
        measured_mm: solids.map(|count| count as f64),
        requirement: None,
        needs: vec![need.to_string()],
    }
}

fn pass(volume_mm3: f64, length_units: &[String]) -> Finding {
    Finding {
        id: format!("{INPUT_GATE}/pass"),
        rules: vec![INPUT_GATE.to_string()],
        status: RuleStatus::Pass,
        state: AppState::Checked,
        error_code: None,
        reason_code: None,
        message: format!(
            "The file holds one closed solid, valid under BRepCheck_Analyzer and BRepAlgoAPI_Check. Length unit in the file: {}. This only says the file can be read.",
            length_units.join(", ")
        ),
        feature: whole_part(),
        location: None,
        measured_mm: Some(volume_mm3),
        requirement: None,
        needs: Vec::new(),
    }
}

fn whole_part() -> Feature {
    Feature { kind: "part".into(), number: None, description: "whole part".into() }
}
