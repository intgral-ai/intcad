//! A project folder: the `.intgral/` folder that keeps a design's settings beside it.
//!
//! A project is a design file next to a `.intgral/` folder holding `project.toml` (identity and
//! defaults) and `ci.toml` (checks and the data they use). This module reads, validates and
//! creates those files. It never writes over an existing project: opening one reads it as it is.
//!
//! The files are TOML because people edit them. A file newer than this build is refused instead
//! of being rewritten, and a damaged file names the file and the position of the problem.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The layout this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// The folder that marks a project.
pub const FOLDER: &str = ".intgral";
/// The manifest inside it.
pub const MANIFEST: &str = "project.toml";
/// The check configuration inside it.
pub const CI: &str = "ci.toml";

/// The longest project name this build accepts, in characters.
pub const MAX_NAME_CHARS: usize = 128;

/// What can stop a project file from being read or made.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("no project folder at or above {0}")]
    NoProject(String),
    #[error("{file}: {message}")]
    Parse { file: String, message: String },
    #[error("{file} uses schema version {found}; this build reads {supported} and will not change the file")]
    SchemaTooNew { file: String, found: u32, supported: u32 },
    #[error("{file}: {message}")]
    Invalid { file: String, message: String },
    #[error("{path} already exists; choose another name or folder")]
    TargetExists { path: String },
    #[error("{path} is already inside a project")]
    AlreadyInProject { path: String },
    #[error("{path}: {message}")]
    Io { path: String, message: String },
}

impl ProjectError {
    fn io(path: &Path, error: &std::io::Error) -> Self {
        Self::Io { path: path.display().to_string(), message: error.to_string() }
    }
}

/// `project.toml`: the project's identity and its defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub schema_version: u32,
    /// Created once and never changed; the design's link to this project uses it.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub process: String,
    /// A material name as the data files spell it, temper included (`5052-H32`); empty: none.
    #[serde(default)]
    pub material: String,
    /// The declared thickness in millimetres; `None`: each sheet body's own thickness is used.
    #[serde(default)]
    pub thickness_mm: Option<f64>,
    /// Design files inside the project, relative to it.
    #[serde(default)]
    pub designs: Vec<String>,
}

/// `ci.toml`: whether checks run, which data they use, and the project's overrides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiConfig {
    pub schema_version: u32,
    /// Automatic checks after a model change. Manual checks work either way.
    pub enabled: bool,
    #[serde(default)]
    pub on_manual_edit: ManualEdit,
    pub data: DataRef,
    #[serde(default)]
    pub checks: Checks,
    /// `[[override]]` tables. Called `override` in the file, which is a keyword in Rust.
    #[serde(default, rename = "override")]
    pub overrides: Vec<Override>,
}

/// What a human edit does to an earlier result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManualEdit {
    /// Mark the old result stale; check again on request.
    #[default]
    Stale,
    /// Check again after a human edit.
    Rerun,
}

/// Which data a check uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataKind {
    /// The generic public values that ship with the build.
    Generic,
    /// A shop's own table, copied into the project.
    Factory,
    /// A published standard, copied into the project.
    Standard,
}

/// The chosen data: its name, version and content digest, plus where the project keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRef {
    pub kind: DataKind,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// SHA-256 of the bytes the project keeps; empty for `generic`.
    #[serde(default)]
    pub sha256: String,
    /// The copy inside the project, relative to it; empty for `generic`.
    #[serde(default)]
    pub file: String,
}

/// Which rules a project does not run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checks {
    #[serde(default)]
    pub skip: Vec<String>,
}

/// One project override: a rule's number replaced, with who replaced it and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub rule: String,
    pub limit: OverrideLimit,
    pub value: BTreeMap<String, f64>,
    pub reason: String,
    pub by: String,
    #[serde(default)]
    pub date: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideLimit {
    Min,
    Max,
}

/// A project folder and its two files, read and validated.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    root: PathBuf,
    manifest: ProjectManifest,
    ci: CiConfig,
}

