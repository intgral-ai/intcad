use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNT: AtomicU32 = AtomicU32::new(0);

/// A folder of this test's own, removed when `done` drops.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("intcad-files-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, body).unwrap();
        path
    }
    fn dir(&self, name: &str) -> PathBuf {
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

#[test]
fn names_sort_naturally_with_folders_first() {
    let scratch = Scratch::new();
    scratch.file("part10.sc", "");
    scratch.file("part2.sc", "");
    scratch.file("Part1.sc", "");
    scratch.dir("zeta");
    scratch.dir("alpha");
    let names: Vec<String> = read_dir(scratch.path()).unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["alpha", "zeta", "Part1.sc", "part2.sc", "part10.sc"]);
    assert!(natural_cmp("part 2", "part 10").is_lt());
    assert!(natural_cmp("Zebra", "apple").is_gt());
    assert_eq!(natural_cmp("same", "Same"), std::cmp::Ordering::Equal);
}

#[test]
fn an_entry_knows_its_kind_and_size() {
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "abc");
    let dir = scratch.dir("parts");
    let entries = read_dir(scratch.path()).unwrap();
    let design = entries.iter().find(|e| e.name == "bracket.sc").unwrap();
    assert_eq!(design.kind, Kind::File);
    assert_eq!(design.size, 3);
    assert!(design.is_design());
    let folder = entries.iter().find(|e| e.name == "parts").unwrap();
    assert_eq!(folder.kind, Kind::Dir);
    assert_eq!(folder.path, dir);
}

#[test]
fn a_copy_of_a_design_keeps_both_and_never_touches_the_source() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "original");
    let copy = copy_into(&source, scratch.path(), Conflict::KeepBoth);
    assert!(copy.succeeded(), "{copy:?}");
    assert_eq!(copy.target.file_name().and_then(|n| n.to_str()), Some("bracket (copy).sc"));
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read(&copy.target).unwrap(), b"original");
    let again = copy_into(&source, scratch.path(), Conflict::KeepBoth);
    assert_eq!(again.target.file_name().and_then(|n| n.to_str()), Some("bracket (copy 2).sc"));
    assert!(again.succeeded());
}

#[test]
fn a_conflict_is_skipped_or_replaced_only_when_the_person_said_so() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "source");
    let into = scratch.dir("into");
    fs::write(into.join("bracket.sc"), "old").unwrap();

    let copy = copy_into(&source, &into, Conflict::Skip);
    assert_eq!(copy.outcome, Outcome::Skipped);
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"old");
    assert_eq!(fs::read(&source).unwrap(), b"source");

    // Replacing a file: the source's bytes win, and the source stays where it was.
    let replaced = copy_into(&source, &into, Conflict::Replace);
    assert!(replaced.succeeded(), "{replaced:?}");
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"source");
    assert_eq!(fs::read(&source).unwrap(), b"source");
}

#[test]
fn a_folder_copy_takes_its_whole_tree_as_its_own_copy() {
    let scratch = Scratch::new();
    let source = scratch.dir("parts");
    fs::write(source.join("a.sc"), "a").unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/b.sc"), "b").unwrap();
    let copy = copy_into(&source, scratch.path(), Conflict::KeepBoth);
    assert!(copy.succeeded(), "{copy:?}");
    assert_eq!(copy.target.file_name().and_then(|n| n.to_str()), Some("parts (copy)"));
    assert_eq!(fs::read(copy.target.join("a.sc")).unwrap(), b"a");
    assert_eq!(fs::read(copy.target.join("nested/b.sc")).unwrap(), b"b");
    assert_eq!(fs::read(source.join("a.sc")).unwrap(), b"a", "the source keeps its content");
    // Changing the copy changes only the copy.
    fs::write(copy.target.join("a.sc"), "changed").unwrap();
    assert_eq!(fs::read(source.join("a.sc")).unwrap(), b"a");
}

