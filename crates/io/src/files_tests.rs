use super::*;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
use std::sync::Arc;
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

/// A Windows child is one directory entry, not a drive and not an alternate data stream.
/// The rule is checked for both platform answers here; the live runtime assertion is the
/// `#[cfg(windows)]` test beside this one, so Linux can validate the rule without claiming
/// Windows behavior.
#[test]
fn a_windows_child_name_is_one_component_and_refuses_colons() {
    for name in ["part.sc:stream", "C:part.sc", ":", "a:b", ":stream", "stream:", "C:"] {
        assert!(!one_child_name(name, true), "{name}");
        assert!(one_child_name(name, false), "non-Windows accepted names stay unchanged: {name}");
    }
    for name in ["bracket.sc", "part 2.sc", "雪.sc", "a.b", "C part.sc", "$data", "name with spaces"] {
        assert!(one_child_name(name, true), "{name}");
        assert!(one_child_name(name, false), "{name}");
    }
    // The live answer still follows the platform this test binary was built for.
    assert_eq!(valid_name("part.sc:stream").is_ok(), !cfg!(windows));
    assert_eq!(valid_name("C:part.sc").is_ok(), !cfg!(windows));
    assert!(valid_name("bracket.sc").is_ok());
    assert_eq!(valid_name("  bracket.sc ").unwrap(), "bracket.sc");
}

#[cfg(windows)]
#[test]
fn windows_validation_refuses_drive_and_stream_names() {
    for name in ["C:part.sc", "part.sc:stream", ":stream", "C:"] {
        assert!(valid_name(name).is_err(), "{name}");
    }
    assert!(valid_name("bracket.sc").is_ok());
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

// A failed publication may leave a complete new destination behind. Clearing it so the old
// entry can be renamed back would destroy the new bytes. Both copies must survive, and the
// error must name the retained previous entry.
#[test]
fn a_failed_replace_with_a_new_destination_keeps_both_copies_and_reports_the_backup() {
    let scratch = Scratch::new();
    let target = scratch.file("keep.txt", "old");
    let error = swap_in(&target, || {
        fs::write(&target, "new destination").map_err(|e| e.to_string())?;
        Err("publication stopped".into())
    })
    .unwrap_err();
    assert!(error.contains("publication stopped"), "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"new destination", "the new destination survives");
    let retained: Vec<_> = fs::read_dir(scratch.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".intcad-replace-")))
        .collect();
    assert_eq!(retained.len(), 1, "{retained:?}");
    let backup = retained[0].join("previous");
    assert_eq!(fs::read(&backup).unwrap(), b"old", "the previous bytes survive");
    assert!(error.contains(&backup.display().to_string()), "{error}");
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

// The Trash classification is decided from the error's domain and code, not from a bare number.
// A Linux test can cover that decision while the Foundation call itself stays macOS-only.
#[test]
fn only_cocoas_feature_unsupported_error_means_there_is_no_trash() {
    assert_eq!(NO_TRASH_HERE, 3328, "NSFeatureUnsupportedError");
    assert!(trash_error_means_no_trash("NSCocoaErrorDomain", NO_TRASH_HERE));
}

#[test]
fn access_denial_in_the_cocoa_domain_stays_a_refusal() {
    // NSFileReadNoPermissionError and NSFileWriteNoPermissionError live in the same domain.
    for code in [257, 513, 0, NO_TRASH_HERE + 1, NO_TRASH_HERE - 1] {
        assert!(!trash_error_means_no_trash("NSCocoaErrorDomain", code), "code {code}");
    }
}

#[test]
fn the_feature_unsupported_code_in_another_domain_stays_a_refusal() {
    for domain in ["NSPOSIXErrorDomain", "NSOSStatusErrorDomain", "", "com.example.intcad"] {
        assert!(!trash_error_means_no_trash(domain, NO_TRASH_HERE), "{domain}");
    }
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

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn scratch_trash(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let files = scratch.path().join("Trash/files");
    let info = scratch.path().join("Trash/info");
    fs::create_dir_all(&files).unwrap();
    fs::create_dir_all(&info).unwrap();
    (files, info)
}

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn assert_trash_pair(files: &Path, info: &Path, name: &str, body: &[u8], original: &Path) {
    assert_eq!(fs::read(files.join(name)).unwrap(), body, "{name}");
    let record = fs::read_to_string(info.join(format!("{name}.trashinfo"))).unwrap();
    assert!(record.starts_with("[Trash Info]\n"), "{record}");
    let original = original.to_string_lossy();
    assert!(record.contains(&format!("Path={}\n", percent_encode(&original))), "{record}");
    assert!(record.contains("DeletionDate="), "{record}");
}

// A dangling link is an entry: `exists()` follows it and reports false, which used to let the
// payload rename replace another writer's link. The next free name keeps both entries.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_does_not_clobber_a_dangling_payload_name() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "source");
    let occupied = files.join("part.sc");
    std::os::unix::fs::symlink("missing", &occupied).unwrap();
    move_into_trash(&files, &info, &source).unwrap();
    assert_eq!(fs::read_link(&occupied).unwrap(), Path::new("missing"));
    assert!(!source.exists());
    assert_trash_pair(&files, &info, "part.sc.2", b"source", &source);
    assert!(!info.join("part.sc.trashinfo").exists(), "a withdrawn attempt leaves no record");
}

// The payload and its record are one pair. Both existing entries keep their bytes, and the
// new item takes a free pair name instead of overwriting either half.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_keeps_an_existing_record_and_payload_pair() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "new source");
    let old_payload = files.join("part.sc");
    let old_record = info.join("part.sc.trashinfo");
    fs::write(&old_payload, "old payload").unwrap();
    fs::write(&old_record, "old record").unwrap();
    move_into_trash(&files, &info, &source).unwrap();
    assert_eq!(fs::read(&old_payload).unwrap(), b"old payload");
    assert_eq!(fs::read(&old_record).unwrap(), b"old record");
    assert!(!source.exists());
    assert_trash_pair(&files, &info, "part.sc.2", b"new source", &source);
}

