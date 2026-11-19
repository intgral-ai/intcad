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
/// A rename that only changes letter case is a rename everywhere. On a file system that does not
/// tell case apart (macOS and Windows by default) the new spelling already "exists", as this very
/// entry; that used to be refused as a name already taken.
#[test]
fn a_rename_that_only_changes_letter_case_gives_the_new_spelling() {
    let scratch = Scratch::new();
    let file = scratch.file("bracket.sc", "x");
    let result = rename(&file, "Bracket.sc", Conflict::Skip);
    assert_eq!(result.outcome, Outcome::Done, "{result:?}");
    let names: Vec<String> = read_dir(scratch.path()).unwrap().into_iter().map(|entry| entry.name).collect();
    assert_eq!(names, ["Bracket.sc"]);
    let folder = scratch.dir("parts");
    assert_eq!(rename(&folder, "Parts", Conflict::Skip).outcome, Outcome::Done);
    assert!(read_dir(scratch.path()).unwrap().iter().any(|entry| entry.name == "Parts"));
    // Another entry under the wanted name is still a conflict.
    scratch.file("notes.txt", "kept");
    let other = scratch.file("draft.txt", "moving");
    assert_eq!(rename(&other, "notes.txt", Conflict::Skip).outcome, Outcome::Skipped);
    assert_eq!(fs::read_to_string(scratch.path().join("notes.txt")).unwrap(), "kept");
}

