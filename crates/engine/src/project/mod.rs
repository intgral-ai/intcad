//! A project folder: the `.intcad/` folder that keeps a design's settings beside it.
//!
//! A project is a design file next to a `.intcad/` folder holding `project.toml` (identity and
//! defaults), `ci.toml` (checks and the data they use) and `critical.toml` (the critical
//! hole-to-hole dimensions a drawing gives with a plus or minus). This module reads, validates
//! and creates those files. It never writes over an existing project: opening one reads it as it
//! is.
//!
//! A new project also gets its Spec files at its root, beside the design: `requirements.md` and
//! `design-intent.md`, ordinary files that the Files panel and the Assistant see. One of those
//! names that is already taken is left as it is. `.intcad/` holds only the app's own files.
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

pub mod freeze;
pub mod tickets;
pub use freeze::{FrozenDataRef, FrozenProject};

/// The layout this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// The folder that marks a project.
pub const FOLDER: &str = ".intcad";
/// The manifest inside it.
pub const MANIFEST: &str = "project.toml";
/// The check configuration inside it.
pub const CI: &str = "ci.toml";
/// The critical hole-to-hole dimensions the project declares.
pub const CRITICAL: &str = "critical.toml";

/// The Spec files a new project starts with, at its root: each name, and the text it starts with.
/// The text is prompts only, with no example values, so nothing in it reads as a requirement.
const SPEC_TEMPLATES: &[(&str, &str)] = &[
    (
        "requirements.md",
        "# Requirements\n\n\
         One requirement per line, with its number and unit, so each line can be cited on its own. \
         Fill in the lines that apply and delete the rest.\n\n\
         - Material and thickness:\n\
         - Overall size limits:\n\
         - Holes, slots and their tolerances:\n\
         - Bends, flanges and smallest wall:\n\
         - Parts it must fit or clear:\n\
         - Finish and quantity:\n",
    ),
    (
        "design-intent.md",
        "# Design intent\n\n\
         What this design is for, and what must not change.\n\n\
         ## Purpose\n\nWhat the part does and where it goes.\n\n\
         ## Must not change\n\nFaces, holes and dimensions that other parts depend on.\n\n\
         ## Free to change\n\nWhat a fix may move, resize or remove.\n",
    ),
];

/// The longest project name this build accepts, in characters.
pub const MAX_NAME_CHARS: usize = 128;
/// The most critical dimensions one project may declare.
pub const MAX_CRITICAL: usize = 64;
/// The largest coordinate or tolerance a critical dimension may use, in mm.
pub const MAX_CRITICAL_MM: f64 = 1.0e6;

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

/// Which rules a project does not run, and how its part tickets are checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checks {
    #[serde(default)]
    pub skip: Vec<String>,
    /// Whether a new part ticket's check starts switched on; a person switches it on the ticket.
    #[serde(default = "checked")]
    pub new_tickets: bool,
    /// The most rounds of fixing the Assistant takes after a ticket's first automatic check, at most
    /// [`MAX_ROUNDS`]; 0: the first check's findings go to a person.
    #[serde(default = "rounds")]
    pub rounds: u32,
}

/// The most rounds of fixing a project may give the Assistant.
pub const MAX_ROUNDS: u32 = 10;

fn checked() -> bool {
    true
}

fn rounds() -> u32 {
    tickets::DEFAULT_ROUNDS
}

impl Default for Checks {
    fn default() -> Self {
        Self { skip: Vec::new(), new_tickets: checked(), rounds: rounds() }
    }
}

/// One project override: a rule's number replaced, with who replaced it and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub rule: String,
    pub limit: OverrideLimit,
    /// The number the project replaces, as the data it came from spelled it.
    #[serde(default)]
    pub old: BTreeMap<String, f64>,
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

/// `critical.toml`: the hole-to-hole dimensions a drawing gives with a plus or minus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CriticalFile {
    pub schema_version: u32,
    /// `[[critical]]` tables. Called `critical` in the file, like `[[override]]` in `ci.toml`.
    #[serde(default, rename = "critical")]
    pub dimensions: Vec<CriticalDimension>,
}

/// One declared critical dimension. Its points are matched to round through holes; the rule
/// counts the bends between the holes' flats.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriticalDimension {
    /// The name a drawing or a person uses for this dimension.
    pub name: String,
    /// A point on the sheet near the first hole's centre, in model mm.
    pub from_mm: [f64; 3],
    /// A point on the sheet near the second hole's centre, in model mm.
    pub to_mm: [f64; 3],
    /// The plus-or-minus the drawing allows, in mm.
    pub tolerance_mm: f64,
}