impl Project {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    pub fn ci(&self) -> &CiConfig {
        &self.ci
    }

    /// The `.intgral` folder.
    pub fn folder(&self) -> PathBuf {
        self.root.join(FOLDER)
    }

    /// Find the nearest folder at or above `start` that holds `.intgral/project.toml`.
    ///
    /// `start` may be a design file or a folder; a file is taken as its folder.
    pub fn locate(start: &Path) -> Option<PathBuf> {
        let mut at = if start.is_dir() { start.to_path_buf() } else { start.parent()?.to_path_buf() };
        loop {
            if at.join(FOLDER).join(MANIFEST).is_file() {
                return Some(at);
            }
            if !at.pop() {
                return None;
            }
        }
    }

    /// Read and validate the project at `root`. Nothing is written.
    pub fn open(root: &Path) -> Result<Self, ProjectError> {
        let folder = root.join(FOLDER);
        let manifest_path = folder.join(MANIFEST);
        let ci_path = folder.join(CI);
        if !manifest_path.is_file() {
            return Err(ProjectError::NoProject(root.display().to_string()));
        }
        let manifest: ProjectManifest = read_toml(&manifest_path)?;
        let ci: CiConfig = read_toml(&ci_path)?;
        validate_manifest(&manifest, &manifest_path)?;
        validate_ci(&ci, &ci_path)?;
        Ok(Self { root: root.to_path_buf(), manifest, ci })
    }

    /// Read the project that holds `design_or_folder`, if there is one.
    pub fn of(design_or_folder: &Path) -> Result<Option<Self>, ProjectError> {
        match Self::locate(design_or_folder) {
            None => Ok(None),
            Some(root) => Self::open(&root).map(Some),
        }
    }
}

/// The result of making a project: its folder, its manifest, and the empty design's file.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatedProject {
    pub project: Project,
    pub design: PathBuf,
}

/// Make a project folder at `parent/name`: the folder, `.intgral/`, its templates and an empty
/// design. The folder must not exist. On any failure nothing is left behind, so a retry starts
/// from the same state and never looks like a project.
pub fn create(parent: &Path, name: &str, design_name: &str) -> Result<CreatedProject, ProjectError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ProjectError::Invalid { file: "project".into(), message: "the project name is empty".into() });
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ProjectError::Invalid {
            file: "project".into(),
            message: format!("the project name is longer than {MAX_NAME_CHARS} characters"),
        });
    }
    if name.contains(['/', '\\']) || name.contains('\0') || name == "." || name == ".." {
        return Err(ProjectError::Invalid { file: "project".into(), message: "the project name cannot contain a path separator".into() });
    }
    if !parent.is_dir() {
        return Err(ProjectError::Io { path: parent.display().to_string(), message: "the chosen folder is not a folder".into() });
    }
    let root = parent.join(name);
    if root.exists() {
        return Err(ProjectError::TargetExists { path: root.display().to_string() });
    }
    let design = root.join(design_file_name(if design_name.trim().is_empty() { name } else { design_name }));
    fs::create_dir(&root).map_err(|error| ProjectError::io(&root, &error))?;
    // Build the inside under a temporary name, then give it its real name: a half-written
    // `.intgral` never exists.
    let building = root.join(format!("{FOLDER}.new"));
    let outcome = build_project(&root, &building, name, &design);
    match outcome {
        Ok(()) => {}
        Err(error) => {
            let _ = fs::remove_dir_all(&building);
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
    }
    let folder = root.join(FOLDER);
    if let Err(error) = fs::rename(&building, &folder) {
        let _ = fs::remove_dir_all(&building);
        let _ = fs::remove_dir_all(&root);
        return Err(ProjectError::io(&folder, &error));
    }
    let project = Project::open(&root)?;
    // An empty design file is written only after the project exists, so `open` above saw the
    // project the user will get. A failed write leaves the folder, which the caller removes.
    if let Err(error) = write_empty_design(&design) {
        let _ = fs::remove_dir_all(&root);
        return Err(error);
    }
    Ok(CreatedProject { project, design })
}