/// The Trash on macOS takes the path as data: a name with a quote, a backslash or script text is
/// an ordinary name, and a link goes to the Trash as a link (its target stays).
#[cfg(target_vendor = "apple")]
#[test]
#[ignore = "puts test files in the real Trash; run on macOS with --ignored"]
fn the_trash_takes_awkward_names_and_links_as_they_are() {
    let scratch = Scratch::new();
    let target = scratch.dir("real");
    fs::write(target.join("keep.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(&target, scratch.path().join("link")).unwrap();
    for name in ["plain.txt", "quote\"name.txt", "back\\slash.txt", "x\" & (do shell script \"true\") & \".txt"] {
        let path = scratch.file(name, "x");
        assert_eq!(trash(&path), Ok(()), "{name}");
        assert!(!path.exists(), "{name} left the folder");
    }
    assert_eq!(trash(&scratch.path().join("link")), Ok(()));
    assert!(fs::symlink_metadata(scratch.path().join("link")).is_err(), "the link is gone");
    assert_eq!(fs::read_to_string(target.join("keep.txt")).unwrap(), "keep", "its target is untouched");
    assert!(matches!(trash(&scratch.path().join("never-there.txt")), Err(TrashProblem::Refused(_))));
}

/// A file no program opens gets the same plain reason on macOS as on the other systems.
#[cfg(target_vendor = "apple")]
#[test]
fn a_file_no_program_opens_says_so_in_plain_words() {
    let scratch = Scratch::new();
    let path = scratch.file("orphan.intcad-no-program", "x");
    assert_eq!(open_default_waiting(&path), Err("no program is associated with this file".to_string()));
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

/// A replace is not allowed to lose the only copy: the old destination is put back when the
/// replacement fails.
#[test]
fn a_failed_replace_puts_the_destination_back() {
    let scratch = Scratch::new();
    let target = scratch.file("keep.txt", "old");
    let error = swap_in(&target, || Err("the move was refused".into())).unwrap_err();
    assert!(error.contains("refused"), "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"old");
    let leftovers: Vec<_> = fs::read_dir(scratch.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().starts_with(".intcad-replace-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_completed_replace_removes_the_kept_copy_and_keeps_the_new_file() {
    let scratch = Scratch::new();
    let target = scratch.file("keep.txt", "old");
    swap_in(&target, || fs::write(&target, "new").map_err(|e| e.to_string())).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new");
    let leftovers: Vec<_> = fs::read_dir(scratch.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().starts_with(".intcad-replace-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_replace_clears_a_partial_destination_before_restoring_the_old_one() {
    let scratch = Scratch::new();
    let target = scratch.file("keep.txt", "old");
    let error = swap_in(&target, || {
        fs::write(&target, "half-written").map_err(|e| e.to_string())?;
        Err("copy stopped".into())
    })
    .unwrap_err();
    assert!(error.contains("copy stopped"), "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"old");
}

// Existing replacement tests never exercised a failure while creating a new tree.
#[test]
fn a_failed_recursive_copy_does_not_publish_a_partial_tree() {
    let scratch = Scratch::new();
    let source = scratch.dir("source");
    fs::write(source.join("keep.sc"), "source content").unwrap();
    let mut nested = source.clone();
    for _ in 0..258 {
        nested.push("d");
        fs::create_dir(&nested).unwrap();
    }
    let into = scratch.dir("into");
    let result = copy_into(&source, &into, Conflict::Skip);
    assert!(matches!(result.outcome, Outcome::Failed(_)), "{result:?}");
    assert_eq!(fs::read(source.join("keep.sc")).unwrap(), b"source content");
    assert!(!into.join("source").exists(), "a failed copy must not look complete");
    assert_eq!(fs::read_dir(&into).unwrap().count(), 0, "owned staging must be cleaned");
}

#[cfg(unix)]
fn fake_opener(scratch: &Scratch, stderr: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let program = scratch.file("open", &format!("#!/bin/sh\nprintf '%s' '{}' >&2\nexit 1\n", stderr.replace('\'', "'\\''")));
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    program
}

#[cfg(unix)]
#[test]
fn a_known_launchservices_missing_association_has_a_stable_reason() {
    let scratch = Scratch::new();
    let program = fake_opener(
        &scratch,
        "No application knows how to open URL file:///part.unknown (Error Domain=NSOSStatusErrorDomain Code=-10814 \"kLSApplicationNotFoundErr\")",
    );
    assert_eq!(wait_for(program.to_str().unwrap(), &[]).unwrap_err(), "no program is associated with this file");
}

#[cfg(unix)]
#[test]
fn an_opener_failure_without_diagnostics_is_not_a_missing_association() {
    let scratch = Scratch::new();
    let program = fake_opener(&scratch, "");
    let error = wait_for(program.to_str().unwrap(), &[]).unwrap_err();
    assert!(!error.contains("no program is associated"), "{error}");
}

#[cfg(unix)]
#[test]
fn unrelated_opener_errors_keep_their_diagnostics() {
    for reason in [
        "The file /part.sc does not exist.",
        "Permission denied",
        "No application knows how to open URL file:///part.sc (Error Domain=NSOSStatusErrorDomain Code=-10810 \"launch failed\")",
        "failed to read file named Error Domain=NSOSStatusErrorDomain Code=-10814 example",
    ] {
        let scratch = Scratch::new();
        let program = fake_opener(&scratch, reason);
        assert_eq!(wait_for(program.to_str().unwrap(), &[]).unwrap_err(), reason);
    }
}

#[cfg(unix)]
#[test]
fn launchservices_text_from_an_unrelated_program_is_not_reclassified() {
    let scratch = Scratch::new();
    let program =
        fake_opener(&scratch, "No application knows how to open URL file:///part.sc (Error Domain=NSOSStatusErrorDomain Code=-10814 \"missing\")");
    let other = scratch.path().join("other-opener");
    fs::rename(program, &other).unwrap();
    assert_ne!(wait_for(other.to_str().unwrap(), &[]).unwrap_err(), NO_ASSOCIATED_PROGRAM);
}

#[cfg(unix)]
#[test]
fn an_error_code_in_the_url_is_not_a_missing_association() {
    for reason in [
        "No application knows how to open URL file:///Error Domain=NSOSStatusErrorDomain Code=-10814 part.sc (Error Domain=NSOSStatusErrorDomain Code=-10810 \"launch failed\")",
        "No application knows how to open URL file:///part (Error Domain=NSOSStatusErrorDomain Code=-10814 payload.sc (Error Domain=NSCocoaErrorDomain Code=257 \"permission denied\")",
    ] {
        let scratch = Scratch::new();
        let program = fake_opener(&scratch, reason);
        assert_eq!(wait_for(program.to_str().unwrap(), &[]).unwrap_err(), reason);
    }
}

fn assert_stored_name(parent: &Path, name: &str) {
    assert!(fs::read_dir(parent).unwrap().any(|entry| entry.unwrap().file_name() == name));
}

#[test]
fn case_only_file_and_folder_renames_store_the_requested_spelling() {
    let scratch = Scratch::new();
    let file = scratch.file("bracket.sc", "dirty design");
    #[cfg(unix)]
    let file_identity = entry_identity(&file);
    let renamed = rename(&file, "Bracket.sc", Conflict::Skip);
    assert!(renamed.succeeded(), "{renamed:?}");
    assert_stored_name(scratch.path(), "Bracket.sc");
    assert_eq!(fs::read(&renamed.target).unwrap(), b"dirty design");
    #[cfg(unix)]
    assert_eq!(entry_identity(&renamed.target), file_identity);
    let folder = scratch.dir("parts");
    fs::write(folder.join("part.sc"), "part").unwrap();
    #[cfg(unix)]
    let folder_identity = entry_identity(&folder);
    let renamed = rename(&folder, "Parts", Conflict::Skip);
    assert!(renamed.succeeded(), "{renamed:?}");
    assert_stored_name(scratch.path(), "Parts");
    assert_eq!(fs::read(renamed.target.join("part.sc")).unwrap(), b"part");
    #[cfg(unix)]
    assert_eq!(entry_identity(&renamed.target), folder_identity);
}

#[cfg(unix)]
fn entry_identity(path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

#[cfg(unix)]
#[test]
fn case_only_symlink_rename_preserves_the_link_and_target() {
    let scratch = Scratch::new();
    let target = scratch.file("target.sc", "untouched");
    let link = scratch.path().join("link.sc");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let identity = entry_identity(&link);
    let result = rename(&link, "Link.sc", Conflict::Skip);
    assert!(result.succeeded(), "{result:?}");
    assert_stored_name(scratch.path(), "Link.sc");
    assert_eq!(fs::read_link(&result.target).unwrap(), target);
    assert_eq!(entry_identity(&result.target), identity);
    assert_eq!(fs::read(target).unwrap(), b"untouched");
}

#[test]
fn a_distinct_hardlink_is_not_a_case_only_rename() {
    let scratch = Scratch::new();
    let source = scratch.file("part.sc", "source");
    let target = scratch.path().join("Part.sc");
    // On insensitive volumes these spellings already resolve to the same entry.
    if target.exists() {
        return;
    }
    fs::hard_link(&source, &target).unwrap();
    let result = rename(&source, "Part.sc", Conflict::Skip);
    assert_eq!(result.outcome, Outcome::Skipped);
    assert_stored_name(scratch.path(), "part.sc");
    assert_stored_name(scratch.path(), "Part.sc");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(fs::read(target).unwrap(), b"source");
}

#[test]
fn a_differently_named_hardlink_is_a_conflict_even_on_insensitive_volumes() {
    let scratch = Scratch::new();
    let source = scratch.file("source.sc", "source");
    let target = scratch.path().join("other.sc");
    fs::hard_link(&source, &target).unwrap();
    assert_eq!(rename(&source, "other.sc", Conflict::Skip).outcome, Outcome::Skipped);
    assert_stored_name(scratch.path(), "source.sc");
    assert_stored_name(scratch.path(), "other.sc");
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(target).unwrap(), b"source");
}

#[test]
fn distinct_case_sensitive_entries_keep_their_own_content_on_rename_conflict() {
    let scratch = Scratch::new();
    let source = scratch.file("part.sc", "source");
    let target = scratch.path().join("Part.sc");
    if target.exists() {
        return; // The separate same-entry rename test covers insensitive volumes.
    }
    fs::write(&target, "different destination").unwrap();
    assert_eq!(rename(&source, "Part.sc", Conflict::Skip).outcome, Outcome::Skipped);
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(target).unwrap(), b"different destination");
}

#[cfg(unix)]
#[test]
fn distinct_symlinks_to_one_target_are_not_the_same_entry() {
    let scratch = Scratch::new();
    let target = scratch.file("target.sc", "untouched");
    let a = scratch.path().join("a.sc");
    let b = scratch.path().join("b.sc");
    std::os::unix::fs::symlink(&target, &a).unwrap();
    std::os::unix::fs::symlink(&target, &b).unwrap();
    assert_eq!(rename(&a, "b.sc", Conflict::Skip).outcome, Outcome::Skipped);
    assert_eq!(fs::read_link(a).unwrap(), target);
    assert_eq!(fs::read_link(b).unwrap(), target);
    assert_eq!(fs::read(target).unwrap(), b"untouched");
}

// Native Foundation/TCC behavior cannot be established by a Linux unit test.
#[cfg(target_os = "macos")]
#[test]
fn native_trash_treats_hostile_names_as_data_and_preserves_link_targets() {
    let scratch = Scratch::new();
    let target = scratch.file("keep.sc", "untouched");
    for name in ["quote\"name", "back\\slash", "雪\npart", "x\" & (do shell script \"false\") & \".txt"] {
        let file = scratch.file(name, "trash me");
        trash(&file).unwrap();
        assert!(!file.exists());
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
    }
    let folder = scratch.dir("folder\"雪");
    fs::write(folder.join("child"), "child").unwrap();
    trash(&folder).unwrap();
    assert!(!folder.exists());
    let link = scratch.path().join("link\"\\雪\n");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    trash(&link).unwrap();
    assert!(fs::symlink_metadata(link).is_err());
    assert_eq!(fs::read(target).unwrap(), b"untouched");
}

#[cfg(target_os = "macos")]
#[test]
fn native_trash_refuses_an_invalid_url_without_permanent_deletion() {
    let scratch = Scratch::new();
    let keep = scratch.file("keep.sc", "untouched");
    assert!(matches!(trash(&scratch.path().join("invalid\0path")), Err(TrashProblem::Refused(_))));
    assert_eq!(fs::read(keep).unwrap(), b"untouched");
}

// Faults are scoped to one thread/operation, not environment variables or global process state.
// Only the failing OS boundary is substituted; traversal, publication and cleanup stay real.
type Fault = Box<dyn FnMut(&str, &Path, &Path) -> io::Result<()>>;
thread_local! {
    static FAULT: std::cell::RefCell<Option<Fault>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn checkpoint(event: &str, source: &Path, target: &Path) -> io::Result<()> {
    FAULT.with_borrow_mut(|fault| match fault.as_mut() {
        Some(fault) => fault(event, source, target),
        None => Ok(()),
    })
}

fn with_fault<T>(fault: impl FnMut(&str, &Path, &Path) -> io::Result<()> + 'static, run: impl FnOnce() -> T) -> T {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            FAULT.with_borrow_mut(|fault| *fault = None);
        }
    }
    FAULT.with_borrow_mut(|slot| {
        assert!(slot.is_none());
        *slot = Some(Box::new(fault));
    });
    let _clear = Clear;
    run()
}

fn injected() -> io::Error {
    io::Error::other("injected no space left on device")
}

fn exdev() -> io::Error {
    io::Error::from_raw_os_error(if cfg!(windows) { 17 } else { 18 })
}

#[test]
fn a_mid_file_copy_or_cross_device_move_never_publishes_partial_bytes() {
    for moving in [false, true] {
        let scratch = Scratch::new();
        let source = scratch.file("part.sc", "complete source");
        let into = scratch.dir("into");
        let result = with_fault(
            |event, _, target| match event {
                "move" => Err(exdev()),
                "copy" => {
                    fs::write(target, "partial")?;
                    Err(injected())
                }
                _ => Ok(()),
            },
            || if moving { move_into(&source, &into, Conflict::Skip) } else { copy_into(&source, &into, Conflict::Skip) },
        );
        assert!(matches!(result.outcome, Outcome::Failed(_)), "{result:?}");
        assert_eq!(fs::read(&source).unwrap(), b"complete source");
        assert_eq!(fs::read_dir(&into).unwrap().count(), 0, "{result:?}");
    }
}

#[test]
fn a_mid_directory_copy_or_cross_device_move_cleans_the_owned_tree() {
    for moving in [false, true] {
        let scratch = Scratch::new();
        let source = scratch.dir("parts");
        fs::write(source.join("a"), "first").unwrap();
        fs::write(source.join("b"), "second").unwrap();
        let into = scratch.dir("into");
        let mut copies = 0;
        let result = with_fault(
            move |event, _, target| {
                if event == "move" {
                    return Err(exdev());
                }
                if event == "copy" {
                    copies += 1;
                    if copies == 2 {
                        fs::write(target, "partial")?;
                        return Err(injected());
                    }
                }
                Ok(())
            },
            || if moving { move_into(&source, &into, Conflict::Skip) } else { copy_into(&source, &into, Conflict::Skip) },
        );
        assert!(matches!(result.outcome, Outcome::Failed(_)), "{result:?}");
        assert_eq!(fs::read(source.join("a")).unwrap(), b"first");
        assert_eq!(fs::read(source.join("b")).unwrap(), b"second");
        assert_eq!(fs::read_dir(&into).unwrap().count(), 0, "{result:?}");
    }
}

#[test]
fn failed_copy_and_publication_preserve_an_existing_replacement() {
    for moving in [false, true] {
        for failure in ["copy", "publish"] {
            let scratch = Scratch::new();
            let source = scratch.file("part.sc", "source");
            let into = scratch.dir("into");
            let target = into.join("part.sc");
            fs::write(&target, "old destination").unwrap();
            let result = with_fault(
                move |event, _, path| {
                    if event == "move" {
                        return Err(exdev());
                    }
                    if event == failure {
                        if event == "copy" {
                            fs::write(path, "partial")?;
                        }
                        return Err(injected());
                    }
                    Ok(())
                },
                || if moving { move_into(&source, &into, Conflict::Replace) } else { copy_into(&source, &into, Conflict::Replace) },
            );
            assert!(matches!(result.outcome, Outcome::Failed(_)), "{moving} {failure}: {result:?}");
            assert_eq!(fs::read(source).unwrap(), b"source");
            assert_eq!(fs::read(target).unwrap(), b"old destination");
            assert_eq!(fs::read_dir(into).unwrap().count(), 1);
        }
    }
}

#[test]
fn failed_publication_of_a_new_copy_or_cross_device_move_cleans_staging() {
    for moving in [false, true] {
        let scratch = Scratch::new();
        let source = scratch.file("part.sc", "source");
        let into = scratch.dir("into");
        let result = with_fault(
            |event, _, _| match event {
                "move" => Err(exdev()),
                "publish" => Err(injected()),
                _ => Ok(()),
            },
            || if moving { move_into(&source, &into, Conflict::Skip) } else { copy_into(&source, &into, Conflict::Skip) },
        );
        assert!(matches!(result.outcome, Outcome::Failed(_)), "{result:?}");
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read_dir(into).unwrap().count(), 0);
    }
}

#[test]
fn skip_and_keep_both_do_not_clobber_an_entry_introduced_during_copy_or_move() {
    for conflict in [Conflict::Skip, Conflict::KeepBoth] {
        for directory in [false, true] {
            for mode in ["copy", "cross-device move", "move"] {
                let scratch = Scratch::new();
                let source = if directory { scratch.dir("parts") } else { scratch.file("part.sc", "source") };
                if directory {
                    fs::write(source.join("part"), "source").unwrap();
                }
                let into = scratch.dir("into");
                if conflict == Conflict::KeepBoth {
                    if directory {
                        fs::create_dir(into.join("parts")).unwrap();
                    } else {
                        fs::write(into.join("part.sc"), "original destination").unwrap();
                    }
                }
                let result = with_fault(
                    move |event, _, target| {
                        if event == "move" && mode == "cross-device move" {
                            return Err(exdev());
                        }
                        if event == "publish" || (event == "move" && mode == "move") {
                            if directory {
                                fs::create_dir(target)?;
                            } else {
                                fs::write(target, "unrelated")?;
                            }
                        }
                        Ok(())
                    },
                    || if mode == "copy" { copy_into(&source, &into, conflict) } else { move_into(&source, &into, conflict) },
                );
                assert!(matches!(result.outcome, Outcome::Failed(_)), "{result:?}");
                if directory {
                    assert_eq!(fs::read_dir(result.target).unwrap().count(), 0, "the concurrent empty directory must not be replaced");
                    assert_eq!(fs::read(source.join("part")).unwrap(), b"source");
                } else {
                    assert_eq!(fs::read(result.target).unwrap(), b"unrelated");
                    assert_eq!(fs::read(source).unwrap(), b"source");
                }
                assert_eq!(fs::read_dir(into).unwrap().count(), if conflict == Conflict::KeepBoth { 2 } else { 1 });
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn a_complete_cross_device_move_preserves_nested_and_dangling_symlinks() {
    let scratch = Scratch::new();
    let source = scratch.dir("parts");
    let target = scratch.file("target.sc", "untouched");
    fs::write(source.join("part.sc"), "complete source").unwrap();
    std::os::unix::fs::symlink(&target, source.join("link")).unwrap();
    std::os::unix::fs::symlink("missing", source.join("dangling")).unwrap();
    let into = scratch.dir("into");
    let result = with_fault(|event, _, _| if event == "move" { Err(exdev()) } else { Ok(()) }, || move_into(&source, &into, Conflict::Skip));
    assert!(result.succeeded(), "{result:?}");
    assert!(!source.exists());
    assert_eq!(fs::read(result.target.join("part.sc")).unwrap(), b"complete source");
    assert_eq!(fs::read_link(result.target.join("link")).unwrap(), target);
    assert_eq!(fs::read_link(result.target.join("dangling")).unwrap(), Path::new("missing"));
    assert_eq!(fs::read(target).unwrap(), b"untouched");
    assert_eq!(fs::read_dir(into).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_dangling_destination_link_is_a_conflict_and_keep_both_uses_a_new_name() {
    let scratch = Scratch::new();
    let source = scratch.file("part.sc", "source");
    let into = scratch.dir("into");
    let target = into.join("part.sc");
    std::os::unix::fs::symlink("missing", &target).unwrap();
    assert_eq!(copy_into(&source, &into, Conflict::Skip).outcome, Outcome::Skipped);
    let copy = copy_into(&source, &into, Conflict::KeepBoth);
    assert!(copy.succeeded(), "{copy:?}");
    assert_eq!(copy.target.file_name().unwrap(), "part (copy).sc");
    assert_eq!(fs::read_link(target).unwrap(), Path::new("missing"));
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(copy.target).unwrap(), b"source");
}

#[test]
fn a_staging_cleanup_failure_reports_the_retained_recovery_path() {
    let scratch = Scratch::new();
    let source = scratch.file("part.sc", "source");
    let into = scratch.dir("into");
    let unrelated = into.join(".intcad-stage-unrelated");
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("keep"), "unrelated").unwrap();
    let result = with_fault(
        |event, _, target| match event {
            "copy" => {
                fs::write(target, "partial")?;
                Err(injected())
            }
            "cleanup" => Err(io::Error::other("injected cleanup refused")),
            _ => Ok(()),
        },
        || copy_into(&source, &into, Conflict::Skip),
    );
    let Outcome::Failed(error) = result.outcome else { panic!("{result:?}") };
    let retained: Vec<_> = fs::read_dir(&into).unwrap().map(|e| e.unwrap().path()).filter(|p| *p != unrelated).collect();
    assert_eq!(retained.len(), 1);
    assert!(error.contains(&retained[0].display().to_string()), "{error}");
    assert!(error.contains("cleanup refused"), "{error}");
    assert!(!into.join("part.sc").exists());
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(unrelated.join("keep")).unwrap(), b"unrelated");
}

#[test]
fn source_removal_failure_keeps_the_complete_published_move_even_when_replacing() {
    for replacing in [false, true] {
        let scratch = Scratch::new();
        let source = scratch.file("part.sc", "complete source");
        let into = scratch.dir("into");
        if replacing {
            fs::write(into.join("part.sc"), "old").unwrap();
        }
        let result = with_fault(
            |event, _, _| match event {
                "move" => Err(exdev()),
                "remove source" => Err(io::Error::other("source removal refused")),
                _ => Ok(()),
            },
            || move_into(&source, &into, if replacing { Conflict::Replace } else { Conflict::Skip }),
        );
        let Outcome::Failed(error) = &result.outcome else { panic!("{result:?}") };
        assert!(error.contains("source could not be removed"), "{error}");
        assert!(error.contains(&result.target.display().to_string()), "{error}");
        assert_eq!(fs::read(source).unwrap(), b"complete source");
        assert_eq!(fs::read(result.target).unwrap(), b"complete source");
        assert_eq!(fs::read_dir(into).unwrap().count(), 1);
    }
}

#[cfg(windows)]
#[test]
fn windows_native_publication_rejects_nul_without_touching_the_prefix_entry() {
    let scratch = Scratch::new();
    let source = scratch.file("source.sc", "source");
    let target = scratch.file("target.sc", "destination");
    let error = rename_no_replace(&source, &scratch.path().join("target.sc\0suffix")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(target).unwrap(), b"destination");
}

#[cfg(windows)]
#[test]
fn windows_native_publication_accepts_long_parent_paths() {
    let scratch = Scratch::new();
    let mut parent = scratch.path().to_path_buf();
    for _ in 0..6 {
        parent.push("a-long-folder-name-with-more-than-forty-characters");
        fs::create_dir(&parent).unwrap();
    }
    let source = parent.join("source.sc");
    fs::write(&source, "source").unwrap();
    let into = parent.join("into");
    fs::create_dir(&into).unwrap();
    let result = move_into(&source, &into, Conflict::Skip);
    assert!(result.succeeded(), "{result:?}");
    assert_eq!(fs::read(result.target).unwrap(), b"source");
    assert!(!source.exists());
}
