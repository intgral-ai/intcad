use super::*;
use crate::documents;
use solidcraft_engine::Session;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNT: AtomicU32 = AtomicU32::new(0);

/// A folder of this test's own, removed when it drops.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("intcad-file-tree-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn file(&self, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        fs::write(&path, body).unwrap();
        path
    }
    fn dir(&self, name: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A tree with its root already listing `entries`, without waiting for the scan worker.
fn listing(root: &std::path::Path) -> FileTree {
    let mut tree = FileTree::new();
    tree.set_root(Some(root.to_path_buf()));
    if let Ok(entries) = files::read_dir(root) {
        tree.dirs.insert(root.to_path_buf(), Dir { entries, state: Listing::Ready });
    }
    tree
}

#[test]
fn the_tree_shows_folders_and_files_and_opens_a_folder_on_request() {
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "");
    scratch.file(".hidden.sc", "");
    let folder = scratch.dir("parts");
    let mut tree = listing(scratch.path());
    let rows = tree.rows();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["parts", "bracket.sc"], "folders first, hidden entries out: {names:?}");
    tree.toggle_dir(&folder);
    tree.dirs.insert(
        folder.clone(),
        Dir {
            entries: vec![files::Entry { name: "pin.sc".into(), path: folder.join("pin.sc"), kind: files::Kind::File, size: 0, modified: None }],
            state: Listing::Ready,
        },
    );
    let rows = tree.rows();
    assert!(rows.iter().any(|r| r.name == "pin.sc" && r.depth == 1), "an open folder shows its children: {rows:?}");
    tree.show_hidden = true;
    assert!(tree.rows().iter().any(|r| r.name == ".hidden.sc"));
    tree.toggle_dir(&folder);
    assert_eq!(tree.rows().len(), 3, "a closed folder shows only its own row");
}

#[test]
fn clicking_selects_one_entry_and_the_modifiers_extend_or_toggle() {
    let scratch = Scratch::new();
    let a = scratch.file("a.sc", "");
    let b = scratch.file("b.sc", "");
    let c = scratch.file("c.sc", "");
    let mut tree = listing(scratch.path());
    let visible: Vec<PathBuf> = tree.rows().iter().map(|r| r.path.clone()).collect();
    tree.click(&a, Modifiers::NONE, &visible);
    assert_eq!(tree.selection.iter().collect::<Vec<_>>(), vec![&a]);
    tree.click(&c, Modifiers { shift: true, ..Default::default() }, &visible);
    assert_eq!(tree.selection.len(), 3, "Shift selects the run from the anchor: {:?}", tree.selection);
    assert!(tree.selection.contains(&b));
    tree.click(&b, Modifiers { command: true, ..Default::default() }, &visible);
    assert_eq!(tree.selection.len(), 2, "Ctrl/Cmd toggles one row");
    assert!(!tree.selection.contains(&b));
}

#[test]
fn a_folder_and_a_child_in_the_selection_are_deleted_once() {
    let scratch = Scratch::new();
    let folder = scratch.dir("parts");
    let child = folder.join("pin.sc");
    fs::write(&child, "").unwrap();
    let other = scratch.file("bracket.sc", "");
    let mut tree = listing(scratch.path());
    tree.selection.insert(folder.clone());
    tree.selection.insert(child);
    tree.selection.insert(other.clone());
    let picked = tree.selected(None);
    assert_eq!(picked, vec![other, folder], "the child of a selected folder is not a second item");
}

#[test]
fn a_name_already_taken_asks_before_it_is_replaced() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    tree.place(vec![source], &into, false, None);
    match tree.modal.as_ref() {
        Some(Modal::Conflict { names, files_only, .. }) => {
            assert_eq!(names, &["bracket.sc".to_string()]);
            assert!(files_only);
        }
        _ => panic!("a conflict asks first: {:?}", tree.modal.is_some()),
    }
    assert!(tree.take_answers().is_empty(), "nothing runs before the answer");
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"old");
}