// A record name another writer already holds is not adopted or truncated.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_does_not_clobber_a_dangling_record_name() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "source");
    let occupied = info.join("part.sc.trashinfo");
    std::os::unix::fs::symlink("missing", &occupied).unwrap();
    move_into_trash(&files, &info, &source).unwrap();
    assert_eq!(fs::read_link(&occupied).unwrap(), Path::new("missing"), "the other writer's entry is untouched");
    assert!(!source.exists());
    assert_trash_pair(&files, &info, "part.sc.2", b"source", &source);
}

// Publication is exclusive rather than a check followed by a clobbering rename. A writer that
// takes the payload name between our metadata creation and publication wins; our withdrawn
// record is removed and the item takes the next pair.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_retries_a_payload_name_taken_during_publication() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "source");
    let attempts = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&attempts);
    let result = with_fault(
        move |event, _, target| {
            if event == "trash publish" && seen.fetch_add(1, Ordering::Relaxed) == 0 {
                fs::write(target, "other writer")?;
            }
            Ok(())
        },
        || move_into_trash(&files, &info, &source),
    );
    result.unwrap();
    assert_eq!(attempts.load(Ordering::Relaxed), 2, "the first attempt was refused and retried");
    assert_eq!(fs::read(files.join("part.sc")).unwrap(), b"other writer");
    assert!(fs::symlink_metadata(info.join("part.sc.trashinfo")).is_err(), "our withdrawn record is gone");
    assert!(!source.exists());
    assert_trash_pair(&files, &info, "part.sc.2", b"source", &source);
}

// Forced EXDEV runs the real staged copy, publication and source removal on a disposable
// fixture; the record is written before the copy and survives beside the payload.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_cross_device_copy_publishes_the_pair_and_removes_the_source() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "complete source");
    let moves = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&moves);
    let result = with_fault(
        move |event, _, _| {
            if event == "trash move" {
                seen.fetch_add(1, Ordering::Relaxed);
                return Err(exdev());
            }
            Ok(())
        },
        || move_into_trash(&files, &info, &source),
    );
    result.unwrap();
    assert_eq!(moves.load(Ordering::Relaxed), 1, "the forced cross-device path ran");
    assert!(!source.exists());
    assert_trash_pair(&files, &info, "part.sc", b"complete source", &source);
}

