//! Profile data: the effective requirements, INTdfm's applying test, the requirements digest and
//! the gauge-table parser.
//!
//! This is pure data. It reads no profile file and decides no rule result. The App loads profiles
//! and passes them in; this module only merges them, answers which rows apply, hashes a row list
//! the way INTdfm does, and parses the gauge table the App supplies for the thickness message.
//!
//! # The requirements digest
//!
//! [`requirements_sha256`] reproduces `profile.requirements_sha256`: CPython's
//! `json.dumps(requirements, sort_keys=True, separators=(",", ":"), allow_nan=False)` encoded as
//! UTF-8, then SHA-256. The crate has no Python, so the text is written here: compact separators,
//! sorted object keys, `ensure_ascii=True` escaping, and [`python_float_text`] for every float.
//!
//! One representational limit is unavoidable. INTdfm hashes the decoded JSON, so a source literal
//! `2` stays an integer and is written `2`, while `1.0` stays a float and is written `1.0`.
//! [`Requirement`] stores every number as `f64`, so that distinction is gone by the time this
//! module sees the rows. The digest is therefore exact for profiles whose numbers were all floats
//! (both committed shop profiles are), and for a generic-layer row written `"t": 2` it hashes the
//! normalized `2.0`, which CPython writes `2.0`. A profile that mixes integer and float spellings
//! cannot be reproduced from the typed row without the original JSON bytes.
//!
//! # Gauge tables
//!
//! No third-party gauge table is compiled into this crate. The App supplies the data file; this
//! module parses it and preserves its order and tie behaviour. The INTdfm publisher-derived
//! tables have unclear rights, so only the parser and tests with invented values are published.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::intdfm_report::{python_float_text, python_round};
use crate::{DfmError, Profile, Requirement};

/// The first value that left no applying row, in INTdfm's narrowing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyGap {
    Process,
    Material,
    /// The material is listed only with a temper, and the declared value has none.
    Temper,
    Thickness,
}

impl ApplyGap {
    /// The word `profile.applying` returns in Python.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Material => "material",
            Self::Temper => "temper",
            Self::Thickness => "thickness",
        }
    }
}

/// A shop's requirements replace the generic ones rule by rule, so a part gets one answer.
///
/// The kept generic rows stay in their original order, followed by the shop's rows in file order,
/// exactly as `profile.effective` returns them.
pub fn effective(generic: &Profile, shop: Option<&Profile>) -> Vec<Requirement> {
    let Some(shop) = shop else {
        return generic.requirements.clone();
    };
    let own: BTreeSet<&str> = shop.requirements.iter().map(|row| row.rule.as_str()).collect();
    let mut rows: Vec<Requirement> = generic.requirements.iter().filter(|row| !own.contains(row.rule.as_str())).cloned().collect();
    rows.extend(shop.requirements.iter().cloned());
    rows
}

/// The shop's requirements for these values, and the first value that left none.
///
/// `None` is the gap. The test narrows process, then material (or temper), then thickness, which
/// also catches a material the shop runs on another process. Names compare case-insensitively;
/// INTdfm uses `str.casefold`, and this uses Unicode lowercasing, which differs only for a few
/// non-ASCII spellings no supported profile uses.
pub fn applying(shop: &Profile, process: &str, material: &str, thickness_mm: f64) -> (Vec<Requirement>, Option<ApplyGap>) {
    let process_name = process.to_lowercase();
    let material_name = material.to_lowercase();
    let mut found: Vec<&Requirement> = shop.requirements.iter().filter(|row| listed(&row.applies.process, &process_name)).collect();
    if found.is_empty() {
        return (Vec::new(), Some(ApplyGap::Process));
    }
    let with_process = found.clone();
    found.retain(|row| material_listed(&row.applies.material, &material_name));
    if found.is_empty() {
        let needs_temper =
            with_process.iter().flat_map(|row| row.applies.material.iter()).any(|name| name.to_lowercase().starts_with(&format!("{material_name}-")));
        return (Vec::new(), Some(if needs_temper { ApplyGap::Temper } else { ApplyGap::Material }));
    }
    found.retain(|row| thickness_listed(row.applies.thickness_mm, thickness_mm));
    if found.is_empty() {
        return (Vec::new(), Some(ApplyGap::Thickness));
    }
    (found.into_iter().cloned().collect(), None)
}

fn listed(names: &[String], name: &str) -> bool {
    names.is_empty() || names.iter().any(|item| item.to_lowercase() == name)
}

fn material_listed(names: &[String], material: &str) -> bool {
    if names.is_empty() {
        return true;
    }
    names.iter().any(|name| {
        let entry = name.to_lowercase();
        material == entry || material.starts_with(&format!("{entry}-"))
    })
}

fn thickness_listed(limits: Option<[f64; 2]>, thickness_mm: f64) -> bool {
    match limits {
        None => true,
        Some([low, high]) => low <= thickness_mm && thickness_mm <= high,
    }
}

