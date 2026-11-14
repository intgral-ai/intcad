//! Project folders: create, read, validate, and never overwrite.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::Session;

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
    assert!(root.join(".intcad/project.toml").is_file());
    assert!(root.join(".intcad/ci.toml").is_file());
    assert!(root.join(".intcad/specs/requirements.toml").is_file());
    assert!(root.join(".intcad/checks/runs").is_dir());
    assert!(root.join(".intcad/.gitignore").is_file());
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
    assert_eq!(Project::locate(&root.join(".intcad")), Some(root.clone()));
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

/// A `.intcad/` folder can be added beside a saved design without moving it.
#[test]
fn attach_keeps_the_design_and_puts_the_project_beside_it() {
    let dir = scratch("attach");
    let design = dir.join("legacy part.sc");
    fs::write(&design, b"the design bytes").expect("design");
    let before = fs::read(&design).expect("read");
    let made = attach(&design).expect("attach");
    assert_eq!(made.design, design);
    assert_eq!(fs::read(&design).expect("read"), before, "the design is not rewritten");
    let root = dir.as_path();
    assert_eq!(made.project.root(), root);
    assert!(root.join(".intcad/project.toml").is_file());
    assert_eq!(made.project.manifest().designs, vec!["legacy part.sc".to_string()]);
    assert_eq!(made.project.manifest().name, "legacy part");
    assert_eq!(Project::locate(&design), Some(root.to_path_buf()));
    let reopened = Project::open(root).expect("reopen");
    assert_eq!(reopened.manifest().id, made.project.manifest().id);
    let _ = fs::remove_dir_all(&dir);
}

