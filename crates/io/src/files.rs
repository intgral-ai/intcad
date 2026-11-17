//! A local folder as the Files panel shows it, and the file operations it runs.
//!
//! A directory is read on demand, folders first and each group in natural name order (so
//! `part2` comes before `part10`). The operations work on one item at a time and answer one
//! result per item, so a batch can report what succeeded, what was skipped and what failed
//! without pretending it was all one transaction. Copying never follows a symbolic link:
//! links are copied or removed as links, and a recursive copy refuses a destination inside its
//! own source. Moving is a rename, and across volumes the content is copied and checked before
//! the source is removed.
//!
//! Deleting goes to the system's trash first (`trash`), which follows the
//! [freedesktop.org trash specification] on Linux. A caller that finds no usable trash asks the
//! person again before deleting for good; nothing here falls back to erasing by itself.
//!
//! Opening a file with the system's default program and showing it in the system file manager
//! are the two ways out of the application; both are detached, so a slow program never waits
//! for the window.
//!
//! [freedesktop.org trash specification]: https://specifications.freedesktop.org/trash-spec/latest/

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How a directory entry is shown and what may be done with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A real directory.
    Dir,
    /// A real file (or a device, socket or other special entry: it is treated as a file).
    File,
    /// A symbolic link. `to_dir` says its target is a directory; the link is never followed for
    /// a recursive operation.
    Link { to_dir: bool },
}

impl Kind {
    /// Folders first in the tree, links next to what they point at.
    pub fn is_dir(self) -> bool {
        matches!(self, Kind::Dir | Kind::Link { to_dir: true })
    }
}

/// One row of a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name inside its folder, as the file system reports it.
    pub name: String,
    /// The absolute path this entry was read at (or the path the caller gave, joined).
    pub path: PathBuf,
    pub kind: Kind,
    /// The file's size in bytes; 0 for a directory.
    pub size: u64,
    /// When the entry last changed, when the file system reports it.
    pub modified: Option<SystemTime>,
}

impl Entry {
    /// Is this a design the application opens in a tab (`.sc`, any case)?
    pub fn is_design(&self) -> bool {
        matches!(self.kind, Kind::File) && self.path.extension().and_then(OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("sc"))
    }
}

/// Read one folder, folders first and then by natural name. Hidden entries (a leading dot) are
/// included; the panel decides whether to show them.
pub fn read_dir(path: &Path) -> Result<Vec<Entry>, String> {
    let reader = fs::read_dir(path).map_err(|e| describe("read", path, &e))?;
    let mut entries = Vec::new();
    for item in reader {
        let item = item.map_err(|e| describe("read", path, &e))?;
        let name = item.file_name().to_string_lossy().into_owned();
        let entry_path = item.path();
        let (kind, size, modified) = match fs::symlink_metadata(&entry_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    let to_dir = fs::metadata(&entry_path).map(|target| target.is_dir()).unwrap_or(false);
                    (Kind::Link { to_dir }, 0, metadata.modified().ok())
                } else if metadata.is_dir() {
                    (Kind::Dir, 0, metadata.modified().ok())
                } else {
                    (Kind::File, metadata.len(), metadata.modified().ok())
                }
            }
            // A link to a path that is gone, or a file removed between the listing and the
            // stat: show it as a file rather than dropping a row the person can see on disk.
            Err(_) => (Kind::File, 0, None),
        };
        entries.push(Entry { name, path: entry_path, kind, size, modified });
    }
    entries.sort_by(|a, b| a.kind.is_dir().cmp(&b.kind.is_dir()).reverse().then_with(|| natural_cmp(&a.name, &b.name)));
    Ok(entries)
}

/// Compare two names the way a file manager does: digit runs by their value, letters without
/// case, so `part 2` comes before `part 10`.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                if x.is_ascii_digit() && y.is_ascii_digit() {
                    let (na, nb) = (digits(&mut left), digits(&mut right));
                    let order = na.cmp(&nb);
                    if order != Ordering::Equal {
                        return order;
                    }
                } else {
                    let (lx, ly) = (x.to_lowercase().next(), y.to_lowercase().next());
                    let order = lx.cmp(&ly);
                    if order != Ordering::Equal {
                        return order;
                    }
                    left.next();
                    right.next();
                }
            }
        }
    }
}