/// CPython's digest of `json.dumps(requirements, sort_keys=True, separators=(",", ":"),
/// allow_nan=False)`.
///
/// The top-level optional keys that INTdfm's profile format omits (`die`, `bends`, `confirmation`)
/// are dropped when they are `None`. A non-finite number is an error, because `allow_nan=False`
/// refuses it rather than writing `NaN`. See the module note about integer literals.
pub fn requirements_sha256(requirements: &[Requirement]) -> Result<String, DfmError> {
    let mut text = String::from("[");
    for (index, requirement) in requirements.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        ensure_finite(requirement)?;
        let mut value = serde_json::to_value(requirement).map_err(|error| DfmError::Invalid(format!("requirements JSON: {error}")))?;
        if let Value::Object(map) = &mut value {
            for key in ["die", "bends", "angles_deg", "confirmation"] {
                if map.get(key).is_some_and(Value::is_null) {
                    map.remove(key);
                }
            }
        }
        write_compact(&value, &mut text)?;
    }
    text.push(']');
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// `allow_nan=False` refuses a non-finite number. serde_json would turn one into JSON `null`, so
/// the check happens on the typed row first.
fn ensure_finite(requirement: &Requirement) -> Result<(), DfmError> {
    let check = |value: f64| {
        if value.is_finite() { Ok(()) } else { Err(DfmError::Invalid(format!("{}: a number must be finite", requirement.rule))) }
    };
    for value in requirement.value.values() {
        check(*value)?;
    }
    if let Some([low, high]) = requirement.applies.thickness_mm {
        check(low)?;
        check(high)?;
    }
    if let Some(die) = &requirement.die {
        check(die.v_mm)?;
        for value in &die.forms_radius_mm {
            check(*value)?;
        }
        check(die.required_share)?;
    }
    Ok(())
}

fn write_compact(value: &Value, out: &mut String) -> Result<(), DfmError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number_text(number)?),
        Value::String(text) => write_string(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_compact(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_compact(&map[*key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn number_text(number: &serde_json::Number) -> Result<String, DfmError> {
    if number.is_f64() {
        let value = number.as_f64().ok_or_else(|| DfmError::Invalid("requirements JSON: an unreadable number".into()))?;
        return python_float_text(value);
    }
    if let Some(value) = number.as_u64() {
        return Ok(value.to_string());
    }
    if let Some(value) = number.as_i64() {
        return Ok(value.to_string());
    }
    Err(DfmError::Invalid("requirements JSON: an unreadable number".into()))
}

/// CPython's `json.dumps` escaping with its default `ensure_ascii=True`.
fn write_string(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            character if (character as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", character as u32);
            }
            character if (' '..='~').contains(&character) => out.push(character),
            character => {
                let code = character as u32;
                if code <= 0xffff {
                    let _ = write!(out, "\\u{code:04x}");
                } else {
                    let rest = code - 0x1_0000;
                    let high = 0xd800 + (rest >> 10);
                    let low = 0xdc00 + (rest & 0x3ff);
                    let _ = write!(out, "\\u{high:04x}\\u{low:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// One of the bundled gauge tables. The file gives its source and rights.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GaugeSystem {
    #[serde(rename = "system")]
    pub name: String,
    pub standard: String,
    pub source: String,
    pub rights: String,
    /// In the file's order, which is the order every lookup and tie keeps.
    pub gauges: Vec<GaugeRow>,
}

impl GaugeSystem {
    /// The table's rows, in the file's order.
    pub fn gauges(&self) -> &[GaugeRow] {
        &self.gauges
    }
}

/// One row of a gauge table.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GaugeRow {
    pub gauge: u32,
    pub mm: f64,
}

/// A gauge a lookup found, tagged with the table it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Gauge {
    pub system: String,
    pub number: u32,
    pub mm: f64,
}

/// Parse a caller-supplied gauge table. The App supplies the data file, so no third-party
/// table is compiled into the binary.
pub fn gauge_systems_from_json(json: &str) -> Result<Vec<GaugeSystem>, DfmError> {
    #[derive(Deserialize)]
    struct GaugeFile {
        systems: Vec<GaugeSystem>,
    }
    serde_json::from_str::<GaugeFile>(json).map(|file| file.systems).map_err(|error| DfmError::Invalid(format!("gauge data: {error}")))
}

/// Every gauge within this distance of `mm`, closest first, then in the file's order.
pub fn gauges_within(systems: &[GaugeSystem], mm: f64, within_mm: f64) -> Result<Vec<Gauge>, DfmError> {
    let mut found: Vec<(f64, Gauge)> = Vec::new();
    for system in systems {
        for row in &system.gauges {
            let distance = rounded_distance(row.mm, mm);
            if distance <= within_mm {
                found.push((distance, Gauge { system: system.name.clone(), number: row.gauge, mm: row.mm }));
            }
        }
    }
    // Python's `sorted` is stable, so an equal rounded distance keeps the file's order.
    found.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap_or(Ordering::Equal));
    Ok(found.into_iter().map(|(_, gauge)| gauge).collect())
}

/// The system's closest gauge to `mm`. A tie goes to the one the file lists first.
///
/// `None` only for an empty table; every bundled table has rows.
pub fn nearest_gauge(system: &GaugeSystem, mm: f64) -> Option<Gauge> {
    let mut rows = system.gauges.iter();
    let first = rows.next()?;
    let mut best = first;
    let mut best_key = rounded_distance(first.mm, mm);
    for row in rows {
        let key = rounded_distance(row.mm, mm);
        // Python's `min` keeps the current row unless the new key is strictly smaller; a NaN key
        // compares false, so the first row stays.
        if key.partial_cmp(&best_key) == Some(Ordering::Less) {
            best = row;
            best_key = key;
        }
    }
    Some(Gauge { system: system.name.clone(), number: best.gauge, mm: best.mm })
}

/// Python's `round(abs(value - mm), 6)`, including infinities and NaN as keys.
fn rounded_distance(value: f64, mm: f64) -> f64 {
    let distance = (value - mm).abs();
    match python_round(distance, 6) {
        Ok(rounded) => rounded,
        Err(_) => distance,
    }
}
