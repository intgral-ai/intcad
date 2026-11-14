//! `project.create` and `project.inspect`: the `.intgral/` folder beside a design.
//!
//! Creating a project is one operation: the folder, its templates, and the open design saved
//! inside it. A failure removes the folder it made, so a retry never finds a half project.
//! Inspecting never writes: it reads the nearest project at or above the design.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::CommandSpec;
use crate::params::{bad, str_};
use crate::schema::TypedContract;
use crate::schema::project::{AttachParams, CiSummary, CreateParams, CreateReply, InspectParams, InspectReply, ManifestSummary};
use crate::{Result, Session};

const CREATE_CONTRACT: TypedContract<CreateParams> = TypedContract::new();
const ATTACH_CONTRACT: TypedContract<AttachParams> = TypedContract::new();
const INSPECT_CONTRACT: TypedContract<InspectParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("project.create", "New Project", crate::checked!("project.create", create))
        .icon("new")
        .noundo()
        .contract(&CREATE_CONTRACT)
        .output(crate::schema::output_schema_for::<CreateReply>)
        .params(
            "name: the project's name, also its folder's name (1 to 128 characters, no path separator); \
             folder: the existing folder the project folder is made in \
             → project (the project folder), design (the design file inside it), id (the project's identity), name. \
             The open design is saved into the project. A refused call leaves no folder behind",
        ),
    CommandSpec::new("project.attach", "Project Beside This Design", crate::checked!("project.attach", attach))
        .icon("folder")
        .noundo()
        .contract(&ATTACH_CONTRACT)
        .output(crate::schema::output_schema_for::<CreateReply>)
        .params(
            "path?: the saved design file (default: the open design) → project (the folder holding .intgral), \
             design, id, name. The design is not moved or rewritten; a design already inside a project is refused",
        ),
    CommandSpec::new("project.inspect", "Project Settings", crate::checked!("project.inspect", inspect))
        .icon("settings")
        .noundo()
        .contract(&INSPECT_CONTRACT)
        .output(crate::schema::output_schema_for::<InspectReply>)
        .params(
            "path?: a design file or folder to start from (default: the open design) → hasProject; root; \
             project: {schemaVersion, id, name, process, material, thicknessMm?, designs}; \
             ci: {schemaVersion, enabled, onManualEdit, dataKind, dataName, dataVersion, dataSha256, dataFile, skip, \
             overriddenRules}; error (when the folder exists but its files cannot be read)",
        ),
];

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_(p, "name").ok_or_else(|| bad("project.create", "missing `name`"))?.trim().to_string();
    let folder = str_(p, "folder").ok_or_else(|| bad("project.create", "missing `folder`"))?.to_string();
    let parent = PathBuf::from(&folder);
    let made = crate::project::create(&parent, &name, &name).map_err(|error| bad("project.create", error.to_string()))?;
    let root = made.project.root().to_path_buf();
    // The design that is open goes into the project, so one call leaves a project that opens and
    // a design that is already saved in it. A save failure removes the folder just made.
    let saved = s.execute("file.save_as", &json!({ "path": made.design.to_string_lossy(), "overwrite": true }));
    if let Err(error) = saved {
        let _ = std::fs::remove_dir_all(&root);
        return Err(error);
    }
    let reply = CreateReply {
        project: root.display().to_string(),
        design: made.design.display().to_string(),
        id: made.project.manifest().id.clone(),
        name: made.project.manifest().name.clone(),
    };
    serde_json::to_value(reply).map_err(|error| bad("project.create", format!("the result could not be written: {error}")))
}

fn attach(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_(p, "path").map(str::to_string).or_else(|| s.path.clone());
    let path = path.ok_or_else(|| bad("project.attach", "the design has no saved path; save it before making a project"))?;
    let made = crate::project::attach(Path::new(&path)).map_err(|error| bad("project.attach", error.to_string()))?;
    let reply = CreateReply {
        project: made.project.root().display().to_string(),
        design: made.design.display().to_string(),
        id: made.project.manifest().id.clone(),
        name: made.project.manifest().name.clone(),
    };
    serde_json::to_value(reply).map_err(|error| bad("project.attach", format!("the result could not be written: {error}")))
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let start = str_(p, "path").map(str::to_string).or_else(|| s.path.clone());
    let Some(start) = start else {
        return reply(InspectReply { has_project: false, root: None, project: None, ci: None, error: None });
    };
    let start = Path::new(&start);
    let Some(root) = crate::project::Project::locate(start) else {
        return reply(InspectReply { has_project: false, root: None, project: None, ci: None, error: None });
    };
    match crate::project::Project::open(&root) {
        Ok(project) => reply(InspectReply {
            has_project: true,
            root: Some(root.display().to_string()),
            project: Some(manifest_summary(project.manifest())),
            ci: Some(ci_summary(project.ci())),
            error: None,
        }),
        Err(error) => {
            reply(InspectReply { has_project: true, root: Some(root.display().to_string()), project: None, ci: None, error: Some(error.to_string()) })
        }
    }
}

fn reply(value: InspectReply) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| bad("project.inspect", format!("the result could not be written: {error}")))
}

fn manifest_summary(manifest: &crate::project::ProjectManifest) -> ManifestSummary {
    ManifestSummary {
        schema_version: manifest.schema_version,
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        process: manifest.process.clone(),
        material: manifest.material.clone(),
        thickness_mm: manifest.thickness_mm,
        designs: manifest.designs.clone(),
    }
}