// A copy that stops before completion removes its own record and keeps the source. It never
// publishes a partial payload and never falls back to permanent deletion.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_cross_device_copy_failure_keeps_the_source_and_leaves_no_record() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "complete source");
    let moves = Arc::new(AtomicU32::new(0));
    let copies = Arc::new(AtomicU32::new(0));
    let (seen_moves, seen_copies) = (Arc::clone(&moves), Arc::clone(&copies));
    let result = with_fault(
        move |event, _, target| match event {
            "trash move" => {
                seen_moves.fetch_add(1, Ordering::Relaxed);
                Err(exdev())
            }
            "copy" => {
                seen_copies.fetch_add(1, Ordering::Relaxed);
                fs::write(target, "partial")?;
                Err(injected())
            }
            _ => Ok(()),
        },
        || move_into_trash(&files, &info, &source),
    );
    assert!(result.is_err(), "{result:?}");
    assert_eq!(moves.load(Ordering::Relaxed), 1);
    assert_eq!(copies.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(&source).unwrap(), b"complete source");
    assert_eq!(fs::read_dir(&files).unwrap().count(), 0, "no partial payload or staging remains");
    assert_eq!(fs::read_dir(&info).unwrap().count(), 0, "the record created for the failed attempt was withdrawn");
}

// Removal can fail after part of the source is gone. The complete copy and its record stay and
// the error names both, so the person still has everything the trash holds. The call reports
// failure; it does not claim success or delete anything for good.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_partial_source_removal_reports_the_retained_pair() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.dir("parts");
    fs::write(source.join("a"), "first").unwrap();
    fs::write(source.join("b"), "second").unwrap();
    let removals = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&removals);
    let source_for_fault = source.clone();
    let result = with_fault(
        move |event, _, _| match event {
            "trash move" => Err(exdev()),
            "trash remove source" => {
                seen.fetch_add(1, Ordering::Relaxed);
                fs::remove_file(source_for_fault.join("a"))?;
                Err(injected())
            }
            _ => Ok(()),
        },
        || move_into_trash(&files, &info, &source),
    );
    let error = result.unwrap_err();
    assert_eq!(removals.load(Ordering::Relaxed), 1);
    let payload = files.join("parts");
    let record = info.join("parts.trashinfo");
    assert_eq!(fs::read(payload.join("a")).unwrap(), b"first", "the recovery copy is complete");
    assert_eq!(fs::read(payload.join("b")).unwrap(), b"second");
    let text = fs::read_to_string(&record).unwrap();
    assert!(text.contains("DeletionDate="), "{text}");
    assert!(error.contains(&payload.display().to_string()), "the error names the retained payload: {error}");
    assert!(error.contains(&record.display().to_string()), "the error names the retained record: {error}");
    assert!(error.contains("source could not be removed"), "{error}");
    assert!(source.join("b").exists(), "the source is not deleted after a failed removal");
    assert!(!source.join("a").exists(), "the injected partial removal is what the fault did");
}

// A successful staged publication followed by a cleanup failure is not an unpublished attempt.
// The complete payload and its record stay, the source stays, and the error names the payload,
// record and retained staging tree. Withdrawing the record here used to leave a complete unpaired
// payload.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_cleanup_failure_after_cross_device_publish_keeps_the_pair_and_source() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "complete source");
    let result = with_fault(
        |event, _, _| match event {
            "trash move" => Err(exdev()),
            "cleanup" => Err(io::Error::other("injected cleanup refused")),
            _ => Ok(()),
        },
        || move_into_trash(&files, &info, &source),
    );
    let error = result.unwrap_err();
    assert_trash_pair(&files, &info, "part.sc", b"complete source", &source);
    assert_eq!(fs::read(&source).unwrap(), b"complete source", "the source is not removed after a cleanup failure");
    let staging: Vec<_> = fs::read_dir(&files)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".intcad-stage-")))
        .collect();
    assert_eq!(staging.len(), 1, "{staging:?}");
    for path in [files.join("part.sc"), info.join("part.sc.trashinfo"), staging[0].clone()] {
        assert!(error.contains(&path.display().to_string()), "the error names {}: {error}", path.display());
    }
}