/// The number under the digits `chars` starts with, saturating instead of overflowing.
fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> u128 {
    let mut value: u128 = 0;
    while let Some(c) = chars.peek().copied() {
        let Some(digit) = c.to_digit(10) else { break };
        value = value.saturating_mul(10).saturating_add(u128::from(digit));
        chars.next();
    }
    value
}

/// A name a person typed, as a name for one entry: no separators, not empty, not `.` or `..`.
pub fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a name is needed".into());
    }
    if name == "." || name == ".." || name.contains(['/', '\\']) || name.contains('\0') {
        return Err(format!("“{name}” cannot be a file or folder name"));
    }
    Ok(name)
}

/// A path in `dir` for a new entry called `name`.
pub fn child(dir: &Path, name: &str) -> Result<PathBuf, String> {
    Ok(dir.join(valid_name(name)?))
}

/// The first free name in `dir` for a copy of `name`: `bracket (copy).sc`, then
/// `bracket (copy 2).sc`, and so on. The extension stays at the end.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = first.as_path();
    let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or(name);
    let extension = path.extension().and_then(OsStr::to_str);
    let named = |suffix: &str| match extension {
        Some(extension) => format!("{stem}{suffix}.{extension}"),
        None => format!("{stem}{suffix}"),
    };
    let mut candidate = dir.join(named(" (copy)"));
    let mut number = 2u32;
    while candidate.exists() {
        candidate = dir.join(named(&format!(" (copy {number})")));
        number = number.saturating_add(1);
    }
    candidate
}

/// Create an empty file.
pub fn create_file(path: &Path) -> Result<(), String> {
    match fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => Ok(()),
        Err(e) => Err(describe("create", path, &e)),
    }
}

/// Create a folder (its parents must exist).
pub fn create_dir(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|e| describe("create", path, &e))
}

/// What to do when the destination already has an entry of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// Put the source in under the first free name.
    KeepBoth,
    /// Leave the destination as it is and report the item as skipped.
    Skip,
    /// Replace an existing file with the source file. Folders are never merged or replaced:
    /// a folder destination falls back to [`Conflict::KeepBoth`].
    Replace,
}

/// What became of one item of a batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The item is at its destination.
    Done,
    /// The item was left as it was (a conflict the person chose to skip, or a folder conflict).
    Skipped,
    /// Nothing was changed for this item; the reason says what the file system answered.
    Failed(String),
}

/// One item's result, which the panel lists when a batch does not finish cleanly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemResult {
    pub source: PathBuf,
    /// Where the item now is; for a skip or a failure, where it would have gone.
    pub target: PathBuf,
    pub outcome: Outcome,
}

impl ItemResult {
    fn done(source: &Path, target: &Path) -> Self {
        Self { source: source.to_path_buf(), target: target.to_path_buf(), outcome: Outcome::Done }
    }
    fn skipped(source: &Path, target: &Path) -> Self {
        Self { source: source.to_path_buf(), target: target.to_path_buf(), outcome: Outcome::Skipped }
    }
    fn failed(source: &Path, target: &Path, error: impl Into<String>) -> Self {
        Self { source: source.to_path_buf(), target: target.to_path_buf(), outcome: Outcome::Failed(error.into()) }
    }
    /// Did the item reach its destination?
    pub fn succeeded(&self) -> bool {
        matches!(self.outcome, Outcome::Done)
    }
}

/// Copy `source` into the folder `destination`, following `conflict`.
pub fn copy_into(source: &Path, destination: &Path, conflict: Conflict) -> ItemResult {
    let Some(name) = source.file_name() else {
        return ItemResult::failed(source, destination, "the item has no name");
    };
    let direct = destination.join(name);
    if let Err(e) = ensure_destination(source, destination) {
        return ItemResult::failed(source, &direct, e);
    }
    let target = match resolve(source, &direct, conflict) {
        Ok(Some(target)) => target,
        Ok(None) => return ItemResult::skipped(source, &direct),
        Err(e) => return ItemResult::failed(source, &direct, e),
    };
    if target == source {
        return ItemResult::failed(source, &direct, "the item is already there");
    }
    match copy_entry(source, &target) {
        Ok(()) => ItemResult::done(source, &target),
        Err(e) => ItemResult::failed(source, &target, e),
    }
}

