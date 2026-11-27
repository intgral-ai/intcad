//! Contract and result types for the project commands in `cmd/project.rs` and `cmd/dfm.rs`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Make a project folder: a design, `.intcad/` and its templates, offline.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CreateParams {
    /// The project's name, also the folder's name inside `folder`: 1 to 128 characters, with no
    /// path separator.
    pub name: String,
    /// The folder the project folder is made in. It must exist.
    pub folder: String,
}

/// Put a `.intcad/` folder beside an existing, saved design.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct AttachParams {
    /// The saved design file (default: the open design).
    #[serde(default)]
    pub path: Option<String>,
}

/// What `project.create` made.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateReply {
    /// The project folder, absolute as it was given.
    pub project: String,
    /// The design file inside the project, saved from the design that was open.
    pub design: String,
    /// The project's identity: created once, never changed.
    pub id: String,
    /// The project's name as the folder and `project.toml` spell it.
    pub name: String,
}

/// Describe the project that holds a design, or say there is none. Read-only.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct InspectParams {
    /// A design file or a folder to start from (default: the open design). The nearest folder at
    /// or above it that holds `.intcad/project.toml` is the project.
    #[serde(default)]
    pub path: Option<String>,
}

/// What `project.inspect` found.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InspectReply {
    /// Whether a project was found at or above the path.
    pub has_project: bool,
    /// The project folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<ManifestSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci: Option<CiSummary>,
    /// The critical hole-to-hole dimensions `critical.toml` declares.
    pub critical: Vec<CriticalSummary>,
    /// Why a project folder that exists could not be read. The settings are not shown as valid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `project.toml` as the interface shows it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSummary {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub process: String,
    pub material: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness_mm: Option<f64>,
    pub designs: Vec<String>,
}

/// `ci.toml` as the interface shows it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CiSummary {
    pub schema_version: u32,
    pub enabled: bool,
    /// `stale` or `rerun`.
    pub on_manual_edit: String,
    /// `generic`, `factory` or `standard`.
    pub data_kind: String,
    pub data_name: String,
    pub data_version: String,
    pub data_sha256: String,
    pub data_file: String,
    pub skip: Vec<String>,
    /// Whether a new part ticket's check starts switched on.
    pub new_tickets: bool,
    /// The most rounds of fixing the Assistant takes after a ticket's first automatic check.
    pub rounds: u32,
    /// The rules whose numbers this project overrides.
    pub overridden_rules: Vec<String>,
    pub overrides: Vec<OverrideSummary>,
    /// Whether the selected data copy was read and hashed.
    pub data_available: bool,
    /// Why the selected data cannot be used, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_unavailable: Option<String>,
}

/// One project override as the interface shows it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverrideSummary {
    pub rule: String,
    /// `min` or `max`.
    pub limit: String,
    pub old: BTreeMap<String, f64>,
    pub value: BTreeMap<String, f64>,
    pub reason: String,
    pub by: String,
    pub date: String,
}

/// Change the project's process, material, thickness and automatic checks.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SettingsParams {
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub material: Option<String>,
    /// The declared thickness in mm; omitted keeps it, `null` clears it.
    #[serde(default)]
    pub thickness_mm: Option<f64>,
    /// Clear the declared thickness (each sheet body then uses its own).
    #[serde(default)]
    pub clear_thickness: bool,
    #[serde(default)]
    pub auto_check: Option<bool>,
}

/// Turn automatic checks on or off and set the rules the project skips.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CiParams {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub skip: Option<Vec<String>>,
    /// Whether a new part ticket's check starts switched on.
    #[serde(default, rename = "newTickets")]
    pub new_tickets: Option<bool>,
    /// The most rounds of fixing the Assistant takes after a ticket's first automatic check.
    #[serde(default)]
    pub rounds: Option<u32>,
}

/// Import a whole factory or standard file, or choose the built-in generic data.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DataParams {
    /// `generic`, `factory` or `standard`.
    pub kind: String,
    /// The file to import, for `factory` and `standard`.
    #[serde(default)]
    pub path: Option<String>,
    /// Replace a different data selection the project already has.
    #[serde(default)]
    pub upgrade: bool,
}

/// What `project.data_set` selected.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataReply {
    pub kind: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub file: String,
    /// Whether a file was copied; false when the built-in generic data was chosen.
    pub imported: bool,
}

/// Add or remove one project override.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct OverrideParams {
    pub rule: String,
    /// `min` or `max`; defaults to `min`.
    #[serde(default)]
    pub limit: Option<String>,
    /// The number being replaced.
    #[serde(default)]
    pub old: Option<BTreeMap<String, f64>>,
    pub value: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    /// Remove the matching override instead of adding it.
    #[serde(default)]
    pub remove: bool,
}

/// The project's overrides after an override command.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverridesReply {
    pub overrides: Vec<OverrideSummary>,
}

/// Replace the project's critical hole-to-hole dimensions.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct CriticalParams {
    /// Every critical dimension the project declares. An empty list removes them all.
    #[serde(default)]
    pub critical: Vec<CriticalInput>,
}

/// One critical dimension as a caller writes it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct CriticalInput {
    /// The name a drawing or a person uses for this dimension.
    pub name: String,
    /// A model mm point near the first round through hole's centre.
    pub from_mm: [f64; 3],
    /// A model mm point near the second round through hole's centre.
    pub to_mm: [f64; 3],
    /// The plus-or-minus the drawing allows, in mm.
    pub tolerance_mm: f64,
}

/// One critical dimension as the project reports it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CriticalSummary {
    pub name: String,
    pub from_mm: [f64; 3],
    pub to_mm: [f64; 3],
    pub tolerance_mm: f64,
}

/// The project's critical dimensions after a `project.critical_set`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CriticalReply {
    pub critical: Vec<CriticalSummary>,
}