/// A project folder and its files, read and validated.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    root: PathBuf,
    manifest: ProjectManifest,
    ci: CiConfig,
    critical: Vec<CriticalDimension>,
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

    /// The critical dimensions the project declares, in file order.
    pub fn critical(&self) -> &[CriticalDimension] {
        &self.critical
    }

    /// The `.intcad` folder.
    pub fn folder(&self) -> PathBuf {
        self.root.join(FOLDER)
    }

    /// Find the nearest folder at or above `start` that holds `.intcad/project.toml`.
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
        let critical = read_critical(&folder.join(CRITICAL))?;
        validate_manifest(&manifest, &manifest_path)?;
        validate_ci(&ci, &ci_path)?;
        Ok(Self { root: root.to_path_buf(), manifest, ci, critical })
    }

    /// Read the project that holds `design_or_folder`, if there is one.
    pub fn of(design_or_folder: &Path) -> Result<Option<Self>, ProjectError> {
        match Self::locate(design_or_folder) {
            None => Ok(None),
            Some(root) => Self::open(&root).map(Some),
        }
    }

    /// Change the project defaults; `None` keeps a value and `Some(None)` clears the thickness.
    pub fn update_settings(
        &mut self,
        process: Option<String>,
        material: Option<String>,
        thickness: Option<Option<f64>>,
        auto_check: Option<bool>,
    ) -> Result<(), ProjectError> {
        let mut manifest = self.manifest.clone();
        let mut ci = self.ci.clone();
        if let Some(process) = process {
            manifest.process = process;
        }
        if let Some(material) = material {
            manifest.material = material;
        }
        if let Some(thickness) = thickness {
            manifest.thickness_mm = thickness;
        }
        if let Some(enabled) = auto_check {
            ci.enabled = enabled;
        }
        validate_manifest(&manifest, &self.folder().join(MANIFEST))?;
        validate_ci(&ci, &self.folder().join(CI))?;
        self.write(&manifest, &ci)
    }

    /// Turn automatic checks on or off and choose the rules the project skips.
    pub fn update_ci(&mut self, enabled: Option<bool>, skip: Option<Vec<String>>) -> Result<(), ProjectError> {
        self.update_checks(enabled, skip, None, None)
    }

    /// [`Project::update_ci`], and how part tickets are checked: whether a new ticket's check starts
    /// switched on, and the most rounds of fixing the Assistant takes after a ticket's first
    /// automatic check.
    pub fn update_checks(
        &mut self,
        enabled: Option<bool>,
        skip: Option<Vec<String>>,
        new_tickets: Option<bool>,
        rounds: Option<u32>,
    ) -> Result<(), ProjectError> {
        let mut ci = self.ci.clone();
        if let Some(enabled) = enabled {
            ci.enabled = enabled;
        }
        if let Some(skip) = skip {
            ci.checks.skip = skip;
        }
        if let Some(new_tickets) = new_tickets {
            ci.checks.new_tickets = new_tickets;
        }
        if let Some(rounds) = rounds {
            ci.checks.rounds = rounds;
        }
        validate_ci(&ci, &self.folder().join(CI))?;
        let manifest = self.manifest.clone();
        self.write(&manifest, &ci)
    }

    /// Point the project at a selected data copy. `ci.toml` is written after the copy exists.
    pub fn select_data(&mut self, data: DataRef) -> Result<(), ProjectError> {
        let mut ci = self.ci.clone();
        ci.data = data;
        validate_ci(&ci, &self.folder().join(CI))?;
        let manifest = self.manifest.clone();
        self.write(&manifest, &ci)
    }

    /// Replace the project's overrides after validating each reason and signature.
    pub fn set_overrides(&mut self, overrides: Vec<Override>) -> Result<(), ProjectError> {
        let mut ci = self.ci.clone();
        ci.overrides = overrides;
        validate_ci(&ci, &self.folder().join(CI))?;
        let manifest = self.manifest.clone();
        self.write(&manifest, &ci)
    }

    /// Replace the project's critical dimensions. An empty list removes them all.
    pub fn set_critical(&mut self, dimensions: Vec<CriticalDimension>) -> Result<(), ProjectError> {
        let path = self.folder().join(CRITICAL);
        let text = critical_text(&dimensions);
        validate_critical(&dimensions, &text, &path)?;
        write_text_atomic(&path, &text)?;
        self.critical = dimensions;
        Ok(())
    }

    fn write(&mut self, manifest: &ProjectManifest, ci: &CiConfig) -> Result<(), ProjectError> {
        let folder = self.folder();
        write_text_atomic(&folder.join(MANIFEST), &manifest_text(manifest))?;
        write_text_atomic(&folder.join(CI), &ci_text(ci))?;
        self.manifest = manifest.clone();
        self.ci = ci.clone();
        Ok(())
    }
}