/// Put a `.intgral/` folder beside an existing, saved design file. The design is never moved or
/// rewritten; only the folder and its templates are made. Refused when a project already covers
/// the design, so opening an old design never changes it by itself.
pub fn attach(design: &Path) -> Result<CreatedProject, ProjectError> {
    if !design.is_file() {
        return Err(ProjectError::Io { path: design.display().to_string(), message: "the design file does not exist; save it first".into() });
    }
    let Some(root) = design.parent() else {
        return Err(ProjectError::Io { path: design.display().to_string(), message: "the design has no folder".into() });
    };
    if Project::locate(root).is_some() {
        return Err(ProjectError::AlreadyInProject { path: root.display().to_string() });
    }
    let folder = root.join(FOLDER);
    let building = root.join(format!("{FOLDER}.new"));
    if folder.exists() || building.exists() {
        return Err(ProjectError::TargetExists { path: folder.display().to_string() });
    }
    let name = design.file_stem().and_then(|stem| stem.to_str()).map(str::trim).filter(|stem| !stem.is_empty()).unwrap_or("design").to_string();
    if let Err(error) = build_project(root, &building, &name, design) {
        let _ = fs::remove_dir_all(&building);
        return Err(error);
    }
    if let Err(error) = fs::rename(&building, &folder) {
        let _ = fs::remove_dir_all(&building);
        return Err(ProjectError::io(&folder, &error));
    }
    let project = Project::open(root)?;
    Ok(CreatedProject { project, design: design.to_path_buf() })
}

/// The design file a project starts with: the chosen name with the design extension.
fn design_file_name(name: &str) -> String {
    let stem: String = name
        .trim()
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    let stem = stem.trim_matches([' ', '.', '-']);
    let stem = if stem.is_empty() { "design" } else { stem };
    format!("{stem}.sc")
}