/// Move `source` into the folder `destination`, following `conflict`. Across volumes the
/// content is copied first and the source removed only when the copy is complete.
pub fn move_into(source: &Path, destination: &Path, conflict: Conflict) -> ItemResult {
    let Some(name) = source.file_name() else {
        return ItemResult::failed(source, destination, "the item has no name");
    };
    let direct = destination.join(name);
    if let Err(e) = ensure_destination(source, destination) {
        return ItemResult::failed(source, &direct, e);
    }
    let target = match resolve(source, &direct, conflict) {
        Ok(Some(target)) => target,
        Ok(None) => return ItemResult::skipped(source, &direct),
        Err(e) => return ItemResult::failed(source, &direct, e),
    };
    if target == source {
        return ItemResult::failed(source, &direct, "the item is already there");
    }
    if target.exists() {
        // A replace of one file by another: the destination goes first (Windows will not rename
        // over it), and only a plain file may be replaced.
        if let Err(e) = remove_permanent(&target) {
            return ItemResult::failed(source, &target, e);
        }
    }
    match move_entry(source, &target) {
        Ok(()) => ItemResult::done(source, &target),
        Err(e) => ItemResult::failed(source, &target, e),
    }
}

/// Rename one entry inside its folder.
pub fn rename(source: &Path, new_name: &str, conflict: Conflict) -> ItemResult {
    let Some(parent) = source.parent() else {
        return ItemResult::failed(source, source, "the item has no folder");
    };
    let mut target = match child(parent, new_name) {
        Ok(target) => target,
        Err(e) => return ItemResult::failed(source, source, e),
    };
    if target == source {
        return ItemResult::done(source, &target);
    }
    if target.exists() {
        match conflict {
            Conflict::Skip => return ItemResult::skipped(source, &target),
            Conflict::KeepBoth => target = unique_path(parent, new_name),
            Conflict::Replace => {
                if !is_plain_file(source) || !is_plain_file(&target) {
                    return ItemResult::failed(source, &target, "a folder cannot replace another entry");
                }
                if let Err(e) = remove_permanent(&target) {
                    return ItemResult::failed(source, &target, e);
                }
            }
        }
    }
    match fs::rename(source, &target) {
        Ok(()) => ItemResult::done(source, &target),
        Err(e) => ItemResult::failed(source, &target, describe("rename", source, &e)),
    }
}

/// Where `source` goes when the destination already holds `direct`; `None` means skip.
fn resolve(source: &Path, direct: &Path, conflict: Conflict) -> Result<Option<PathBuf>, String> {
    if !direct.exists() {
        return Ok(Some(direct.to_path_buf()));
    }
    match conflict {
        Conflict::Skip => Ok(None),
        Conflict::Replace if is_plain_file(source) && is_plain_file(direct) => Ok(Some(direct.to_path_buf())),
        // A folder is never merged into another folder and a file never replaces a folder.
        Conflict::Replace | Conflict::KeepBoth => {
            let Some(parent) = direct.parent() else { return Err("the destination has no folder".into()) };
            let Some(name) = direct.file_name().and_then(OsStr::to_str) else {
                return Err("the destination name is not readable".into());
            };
            Ok(Some(unique_path(parent, name)))
        }
    }
}

fn ensure_destination(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::metadata(destination).map_err(|e| describe("open", destination, &e))?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a folder", destination.display()));
    }
    if is_within(source, destination) {
        return Err("a folder cannot be copied or moved into itself".into());
    }
    Ok(())
}

/// Is `child` the same as `parent` or inside it? Both paths must exist for the canonical
/// answer; a missing path falls back to the written form.
pub fn is_within(parent: &Path, child: &Path) -> bool {
    let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let child = fs::canonicalize(child).unwrap_or_else(|_| child.to_path_buf());
    child.starts_with(&parent)
}

/// Copy one entry to `target`, recursing into a real folder.
fn copy_entry(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|e| describe("read", source, &e))?;
    if metadata.file_type().is_symlink() {
        return copy_link(source, target);
    }
    if metadata.is_dir() {
        return copy_dir(source, target, 0);
    }
    fs::copy(source, target).map(|_| ()).map_err(|e| describe("copy", source, &e))
}