/// The result of making a project: its folder, its manifest, and the empty design's file.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatedProject {
    pub project: Project,
    pub design: PathBuf,
}

/// Make a project folder at `parent/name`: the folder, `.intcad/`, the Spec files and an empty
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
    // `.intcad` never exists.
    let building = root.join(format!("{FOLDER}.new"));
    let outcome = build_project(&root, &building, name, &design);
    match outcome {
        Ok(_) => {}
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

/// Put a `.intcad/` folder beside an existing, saved design file. The design is never moved or
/// rewritten; only `.intcad/` and the Spec files whose names are free are made. Refused when a
/// project already covers the design, so opening an old design never changes it by itself.
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
    let specs = match build_project(root, &building, &name, design) {
        Ok(specs) => specs,
        Err(error) => {
            let _ = fs::remove_dir_all(&building);
            return Err(error);
        }
    };
    if let Err(error) = fs::rename(&building, &folder) {
        let _ = fs::remove_dir_all(&building);
        // Only the Spec files this call made: the folder's own files stay.
        for spec in specs {
            let _ = fs::remove_file(spec);
        }
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

/// Make `.intcad/`'s files in `folder`, then the Spec files at `root` whose names are free. Every
/// new project, made by `create` or `attach`, is made here. Returns the Spec files it made.
fn build_project(root: &Path, folder: &Path, name: &str, design: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    fs::create_dir(folder).map_err(|error| ProjectError::io(folder, &error))?;
    let runs = folder.join("checks/runs");
    fs::create_dir_all(&runs).map_err(|error| ProjectError::io(&runs, &error))?;
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
    write_text(&folder.join(CRITICAL), &critical_text(&[]))?;
    write_text(&folder.join(".gitignore"), "cache/\n")?;
    write_spec_templates(root)
}

/// Write each Spec file at `root` whose name is free. A name already taken (a file, a folder, a
/// link) is the user's and is never opened for writing: `create_new` refuses it, which also
/// leaves no gap between a look and the write. On a failure the files it made are removed.
fn write_spec_templates(root: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    let mut made = Vec::new();
    for &(name, text) in SPEC_TEMPLATES {
        let path = root.join(name);
        let written = match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                made.push(path.clone());
                file.write_all(text.as_bytes()).and_then(|()| file.sync_all())
            }
            Err(_) if fs::symlink_metadata(&path).is_ok() => continue,
            Err(error) => Err(error),
        };
        if let Err(error) = written {
            for spec in &made {
                let _ = fs::remove_file(spec);
            }
            return Err(ProjectError::io(&path, &error));
        }
    }
    Ok(made)
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
    text.push_str("# Part tickets: whether a new ticket's check starts switched on, and the most rounds of fixing\n");
    text.push_str("# the Assistant takes after a ticket's first automatic check.\n");
    text.push_str(&format!("new_tickets = {}\n", ci.checks.new_tickets));
    text.push_str(&format!("rounds = {}\n", ci.checks.rounds));
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
        let old = rule.old.iter().map(|(name, number)| format!("{name} = {number}")).collect::<Vec<_>>().join(", ");
        text.push_str(&format!("old = {{ {old} }}\n"));
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
    if ci.checks.rounds > MAX_ROUNDS {
        return Err(invalid(format!("checks.rounds must be at most {MAX_ROUNDS}")));
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

/// Read `critical.toml`. A project without the file (one made by an older build) declares none.
fn read_critical(path: &Path) -> Result<Vec<CriticalDimension>, ProjectError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ProjectError::io(path, &error)),
    };
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
    let file: CriticalFile = value
        .try_into()
        .map_err(|error: toml::de::Error| ProjectError::Parse { file: path.display().to_string(), message: error.to_string().trim().to_string() })?;
    if file.schema_version == 0 {
        return Err(ProjectError::Invalid { file: path.display().to_string(), message: "schema_version must be 1 or later".into() });
    }
    validate_critical(&file.dimensions, &text, path)?;
    Ok(file.dimensions)
}