fn ci_summary(ci: &crate::project::CiConfig) -> CiSummary {
    CiSummary {
        schema_version: ci.schema_version,
        enabled: ci.enabled,
        on_manual_edit: match ci.on_manual_edit {
            crate::project::ManualEdit::Stale => "stale".into(),
            crate::project::ManualEdit::Rerun => "rerun".into(),
        },
        data_kind: match ci.data.kind {
            crate::project::DataKind::Generic => "generic".into(),
            crate::project::DataKind::Factory => "factory".into(),
            crate::project::DataKind::Standard => "standard".into(),
        },
        data_name: ci.data.name.clone(),
        data_version: ci.data.version.clone(),
        data_sha256: ci.data.sha256.clone(),
        data_file: ci.data.file.clone(),
        skip: ci.checks.skip.clone(),
        overridden_rules: ci.overrides.iter().map(|rule| rule.rule.clone()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("solidcraft-project-cmd-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn create_makes_a_project_and_saves_the_open_design_into_it() {
        let dir = scratch("create");
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length": 10, "width": 20, "height": 30})).expect("box");
        let r = s.execute("project.create", &json!({"name": "Bracket A", "folder": dir.to_string_lossy()})).expect("create");
        let root = PathBuf::from(r["project"].as_str().expect("project"));
        let design = PathBuf::from(r["design"].as_str().expect("design"));
        assert_eq!(root, dir.join("Bracket A"));
        assert!(design.is_file());
        assert_eq!(s.path.as_deref(), Some(design.to_string_lossy().as_ref()), "the open design is the project's design");
        assert!(root.join(".intgral/project.toml").is_file());
        // The saved design is the one that was open, not an empty one.
        let bytes = fs::read(&design).expect("read");
        let doc = solidcraft_io::read_design(&bytes).expect("design");
        assert_eq!(doc.features.len(), 1);

        let inspected = s.execute("project.inspect", &json!({})).expect("inspect");
        assert_eq!(inspected["hasProject"], true);
        assert_eq!(inspected["project"]["name"], "Bracket A");
        assert_eq!(inspected["ci"]["enabled"], true);
        assert_eq!(inspected["ci"]["dataKind"], "generic");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_refuses_a_second_project_with_the_same_name() {
        let dir = scratch("duplicate");
        let mut s = Session::default();
        s.execute("project.create", &json!({"name": "Panel", "folder": dir.to_string_lossy()})).expect("first");
        let error = s.execute("project.create", &json!({"name": "Panel", "folder": dir.to_string_lossy()})).expect_err("second");
        assert!(error.to_string().contains("already exists"), "{error}");
        assert!(dir.join("Panel/.intgral/project.toml").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_design_outside_a_project_says_so_and_writes_nothing() {
        let dir = scratch("plain");
        let design = dir.join("plain.sc");
        fs::write(&design, b"not really a design").expect("write");
        let mut s = Session { path: Some(design.to_string_lossy().to_string()), ..Session::default() };
        let inspected = s.execute("project.inspect", &json!({})).expect("inspect");
        assert_eq!(inspected["hasProject"], false);
        assert_eq!(fs::read_dir(&dir).expect("list").count(), 1, "no folder was made");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_reports_a_damaged_project_instead_of_pretending_it_is_valid() {
        let dir = scratch("damaged");
        let mut s = Session::default();
        s.execute("project.create", &json!({"name": "Bracket", "folder": dir.to_string_lossy()})).expect("create");
        fs::write(dir.join("Bracket/.intgral/ci.toml"), "schema_version = 1\nenabled =\n").expect("damage");
        let inspected = s.execute("project.inspect", &json!({})).expect("inspect");
        assert_eq!(inspected["hasProject"], true);
        assert_eq!(inspected["project"], Value::Null);
        let error = inspected["error"].as_str().expect("error");
        assert!(error.contains("ci.toml") && error.contains("line 2"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod attach_tests {
    use super::*;
    use std::fs;

    #[test]
    fn attach_uses_the_open_designs_path_and_inspect_finds_the_project() {
        let dir = std::env::temp_dir().join(format!("solidcraft-attach-cmd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch");
        let design = dir.join("plate.sc");
        fs::write(&design, b"design").expect("design");
        let mut s = Session { path: Some(design.to_string_lossy().to_string()), ..Session::default() };
        let reply = s.execute("project.attach", &json!({})).expect("attach");
        assert_eq!(reply["design"], design.to_string_lossy().as_ref());
        assert_eq!(reply["project"], dir.to_string_lossy().as_ref());
        assert!(dir.join(".intgral/project.toml").is_file());
        let inspected = s.execute("project.inspect", &json!({})).expect("inspect");
        assert_eq!(inspected["hasProject"], true);
        assert_eq!(inspected["project"]["designs"][0], "plate.sc");
        let error = s.execute("project.attach", &json!({})).expect_err("second");
        assert!(error.to_string().contains("already inside a project"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn attach_refuses_an_unsaved_design() {
        let mut s = Session::default();
        let error = s.execute("project.attach", &json!({})).expect_err("no path");
        assert!(error.to_string().contains("no saved path"), "{error}");
    }
}