// A competing writer can take the staged payload name after our record was created. That
// publication did not happen, so our record is withdrawn; the other writer's payload and the
// source are untouched. Publication is decided from the primitive's outcome, never from whether
// the target now exists.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[test]
fn linux_trash_staged_publish_collision_withdraws_only_the_unpublished_record() {
    let scratch = Scratch::new();
    let (files, info) = scratch_trash(&scratch);
    let source = scratch.file("part.sc", "source");
    let result = with_fault(
        |event, _, target| {
            if event == "trash move" {
                return Err(exdev());
            }
            if event == "publish" {
                fs::write(target, "other writer")?;
            }
            Ok(())
        },
        || move_into_trash(&files, &info, &source),
    );
    assert!(result.is_err(), "{result:?}");
    assert_eq!(fs::read(files.join("part.sc")).unwrap(), b"other writer");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert!(fs::symlink_metadata(info.join("part.sc.trashinfo")).is_err(), "only the unpublished record is withdrawn");
    assert_eq!(fs::read_dir(&info).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&files).unwrap().count(), 1, "staging is cleaned and the unrelated payload stays");
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

// The recycle adapter's executable instructions are constant; an item path crosses the
// process boundary as environment data and is never interpreted as PowerShell. Benign
// names, including an apostrophe, and ordinary Windows path separators are enough to assert
// the boundary: this test builds no attack payload and runs no process.
#[test]
fn windows_recycle_adapter_keeps_instructions_constant_and_transfers_the_path_as_data() {
    use std::ffi::OsStr;
    let paths = [Path::new(r"C:\Users\person\parts\plain design.sc"), Path::new(r"C:\Users\person\parts\person's design.sc")];
    for path in paths {
        for is_dir in [false, true] {
            let command = windows_recycle_command(path, is_dir).unwrap();
            assert_eq!(command.get_program(), OsStr::new("powershell"));
            let args: Vec<_> = command.get_args().map(OsStr::to_owned).collect();
            assert_eq!(args.len(), 4, "{args:?}");
            assert_eq!(args[0], "-NoProfile");
            assert_eq!(args[1], "-NonInteractive");
            assert_eq!(args[2], "-Command");
            assert_eq!(args[3], if is_dir { WINDOWS_RECYCLE_DIRECTORY } else { WINDOWS_RECYCLE_FILE }, "the instructions are the constant ones");
            let script = args[3].to_string_lossy();
            assert!(!script.contains(path.to_string_lossy().as_ref()), "the path must stay out of the script: {script}");
            assert!(script.contains(WINDOWS_RECYCLE_PATH), "the script reads the path by its constant name: {script}");
            assert!(script.contains("SendToRecycleBin"), "the Recycle Bin semantics stay: {script}");
            let environment: Vec<_> = command.get_envs().map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned))).collect();
            assert_eq!(environment.len(), 1, "{environment:?}");
            assert_eq!(environment[0].0, OsStr::new(WINDOWS_RECYCLE_PATH));
            assert_eq!(environment[0].1.as_deref(), Some(path.as_os_str()), "the exact path bytes are transferred");
        }
    }
    // Windows process environments cannot carry NUL: fail explicitly rather than let the
    // path cross that boundary or fall back to a permanent deletion.
    assert!(windows_recycle_command(Path::new("nul\0path.sc"), false).is_err());
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

// Transfer publication tests do not exercise rename's separate conflict-resolution path.
// Introduce the collision after the direct/KeepBoth name has been selected, before the OS call.
fn assert_rename_preserves_a_concurrent_destination(conflict: Conflict) {
    for directory in [false, true] {
        let scratch = Scratch::new();
        let source = if directory { scratch.dir("source.sc") } else { scratch.file("source.sc", "source content") };
        if directory {
            fs::write(source.join("child"), "source content").unwrap();
        }
        let requested = scratch.path().join("requested.sc");
        if conflict == Conflict::KeepBoth {
            fs::write(&requested, "existing destination").unwrap();
        }
        let result = with_fault(
            move |event, _, target| {
                if event == "rename publish" {
                    assert_eq!(target.file_name().unwrap(), if conflict == Conflict::KeepBoth { "requested (copy).sc" } else { "requested.sc" });
                    if directory {
                        // An empty directory is important: POSIX rename can replace it.
                        fs::create_dir(target)?;
                    } else {
                        fs::write(target, "concurrent destination")?;
                    }
                }
                Ok(())
            },
            || rename(&source, "requested.sc", conflict),
        );
        assert!(matches!(result.outcome, Outcome::Failed(_)), "{conflict:?} directory={directory}: {result:?}");
        assert_stored_name(scratch.path(), "source.sc");
        if directory {
            assert_eq!(fs::read(source.join("child")).unwrap(), b"source content");
            assert_eq!(fs::read_dir(&result.target).unwrap().count(), 0, "the concurrent directory must remain empty");
        } else {
            assert_eq!(fs::read(&source).unwrap(), b"source content");
            assert_eq!(fs::read(&result.target).unwrap(), b"concurrent destination");
        }
        if conflict == Conflict::KeepBoth {
            assert_eq!(fs::read(requested).unwrap(), b"existing destination");
        }
    }
}