#[test]
fn an_answer_of_keep_both_leaves_both_files() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    tree.resolve_conflict(vec![source], into.clone(), false, Conflict::KeepBoth);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 1);
    assert!(answer.results[0].succeeded(), "{:?}", answer.results[0]);
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"old", "the existing file is kept");
    assert_eq!(fs::read(into.join("bracket (copy).sc")).unwrap(), b"new");
}

/// Wait for the operation worker's answer without a wall-clock deadline: the test process has a
/// few frames of its own.
fn wait_for_answer(tree: &mut FileTree) -> OpAnswer {
    for _ in 0..10_000 {
        let answers = tree.take_answers();
        if let Some(answer) = answers.into_iter().next() {
            return answer;
        }
        std::thread::yield_now();
    }
    panic!("the file operation did not finish");
}

#[test]
fn a_design_that_moved_with_its_folder_keeps_its_open_tab_and_dirty_edits() {
    let scratch = Scratch::new();
    let folder = scratch.dir("parts");
    let old = folder.join("bracket.sc");
    let mut session = Session::default();
    session.execute("file.save_as", &serde_json::json!({"path": old.to_string_lossy(), "cache": false})).unwrap();
    session.execute("solid.box", &serde_json::json!({"length": 10, "width": 10, "height": 10})).unwrap();
    assert!(session.is_dirty());
    let mut app = crate::SolveApp::new(session, Default::default());
    let new_folder = scratch.dir("new-parts");
    let new = new_folder.join("bracket.sc");
    fs::rename(&folder, &new_folder).unwrap();
    let count = documents::retarget_paths(&mut app, &folder, &new_folder);
    assert_eq!(count, 1);
    assert_eq!(app.session.path, Some(new.to_string_lossy().into_owned()));
    assert!(app.session.is_dirty(), "the unsaved edit is still there");
    let outcome = files::rename(&new, "renamed.sc", Conflict::Skip);
    assert!(outcome.succeeded(), "{outcome:?}");
    assert_eq!(documents::retarget_paths(&mut app, &new, &outcome.target), 1);
    assert_eq!(app.session.path, Some(outcome.target.to_string_lossy().into_owned()));
    assert_eq!(app.session.doc.name, "renamed", "a name borrowed from the file follows it");
    app.session.execute("file.save", &serde_json::json!({})).unwrap();
    assert!(outcome.target.is_file(), "Save writes the renamed file");
    assert!(!new.exists(), "the old name is not written again");
    assert!(!app.session.is_dirty());
}

#[test]
fn a_new_design_is_written_as_a_loadable_sc_and_asks_to_open() {
    let scratch = Scratch::new();
    let mut tree = listing(scratch.path());
    tree.editing =
        Some(Edit { parent: scratch.path().to_path_buf(), path: None, kind: EditKind::Model, name: "Bracket".into(), error: None, focus: true });
    let open = tree.commit_edit();
    let file = scratch.path().join("Bracket.sc");
    assert_eq!(open.as_deref(), Some(file.as_path()), "the new design is the one to open");
    assert!(file.is_file());
    let loaded = solidcraft_engine::io::read_design_file(&file.to_string_lossy(), None::<fn(solidcraft_engine::io::DesignPreview)>);
    assert!(loaded.is_ok(), "the file loads as a design");
    assert_eq!(tree.selection.iter().next(), Some(&file));
}

#[test]
fn a_name_that_is_taken_keeps_the_editor_open_with_the_reason() {
    let scratch = Scratch::new();
    scratch.file("taken.sc", "");
    let mut tree = listing(scratch.path());
    tree.editing =
        Some(Edit { parent: scratch.path().to_path_buf(), path: None, kind: EditKind::File, name: "taken.sc".into(), error: None, focus: true });
    assert!(tree.commit_edit().is_none());
    let edit = tree.editing.as_ref().expect("the editor stays for another name");
    assert!(edit.error.is_some());
}