fn copy_dir(source: &Path, target: &Path, depth: usize) -> Result<(), String> {
    // A symbolic loop cannot be followed (links are not recursed), but a hostile deep tree
    // still must not exhaust the window's stack.
    if depth > 256 {
        return Err("the folder is nested too deeply to copy".into());
    }
    fs::create_dir(target).map_err(|e| describe("create", target, &e))?;
    let reader = fs::read_dir(source).map_err(|e| describe("read", source, &e))?;
    for item in reader {
        let item = item.map_err(|e| describe("read", source, &e))?;
        let from = item.path();
        let to = target.join(item.file_name());
        let metadata = fs::symlink_metadata(&from).map_err(|e| describe("read", &from, &e))?;
        if metadata.file_type().is_symlink() {
            copy_link(&from, &to)?;
        } else if metadata.is_dir() {
            copy_dir(&from, &to, depth + 1)?;
        } else {
            fs::copy(&from, &to).map_err(|e| describe("copy", &from, &e))?;
        }
    }
    // Keep the folder's own time where the platform allows it (best effort; a copy that worked
    // is not a failure because the timestamp could not be set).
    if let Ok(modified) = fs::metadata(source).and_then(|m| m.modified()) {
        let times = fs::FileTimes::new().set_modified(modified);
        if let Ok(file) = fs::File::open(target) {
            let _ = file.set_times(times);
        }
    }
    Ok(())
}

/// Copy a symbolic link as a link, never as its target. A platform that cannot make the link
/// reports the failure; it is not silently replaced by a copy of the target's content.
fn copy_link(source: &Path, target: &Path) -> Result<(), String> {
    let to = fs::read_link(source).map_err(|e| describe("read link", source, &e))?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&to, target).map_err(|e| describe("copy link", source, &e))
    }
    #[cfg(windows)]
    {
        let is_dir = fs::metadata(source).map(|m| m.is_dir()).unwrap_or(false);
        let made = if is_dir { std::os::windows::fs::symlink_dir(&to, target) } else { std::os::windows::fs::symlink_file(&to, target) };
        made.map_err(|e| describe("copy link", source, &e))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (source, target, to);
        Err("this platform cannot copy a symbolic link".into())
    }
}

/// Move one entry, copying across volumes and removing the source only after the copy is in.
fn move_entry(source: &Path, target: &Path) -> Result<(), String> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            copy_entry(source, target)?;
            remove_permanent(source).map_err(|e| format!("copied to {}, but the source could not be removed: {e}", target.display()))
        }
        Err(e) => Err(describe("move", source, &e)),
    }
}

fn is_cross_device(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(18) // EXDEV
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(17) // ERROR_NOT_SAME_DEVICE
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        false
    }
}

fn is_plain_file(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

/// Why a delete needs the person's answer before it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashProblem {
    /// The file system or platform has no trash this application can use.
    Unavailable,
    /// There is a trash, but this item could not be put in it.
    Refused(String),
}

/// Put one entry in the system's trash.
pub fn trash(path: &Path) -> Result<(), TrashProblem> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        if home_trash_available() {
            return move_to_home_trash(path).map_err(TrashProblem::Refused);
        }
        if let Some(volume) = volume_trash(path) {
            return move_to_volume_trash(path, &volume).map_err(TrashProblem::Refused);
        }
        Err(TrashProblem::Unavailable)
    }
    #[cfg(all(windows, not(target_arch = "wasm32")))]
    {
        let script = format!(
            "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile('{}', 'OnlyErrorDialogs', 'SendToRecycleBin')",
            path.display().to_string().replace('\'', "''")
        );
        let script = if fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
            format!(
                "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory('{}', 'OnlyErrorDialogs', 'SendToRecycleBin')",
                path.display().to_string().replace('\'', "''")
            )
        } else {
            script
        };
        match std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output() {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(TrashProblem::Refused(describe_text(&String::from_utf8_lossy(&output.stderr)))),
            Err(e) => Err(TrashProblem::Refused(describe_text(&e.to_string()))),
        }
    }
    #[cfg(all(target_vendor = "apple", not(target_arch = "wasm32")))]
    {
        match std::process::Command::new("osascript")
            .args(["-e", &format!("tell application \"Finder\" to delete POSIX file \"{}\"", path.display())])
            .output()
        {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(TrashProblem::Refused(describe_text(&String::from_utf8_lossy(&output.stderr)))),
            Err(e) => Err(TrashProblem::Refused(describe_text(&e.to_string()))),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err(TrashProblem::Unavailable)
    }
}

