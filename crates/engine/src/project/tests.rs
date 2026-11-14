//! Project folders: create, read, validate, and never overwrite.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static NEXT: AtomicU64 = AtomicU64::new(1);

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("solidcraft-project-{label}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch folder");
    dir
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

#[test]
fn a_created_project_reopens_with_the_same_identity_and_settings() {
    let dir = scratch("reopen");
    let made = create(&dir, "Bracket A", "Bracket A").expect("create");
    let root = made.project.root().to_path_buf();
    assert_eq!(root, dir.join("Bracket A"));
    assert_eq!(made.design, root.join("Bracket A.sc"));
    assert!(made.design.is_file());
    assert!(root.join(".intgral/project.toml").is_file());
    assert!(root.join(".intgral/ci.toml").is_file());
    assert!(root.join(".intgral/specs/requirements.toml").is_file());
    assert!(root.join(".intgral/checks/runs").is_dir());
    assert!(root.join(".intgral/.gitignore").is_file());
    let id = made.project.manifest().id.clone();

    // Closing and reopening is a fresh read of the two files, not a template.
    drop(made);
    let reopened = Project::open(&root).expect("open");
    assert_eq!(reopened.manifest().id, id);
    assert_eq!(reopened.manifest().name, "Bracket A");
    assert_eq!(reopened.manifest().process, "sheet-metal");
    assert_eq!(reopened.manifest().thickness_mm, None);
    assert_eq!(reopened.manifest().designs, vec!["Bracket A.sc".to_string()]);
    assert!(reopened.ci().enabled);
    assert_eq!(reopened.ci().data.kind, DataKind::Generic);
    assert_eq!(reopened.ci().on_manual_edit, ManualEdit::Stale);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_second_create_leaves_the_first_project_untouched() {
    let dir = scratch("duplicate");
    let made = create(&dir, "Panel", "Panel").expect("create");
    let manifest = made.project.folder().join(MANIFEST);
    let before = read(&manifest);
    let again = create(&dir, "Panel", "Panel");
    assert!(matches!(again, Err(ProjectError::TargetExists { .. })), "{again:?}");
    assert_eq!(read(&manifest), before, "a refused create must not rewrite the project");
    assert!(made.design.is_file());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_parent_folder_is_reported_and_leaves_nothing() {
    let dir = scratch("missing");
    let gone = dir.join("no-such-folder");
    let error = create(&gone, "Bracket", "Bracket").expect_err("no parent");
    assert!(matches!(error, ProjectError::Io { .. }), "{error}");
    assert!(!gone.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_that_cannot_be_written_leaves_no_half_project() {
    let dir = scratch("file-parent");
    let file = dir.join("not-a-folder");
    fs::write(&file, b"x").expect("file");
    let error = create(&file, "Bracket", "Bracket").expect_err("a file is not a folder");
    assert!(matches!(error, ProjectError::Io { .. }), "{error}");
    assert_eq!(read(&file), "x");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_file_names_the_file_and_the_position() {
    let dir = scratch("damaged");
    let made = create(&dir, "Bracket", "Bracket").expect("create");
    let ci = made.project.folder().join(CI);
    fs::write(&ci, "schema_version = 1\nenabled = \n").expect("write");
    let error = Project::open(made.project.root()).expect_err("damaged");
    match error {
        ProjectError::Parse { file, message } => {
            assert!(file.ends_with("ci.toml"), "{file}");
            assert!(message.contains("line 2"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_newer_schema_is_refused_without_rewriting() {
    let dir = scratch("newer");
    let made = create(&dir, "Bracket", "Bracket").expect("create");
    let manifest = made.project.folder().join(MANIFEST);
    let before = read(&manifest).replace("schema_version = 1", "schema_version = 99");
    fs::write(&manifest, &before).expect("write");
    let error = Project::open(made.project.root()).expect_err("newer");
    assert!(matches!(error, ProjectError::SchemaTooNew { found: 99, .. }), "{error}");
    assert_eq!(read(&manifest), before, "a newer file is read-only");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn locate_finds_the_project_from_a_design_file_and_a_subfolder() {
    let dir = scratch("locate");
    let made = create(&dir, "Bracket", "Bracket").expect("create");
    let root = made.project.root().to_path_buf();
    assert_eq!(Project::locate(&made.design), Some(root.clone()));
    assert_eq!(Project::locate(&root), Some(root.clone()));
    assert_eq!(Project::locate(&root.join(".intgral")), Some(root.clone()));
    let elsewhere = dir.join("plain.step");
    fs::write(&elsewhere, b"not a step").expect("write");
    assert_eq!(Project::locate(&elsewhere), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn opening_an_existing_project_never_applies_a_template() {
    let dir = scratch("no-template");
    let made = create(&dir, "Bracket", "Bracket").expect("create");
    let manifest = made.project.folder().join(MANIFEST);
    let edited = read(&manifest).replace("name = \"Bracket\"", "name = \"Renamed by hand\"");
    fs::write(&manifest, &edited).expect("write");
    let reopened = Project::open(made.project.root()).expect("open");
    assert_eq!(reopened.manifest().name, "Renamed by hand");
    assert_eq!(read(&manifest), edited);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_name_and_a_path_in_the_name_are_refused() {
    let dir = scratch("names");
    for name in ["", "   ", "a/b", "a\\b", ".."] {
        let error = create(&dir, name, "design").expect_err(name);
        assert!(matches!(error, ProjectError::Invalid { .. }), "{name}: {error}");
    }
    assert_eq!(fs::read_dir(&dir).expect("list").count(), 0, "nothing was left behind");
    let made = create(&dir, "Design", "").expect("a blank design name gets a fallback");
    assert!(made.design.ends_with("Design.sc"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_design_file_kind_is_the_apps_own() {
    let dir = scratch("design-kind");
    let made = create(&dir, "Bracket", "Bracket").expect("create");
    let bytes = fs::read(&made.design).expect("read");
    assert!(solidcraft_io::read_design(&bytes).is_ok(), "the empty design opens");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn overrides_need_a_reason_and_a_name() {
    let dir = scratch("override");
    let made = create(&dir, "Bracket", "Bracket").expect("create");
    let ci = made.project.folder().join(CI);
    let text = read(&ci);
    let with_override = format!(
        "{text}\n[[override]]\nrule = \"sm.hole_to_edge\"\nlimit = \"min\"\nvalue = {{ mm = 2.5 }}\nreason = \"\"\nby = \"\"\ndate = \"2026-11-14\"\n"
    );
    fs::write(&ci, &with_override).expect("write");
    let error = Project::open(made.project.root()).expect_err("no reason");
    assert!(error.to_string().contains("reason"), "{error}");
    let named = with_override.replace("reason = \"\"", "reason = \"customer drawing\"").replace("by = \"\"", "by = \"Zhang\"");
    fs::write(&ci, named).expect("write");
    let project = Project::open(made.project.root()).expect("with a reason and a name");
    assert_eq!(project.ci().overrides.len(), 1);
    assert_eq!(project.ci().overrides[0].by, "Zhang");
    let _ = fs::remove_dir_all(&dir);
}