#[test]
fn a_rename_reports_the_move_so_open_designs_can_follow() {
    let scratch = Scratch::new();
    let old = scratch.file("bracket.sc", "");
    let mut tree = listing(scratch.path());
    tree.editing = Some(Edit {
        parent: scratch.path().to_path_buf(),
        path: Some(old.clone()),
        kind: EditKind::Rename,
        name: "bracket-v2.sc".into(),
        error: None,
        focus: true,
    });
    tree.commit_edit();
    assert_eq!(tree.take_retargets(), vec![(old, scratch.path().join("bracket-v2.sc"))]);
    assert!(!scratch.path().join("bracket.sc").exists());
}

#[test]
fn deleting_a_dirty_design_names_it_in_the_question_and_a_copy_saves_the_work_first() {
    let scratch = Scratch::new();
    let path = scratch.file("bracket.sc", "old");
    let mut session = Session::default();
    session.path = Some(path.to_string_lossy().into_owned());
    session.execute("solid.box", &serde_json::json!({"length": 4, "width": 4, "height": 4})).unwrap();
    let mut app = crate::SolveApp::new(session, Default::default());
    let mut tree = listing(scratch.path());
    tree.select_only(&path);
    let open: Vec<(String, bool)> = vec![(path.to_string_lossy().into_owned(), true)];
    tree.request_delete(&open);
    match &tree.modal {
        Some(Modal::Delete { dirty, .. }) => assert_eq!(dirty.len(), 1, "the dirty design is named"),
        _ => panic!("the delete question is open"),
    }
    let saved = crate::file_tree::save_copies(&mut app, std::slice::from_ref(&path)).unwrap();
    assert_eq!(saved, 1);
    assert!(scratch.path().join("bracket (copy).sc").is_file(), "the work is kept as a copy");
    assert!(app.session.path.as_deref().is_some_and(|p| p.ends_with("bracket (copy).sc")));
    assert!(!app.session.is_dirty(), "the copy holds the work");
}

#[test]
fn an_operation_that_cannot_trash_asks_before_deleting_for_good() {
    let mut tree = FileTree::new();
    let path = std::path::PathBuf::from("/tmp/intcad-no-trash-test.sc");
    let answer = OpAnswer { action: Action::Trash, results: Vec::new(), no_trash: vec![path.clone()], cancelled: false };
    let _ = tree.apply_answer(answer);
    match &tree.modal {
        Some(Modal::Permanent { paths, .. }) => assert_eq!(paths, &vec![path]),
        _ => panic!("a second question is asked"),
    }
}

#[test]
fn an_external_change_is_shown_and_kept_until_dismissed() {
    let scratch = Scratch::new();
    let path = scratch.file("bracket.sc", "v1");
    let mut tree = listing(scratch.path());
    tree.poll_documents(&[(path.to_string_lossy().into_owned(), false, true, false)], 100.0);
    assert_eq!(tree.external.len(), 1);
    assert!(!tree.external[0].missing);
    tree.dismiss_external(&path.to_string_lossy());
    tree.poll_documents(&[(path.to_string_lossy().into_owned(), false, true, false)], 200.0);
    assert!(tree.external.is_empty(), "a dismissed change is not shown again");
}

#[test]
fn a_missing_design_is_reported_as_gone_rather_than_changed() {
    let scratch = Scratch::new();
    let path = scratch.path().join("gone.sc");
    let mut tree = listing(scratch.path());
    tree.poll_documents(&[(path.to_string_lossy().into_owned(), true, true, true)], 10.0);
    assert!(tree.external[0].missing && tree.external[0].dirty);
}

/// The panel in a real window: its empty state offers a folder, and a chosen folder shows its
/// rows in the right rail.
#[test]
fn the_files_panel_draws_in_the_right_rail_and_lists_the_chosen_folder() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "");
    scratch.dir("parts");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.frames(3);
    assert!(rect_of("files:empty").is_some(), "the empty state offers a folder");
    assert!(rect_of("files_open_folder").is_some());
    h.app.file_tree.set_root(Some(scratch.path().to_path_buf()));
    h.frames(30);
    let root = rect_of("files:root").expect("the folder name is in the panel");
    let parts = rect_of("files:row:parts").expect("a folder row");
    let bracket = rect_of("files:row:bracket.sc").expect("a file row");
    let panel = rect_of("rail:panel").expect("the panel is open");
    assert!(panel.contains_rect(root) && panel.contains_rect(parts) && panel.contains_rect(bracket), "{panel:?}");
    assert!(parts.top() < bracket.top(), "folders stand first");
}