/// Delete one entry for good. A symbolic link is removed as a link: its target is untouched.
pub fn remove_permanent(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| describe("delete", path, &e))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|e| describe("delete", path, &e))
    } else {
        fs::remove_file(path).map_err(|e| describe("delete", path, &e))
    }
}

/// Is a home trash usable for `path`? (Linux and other freedesktop systems.)
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn home_trash_available() -> bool {
    match home_trash() {
        Some((files, info)) => ensure_trash_dirs(&files, &info),
        None => false,
    }
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn home_trash() -> Option<(PathBuf, PathBuf)> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
    let trash = data.join("Trash");
    Some((trash.join("files"), trash.join("info")))
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn ensure_trash_dirs(files: &Path, info: &Path) -> bool {
    let made = fs::create_dir_all(files).and_then(|_| fs::create_dir_all(info));
    made.is_ok() && fs::metadata(files).map(|m| m.is_dir()).unwrap_or(false)
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn move_to_home_trash(path: &Path) -> Result<(), String> {
    let Some((files, info)) = home_trash() else { return Err("there is no home folder".into()) };
    move_into_trash(&files, &info, path)
}

/// A trash on the same volume, for a file the home trash should not hold (another disk).
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn volume_trash(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let mount = mount_point(path)?;
    let uid = current_uid()?;
    for candidate in [mount.join(".Trash").join(uid.to_string()), mount.join(format!(".Trash-{uid}"))] {
        let (files, info) = (candidate.join("files"), candidate.join("info"));
        if ensure_trash_dirs(&files, &info) {
            return Some((files, info));
        }
    }
    None
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn move_to_volume_trash(path: &Path, trash: &(PathBuf, PathBuf)) -> Result<(), String> {
    move_into_trash(&trash.0, &trash.1, path)
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn move_into_trash(files: &Path, info: &Path, original: &Path) -> Result<(), String> {
    let name = original.file_name().and_then(OsStr::to_str).unwrap_or("item");
    let mut target = files.join(name);
    let mut number = 2u32;
    while target.exists() {
        target = files.join(format!("{name}.{number}"));
        number = number.saturating_add(1);
    }
    let target_name = target.file_name().and_then(OsStr::to_str).unwrap_or(name);
    let info_file = info.join(format!("{target_name}.trashinfo"));
    let when = utc_now();
    let body = format!("[Trash Info]\nPath={}\nDeletionDate={when}\n", percent_encode(&original.to_string_lossy()));
    fs::write(&info_file, body).map_err(|e| describe("write", &info_file, &e))?;
    if let Err(e) = fs::rename(original, &target) {
        let _ = fs::remove_file(&info_file);
        if is_cross_device(&e) {
            copy_entry(original, &target)?;
            remove_permanent(original)?;
            return Ok(());
        }
        return Err(describe("move to the trash", original, &e));
    }
    Ok(())
}

/// The mount point a path is on: the longest prefix of its ancestors with the same device id.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn mount_point(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let mut current = path.canonicalize().ok()?;
    while !current.is_dir() {
        if !current.pop() {
            return None;
        }
    }
    let device = fs::metadata(&current).ok()?.dev();
    while let Some(parent) = current.parent() {
        match fs::metadata(parent) {
            Ok(metadata) if metadata.dev() == device => current = parent.to_path_buf(),
            _ => break,
        }
    }
    Some(current)
}

/// The user id that names this user's trash on a volume, read from the home folder's owner
/// (no extra dependency, no system call beyond `stat`).
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let home = std::env::var_os("HOME")?;
    fs::metadata(home).ok().map(|metadata| metadata.uid())
}

/// `aaaa-mm-ddThh:mm:ss` in UTC, for a `.trashinfo` file, without a date-time dependency.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn utc_now() -> String {
    let seconds = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = seconds / 86_400;
    let time = seconds % 86_400;
    let (year, month, day) = civil_from_days(i64::try_from(days).unwrap_or(0));
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}", time / 3_600, (time % 3_600) / 60, time % 60)
}