/// Validate the declarations and name the `[[critical]]` line that is wrong.
fn validate_critical(dimensions: &[CriticalDimension], text: &str, path: &Path) -> Result<(), ProjectError> {
    let invalid = |line: usize, message: String| ProjectError::Invalid { file: format!("{}:{line}", path.display()), message };
    if dimensions.len() > MAX_CRITICAL {
        return Err(invalid(1, format!("a project declares at most {MAX_CRITICAL} critical dimensions")));
    }
    let mut names: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (index, dimension) in dimensions.iter().enumerate() {
        let line = critical_line(text, index);
        let name = dimension.name.trim();
        if name.is_empty() {
            return Err(invalid(line, "a critical dimension needs a name".into()));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(invalid(line, format!("the name is longer than {MAX_NAME_CHARS} characters")));
        }
        if name.chars().any(char::is_control) {
            return Err(invalid(line, "the name holds a control character".into()));
        }
        if !names.insert(name) {
            return Err(invalid(line, format!("`{name}` is declared twice")));
        }
        for (label, point) in [("from_mm", dimension.from_mm), ("to_mm", dimension.to_mm)] {
            if point.iter().any(|value| !value.is_finite() || value.abs() > MAX_CRITICAL_MM) {
                return Err(invalid(line, format!("{label} needs three finite mm values no larger than {MAX_CRITICAL_MM}")));
            }
        }
        if !dimension.tolerance_mm.is_finite() || dimension.tolerance_mm <= 0.0 || dimension.tolerance_mm > MAX_CRITICAL_MM {
            return Err(invalid(line, "tolerance_mm must be a finite number greater than 0".into()));
        }
    }
    Ok(())
}

/// The 1-based line of the `index`th `[[critical]]` table, or line 1 when it cannot be found.
fn critical_line(text: &str, index: usize) -> usize {
    text.lines().enumerate().filter(|(_, line)| line.trim() == "[[critical]]").nth(index).map(|(zero, _)| zero + 1).unwrap_or(1)
}

/// The `critical.toml` text for these declarations, as a stable template.
fn critical_text(dimensions: &[CriticalDimension]) -> String {
    let mut text = String::new();
    text.push_str("# Critical hole-to-hole dimensions: the ones a drawing gives with a plus or minus.\n");
    text.push_str("# Each point is a model mm point near a round through hole's centre.\n");
    text.push_str(&format!("schema_version = {}\n", SCHEMA_VERSION));
    for dimension in dimensions {
        text.push_str("\n[[critical]]\n");
        text.push_str(&format!("name = {}\n", toml_string(&dimension.name)));
        text.push_str(&format!("from_mm = {}\n", toml_point(dimension.from_mm)));
        text.push_str(&format!("to_mm = {}\n", toml_point(dimension.to_mm)));
        text.push_str(&format!("tolerance_mm = {}\n", toml_number(dimension.tolerance_mm)));
    }
    text
}

fn toml_point(point: [f64; 3]) -> String {
    format!("[{}, {}, {}]", toml_number(point[0]), toml_number(point[1]), toml_number(point[2]))
}

/// A TOML float: `10.0`, never `10`, so reading it back needs no integer-to-float guess.
fn toml_number(value: f64) -> String {
    format!("{value:?}")
}

fn write_text(path: &Path, text: &str) -> Result<(), ProjectError> {
    let mut file = fs::File::create(path).map_err(|error| ProjectError::io(path, &error))?;
    file.write_all(text.as_bytes()).map_err(|error| ProjectError::io(path, &error))?;
    file.sync_all().map_err(|error| ProjectError::io(path, &error))
}

/// Write beside the target and rename, so a reader sees the old file or the new one.
fn write_text_atomic(path: &Path, text: &str) -> Result<(), ProjectError> {
    let temporary = path.with_file_name(format!(".{}.tmp", std::process::id()));
    write_text(&temporary, text)?;
    if fs::rename(&temporary, path).is_err() {
        // Windows refuses to rename over an existing file.
        let _ = fs::remove_file(path);
        fs::rename(&temporary, path).map_err(|error| ProjectError::io(path, &error))?;
    }
    Ok(())
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