/// A design opened from the tree goes through the document path: its tab shows the file, and a
/// second activation focuses that tab instead of opening a second one.
#[test]
fn opening_a_design_from_the_tree_uses_one_tab() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    let path = scratch.path().join("bracket.sc");
    Session::default().execute("file.save_as", &serde_json::json!({"path": path.to_string_lossy(), "cache": false})).unwrap();
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.app.file_tree.set_root(Some(scratch.path().to_path_buf()));
    h.frames(30);
    assert!(rect_of("files:row:bracket.sc").is_some());
    h.step(&json!({"click": {"handle": "files:row:bracket.sc"}, "double": true})).unwrap();
    h.frames(60);
    assert_eq!(h.app.session.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    let tabs = h.app.docs.count();
    h.step(&json!({"click": {"handle": "files:row:bracket.sc"}, "double": true})).unwrap();
    h.frames(60);
    assert_eq!(h.app.docs.count(), tabs, "the same path focuses its tab");
}

#[test]
fn a_copy_of_a_design_with_unsaved_edits_says_the_copy_uses_the_saved_file() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "saved");
    let into = scratch.dir("parts");
    let mut tree = listing(scratch.path());
    let path = source.to_string_lossy().into_owned();
    tree.set_open_docs(&[(path, true, false, false)]);
    tree.place(vec![source], &into, false, None);
    let notice = tree.notice.as_ref().map(|(text, _)| text.as_str()).unwrap_or_default();
    assert!(notice.contains("saved file"), "the copy's content is explained: {notice}");
}

#[test]
fn locating_a_design_opens_the_folders_above_it_and_selects_it() {
    let scratch = Scratch::new();
    let mut parent = scratch.path().to_path_buf();
    for name in ["a", "b", "c"] {
        parent = parent.join(name);
        fs::create_dir(&parent).unwrap();
    }
    let file = parent.join("deep.sc");
    fs::write(&file, "").unwrap();
    let mut tree = listing(scratch.path());
    tree.reveal_path(&file);
    assert!(tree.expanded.contains(&parent), "the folders above it are open");
    assert!(tree.selection.contains(&file));
    assert_eq!(tree.scroll_to.as_deref(), Some(file.as_path()));
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_is_not_opened_as_a_folder_in_the_tree() {
    let scratch = Scratch::new();
    let real = scratch.dir("real");
    fs::write(real.join("inside.sc"), "").unwrap();
    let link = scratch.path().join("shortcut");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut tree = listing(scratch.path());
    assert!(matches!(tree.kind_at(&link), Some(files::Kind::Link { to_dir: true })));
    let opened = tree.activate(&link).expect("a link is handed to the system, not expanded");
    assert_eq!(opened, link);
    assert!(!tree.expanded.contains(&link), "the link's target is not walked");
}

/// The control channel can browse a folder and read its rows: the same door the desktop E2E
/// uses to drive the panel.
#[test]
fn the_control_channel_reads_the_panels_rows() {
    use crate::scenario::Harness;
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "");
    scratch.dir("parts");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    let reply = h.call("ui.files", json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    let rows = reply["result"]["rows"].as_array().expect("rows");
    let names: Vec<&str> = rows.iter().filter_map(|row| row["name"].as_str()).collect();
    assert!(names.contains(&"bracket.sc") && names.contains(&"parts"), "{names:?}");
    assert_eq!(reply["result"]["root"], json!(scratch.path().to_string_lossy()));
    let refused = h.call("ui.files", json!({"root": scratch.path().join("missing")}));
    assert_eq!(refused["ok"], false, "a missing folder is refused");
}

/// The `+` menu opens on a click and its New File asks for a name in the tree; Enter makes the
/// file with exactly the typed name.
#[test]
fn the_new_menu_makes_a_file_with_the_typed_name() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    h.step(&json!({"click": {"handle": "files:new"}})).unwrap();
    h.frames(3);
    assert!(rect_of("menu:files_new_file").is_some(), "the new menu opens on a click");
    h.step(&json!({"click": {"handle": "menu:files_new_file"}})).unwrap();
    h.frames(3);
    assert!(rect_of("files:edit").is_some(), "the name field is in the tree");
    h.step(&json!({"text": "notes.txt"})).unwrap();
    h.step(&json!({"key": "Enter"})).unwrap();
    h.frames(5);
    assert!(scratch.path().join("notes.txt").is_file(), "the typed name made the file");
}