#[test]
fn a_folder_cannot_be_copied_into_itself() {
    let scratch = Scratch::new();
    let source = scratch.dir("parts");
    let inside = source.join("inside");
    fs::create_dir(&inside).unwrap();
    let result = copy_into(&source, &inside, Conflict::KeepBoth);
    assert!(matches!(result.outcome, Outcome::Failed(_)), "{result:?}");
    assert!(!inside.join("parts").exists());
    assert!(is_within(&source, &inside));
    assert!(!is_within(&inside, &source));
}

#[test]
fn a_move_or_rename_updates_the_name_and_leaves_no_copy() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "data");
    let into = scratch.dir("parts");
    let moved = move_into(&source, &into, Conflict::Skip);
    assert!(moved.succeeded(), "{moved:?}");
    assert!(!source.exists());
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"data");

    let renamed = rename(&into.join("bracket.sc"), "bracket-v2.sc", Conflict::Skip);
    assert!(renamed.succeeded(), "{renamed:?}");
    assert_eq!(fs::read(into.join("bracket-v2.sc")).unwrap(), b"data");
    assert!(!into.join("bracket.sc").exists());
}

#[test]
fn renaming_over_another_file_needs_a_replace_and_keeps_a_folder() {
    let scratch = Scratch::new();
    let a = scratch.file("a.sc", "a");
    scratch.file("b.sc", "b");
    let skipped = rename(&a, "b.sc", Conflict::Skip);
    assert_eq!(skipped.outcome, Outcome::Skipped);
    assert_eq!(fs::read(&a).unwrap(), b"a");
    let replaced = rename(&a, "b.sc", Conflict::Replace);
    assert!(replaced.succeeded(), "{replaced:?}");
    assert_eq!(fs::read(scratch.path().join("b.sc")).unwrap(), b"a");
    assert!(!a.exists());

    // A folder keeps both instead of replacing a file, and a file never replaces a folder.
    let folder = scratch.dir("bodies");
    let kept = copy_into(&folder, scratch.path(), Conflict::Replace);
    assert!(kept.succeeded(), "{kept:?}");
    assert!(folder.is_dir());
    assert_ne!(kept.target, folder);
    let file = scratch.file("c.sc", "c");
    let refused = rename(&file, "bodies", Conflict::Replace);
    assert!(matches!(refused.outcome, Outcome::Failed(_)), "{refused:?}");
    assert!(folder.is_dir() && scratch.path().join("c.sc").exists());
}

#[test]
fn a_name_with_a_separator_is_not_a_file_name() {
    assert!(valid_name("bracket.sc").is_ok());
    assert!(valid_name("  ").is_err());
    assert!(valid_name(".").is_err());
    assert!(valid_name("..").is_err());
    assert!(valid_name("a/b").is_err());
    assert!(valid_name("a\\b").is_err());
    assert!(valid_name("a\0b").is_err());
    assert_eq!(child(Path::new("/tmp"), " part.sc ").unwrap(), PathBuf::from("/tmp/part.sc"));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_copied_and_removed_as_a_link() {
    let scratch = Scratch::new();
    let target = scratch.file("part.sc", "content");
    let link = scratch.path().join("shortcut.sc");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let entries = read_dir(scratch.path()).unwrap();
    let shown = entries.iter().find(|e| e.name == "shortcut.sc").unwrap();
    assert!(matches!(shown.kind, Kind::Link { to_dir: false }), "{shown:?}");

    let into = scratch.dir("into");
    let copy = copy_into(&link, &into, Conflict::Skip);
    assert!(copy.succeeded(), "{copy:?}");
    assert!(fs::symlink_metadata(&copy.target).unwrap().file_type().is_symlink());
    // Removing the link does not touch what it points at.
    remove_permanent(&copy.target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"content");
}

#[test]
fn a_permanent_delete_removes_a_whole_folder() {
    let scratch = Scratch::new();
    let folder = scratch.dir("old");
    fs::write(folder.join("a.sc"), "a").unwrap();
    remove_permanent(&folder).unwrap();
    assert!(!folder.exists());
    assert!(scratch.path().is_dir());
}