/// Days since the epoch as a calendar date (Howard Hinnant's algorithm, public domain).
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    year += i64::from(month <= 2);
    (year, month, day)
}

/// Percent-encode a path for a `.trashinfo` file (UTF-8, with `/` kept, as the spec says).
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn percent_encode(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => encoded.push(char::from(byte)),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Open a file (or folder) with the system's default program, waiting for the opener to answer.
/// Some systems start a program that stays attached for its lifetime; call this on a worker so
/// the window keeps drawing. An answer here is the opener's: a missing association is an error.
pub fn open_default_waiting(path: &Path) -> Result<(), String> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        if !has_association(path) {
            return Err("no program is associated with this file".into());
        }
        wait_for("xdg-open", &[path.as_os_str()])
    }
    #[cfg(all(windows, not(target_arch = "wasm32")))]
    {
        wait_for("cmd", &[OsStr::new("/C"), OsStr::new("start"), OsStr::new(""), path.as_os_str()])
    }
    #[cfg(all(target_vendor = "apple", not(target_arch = "wasm32")))]
    {
        wait_for("open", &[path.as_os_str()])
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err("this build cannot open files with the system".into())
    }
}

/// Open a file (or folder) with the system's default program.
pub fn open_default(path: &Path) -> Result<(), String> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        spawn_detached("xdg-open", &[path.as_os_str()])
    }
    #[cfg(all(windows, not(target_arch = "wasm32")))]
    {
        spawn_detached("cmd", &[OsStr::new("/C"), OsStr::new("start"), OsStr::new(""), path.as_os_str()])
    }
    #[cfg(all(target_vendor = "apple", not(target_arch = "wasm32")))]
    {
        spawn_detached("open", &[path.as_os_str()])
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err("this build cannot open files with the system".into())
    }
}

/// Show an entry in the system's file manager, with the entry selected where the platform can.
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        // The freedesktop file manager interface selects the entry; a plain folder open is the
        // fallback when no file manager answers.
        let uri = format!("file://{}", path.to_string_lossy());
        let shown = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if shown {
            return Ok(());
        }
        let parent = path.parent().unwrap_or(path);
        spawn_detached("xdg-open", &[parent.as_os_str()])
    }
    #[cfg(all(windows, not(target_arch = "wasm32")))]
    {
        spawn_detached("explorer", &[format!("/select,{}", path.display()).as_ref()])
    }
    #[cfg(all(target_vendor = "apple", not(target_arch = "wasm32")))]
    {
        spawn_detached("open", &[OsStr::new("-R"), path.as_os_str()])
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err("this build has no system file manager".into())
    }
}

/// Start a program without waiting for it; a program that is missing is reported, not hidden.
#[cfg(not(target_arch = "wasm32"))]
/// Does the system name a program for this file's type? A desktop without `xdg-mime` is given
/// the benefit of the doubt: `xdg-open` answers for itself then.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn has_association(path: &Path) -> bool {
    let query = |args: &[&str]| -> Option<String> {
        let output = std::process::Command::new("xdg-mime").args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let Some(mime) = query(&["query", "filetype", &path.to_string_lossy()]) else { return true };
    if mime.is_empty() {
        return false;
    }
    query(&["query", "default", &mime]).is_none_or(|default| !default.is_empty())
}

#[cfg(not(target_arch = "wasm32"))]
fn wait_for(program: &str, args: &[&OsStr]) -> Result<(), String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if reason.is_empty() { format!("{program}: no program is associated with this file") } else { reason })
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_detached(program: &str, args: &[&OsStr]) -> Result<(), String> {
    let mut command = std::process::Command::new(program);
    command.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    command.spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"))
}

fn describe(what: &str, path: &Path, error: &io::Error) -> String {
    format!("{what} {}: {error}", path.display())
}

#[cfg(any(windows, target_vendor = "apple"))]
fn describe_text(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() { "the system refused the operation".into() } else { text.to_string() }
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