#[test]
fn skip_renames_preserve_a_concurrent_destination() {
    assert_rename_preserves_a_concurrent_destination(Conflict::Skip);
}

#[test]
fn keep_both_renames_preserve_a_concurrent_destination() {
    assert_rename_preserves_a_concurrent_destination(Conflict::KeepBoth);
}

#[cfg(unix)]
#[test]
fn a_witness_names_the_same_plain_entry_when_it_is_unchanged() {
    let scratch = Scratch::new();
    let file = scratch.file("bracket.sc", "design");
    let first = entry_witness(&file).expect("a plain file has a witness");
    let second = entry_witness(&file).expect("the same plain file has a witness");
    assert!(same_object(&first, &second));
    assert!(same_entry(&first, &second));
    assert!(witness_matches(&first, &file));
}

#[cfg(unix)]
#[test]
fn a_recreated_entry_is_a_different_object_even_with_equal_bytes() {
    let scratch = Scratch::new();
    let file = scratch.file("bracket.sc", "same bytes");
    let before = entry_witness(&file).expect("the first file has a witness");
    fs::remove_file(&file).unwrap();
    scratch.file("bracket.sc", "same bytes");
    let after = entry_witness(&file).expect("the recreated file has a witness");
    assert!(!same_object(&before, &after), "equal bytes do not make the recreated file the same object");
    assert!(!same_entry(&before, &after));
    assert!(!witness_matches(&before, &file));
}

#[cfg(unix)]
#[test]
fn an_in_place_edit_keeps_the_object_but_not_the_consented_entry() {
    let scratch = Scratch::new();
    let file = scratch.file("bracket.sc", "design");
    let before = entry_witness(&file).expect("a plain file has a witness");
    fs::write(&file, "design plus an edit").unwrap();
    let after = entry_witness(&file).expect("the edited file has a witness");
    assert!(same_object(&before, &after), "the edit did not replace the object");
    assert!(!same_entry(&before, &after), "an edit is not the snapshot the person saw");
    assert!(!witness_matches(&before, &file));
}

#[cfg(unix)]
#[test]
fn a_symlink_directory_or_missing_path_has_no_witness() {
    let scratch = Scratch::new();
    let target = scratch.file("bracket.sc", "design");
    let link = scratch.path().join("alias.sc");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(entry_witness(&link).is_none(), "the link itself is not a plain file");
    assert!(entry_witness(&target).is_some());
    assert!(!witness_matches(&entry_witness(&target).unwrap(), &link));
    let directory = scratch.dir("parts");
    assert!(entry_witness(&directory).is_none());
    assert!(entry_witness(&scratch.path().join("absent.sc")).is_none());
}

// No approved safe adapter can prove, from the opened entry itself, that its volume is local
// NTFS. A real NTFS file has a nonzero volume serial and 64-bit file index, but neither of
// those, a drive letter, or a successful metadata query is that proof. This test is written
// for the native Windows run and is NOT_RUN on Linux.
#[cfg(windows)]
#[test]
fn windows_local_ntfs_has_no_replace_witness_without_a_vetted_volume_capability() {
    let scratch = Scratch::new();
    let file = scratch.file("bracket.sc", "design");
    assert!(entry_witness(&file).is_none(), "nonzero Windows identifiers alone must not grant destructive consent");
}