/// Moving a folder that holds two open designs moves both tabs, and both keep their unsaved
/// work: the second test of AC-09, which a single open design cannot prove.
#[test]
fn moving_a_folder_moves_every_open_design_inside_it() {
    let scratch = Scratch::new();
    let folder = scratch.dir("parts");
    let first = folder.join("a.sc");
    let second = folder.join("b.sc");
    let mut one = Session::default();
    one.execute("file.save_as", &serde_json::json!({"path": first.to_string_lossy(), "cache": false})).unwrap();
    one.execute("solid.box", &serde_json::json!({"length": 3, "width": 3, "height": 3})).unwrap();
    let mut two = Session::default();
    two.execute("file.save_as", &serde_json::json!({"path": second.to_string_lossy(), "cache": false})).unwrap();
    two.execute("solid.box", &serde_json::json!({"length": 5, "width": 5, "height": 5})).unwrap();
    let mut app = crate::SolveApp::new(one, Default::default());
    crate::documents::open_tab(&mut app, two).unwrap();
    let moved = scratch.dir("moved");
    fs::rename(&folder, &moved).unwrap();
    assert_eq!(documents::retarget_paths(&mut app, &folder, &moved), 2);
    assert_eq!(app.session.path, Some(moved.join("b.sc").to_string_lossy().into_owned()));
    let open = documents::open_sessions(&app);
    let other = open.iter().find(|session| session.path.as_deref() == Some(moved.join("a.sc").to_string_lossy().as_ref()));
    assert!(other.is_some(), "the design in the other tab followed the folder");
    assert!(open.iter().all(|session| session.is_dirty()), "both keep their edits");
}

/// A batch reports each item: the good copy is made and the missing source is a failure, so a
/// partial success is never shown as one success.
#[test]
fn a_batch_reports_the_good_item_and_the_failed_one() {
    let scratch = Scratch::new();
    let good = scratch.file("good.sc", "good");
    let missing = scratch.path().join("missing.sc");
    let into = scratch.dir("parts");
    let mut tree = listing(scratch.path());
    tree.place(vec![good, missing], &into, false, None);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 2);
    assert_eq!(answer.results.iter().filter(|r| r.succeeded()).count(), 1);
    assert_eq!(answer.results.iter().filter(|r| matches!(r.outcome, files::Outcome::Failed(_))).count(), 1);
    assert!(into.join("good.sc").is_file());
    let report = tree.apply_answer(answer);
    let _ = report;
    let report = tree.report.as_ref().unwrap();
    assert_eq!((report.succeeded(), report.failed()), (1, 1));
    assert_eq!(report.retry.len(), 1, "only the failed item can run again");
}

/// The secondary click opens the row menu, and Rename in it opens the name field. The rows are
/// virtualized, so the menu is drawn by the panel rather than by the row that asked for it.
#[test]
fn a_right_click_opens_the_row_menu_and_rename_edits_in_place() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("notes.txt", "");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    let row = rect_of("files:row:notes.txt").expect("the row");
    h.call("ui.click", json!({"x": row.center().x, "y": row.center().y, "button": "right"}));
    h.frames(2);
    let rename = rect_of("menu:files_rename").expect("the row menu opened");
    h.call("ui.click", json!({"x": rename.center().x, "y": rename.center().y}));
    h.frames(2);
    assert!(rect_of("files:edit").is_some(), "Rename opened the name field");
}