/// A design already inside a project is refused, at the design's folder or above it.
#[test]
fn attach_refuses_a_design_that_already_has_a_project() {
    let dir = scratch("attach-twice");
    let design = dir.join("Bracket.sc");
    fs::write(&design, b"design").expect("design");
    attach(&design).expect("first");
    let manifest = dir.join(".intcad/project.toml");
    let before = read(&manifest);
    let error = attach(&design).expect_err("second");
    assert!(matches!(error, ProjectError::AlreadyInProject { .. }), "{error}");
    assert_eq!(read(&manifest), before, "the existing project is untouched");
    // A design in a subfolder of the project is inside it too.
    let nested = dir.join("sub");
    fs::create_dir_all(&nested).expect("sub");
    let nested_design = nested.join("Part.sc");
    fs::write(&nested_design, b"design").expect("nested");
    let error = attach(&nested_design).expect_err("nested");
    assert!(matches!(error, ProjectError::AlreadyInProject { .. }), "{error}");
    assert!(!nested.join(".intcad").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// An unsaved design (no file) is refused and leaves no folder behind.
#[test]
fn attach_refuses_a_design_with_no_file() {
    let dir = scratch("attach-missing");
    let error = attach(&dir.join("never-saved.sc")).expect_err("missing");
    assert!(error.to_string().contains("does not exist"), "{error}");
    assert_eq!(fs::read_dir(&dir).expect("list").count(), 0);
    let _ = fs::remove_dir_all(&dir);
}

/// Merely asking which project covers a plain file writes nothing.
#[test]
fn opening_a_plain_design_never_makes_a_project() {
    let dir = scratch("attach-passive");
    let design = dir.join("plain.sc");
    fs::write(&design, b"design").expect("design");
    assert!(Project::of(&design).expect("lookup").is_none());
    assert_eq!(Project::locate(&design), None);
    assert_eq!(fs::read_dir(&dir).expect("list").count(), 1, "only the design");
    assert!(!dir.join(".intcad").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// The acceptance shop: two thickness tiers for each of two hole rules.
fn two_tier() -> &'static [u8] {
    include_bytes!("test_data/two_tier_shop.json")
}

fn selected(kind: DataKind, imported: &crate::factory_data::ImportedData) -> DataRef {
    DataRef { kind, name: imported.name.clone(), version: imported.version.clone(), sha256: imported.sha256.clone(), file: imported.file.clone() }
}

fn choose_shop(project: &mut Project) -> crate::factory_data::ImportedData {
    let imported = crate::factory_data::import_into(project.root(), two_tier(), DataKind::Factory).expect("import");
    let data = selected(DataKind::Factory, &imported);
    project.select_data(data).expect("select");
    imported
}

fn rule_value(frozen: &FrozenProject, rule: &str, thickness: f64) -> f64 {
    let shop = frozen.shop.as_ref().expect("shop");
    let declared = &frozen.declared;
    let (rows, gap) = solidcraft_dfm_rules::profile_data::applying(
        shop,
        declared.process.as_deref().unwrap_or("sheet-metal"),
        declared.material.as_deref().unwrap_or("5052-H32"),
        thickness,
    );
    assert!(gap.is_none(), "{gap:?}");
    rows.iter().filter(|row| row.rule == rule).map(|row| row.value.get("mm").copied().unwrap_or_default()).next().expect("row")
}

/// A report from the frozen project with one small hole at the declared thickness.
fn hole_state(frozen: &FrozenProject, diameter: f64) -> solidcraft_dfm_rules::AppState {
    use solidcraft_dfm_rules::{CheckInput, Facts, HoleFact, ModelRef};
    let thickness = frozen.declared.thickness_mm.expect("declared thickness");
    let facts = Facts {
        volume_mm3: Some(1000.0),
        size_mm: Some(vec![40.0, 25.0, thickness]),
        area_mm2: Some(200.0),
        paired_mm: Some(thickness),
        holes: Some(vec![HoleFact {
            diameter_mm: diameter,
            x_mm: 20.0,
            y_mm: 12.5,
            z_mm: thickness / 2.0,
            depth_mm: thickness,
            square: true,
            edge_mm: Some(10.0),
            ..HoleFact::default()
        }]),
        shells_closed: true,
        analyzer_valid: true,
        bop_valid: true,
        faces: 6,
        ..Facts::default()
    };
    let report = solidcraft_dfm_rules::check_catalog(&CheckInput {
        declared: frozen.declared.clone(),
        generic: frozen.generic.clone(),
        shop: frozen.shop.clone(),
        overrides: frozen.overrides.clone(),
        facts,
        model: ModelRef { model_version: "model-1".into(), body: "Plate".into(), source: "native".into() },
        critical: Vec::new(),
    })
    .expect("report");
    let finding = report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_min_diameter")).expect("hole finding");
    eprintln!("hole state {:?} status {:?} code {:?} message {}", finding.state, finding.status, finding.error_code, finding.message);
    finding.state
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("target folder");
    for entry in fs::read_dir(from).expect("list") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
}

/// ① A project copied to a clean folder reads its own data copy, offline: another thickness
/// selects the other tier of the same file.
#[test]
fn a_copied_project_reads_its_own_data_copy_and_another_tier() {
    let dir = scratch("copy");
    let made = create(&dir, "Shop", "Shop").expect("create");
    let mut project = made.project.clone();
    choose_shop(&mut project);
    project.update_settings(None, Some("5052-H32".into()), Some(Some(0.8)), None).expect("settings");
    let source = project.root().to_path_buf();

    // No app data and no link back: only the project folder is copied.
    let target_root = dir.join("clean");
    let target = target_root.join("Shop");
    copy_tree(&source, &target);
    assert!(!source.starts_with(&target_root));
    let mut copied = Project::open(&target).expect("open the copy");
    let first = copied.freeze().expect("freeze");
    assert!(first.data.unavailable.is_none(), "{:?}", first.data.unavailable);
    assert!(first.shop.is_some());
    assert_eq!(rule_value(&first, "sm.hole_min_diameter", 0.8), 2.0);
    assert_eq!(rule_value(&first, "sm.hole_to_edge", 0.8), 3.0);
    let digest = first.config_digest.clone();
    assert_eq!(Project::open(&target).expect("reopen").freeze().expect("freeze").config_digest, digest, "the digest is stable");

    copied.update_settings(None, None, Some(Some(2.0)), None).expect("thickness");
    let second = copied.freeze().expect("freeze");
    assert_eq!(rule_value(&second, "sm.hole_min_diameter", 2.0), 3.5);
    assert_eq!(rule_value(&second, "sm.hole_to_edge", 2.0), 5.0);
    assert_ne!(second.config_digest, digest, "a changed thickness is a different configuration");
    let _ = fs::remove_dir_all(&dir);
}

/// ② A missing copy, a digest mismatch and a material the shop does not list all say "not
/// covered"; none switches to another table. An invalid import keeps the old selection.
#[test]
fn a_missing_mismatched_or_inapplicable_copy_is_not_covered_and_never_switches() {
    let dir = scratch("not-covered");
    let made = create(&dir, "Shop", "Shop").expect("create");
    let mut project = made.project.clone();
    let imported = choose_shop(&mut project);
    project.update_settings(None, Some("5052-H32".into()), Some(Some(2.0)), None).expect("settings");

    // Missing copy.
    fs::remove_file(project.root().join(&imported.file)).expect("remove copy");
    let missing = project.freeze().expect("freeze");
    assert!(missing.shop.is_none());
    let why = missing.data.unavailable.as_deref().expect("reason");
    assert!(why.contains("cannot be read"), "{why}");
    assert_eq!(hole_state(&missing, 2.4), solidcraft_dfm_rules::AppState::Unsupported);

    // Digest mismatch: put the bytes back but record another digest.
    let bytes = fs::read(project.root().join(&imported.file)).unwrap_or_else(|_| two_tier().to_vec());
    let _ = fs::write(project.root().join(&imported.file), &bytes);
    let ci = project.folder().join(CI);
    let text = read(&ci).replace(&imported.sha256, &"0".repeat(64));
    fs::write(&ci, text).expect("write");
    let mismatched = Project::open(project.root()).expect("open").freeze().expect("freeze");
    assert!(mismatched.shop.is_none());
    assert!(mismatched.data.unavailable.as_deref().is_some_and(|why| why.contains("sha256")));
    assert_eq!(hole_state(&mismatched, 2.4), solidcraft_dfm_rules::AppState::Unsupported);

    // Material mismatch: keep the table, but say why no row applies.
    let mut project = Project::open(project.root()).expect("open");
    fs::write(project.folder().join(CI), read(&ci).replace(&"0".repeat(64), &imported.sha256)).expect("restore digest");
    let mut project = Project::open(project.root()).expect("reopen");
    project.update_settings(None, Some("6061-T6".into()), None, None).expect("material");
    let other = project.freeze().expect("freeze");
    assert!(other.shop.is_some(), "the selected table stays selected");
    assert!(other.data.unavailable.as_deref().is_some_and(|why| why.contains("material")), "{:?}", other.data.unavailable);
    assert_eq!(hole_state(&other, 2.4), solidcraft_dfm_rules::AppState::Unsupported);

    // An invalid import is refused before anything is written.
    let mut s = Session { path: Some(made.design.to_string_lossy().to_string()), ..Session::default() };
    let bad = dir.join("bad.json");
    fs::write(&bad, b"{ not a profile").expect("bad file");
    let before = read(&project.folder().join(CI));
    let error =
        s.execute("project.data_set", &serde_json::json!({"kind":"factory","path":bad.to_string_lossy(),"upgrade":true})).expect_err("invalid");
    assert!(error.to_string().to_lowercase().contains("profile"), "{error}");
    assert_eq!(read(&project.folder().join(CI)), before, "the old selection survives");
    let _ = fs::remove_dir_all(&dir);
}

/// ③ 3.0 confirmed, a named 2.5 override and a 2.4 mm hole: the override is advisory; after an
/// explicit upgrade to a 2.5 row with its own factory confirmation, the same hole fails.
#[test]
fn an_override_is_advisory_until_an_independently_confirmed_row_replaces_it() {
    let dir = scratch("override");
    let made = create(&dir, "Shop", "Shop").expect("create");
    let mut project = made.project.clone();
    let shop = br#"{
      "format": 1, "name": "override shop", "version": "1", "layer": "factory",
      "requirements": [{
        "rule": "sm.hole_min_diameter", "limit": "min", "value": {"mm": 3.0},
        "inputs": {"mm": "mm"}, "measuring_line": "The hole diameter.",
        "method": "TEST-ONLY", "source": "TEST-ONLY", "rights": "TEST-ONLY original",
        "applies": {"process": ["sheet-metal"], "material": ["5052"]},
        "confirmation": "factory_confirmed"
      }]
    }"#;
    let import = crate::factory_data::import_into(project.root(), shop, DataKind::Factory).expect("import");
    project.select_data(selected(DataKind::Factory, &import)).expect("select");
    project.update_settings(None, Some("5052-H32".into()), Some(Some(2.0)), None).expect("settings");

    let ci = project.folder().join(CI);
    let text = read(&ci).replace(
        "[checks]",
        "[[override]]\nrule = \"sm.hole_min_diameter\"\nlimit = \"min\"\nvalue = { mm = 2.5 }\nreason = \"customer drawing\"\nby = \"Zhang\"\ndate = \"2026-11-14\"\n\n[checks]",
    );
    fs::write(&ci, text).expect("write override");
    let with_override = Project::open(project.root()).expect("open").freeze().expect("freeze");
    assert_eq!(with_override.overrides.len(), 1);
    assert_eq!(hole_state(&with_override, 2.4), solidcraft_dfm_rules::AppState::Advisory, "a named override is advisory");

    // Explicit upgrade: a new row at 2.5 with its own factory confirmation.
    let upgraded = br#"{
      "format": 1, "name": "override shop", "version": "2", "layer": "factory",
      "requirements": [{
        "rule": "sm.hole_min_diameter", "limit": "min", "value": {"mm": 2.5},
        "inputs": {"mm": "mm"}, "measuring_line": "The hole diameter.",
        "method": "TEST-ONLY", "source": "TEST-ONLY", "rights": "TEST-ONLY original",
        "applies": {"process": ["sheet-metal"], "material": ["5052"]},
        "confirmation": "factory_confirmed"
      }]
    }"#;
    let next = crate::factory_data::import_into(project.root(), upgraded, DataKind::Factory).expect("upgrade import");
    project.select_data(selected(DataKind::Factory, &next)).expect("select upgrade");
    let mut cleared = project.clone();
    cleared.set_overrides(Vec::new()).expect("clear override");
    let confirmed = cleared.freeze().expect("freeze");
    assert_eq!(hole_state(&confirmed, 2.4), solidcraft_dfm_rules::AppState::Failed, "a confirmed 2.5 decides");
    let _ = fs::remove_dir_all(&dir);
}

/// ④ An override without a reason or a name is an error and changes nothing.
#[test]
fn an_override_without_a_reason_or_a_name_is_refused() {
    let dir = scratch("override-refused");
    let made = create(&dir, "Shop", "Shop").expect("create");
    let mut s = Session { path: Some(made.design.to_string_lossy().to_string()), ..Session::default() };
    let ci = made.project.folder().join(CI);
    let before = read(&ci);
    for params in [
        serde_json::json!({"rule":"sm.hole_min_diameter","value":{"mm":2.5},"by":"Zhang"}),
        serde_json::json!({"rule":"sm.hole_min_diameter","value":{"mm":2.5},"reason":"drawing"}),
        serde_json::json!({"rule":"sm.hole_min_diameter","value":{"mm":2.5},"reason":" ","by":" "}),
    ] {
        let error = s.execute("dfm.override", &params).expect_err("refused");
        let text = error.to_string();
        assert!(text.contains("reason") || text.contains("name"), "{text}");
    }
    assert_eq!(read(&ci), before, "a refused override writes nothing");
    let _ = fs::remove_dir_all(&dir);
}
