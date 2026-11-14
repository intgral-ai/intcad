//! Contract and result types for `project.create` and `project.inspect` in `cmd/project.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Make a project folder: a design, `.intgral/` and its templates, offline.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CreateParams {
    /// The project's name, also the folder's name inside `folder`: 1 to 128 characters, with no
    /// path separator.
    pub name: String,
    /// The folder the project folder is made in. It must exist.
    pub folder: String,
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
    /// or above it that holds `.intgral/project.toml` is the project.
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
    /// The rules whose numbers this project overrides.
    pub overridden_rules: Vec<String>,
}