/// A UUID v4-looking project identity from the standard library's randomly seeded hasher plus the
/// clock and process id. It is a project identity, never a credential, and compiles for wasm,
/// where only reading a project is offered.
fn new_project_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let pid = u64::from(std::process::id());
    let first_state = RandomState::new();
    let second_state = RandomState::new();
    let mut first = first_state.build_hasher();
    first.write_u128(nanos);
    first.write_u64(pid);
    let mut second = second_state.build_hasher();
    second.write_u128(nanos.rotate_left(64));
    second.write_u64(pid);
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&first.finish().to_be_bytes());
    bytes[8..].copy_from_slice(&second.finish().to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

fn build_project(root: &Path, folder: &Path, name: &str, design: &Path) -> Result<(), ProjectError> {
    fs::create_dir(folder).map_err(|error| ProjectError::io(folder, &error))?;
    for inside in ["specs", "checks/runs"] {
        let path = folder.join(inside);
        fs::create_dir_all(&path).map_err(|error| ProjectError::io(&path, &error))?;
    }
    let manifest = ProjectManifest {
        schema_version: SCHEMA_VERSION,
        id: new_project_id(),
        name: name.to_string(),
        process: "sheet-metal".into(),
        material: String::new(),
        thickness_mm: None,
        designs: vec![design.strip_prefix(root).unwrap_or(design).to_string_lossy().replace('\\', "/")],
    };
    let ci = CiConfig {
        schema_version: SCHEMA_VERSION,
        enabled: true,
        on_manual_edit: ManualEdit::Stale,
        data: DataRef { kind: DataKind::Generic, name: "generic".into(), version: String::new(), sha256: String::new(), file: String::new() },
        checks: Checks::default(),
        overrides: Vec::new(),
    };
    write_text(&folder.join(MANIFEST), &manifest_text(&manifest))?;
    write_text(&folder.join(CI), &ci_text(&ci))?;
    write_text(&folder.join("design-intent.md"), "# Design intent\n\nWhat this design is for, and what must not change.\n")?;
    write_text(&folder.join("design-intent.toml"), "schema_version = 1\n")?;
    write_text(&folder.join("specs/requirements.toml"), "schema_version = 1\n")?;
    write_text(&folder.join(".gitignore"), "cache/\n")?;
    Ok(())
}

/// The `project.toml` text for this manifest. Written as a template so its order and comments
/// stay readable; every value is escaped the way TOML escapes it.
fn manifest_text(manifest: &ProjectManifest) -> String {
    let mut text = String::new();
    text.push_str("# This project's identity and defaults. Edits are read on the next open.\n");
    text.push_str(&format!("schema_version = {}\n", manifest.schema_version));
    text.push_str(&format!("id = {}\n", toml_string(&manifest.id)));
    text.push_str(&format!("name = {}\n", toml_string(&manifest.name)));
    text.push_str(&format!("process = {}\n", toml_string(&manifest.process)));
    text.push_str(&format!("material = {}\n", toml_string(&manifest.material)));
    if let Some(thickness) = manifest.thickness_mm {
        text.push_str(&format!("# Declared thickness in mm; without it each sheet body uses its own.\nthickness_mm = {thickness}\n"));
    } else {
        text.push_str("# thickness_mm = 2.0\n");
    }
    let designs: Vec<String> = manifest.designs.iter().map(|design| toml_string(design)).collect();
    text.push_str(&format!("designs = [{}]\n", designs.join(", ")));
    text
}

fn ci_text(ci: &CiConfig) -> String {
    let mut text = String::new();
    text.push_str("# Checks: whether they run by themselves, and which data they use.\n");
    text.push_str(&format!("schema_version = {}\n", ci.schema_version));
    text.push_str(&format!("enabled = {}\n", ci.enabled));
    text.push_str(&format!(
        "on_manual_edit = {}\n",
        toml_string(match ci.on_manual_edit {
            ManualEdit::Stale => "stale",
            ManualEdit::Rerun => "rerun",
        })
    ));
    text.push_str("\n[data]\n");
    text.push_str(&format!(
        "kind = {}\n",
        toml_string(match ci.data.kind {
            DataKind::Generic => "generic",
            DataKind::Factory => "factory",
            DataKind::Standard => "standard",
        })
    ));
    text.push_str(&format!("name = {}\n", toml_string(&ci.data.name)));
    text.push_str(&format!("version = {}\n", toml_string(&ci.data.version)));
    text.push_str(&format!("sha256 = {}\n", toml_string(&ci.data.sha256)));
    text.push_str(&format!("file = {}\n", toml_string(&ci.data.file)));
    text.push_str("\n[checks]\n");
    let skip: Vec<String> = ci.checks.skip.iter().map(|rule| toml_string(rule)).collect();
    text.push_str(&format!("skip = [{}]\n", skip.join(", ")));
    for rule in &ci.overrides {
        text.push_str("\n[[override]]\n");
        text.push_str(&format!("rule = {}\n", toml_string(&rule.rule)));
        text.push_str(&format!(
            "limit = {}\n",
            toml_string(match rule.limit {
                OverrideLimit::Min => "min",
                OverrideLimit::Max => "max",
            })
        ));
        let value = rule.value.iter().map(|(name, number)| format!("{name} = {number}")).collect::<Vec<_>>().join(", ");
        text.push_str(&format!("value = {{ {value} }}\n"));
        text.push_str(&format!("reason = {}\n", toml_string(&rule.reason)));
        text.push_str(&format!("by = {}\n", toml_string(&rule.by)));
        text.push_str(&format!("date = {}\n", toml_string(&rule.date)));
    }
    text
}

/// A TOML basic string, quoted and escaped by the `toml` crate.
fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ProjectError> {
    let text = fs::read_to_string(path).map_err(|error| ProjectError::io(path, &error))?;
    let value: toml::Value = toml::from_str(&text)
        .map_err(|error| ProjectError::Parse { file: path.display().to_string(), message: error.to_string().trim().to_string() })?;
    let version = value.get("schema_version").and_then(toml::Value::as_integer).unwrap_or(0);
    if version > i64::from(SCHEMA_VERSION) {
        return Err(ProjectError::SchemaTooNew {
            file: path.display().to_string(),
            found: u32::try_from(version).unwrap_or(u32::MAX),
            supported: SCHEMA_VERSION,
        });
    }
    value
        .try_into()
        .map_err(|error: toml::de::Error| ProjectError::Parse { file: path.display().to_string(), message: error.to_string().trim().to_string() })
}

fn validate_manifest(manifest: &ProjectManifest, path: &Path) -> Result<(), ProjectError> {
    let invalid = |message: String| ProjectError::Invalid { file: path.display().to_string(), message };
    if manifest.schema_version == 0 {
        return Err(invalid("schema_version must be 1 or later".into()));
    }
    if manifest.id.trim().is_empty() {
        return Err(invalid("id is empty".into()));
    }
    if manifest.name.trim().is_empty() {
        return Err(invalid("name is empty".into()));
    }
    if let Some(thickness) = manifest.thickness_mm
        && (!thickness.is_finite() || thickness <= 0.0)
    {
        return Err(invalid("thickness_mm must be a finite number greater than 0".into()));
    }
    for design in &manifest.designs {
        if design.is_empty() || Path::new(design).is_absolute() || design.split(['/', '\\']).any(|part| part == "..") {
            return Err(invalid(format!("designs holds `{design}`, which is not a path inside the project")));
        }
    }
    Ok(())
}

fn validate_ci(ci: &CiConfig, path: &Path) -> Result<(), ProjectError> {
    let invalid = |message: String| ProjectError::Invalid { file: path.display().to_string(), message };
    if ci.schema_version == 0 {
        return Err(invalid("schema_version must be 1 or later".into()));
    }
    if ci.data.kind != DataKind::Generic {
        if ci.data.name.trim().is_empty() || ci.data.version.trim().is_empty() {
            return Err(invalid("a factory or standard data choice needs a name and a version".into()));
        }
        if ci.data.file.is_empty() || Path::new(&ci.data.file).is_absolute() || ci.data.file.split(['/', '\\']).any(|part| part == "..") {
            return Err(invalid("the chosen data needs a file inside the project".into()));
        }
        if ci.data.sha256.len() != 64 || !ci.data.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid("the chosen data needs a SHA-256 content digest".into()));
        }
    }
    for rule in &ci.overrides {
        if rule.reason.trim().is_empty() {
            return Err(invalid(format!("the override of `{}` has no reason", rule.rule)));
        }
        if rule.by.trim().is_empty() {
            return Err(invalid(format!("the override of `{}` has no name of who wrote it", rule.rule)));
        }
        if rule.value.is_empty() || rule.value.values().any(|number| !number.is_finite()) {
            return Err(invalid(format!("the override of `{}` needs a finite value", rule.rule)));
        }
    }
    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<(), ProjectError> {
    let mut file = fs::File::create(path).map_err(|error| ProjectError::io(path, &error))?;
    file.write_all(text.as_bytes()).map_err(|error| ProjectError::io(path, &error))?;
    file.sync_all().map_err(|error| ProjectError::io(path, &error))
}

/// Write the empty design a new project starts with. The design format is the host's own: this
/// is the smallest file that opens as an empty design, so a project is usable before the app
/// saves over it.
fn write_empty_design(path: &Path) -> Result<(), ProjectError> {
    let bytes = solidcraft_io::write_design_container(&solidcraft_doc::Document::default()).map_err(|error| ProjectError::Invalid {
        file: path.display().to_string(),
        message: format!("the empty design could not be written: {error}"),
    })?;
    let mut file = fs::File::create(path).map_err(|error| ProjectError::io(path, &error))?;
    file.write_all(&bytes).map_err(|error| ProjectError::io(path, &error))
}

#[cfg(test)]
mod tests;
