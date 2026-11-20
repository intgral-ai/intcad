//! The Files panel: a local folder as a tree, the documents it opens, and the file operations
//! that act on it.
//!
//! One window browses one folder. The tree reads a folder the first time it is expanded, on a
//! worker thread, so a large or slow folder never stops the window; a second worker runs copies,
//! moves, renames and deletes one item at a time and answers one result per item. A polling
//! watcher asks the expanded folders for changes, so a program or an agent that writes to the
//! folder shows up here without a manual refresh.
//!
//! The panel is not a second editor: a design (`.sc`) opens through the application's own
//! document path, a repeated path focuses its tab, and every other file is handed to the
//! system's default program. Renaming or moving a design inside the tree — or a folder that
//! holds one — moves its tab's file path with it and keeps unsaved edits. Deleting works
//! through the system trash; when there is no usable trash, the person is asked again before a
//! permanent delete, and unsaved designs are only deleted after their work is saved as a copy
//! or explicitly discarded.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};

use egui::Modifiers;
use solidcraft_engine::io::files::{self, Conflict, Entry, ItemResult, Kind, Outcome};

use crate::SolveApp;
use crate::i18n::{format as trf, tr};

pub(crate) use panel::{show as panel, takes_key};

#[cfg(test)]
#[path = "file_tree_tests.rs"]
mod tests;

#[path = "file_tree_panel.rs"]
mod panel;

/// One folder's listing as the panel has it.
pub(crate) struct Dir {
    pub(crate) entries: Vec<Entry>,
    pub(crate) state: Listing,
}

/// What a folder's listing is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Listing {
    /// Being read; older entries (if any) stay on show meanwhile.
    Loading,
    Ready,
    Failed(String),
}

/// One visible row of the tree.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) kind: Kind,
    pub(crate) depth: usize,
    pub(crate) expanded: bool,
    pub(crate) state: RowState,
}

/// A row's own state: an entry, or the folder's listing standing in for its children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Entry,
    Loading,
    Failed(String),
}

/// An operation the panel asks the worker to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Copy,
    Move,
    Trash,
    Delete,
}

/// One item of an operation: a source, the folder it goes into (or the name it takes), and how
/// it treats an entry that is already there.
#[derive(Clone, Debug)]
pub(crate) struct OpItem {
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) conflict: Conflict,
}

/// The worker's answer for a batch.
pub(crate) struct OpAnswer {
    pub(crate) action: Action,
    pub(crate) results: Vec<ItemResult>,
    /// Items that could not be trashed because the system has no usable trash here. The panel
    /// asks before deleting these for good.
    pub(crate) no_trash: Vec<PathBuf>,
    pub(crate) cancelled: bool,
}

/// What an answer asks of the application outside the tree: designs whose file moved, tabs to
/// close because their file is gone, and a design to open.
#[derive(Default)]
pub(crate) struct Effect {
    pub(crate) retargets: Vec<(PathBuf, PathBuf)>,
    pub(crate) closed: Vec<String>,
    /// Open designs whose file was replaced: their tabs read the new file.
    pub(crate) reload: Vec<String>,
    pub(crate) open: Option<PathBuf>,
}

/// What the system file clipboard holds, when it holds files this panel can paste.
#[derive(Clone, Debug)]
pub(crate) struct Clip {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) cut: bool,
}

/// The system clipboard as a Files copy uses it.
///
/// Windows lets another program keep the clipboard: a new format is refused while its data owns
/// the clipboard, and formats already there stay beside a new one. So the writer may need to
/// empty the clipboard before writing the file list; that step is kept for when the first write
/// cannot leave a file list by itself, because emptying loses what was there.
#[cfg(not(target_arch = "wasm32"))]
trait SystemFileClipboard {
    fn set_files(&mut self, paths: &[PathBuf]) -> Result<(), arboard::Error>;
    fn empty(&mut self) -> Result<(), arboard::Error>;
    /// Whether text still shares the clipboard with our file list.
    fn text_present(&mut self) -> bool;
    /// Whether emptying the clipboard can let this refused write through.
    ///
    /// Windows refuses a new format while another program's data owns the clipboard; the other
    /// platforms replace the clipboard outright, so a refusal there is not about ownership.
    fn empty_can_help(&self, error: &arboard::Error) -> bool;
}

#[cfg(not(target_arch = "wasm32"))]
impl SystemFileClipboard for arboard::Clipboard {
    fn set_files(&mut self, paths: &[PathBuf]) -> Result<(), arboard::Error> {
        self.set().file_list(paths)
    }

    fn empty(&mut self) -> Result<(), arboard::Error> {
        self.clear()
    }

    fn text_present(&mut self) -> bool {
        // Only Windows keeps the previous owner's formats next to a new file list.
        #[cfg(target_os = "windows")]
        {
            self.get().text().is_ok()
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    fn empty_can_help(&self, error: &arboard::Error) -> bool {
        cfg!(target_os = "windows") && !matches!(error, arboard::Error::ConversionFailure)
    }
}

/// Give the system clipboard a file list of its own and nothing else.
///
/// The first write leaves another program's clipboard alone when it can: emptying first would
/// lose that clipboard if the write then failed.
#[cfg(not(target_arch = "wasm32"))]
fn write_file_list<C: SystemFileClipboard>(clipboard: &mut C, paths: &[PathBuf]) -> Result<(), String> {
    match clipboard.set_files(paths) {
        Ok(()) if !clipboard.text_present() => return Ok(()),
        Ok(()) => {}
        // No path could be converted, or this platform cannot take the clipboard back: emptying
        // would only lose the old clipboard without making a file list.
        Err(error) if !clipboard.empty_can_help(&error) => return Err(error.to_string()),
        Err(_) => {}
    }
    clipboard.empty().map_err(|error| error.to_string())?;
    clipboard.set_files(paths).map_err(|error| error.to_string())?;
    if clipboard.text_present() {
        return Err("a previous owner's text stayed beside the file list".to_string());
    }
    Ok(())
}

/// Hand the selection to the system file clipboard. Answers whether it holds it.
fn write_system_file_clipboard(paths: &[PathBuf]) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let written = arboard::Clipboard::new().map_err(|error| error.to_string()).and_then(|mut clipboard| write_file_list(&mut clipboard, paths));
        match written {
            Ok(()) => true,
            Err(error) => {
                log::warn!("files: the system clipboard did not take the selection: {error}");
                false
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = paths;
        true
    }
}

/// The inline name editor: a rename of an existing entry, or a new entry about to be made.
#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub(crate) parent: PathBuf,
    pub(crate) path: Option<PathBuf>,
    pub(crate) kind: EditKind,
    pub(crate) name: String,
    pub(crate) error: Option<String>,
    pub(crate) focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKind {
    Rename,
    File,
    Folder,
    Model,
}

/// The destination entry a conflict answer may replace. `identity` is read while the question
/// opens: an answer may not silently replace a different entry that appeared at that path later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReplaceConsent {
    source: PathBuf,
    identity: EntryIdentity,
}

/// What a file entry was, not just how its path was spelled. Unix gives reliable device and inode
/// identity. Other platforms cannot prove entry identity with only stable standard-library
/// metadata, so they stay [`EntryIdentity::Unknown`] rather than treating equal sizes or timestamps
/// as proof.
#[derive(Clone, Debug, PartialEq, Eq)]
enum EntryIdentity {
    #[cfg(unix)]
    Known(FileIdentity),
    #[cfg(any(not(unix), test))]
    Unknown,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
}

/// A question the panel keeps until the person answers it.
pub(crate) enum Modal {
    /// Pasting or dropping would replace entries that are already there. `replaceable` is the
    /// sources whose destination existed when the question opened: a name that appears later is
    /// not included, and a second source for the same destination is kept as a copy rather than
    /// replacing the first result.
    Conflict { sources: Vec<PathBuf>, destination: PathBuf, cut: bool, names: Vec<String>, files_only: bool, replaceable: Vec<ReplaceConsent> },
    /// Deleting entries, with the open designs that hold unsaved work among them.
    Delete { paths: Vec<PathBuf>, dirty: Vec<String> },
    /// The trash is not usable here; deleting for good needs another, explicit answer.
    Permanent { paths: Vec<PathBuf>, reason: String },
    /// Reloading an open design would throw away its unsaved edits.
    DiscardReload { path: String },
    /// Replacing a file that an open design with unsaved work owns. `replaceable` is carried
    /// through from the conflict answer so the same destinations stay the only replacements.
    ReplaceOpen { sources: Vec<PathBuf>, destination: PathBuf, cut: bool, dirty: Vec<String>, replaceable: Vec<ReplaceConsent> },
}

/// What a finished batch is reported as, under the tree until dismissed.
pub(crate) struct Report {
    pub(crate) action: Action,
    pub(crate) results: Vec<ItemResult>,
    pub(crate) cancelled: bool,
    /// The items that failed, ready to run again after the cause is fixed.
    pub(crate) retry: Vec<OpItem>,
}

impl Report {
    pub(crate) fn failed(&self) -> usize {
        self.results.iter().filter(|r| matches!(r.outcome, Outcome::Failed(_))).count()
    }
    pub(crate) fn skipped(&self) -> usize {
        self.results.iter().filter(|r| matches!(r.outcome, Outcome::Skipped)).count()
    }
    pub(crate) fn succeeded(&self) -> usize {
        self.results.iter().filter(|r| r.succeeded()).count()
    }
}

/// An open design whose file changed under it, or whose file is gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct External {
    pub(crate) path: String,
    pub(crate) missing: bool,
    pub(crate) dirty: bool,
}

/// The Files panel's whole state.
pub(crate) struct FileTree {
    pub(crate) root: Option<PathBuf>,
    pub(crate) dirs: BTreeMap<PathBuf, Dir>,
    pub(crate) expanded: BTreeSet<PathBuf>,
    pub(crate) selection: BTreeSet<PathBuf>,
    pub(crate) anchor: Option<PathBuf>,
    pub(crate) focus: Option<PathBuf>,
    pub(crate) show_hidden: bool,
    pub(crate) editing: Option<Edit>,
    pub(crate) clip: Option<Clip>,
    pub(crate) modal: Option<Modal>,
    pub(crate) report: Option<Report>,
    /// A line under the tree: what just happened, or why something could not be done.
    pub(crate) notice: Option<(String, bool)>,
    /// A file the notice is about, with the entry that shows it in the system file manager.
    pub(crate) notice_reveal: Option<PathBuf>,
    pub(crate) drag: Option<Vec<PathBuf>>,
    pub(crate) drop_target: Option<PathBuf>,
    pub(crate) scroll_to: Option<PathBuf>,
    pub(crate) busy: bool,
    pub(crate) external: Vec<External>,
    /// External changes the person chose to keep on show; they stay on disk and Save still
    /// asks before replacing them.
    dismissed: BTreeSet<String>,
    /// The quick-locate text, matched against the visible rows.
    pub(crate) find: Option<String>,
    /// Every open design's file and whether it has unsaved work, as of this frame: a copy of a
    /// dirty design is made from the file, and says so.
    open_docs: Vec<(String, bool)>,
    scan_jobs: Option<Sender<PathBuf>>,
    scan_results: Receiver<ScanAnswer>,
    scanning: BTreeSet<PathBuf>,
    op_jobs: Option<Sender<OpJob>>,
    op_results: Receiver<OpAnswer>,
    cancel: Arc<AtomicBool>,
    watch: Arc<Mutex<BTreeSet<PathBuf>>>,
    watched_changes: Receiver<PathBuf>,
    last_watch: f64,
    last_docs: f64,
    /// A root first taken from the design shown, so a later folder choice is not overwritten.
    adopted_root: bool,
    /// The pointer pressed on a row, to tell a click from a drag.
    pub(crate) press: Option<PathBuf>,
    /// The tree had the last click inside it, so its keys act until a click lands elsewhere.
    pub(crate) focused: bool,
    /// The panel was drawn this frame: a tree that is not on screen has no keys.
    pub(crate) drawn: bool,
    /// A V press or a Paste event came since the last V release (`begin_frame`).
    v_seen: bool,
    /// This frame released a V whose press never came: a paste with no text on the clipboard.
    pub(crate) paste_released: bool,
    /// The row menu the person opened with the secondary button.
    pub(crate) context: Option<ContextMenu>,
    /// The click that opened the row menu is not a click outside it.
    pub(crate) context_opening: bool,
    /// When the root was chosen, and how long its first listing took once it arrived.
    root_started: Option<std::time::Instant>,
    pub(crate) root_ms: Option<f64>,
    /// Openers still running: a file handed to the system answers here when it is done.
    open_jobs: Vec<std::sync::mpsc::Receiver<(PathBuf, Result<(), String>)>>,
    /// Moves the panel made that open designs must follow, drained after each action.
    retargets: Vec<(PathBuf, PathBuf)>,
    /// What each folder browsed in this session had open and selected, so switching back finds
    /// the tree as it was left.
    roots: BTreeMap<PathBuf, RootMemory>,
}

/// The row menu and the screen point it opened at.
#[derive(Clone)]
pub(crate) struct ContextMenu {
    pub(crate) path: PathBuf,
    pub(crate) at: egui::Pos2,
}

#[derive(Clone, Default)]
struct RootMemory {
    expanded: BTreeSet<PathBuf>,
    selection: BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
    focus: Option<PathBuf>,
}

struct ScanAnswer {
    dir: PathBuf,
    entries: Result<Vec<Entry>, String>,
}

struct OpJob {
    action: Action,
    items: Vec<OpItem>,
    cancel: Arc<AtomicBool>,
}

impl FileTree {
    pub(crate) fn new() -> Self {
        let (scan_jobs, jobs) = channel::<PathBuf>();
        let (answers, scan_results) = channel::<ScanAnswer>();
        let _scanner = std::thread::Builder::new().name("intcad-files-scan".into()).spawn(move || {
            while let Ok(dir) = jobs.recv() {
                let entries = files::read_dir(&dir);
                if answers.send(ScanAnswer { dir, entries }).is_err() {
                    return;
                }
            }
        });
        let (op_jobs, jobs) = channel::<OpJob>();
        let (answers, op_results) = channel::<OpAnswer>();
        let _worker = std::thread::Builder::new().name("intcad-files-ops".into()).spawn(move || {
            while let Ok(job) = jobs.recv() {
                let mut results = Vec::with_capacity(job.items.len());
                let mut no_trash = Vec::new();
                let mut cancelled = false;
                for item in &job.items {
                    if job.cancel.load(Ordering::Relaxed) {
                        cancelled = true;
                        break;
                    }
                    let result = match job.action {
                        Action::Copy => files::copy_into(&item.source, &item.destination, item.conflict),
                        Action::Move => files::move_into(&item.source, &item.destination, item.conflict),
                        Action::Trash => match files::trash(&item.source) {
                            Ok(()) => ItemResult { source: item.source.clone(), target: item.source.clone(), outcome: Outcome::Done },
                            Err(files::TrashProblem::Unavailable) => {
                                no_trash.push(item.source.clone());
                                continue;
                            }
                            Err(files::TrashProblem::Refused(e)) => {
                                ItemResult { source: item.source.clone(), target: item.source.clone(), outcome: Outcome::Failed(e) }
                            }
                        },
                        Action::Delete => match files::remove_permanent(&item.source) {
                            Ok(()) => ItemResult { source: item.source.clone(), target: item.source.clone(), outcome: Outcome::Done },
                            Err(e) => ItemResult { source: item.source.clone(), target: item.source.clone(), outcome: Outcome::Failed(e) },
                        },
                    };
                    results.push(result);
                }
                let answer = OpAnswer { action: job.action, results, no_trash, cancelled };
                if answers.send(answer).is_err() {
                    return;
                }
            }
        });
        let (changes, watched_changes) = channel::<PathBuf>();
        let watch = Arc::new(Mutex::new(BTreeSet::new()));
        let wanted = Arc::clone(&watch);
        let _watcher = std::thread::Builder::new().name("intcad-files-watch".into()).spawn(move || {
            // Remember each folder's last modified time. A folder the watch has just taken on
            // is recorded without an event: its listing is already being read.
            let mut seen: BTreeMap<PathBuf, Option<std::time::SystemTime>> = BTreeMap::new();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(700));
                let wanted = match wanted.lock() {
                    Ok(wanted) => wanted.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };
                seen.retain(|dir, _| wanted.contains(dir));
                for dir in wanted {
                    let stamp = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
                    match seen.get(&dir) {
                        None => {
                            seen.insert(dir, stamp);
                        }
                        Some(known) if *known != stamp => {
                            seen.insert(dir.clone(), stamp);
                            if changes.send(dir).is_err() {
                                return;
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
        });
        FileTree {
            root: None,
            dirs: BTreeMap::new(),
            expanded: BTreeSet::new(),
            selection: BTreeSet::new(),
            anchor: None,
            focus: None,
            show_hidden: false,
            editing: None,
            clip: None,
            modal: None,
            report: None,
            notice: None,
            notice_reveal: None,
            drag: None,
            drop_target: None,
            scroll_to: None,
            busy: false,
            external: Vec::new(),
            dismissed: BTreeSet::new(),
            find: None,
            open_docs: Vec::new(),
            scan_jobs: Some(scan_jobs),
            scan_results,
            scanning: BTreeSet::new(),
            op_jobs: Some(op_jobs),
            op_results,
            cancel: Arc::new(AtomicBool::new(false)),
            watch,
            watched_changes,
            last_watch: 0.0,
            last_docs: 0.0,
            adopted_root: false,
            press: None,
            focused: false,
            drawn: false,
            v_seen: false,
            paste_released: false,
            context: None,
            context_opening: false,
            root_started: None,
            root_ms: None,
            open_jobs: Vec::new(),
            retargets: Vec::new(),
            roots: BTreeMap::new(),
        }
    }

    /// The root folder, if one is chosen.
    pub(crate) fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Browse `root`: its listing is read now and every selection and edit is dropped. A closed
    /// (or missing) folder leaves the panel on its open-folder state.
    pub(crate) fn set_root(&mut self, root: Option<PathBuf>) {
        if let Some(previous) = self.root.take() {
            self.roots.insert(
                previous,
                RootMemory {
                    expanded: std::mem::take(&mut self.expanded),
                    selection: std::mem::take(&mut self.selection),
                    anchor: self.anchor.take(),
                    focus: self.focus.take(),
                },
            );
        }
        self.root = root;
        self.root_started = self.root.as_ref().map(|_| std::time::Instant::now());
        self.root_ms = None;
        self.dirs.clear();
        self.expanded.clear();
        self.selection.clear();
        self.anchor = None;
        self.focus = None;
        self.focused = false;
        self.context = None;
        self.editing = None;
        self.report = None;
        self.notice = None;
        self.external.clear();
        self.dismissed.clear();
        self.find = None;
        self.drag = None;
        self.drop_target = None;
        self.adopted_root = true;
        if let Some(root) = self.root.clone() {
            if let Some(memory) = self.roots.get(&root).cloned() {
                self.expanded = memory.expanded;
                self.selection = memory.selection;
                self.anchor = memory.anchor;
                self.focus = memory.focus;
            }
            self.expanded.insert(root.clone());
            let open: Vec<PathBuf> = self.expanded.iter().cloned().collect();
            for dir in open {
                self.ensure(&dir);
            }
        }
    }

    /// Take the shown design's folder as the root when no folder was chosen yet. Opening a
    /// design elsewhere does not change a root the person chose.
    pub(crate) fn adopt_document_root(&mut self, path: Option<&str>, home_open: bool) {
        if self.adopted_root || self.root.is_some() || home_open {
            return;
        }
        let Some(path) = path else { return };
        let Some(parent) = Path::new(path).parent().filter(|p| p.is_dir()) else { return };
        let parent = parent.to_path_buf();
        self.root = Some(parent.clone());
        self.expanded.insert(parent.clone());
        self.ensure(&parent);
    }

    /// Read `dir` if the panel does not have it yet (or after a refresh has cleared it).
    pub(crate) fn ensure(&mut self, dir: &Path) {
        let known = self.dirs.get(dir);
        if known.is_some_and(|d| d.state != Listing::Loading) || self.scanning.contains(dir) {
            return;
        }
        self.dirs.insert(dir.to_path_buf(), Dir { entries: Vec::new(), state: Listing::Loading });
        self.scanning.insert(dir.to_path_buf());
        if !self.queue(dir) {
            self.read_here(dir);
        }
    }

    /// Read `dir` again, keeping its rows on show until the answer comes.
    pub(crate) fn refresh(&mut self, dir: &Path) {
        self.scanning.remove(dir);
        if let Some(known) = self.dirs.get_mut(dir) {
            known.state = Listing::Loading;
        } else {
            self.dirs.insert(dir.to_path_buf(), Dir { entries: Vec::new(), state: Listing::Loading });
        }
        self.scanning.insert(dir.to_path_buf());
        if !self.queue(dir) {
            self.read_here(dir);
        }
    }

    fn queue(&self, dir: &Path) -> bool {
        self.scan_jobs.as_ref().is_some_and(|jobs| jobs.send(dir.to_path_buf()).is_ok())
    }

    /// The scan thread is not running (a very restricted host): read here, so a small folder
    /// still works, and say why a large one may pause the window.
    fn read_here(&mut self, dir: &Path) {
        self.scanning.remove(dir);
        match files::read_dir(dir) {
            Ok(entries) => {
                self.dirs.insert(dir.to_path_buf(), Dir { entries, state: Listing::Ready });
            }
            Err(e) => {
                self.dirs.insert(dir.to_path_buf(), Dir { entries: Vec::new(), state: Listing::Failed(e) });
            }
        }
    }

    /// Every visible row, depth first: the root's children and, under each open folder, its own.
    pub(crate) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            self.push_rows(root, 0, &mut rows);
        }
        rows
    }

    fn push_rows(&self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        let Some(listing) = self.dirs.get(dir) else { return };
        if listing.entries.is_empty() {
            match &listing.state {
                Listing::Ready => {}
                Listing::Loading => rows.push(placeholder(dir, depth, RowState::Loading)),
                Listing::Failed(e) => rows.push(placeholder(dir, depth, RowState::Failed(e.clone()))),
            }
            return;
        }
        for entry in &listing.entries {
            if !self.show_hidden && entry.name.starts_with('.') {
                continue;
            }
            let is_dir = entry.kind.is_dir();
            let open = is_dir && self.expanded.contains(&entry.path);
            rows.push(Row { path: entry.path.clone(), name: entry.name.clone(), kind: entry.kind, depth, expanded: open, state: RowState::Entry });
            if open {
                self.push_rows(&entry.path, depth + 1, rows);
            }
        }
    }

    /// Open or close a folder.
    pub(crate) fn toggle_dir(&mut self, path: &Path) {
        if self.expanded.remove(path) {
            return;
        }
        self.expanded.insert(path.to_path_buf());
        self.ensure(path);
    }

    /// One click on a row: plain replaces the selection, Ctrl/Cmd toggles it, Shift selects the
    /// run from the anchor.
    pub(crate) fn click(&mut self, path: &Path, modifiers: Modifiers, visible: &[PathBuf]) {
        if modifiers.shift {
            let Some(anchor) = self.anchor.clone() else {
                self.select_only(path);
                return;
            };
            let (from, to) = match (visible.iter().position(|p| p == &anchor), visible.iter().position(|p| p == path)) {
                (Some(a), Some(b)) => (a.min(b), a.max(b)),
                _ => {
                    self.select_only(path);
                    return;
                }
            };
            if !modifiers.command && !modifiers.ctrl {
                self.selection.clear();
            }
            for p in &visible[from..=to] {
                self.selection.insert(p.clone());
            }
            self.focus = Some(path.to_path_buf());
            return;
        }
        if modifiers.command || modifiers.ctrl {
            if !self.selection.remove(path) {
                self.selection.insert(path.to_path_buf());
            }
            self.focus = Some(path.to_path_buf());
            self.anchor = Some(path.to_path_buf());
            return;
        }
        self.select_only(path);
    }

    /// Select one row without changing what a Shift click would extend from.
    pub(crate) fn select_only(&mut self, path: &Path) {
        self.selection.clear();
        self.selection.insert(path.to_path_buf());
        self.focus = Some(path.to_path_buf());
        self.anchor = Some(path.to_path_buf());
    }

    /// The selected entries, with anything inside another selected folder dropped (a folder and
    /// its child are one item).
    pub(crate) fn selected(&self, focus: Option<&Path>) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> =
            if self.selection.is_empty() { focus.into_iter().map(Path::to_path_buf).collect() } else { self.selection.iter().cloned().collect() };
        paths.sort_by_key(|p| p.components().count());
        let mut kept: Vec<PathBuf> = Vec::new();
        for path in paths {
            if kept.iter().any(|parent| path != *parent && path.starts_with(parent)) {
                continue;
            }
            kept.push(path);
        }
        kept
    }

    /// What kind of entry `path` is, according to the listing that showed it. A symbolic link
    /// that points at a folder stays a link: it is never opened as a folder in the tree.
    pub(crate) fn kind_at(&self, path: &Path) -> Option<Kind> {
        for dir in self.dirs.values() {
            if let Some(entry) = dir.entries.iter().find(|entry| entry.path == path) {
                return Some(entry.kind);
            }
        }
        if path.is_dir() {
            Some(Kind::Dir)
        } else if path.exists() {
            Some(Kind::File)
        } else {
            None
        }
    }

    /// What a double click or Enter does here: a real folder opens, a file is handed to the
    /// caller (which knows how designs and other files open).
    pub(crate) fn activate(&mut self, path: &Path) -> Option<PathBuf> {
        if matches!(self.kind_at(path), Some(Kind::Dir)) {
            self.toggle_dir(path);
            return None;
        }
        Some(path.to_path_buf())
    }

    /// The folder a new entry, a paste or a drop in the blank area belongs in: the selected
    /// folder, the folder of the selected file, or the root.
    pub(crate) fn target_folder(&self) -> Option<PathBuf> {
        let focus = self.focus.as_deref();
        match focus {
            Some(path) if path.is_dir() => Some(path.to_path_buf()),
            Some(path) => path.parent().map(Path::to_path_buf),
            None => self.root.clone(),
        }
    }

    /// Run a batch on the worker thread. The panel stays interactive; only conflicting actions
    /// wait for the answer.
    fn submit(&mut self, action: Action, items: Vec<OpItem>) {
        if self.busy || items.is_empty() {
            return;
        }
        self.cancel.store(false, Ordering::Relaxed);
        let Some(jobs) = &self.op_jobs else {
            self.notice = Some((tr("ui.files_operation_unavailable").to_string(), true));
            return;
        };
        if jobs.send(OpJob { action, items, cancel: Arc::clone(&self.cancel) }).is_err() {
            self.notice = Some((tr("ui.files_operation_unavailable").to_string(), true));
            return;
        }
        self.busy = true;
        self.report = None;
    }

    /// Stop the batch after the item being worked on; the items not started stay where they are.
    pub(crate) fn cancel_operations(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The start of a frame. A tree that was not drawn has no keys. And a paste: a real
    /// Ctrl/Cmd+V never arrives as a key press, because the window layer turns it into a Paste
    /// event, and into nothing at all when the clipboard holds no text, as after a copy of files
    /// here or in a file manager. Only the V's release still arrives then, so a release whose
    /// press was never seen, with no Paste event since, is that paste.
    pub(crate) fn begin_frame(&mut self, raw: &egui::RawInput) {
        if !std::mem::take(&mut self.drawn) {
            self.focused = false;
        }
        self.paste_released = false;
        for event in &raw.events {
            match event {
                egui::Event::Key { key: egui::Key::V, pressed: true, .. } | egui::Event::Paste(_) => self.v_seen = true,
                egui::Event::Key { key: egui::Key::V, pressed: false, .. } => self.paste_released = !std::mem::take(&mut self.v_seen),
                _ => {}
            }
        }
    }

    /// Copy (or, cut, move) the selection into `destination`.
    pub(crate) fn copy_selection(&mut self, cut: bool) {
        let focus = self.focus.clone();
        let paths = self.selected(focus.as_deref());
        if paths.is_empty() {
            return;
        }
        let system_written = write_system_file_clipboard(&paths);
        self.record_copy(paths, cut, system_written);
    }

    /// Keep the panel's own copy and say whether the system clipboard got it too.
    fn record_copy(&mut self, paths: Vec<PathBuf>, cut: bool, system_written: bool) {
        self.clip = Some(Clip { paths, cut });
        // The panel's own clipboard still works; the person must know when the system one did not.
        self.notice = Some(if system_written {
            (tr(if cut { "ui.files_cut" } else { "ui.files_copied" }).to_string(), false)
        } else {
            (tr("ui.files_clipboard_unavailable").to_string(), true)
        });
    }

    /// Read the system clipboard's files when the panel's own clipboard is empty.
    fn system_files(&self) -> Vec<PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Some file managers end every URI with CRLF; arboard keeps the CR, which would make
            // the path a different (missing) file.
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.get().file_list())
                .unwrap_or_default()
                .into_iter()
                .map(|path| {
                    let text = path.to_string_lossy();
                    let trimmed = text.trim_end_matches(['\r', '\n']);
                    if trimmed.len() == text.len() { path } else { PathBuf::from(trimmed) }
                })
                .filter(|path| path.exists())
                .collect()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Vec::new()
        }
    }

    /// Paste the clipboard's entries into `destination`. A conflicting entry asks first; nothing
    /// is replaced silently.
    pub(crate) fn paste(&mut self, destination: &Path) {
        let (sources, cut) = match &self.clip {
            Some(clip) if !clip.paths.is_empty() => (clip.paths.clone(), clip.cut),
            _ => (self.system_files(), false),
        };
        if sources.is_empty() {
            self.notice = Some((tr("ui.files_nothing_to_paste").to_string(), false));
            return;
        }
        self.place(sources, destination, cut, None);
    }

    /// Copy or move `sources` into `destination`. With no `conflict` chosen, a destination that
    /// already holds one of the names opens the conflict question first.
    pub(crate) fn place(&mut self, sources: Vec<PathBuf>, destination: &Path, cut: bool, conflict: Option<Conflict>) {
        self.place_with(sources, destination, cut, conflict, &[]);
    }

    /// [`FileTree::place`], with the sources whose existing destination the person specifically
    /// agreed to replace and the entry identity seen then. An item outside `replaceable`, a
    /// destination entry that has changed since the question, or a second source for the same
    /// destination as an earlier item keeps both entries even when the batch answer is Replace.
    fn place_with(&mut self, sources: Vec<PathBuf>, destination: &Path, cut: bool, conflict: Option<Conflict>, replaceable: &[ReplaceConsent]) {
        let mut items = Vec::new();
        let mut conflicts = Vec::new();
        let mut files_only = true;
        let mut asked_replacements: Vec<ReplaceConsent> = Vec::new();
        let mut replacing: Vec<PathBuf> = Vec::new();
        for source in sources {
            let Some(name) = source.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
            let target = destination.join(&name);
            if target.exists() {
                conflicts.push(name.clone());
                let files = source.is_file() && target.is_file();
                files_only &= files;
                if files && let Some(identity) = entry_identity(&target) {
                    asked_replacements.push(ReplaceConsent { source: source.clone(), identity });
                }
            }
            let item_conflict = match conflict {
                Some(Conflict::Replace) => {
                    // This is the bounded orchestration check, made before submission. The worker
                    // still uses its existing publication primitives; no atomic revalidation or
                    // compare-and-swap is claimed between this check and the rename.
                    let unchanged = replaceable
                        .iter()
                        .find(|consent| same_source(&consent.source, &source))
                        .is_some_and(|consent| same_known_entry(&target, &consent.identity));
                    if unchanged && !replacing.iter().any(|earlier| same_destination(earlier, &target)) {
                        replacing.push(target.clone());
                        Conflict::Replace
                    } else {
                        Conflict::KeepBoth
                    }
                }
                Some(chosen) => chosen,
                None => Conflict::KeepBoth,
            };
            items.push(OpItem { source, destination: destination.to_path_buf(), conflict: item_conflict });
        }
        if items.is_empty() {
            return;
        }
        if !cut {
            let dirty_copy = items.iter().any(|item| {
                let source = item.source.to_string_lossy();
                self.open_docs.iter().any(|(path, dirty)| *dirty && crate::documents::same_path(path, &source))
            });
            if dirty_copy {
                self.notice = Some((tr("ui.files_copy_uses_saved").to_string(), false));
            }
        }
        let action = if cut { Action::Move } else { Action::Copy };
        match conflict {
            None if !conflicts.is_empty() => {
                let sources = items.iter().map(|i| i.source.clone()).collect();
                self.modal = Some(Modal::Conflict {
                    sources,
                    destination: destination.to_path_buf(),
                    cut,
                    names: conflicts,
                    files_only,
                    replaceable: asked_replacements,
                });
            }
            Some(_) | None => {
                self.submit(action, items);
                if cut {
                    self.clip = None;
                }
            }
        }
    }

    /// Start renaming `path`: the field is drawn under its folder, its name ready to change.
    pub(crate) fn start_rename(&mut self, path: &Path) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return };
        let Some(parent) = path.parent() else { return };
        self.editing = Some(Edit {
            parent: parent.to_path_buf(),
            path: Some(path.to_path_buf()),
            kind: EditKind::Rename,
            name: name.to_string(),
            error: None,
            focus: true,
        });
    }

    /// Start a new entry in `parent`.
    pub(crate) fn start_new(&mut self, parent: &Path, kind: EditKind) {
        // The field starts empty: the hint names what is being made, and the typed text is the
        // whole name rather than something to erase first.
        self.editing = Some(Edit { parent: parent.to_path_buf(), path: None, kind, name: String::new(), error: None, focus: true });
        self.expanded.insert(parent.to_path_buf());
        self.ensure(parent);
    }

    pub(crate) fn cancel_edit(&mut self) {
        self.editing = None;
    }

    /// Take the typed name. Answers the design to open when a blank `.sc` was made.
    pub(crate) fn commit_edit(&mut self) -> Option<PathBuf> {
        let mut edit = self.editing.take()?;
        let typed = edit.name.trim().to_string();
        if typed.is_empty() {
            edit.error = Some(tr("ui.files_name_needed").to_string());
            self.editing = Some(edit);
            return None;
        }
        let name = if edit.kind == EditKind::Model && !typed.to_ascii_lowercase().ends_with(".sc") { format!("{typed}.sc") } else { typed };
        match edit.kind {
            EditKind::Rename => {
                let path = edit.path.clone()?;
                match files::rename(&path, &name, Conflict::Skip).outcome {
                    Outcome::Done => {
                        let target = edit.parent.join(&name);
                        self.follow_move(&path, &target);
                        self.refresh(&edit.parent);
                        self.request_retarget(&path, &target);
                        if solidcraft_engine::io::is_design_path(&path.to_string_lossy()) || target.is_dir() {
                            self.notice = Some((tr("ui.files_reference_warning").to_string(), false));
                        }
                    }
                    Outcome::Skipped => {
                        edit.error = Some(tr("ui.files_name_taken").to_string());
                        self.editing = Some(edit);
                    }
                    Outcome::Failed(e) => {
                        edit.error = Some(e);
                        self.editing = Some(edit);
                    }
                }
                None
            }
            EditKind::File | EditKind::Folder | EditKind::Model => {
                let target = match files::child(&edit.parent, &name) {
                    Ok(target) => target,
                    Err(e) => {
                        edit.error = Some(e);
                        self.editing = Some(edit);
                        return None;
                    }
                };
                if target.exists() {
                    edit.error = Some(tr("ui.files_name_taken").to_string());
                    self.editing = Some(edit);
                    return None;
                }
                let made = match edit.kind {
                    EditKind::Folder => files::create_dir(&target),
                    EditKind::File => files::create_file(&target),
                    _ => create_blank_design(&target),
                };
                match made {
                    Ok(()) => {
                        if edit.kind == EditKind::Folder {
                            self.expanded.insert(target.clone());
                            self.ensure(&target);
                        }
                        self.refresh(&edit.parent);
                        self.select_only(&target);
                        self.scroll_to = Some(target.clone());
                        if edit.kind == EditKind::Model { Some(target) } else { None }
                    }
                    Err(e) => {
                        edit.error = Some(e);
                        self.editing = Some(edit);
                        None
                    }
                }
            }
        }
    }

    /// A rename the file system accepted is remembered here; the caller moves the open designs
    /// with [`FileTree::retargets`].
    fn request_retarget(&mut self, from: &Path, to: &Path) {
        self.retargets.push((from.to_path_buf(), to.to_path_buf()));
    }

    /// Paths that moved in the tree and whose open designs must follow. The panel drains them
    /// after each action.
    pub(crate) fn take_retargets(&mut self) -> Vec<(PathBuf, PathBuf)> {
        std::mem::take(&mut self.retargets)
    }

    /// Move the paths the panel keeps from `from` to `to`, including everything below a folder.
    fn follow_move(&mut self, from: &Path, to: &Path) {
        let moved = |path: &Path| -> Option<PathBuf> {
            if path == from {
                return Some(to.to_path_buf());
            }
            path.strip_prefix(from).ok().map(|rest| to.join(rest))
        };
        self.selection = self.selection.iter().filter_map(|p| moved(p)).collect();
        self.expanded = self.expanded.iter().filter_map(|p| moved(p)).collect();
        if let Some(path) = self.focus.take() {
            self.focus = moved(&path);
        }
        if let Some(path) = self.anchor.take() {
            self.anchor = moved(&path);
        }
        if let Some(path) = self.scroll_to.take() {
            self.scroll_to = moved(&path);
        }
        self.dirs = std::mem::take(&mut self.dirs).into_iter().filter_map(|(path, dir)| moved(&path).map(|p| (p, dir))).collect();
    }

    /// Ask before deleting the selection. `open` is every open design's file and whether it has
    /// unsaved work.
    pub(crate) fn request_delete(&mut self, open: &[(String, bool)]) {
        let focus = self.focus.clone();
        let paths = self.selected(focus.as_deref());
        if paths.is_empty() {
            return;
        }
        self.editing = None;
        let dirty: Vec<String> = open
            .iter()
            .filter(|(_, dirty)| *dirty)
            .filter(|(path, _)| paths.iter().any(|p| files::is_within(p, Path::new(path))))
            .map(|(path, _)| path.clone())
            .collect();
        self.modal = Some(Modal::Delete { paths, dirty });
    }

    /// Submit the delete after the person answered. Unsaved work must already have been saved as
    /// copies or explicitly discarded.
    pub(crate) fn start_trash(&mut self, paths: Vec<PathBuf>) {
        self.modal = None;
        let items = paths.iter().map(|path| OpItem { source: path.clone(), destination: path.clone(), conflict: Conflict::Skip }).collect();
        self.submit(Action::Trash, items);
    }

    /// Delete for good after the person has been told the trash is not usable here.
    pub(crate) fn delete_permanently(&mut self, paths: Vec<PathBuf>) {
        self.modal = None;
        let items = paths.iter().map(|path| OpItem { source: path.clone(), destination: path.clone(), conflict: Conflict::Skip }).collect();
        self.submit(Action::Delete, items);
    }

    /// Answer the conflict question. A Replace answer replaces only `replaceable`, the sources
    /// whose destination was named when the question opened and whose destination entry is still
    /// that same entry now; every other item keeps both entries. The policy for the rest of the
    /// batch (Keep Both or Skip) applies to every item as before.
    pub(crate) fn resolve_conflict(
        &mut self,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        cut: bool,
        conflict: Conflict,
        replaceable: &[ReplaceConsent],
    ) {
        if conflict == Conflict::Replace {
            let dirty: Vec<String> = sources
                .iter()
                .filter_map(|source| {
                    let target = destination.join(source.file_name()?);
                    let target = target.to_string_lossy();
                    self.open_docs.iter().find(|(path, dirty)| *dirty && crate::documents::same_path(path, &target)).map(|(path, _)| path.clone())
                })
                .collect();
            if !dirty.is_empty() {
                self.modal = Some(Modal::ReplaceOpen { sources, destination, cut, dirty, replaceable: replaceable.to_vec() });
                return;
            }
        }
        self.place_with(sources, &destination, cut, Some(conflict), replaceable);
    }

    /// Keep every open design's file and unsaved state, for the copy warning and the delete
    /// question.
    pub(crate) fn set_open_docs(&mut self, open: &[(String, bool, bool, bool)]) {
        self.open_docs = open.iter().map(|(path, dirty, _, _)| (path.clone(), *dirty)).collect();
    }

    /// Follow the open designs' files: an external rewrite or a file that is gone is shown and
    /// never silently replaces the in-memory design. `open` is (file, unsaved edits, changed on
    /// disk or gone) for every open design.
    pub(crate) fn poll_documents(&mut self, open: &[(String, bool, bool, bool)], now: f64) {
        if (now - self.last_docs).abs() < 1.5 {
            return;
        }
        self.last_docs = now;
        let mut found = Vec::new();
        for (path, dirty, changed, missing) in open {
            if self.dismissed.contains(path) || !(*changed || *missing) {
                continue;
            }
            found.push(External { path: path.clone(), missing: *missing, dirty: *dirty });
        }
        self.external = found;
    }

    /// Leave the external-change line on show until the next launch: the file stays as it is and
    /// Save still asks before replacing it.
    pub(crate) fn dismiss_external(&mut self, path: &str) {
        self.dismissed.insert(path.to_string());
        self.external.retain(|e| e.path != path);
    }

    /// Show `path` in the tree: open the folders above it, select it and scroll to it.
    pub(crate) fn reveal_path(&mut self, path: &Path) {
        let Some(root) = self.root.clone() else { return };
        if !path.starts_with(&root) {
            self.notice = Some((tr("ui.files_not_in_root").to_string(), true));
            return;
        }
        let mut ancestors: Vec<PathBuf> =
            path.parent().map(|parent| parent.ancestors().take_while(|a| a.starts_with(&root)).map(Path::to_path_buf).collect()).unwrap_or_default();
        ancestors.reverse();
        for dir in ancestors {
            if dir.is_dir() {
                self.expanded.insert(dir.clone());
                self.ensure(&dir);
            }
        }
        self.select_only(path);
        self.focused = true;
        self.scroll_to = Some(path.to_path_buf());
        if let Some(parent) = path.parent() {
            self.refresh(parent);
        }
    }

    /// The external-change line hides one design until the next launch.
    pub(crate) fn dismiss_path(&mut self, path: &str) {
        self.dismissed.insert(path.to_string());
    }

    /// Read what the workers answered: the folder listings, and the finished batches.
    pub(crate) fn take_scans(&mut self) {
        while let Ok(answer) = self.scan_results.try_recv() {
            self.scanning.remove(&answer.dir);
            let dir = match answer.entries {
                Ok(entries) => Dir { entries, state: Listing::Ready },
                Err(e) => Dir { entries: Vec::new(), state: Listing::Failed(e) },
            };
            if self.root.as_deref() == Some(answer.dir.as_path())
                && let Some(started) = self.root_started.take()
            {
                self.root_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            self.dirs.insert(answer.dir, dir);
        }
    }

    pub(crate) fn take_answers(&mut self) -> Vec<OpAnswer> {
        let mut answers = Vec::new();
        loop {
            match self.op_results.try_recv() {
                Ok(answer) => {
                    self.busy = false;
                    answers.push(answer);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.busy = false;
                    break;
                }
            }
        }
        answers
    }

    /// Fold one finished batch into the tree and say which designs and tabs must follow.
    pub(crate) fn apply_answer(&mut self, answer: OpAnswer) -> Effect {
        let mut effect = Effect::default();
        let mut touched = BTreeSet::new();
        for result in &answer.results {
            touched.extend(result.source.parent().map(Path::to_path_buf));
            match (answer.action, &result.outcome) {
                (Action::Copy, Outcome::Done) => {
                    touched.extend(result.target.parent().map(Path::to_path_buf));
                    self.selection.remove(&result.source);
                    self.selection.insert(result.target.clone());
                    self.focus = Some(result.target.clone());
                    let target = result.target.to_string_lossy();
                    if self.open_docs.iter().any(|(path, _)| crate::documents::same_path(path, &target)) {
                        effect.reload.push(target.into_owned());
                    }
                }
                (Action::Move, Outcome::Done) => {
                    touched.extend(result.target.parent().map(Path::to_path_buf));
                    effect.retargets.push((result.source.clone(), result.target.clone()));
                    self.follow_move(&result.source, &result.target);
                }
                (Action::Trash | Action::Delete, Outcome::Done) => {
                    effect.closed.push(result.source.to_string_lossy().into_owned());
                }
                _ => {}
            }
        }
        for dir in touched {
            if dir.is_dir() {
                self.refresh(&dir);
            }
        }
        if matches!(answer.action, Action::Copy | Action::Move)
            && answer.results.iter().any(|result| {
                result.succeeded() && (solidcraft_engine::io::is_design_path(&result.source.to_string_lossy()) || result.target.is_dir())
            })
        {
            self.notice = Some((tr("ui.files_reference_warning").to_string(), false));
        }
        if !effect.closed.is_empty() {
            self.selection.retain(|p| !effect.closed.iter().any(|d| files::is_within(Path::new(d), p)));
            if self.focus.as_ref().is_some_and(|f| effect.closed.iter().any(|d| files::is_within(Path::new(d), f))) {
                self.focus = None;
            }
        }
        if !answer.no_trash.is_empty() {
            let reason = tr(if cfg!(target_os = "macos") { "ui.files_trash_unavailable_macos" } else { "ui.files_trash_unavailable" }).to_string();
            self.modal = Some(Modal::Permanent { paths: answer.no_trash, reason });
        }
        let retry: Vec<OpItem> = answer
            .results
            .iter()
            .filter(|result| matches!(result.outcome, Outcome::Failed(_)))
            .map(|result| OpItem {
                source: result.source.clone(),
                destination: result.target.parent().map(Path::to_path_buf).unwrap_or_else(|| result.source.clone()),
                conflict: Conflict::KeepBoth,
            })
            .collect();
        self.report = Some(Report { action: answer.action, results: answer.results, cancelled: answer.cancelled, retry });
        effect
    }

    /// Run the failed items of the last batch again, with the conflicts checked afresh.
    pub(crate) fn retry_failed(&mut self) {
        let Some(report) = self.report.take() else { return };
        if report.retry.is_empty() {
            self.report = Some(report);
            return;
        }
        self.submit(report.action, report.retry);
    }

    /// Ask for frames while the tree is still reading or a batch is running.
    pub(crate) fn wants_frames(&self) -> bool {
        self.busy || !self.scanning.is_empty()
    }

    /// Folders to watch: the root and everything open.
    pub(crate) fn watched(&self) -> BTreeSet<PathBuf> {
        let mut wanted = self.expanded.clone();
        if let Some(root) = &self.root {
            wanted.insert(root.clone());
        }
        wanted
    }

    pub(crate) fn set_watched(&mut self, wanted: BTreeSet<PathBuf>) {
        if let Ok(mut watch) = self.watch.lock() {
            *watch = wanted;
        }
    }

    pub(crate) fn poll_watch(&mut self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        while let Ok(dir) = self.watched_changes.try_recv() {
            changed.push(dir);
        }
        changed
    }

    pub(crate) fn due_for_watch(&mut self, now: f64) -> bool {
        if (now - self.last_watch).abs() < 0.7 {
            return false;
        }
        self.last_watch = now;
        true
    }
}

/// Read what a file entry was. A symbolic link is not a plain file for this purpose, matching the
/// no-follow rule of the file operations. Only Unix has a stable, safe standard-library identity
/// here; another platform returns Unknown so no destructive consent is granted from metadata that
/// cannot prove the entry is the same one.
fn entry_identity(path: &Path) -> Option<EntryIdentity> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(EntryIdentity::Known(FileIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
        }))
    }
    #[cfg(not(unix))]
    {
        Some(EntryIdentity::Unknown)
    }
}

/// Are two known identities the same entry? `None` means the platform or path cannot prove either
/// side, which must behave as unknown rather than as same or different.
#[cfg(unix)]
fn known_relation(a: &EntryIdentity, b: &EntryIdentity) -> Option<bool> {
    match (a, b) {
        (EntryIdentity::Known(a), EntryIdentity::Known(b)) => Some(a == b),
        #[cfg(any(not(unix), test))]
        _ => None,
    }
}

#[cfg(not(unix))]
fn known_relation(_: &EntryIdentity, _: &EntryIdentity) -> Option<bool> {
    None
}

/// Does `path` still hold exactly the entry a saved consent names? Unknown never counts as same.
fn same_known_entry(path: &Path, expected: &EntryIdentity) -> bool {
    entry_identity(path).as_ref().and_then(|current| known_relation(current, expected)) == Some(true)
}

/// Is `b` the source the person allowed? Exact equality covers the normal answer. A different
/// spelling of the same existing entry is also the same source where a known identity proves it;
/// an unknown identity never grants that alias.
fn same_source(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (entry_identity(a), entry_identity(b)) {
        (Some(a), Some(b)) => known_relation(&a, &b) == Some(true),
        _ => false,
    }
}

/// Do two items of one batch write the same destination? Exact paths are equal. Known identities
/// decide same or distinct without relying on a native realpath spelling: a case alias remains one
/// destination, and distinct existing entries stay distinct. Unknown stays conservative only for
/// case-only spellings; differing names remain distinct.
fn same_destination(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if let (Some(a), Some(b)) = (entry_identity(a), entry_identity(b))
        && let Some(same) = known_relation(&a, &b)
    {
        return same;
    }
    let (Some(a_parent), Some(b_parent)) = (a.parent(), b.parent()) else { return false };
    if a_parent != b_parent {
        return false;
    }
    let (Some(a_name), Some(b_name)) = (a.file_name(), b.file_name()) else { return false };
    let (a, b) = (a_name.to_string_lossy(), b_name.to_string_lossy());
    a.to_lowercase() == b.to_lowercase()
}

fn placeholder(dir: &Path, depth: usize, state: RowState) -> Row {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
    Row { path: dir.to_path_buf(), name, kind: Kind::Dir, depth, expanded: false, state }
}

/// A blank design written at `path`, ready to open. It is written by the same save command an
/// ordinary Save uses, so it is a complete, loadable file.
fn create_blank_design(path: &Path) -> Result<(), String> {
    let mut session = solidcraft_engine::Session::default();
    session.execute("file.save_as", &serde_json::json!({"path": path.to_string_lossy(), "cache": false})).map(|_| ()).map_err(|e| e.to_string())
}

/// `ui.files`: the panel as a program sees it. `root` (a folder, or null to close it) browses a
/// folder, `show_hidden` toggles hidden entries, `refresh` reads the open folders again, `open`
/// expands folders, `locate` shows one path. Answers the visible rows, so a test or an agent can
/// act on what the panel shows without a screenshot.
pub(crate) fn control(app: &mut SolveApp, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::json;
    if let Some(root) = p.get("root") {
        let root = match root {
            serde_json::Value::Null => None,
            serde_json::Value::String(path) if path.is_empty() => None,
            serde_json::Value::String(path) => {
                let path = PathBuf::from(path);
                if !path.is_dir() {
                    return Err(format!("not a folder: {path}", path = path.display()));
                }
                Some(path)
            }
            _ => return Err("`root` must be a folder path or null".into()),
        };
        app.file_tree.set_root(root);
    }
    if let Some(hidden) = p.get("show_hidden").and_then(serde_json::Value::as_bool) {
        app.file_tree.show_hidden = hidden;
    }
    if p.get("refresh").and_then(serde_json::Value::as_bool) == Some(true)
        && let Some(root) = app.file_tree.root.clone()
    {
        let open: Vec<PathBuf> = app.file_tree.expanded.iter().cloned().collect();
        app.file_tree.refresh(&root);
        for dir in open {
            if dir != root {
                app.file_tree.refresh(&dir);
            }
        }
    }
    if let Some(open) = p.get("open").and_then(serde_json::Value::as_array) {
        for folder in open.iter().filter_map(serde_json::Value::as_str).map(PathBuf::from) {
            if folder.is_dir() {
                app.file_tree.expanded.insert(folder.clone());
                app.file_tree.ensure(&folder);
            }
        }
    }
    if let Some(path) = p.get("locate").and_then(serde_json::Value::as_str) {
        app.file_tree.reveal_path(Path::new(path));
    }
    let summary = p.get("summary").and_then(serde_json::Value::as_bool) == Some(true);
    let rows: Vec<serde_json::Value> = app
        .file_tree
        .rows()
        .iter()
        .map(|row| {
            json!({
                "name": row.name,
                "path": row.path.to_string_lossy(),
                "depth": row.depth,
                "folder": row.kind.is_dir(),
                "open": row.expanded,
                "state": match &row.state { RowState::Entry => "entry", RowState::Loading => "loading", RowState::Failed(_) => "failed" },
            })
        })
        .collect();
    Ok(json!({
        "root": app.file_tree.root.as_ref().map(|root| root.to_string_lossy()),
        "root_ms": app.file_tree.root_ms,
        "count": rows.iter().filter(|row| row.get("state").and_then(serde_json::Value::as_str) == Some("entry")).count(),
        "rows": if summary { serde_json::Value::Array(Vec::new()) } else { serde_json::Value::Array(rows) },
        "selection": app.file_tree.selection.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        "focused": app.file_tree.focused,
        "clipboard": app.file_tree.clip.as_ref().map(|clip| json!({
            "paths": clip.paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "cut": clip.cut,
        })),
        "notice": app.file_tree.notice.as_ref().map(|(text, error)| json!({"text": text, "error": error})),
        "busy": app.file_tree.busy,
        "report": app.file_tree.report.as_ref().map(|report| json!({
            "done": report.succeeded(),
            "failed": report.failed(),
            "skipped": report.skipped(),
            "failures": report.results.iter().filter_map(|result| match &result.outcome {
                Outcome::Failed(error) => Some(format!("{}: {error}", result.source.display())),
                _ => None,
            }).collect::<Vec<_>>(),
        })),
    }))
}

/// The panel's frame: read the workers, follow the folder and the documents, and keep drawing
/// while something is still coming.
pub(crate) fn tick(app: &mut SolveApp, ctx: &egui::Context, now: f64) {
    let (document, home_open) = (app.session.path.clone(), app.home.open);
    app.file_tree.adopt_document_root(document.as_deref(), home_open);
    app.file_tree.take_scans();
    let changed = app.file_tree.poll_watch();
    for dir in changed {
        let watched = app.file_tree.root.as_deref().is_some_and(|root| root == dir) || app.file_tree.expanded.contains(&dir);
        if watched {
            app.file_tree.refresh(&dir);
        }
    }
    if app.file_tree.due_for_watch(now) {
        let wanted = app.file_tree.watched();
        app.file_tree.set_watched(wanted);
    }
    let open: Vec<(String, bool, bool, bool)> = crate::documents::open_sessions(app)
        .into_iter()
        .filter_map(|session| {
            let path = session.path.clone()?;
            let missing = !Path::new(&path).exists();
            Some((path, session.is_dirty(), session.disk_changed(), missing))
        })
        .collect();
    app.file_tree.set_open_docs(&open);
    app.file_tree.poll_documents(&open, now);
    for answer in app.file_tree.take_answers() {
        let effect = app.file_tree.apply_answer(answer);
        for (from, to) in effect.retargets {
            crate::documents::retarget_paths(app, &from, &to);
        }
        if !effect.closed.is_empty() {
            crate::documents::close_paths(app, &effect.closed);
        }
        if let Some(path) = effect.open {
            open_path(app, &path);
        }
        for path in effect.reload {
            reload_now(app, &path);
        }
    }
    for (from, to) in app.file_tree.take_retargets() {
        crate::documents::retarget_paths(app, &from, &to);
    }
    if app.file_tree.wants_frames() {
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    } else if app.file_tree.root.is_some() {
        // The watcher runs on its own clock; a frame now and then lets its events reach the tree
        // even while the person is not touching the window.
        ctx.request_repaint_after(std::time::Duration::from_millis(700));
    }
    let mut finished = Vec::new();
    app.file_tree.open_jobs.retain_mut(|job| match job.try_recv() {
        Ok((path, answer)) => {
            finished.push((path, answer));
            false
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => true,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
    });
    for (path, answer) in finished {
        if let Err(error) = answer {
            let reason = if error == files::NO_ASSOCIATED_PROGRAM { tr("ui.files_no_associated_app") } else { &error };
            let message = trf("ui.files_open_failed", &[("name", &path.display().to_string()), ("reason", reason)]);
            app.file_tree.notice = Some((message.clone(), true));
            app.file_tree.notice_reveal = Some(path);
            app.set_status(message, true);
        }
    }
}

/// Open one file from the tree: a design in a tab (a path already open focuses its tab), any
/// other file with the system's default program.
pub(crate) fn open_path(app: &mut SolveApp, path: &Path) {
    let text = path.to_string_lossy();
    if solidcraft_engine::io::is_design_path(&text) {
        match crate::documents::reuse_path(app, &text) {
            Ok(true) => {}
            Ok(false) => crate::file_load::begin(app, &text, false),
            Err(e) => app.set_status(e, true),
        }
        return;
    }
    // The opener may stay attached to the program it starts, so it answers on a worker.
    let (sender, receiver) = std::sync::mpsc::channel();
    let file = path.to_path_buf();
    log::info!("files: opening {}", path.display());
    let opened = std::thread::Builder::new().name("intcad-files-open".into()).spawn(move || {
        let answer = files::open_default_waiting(&file);
        log::info!("files: open result for {}: {answer:?}", file.display());
        let _ = sender.send((file, answer));
    });
    match opened {
        Ok(_) => app.file_tree.open_jobs.push(receiver),
        Err(e) => {
            let message = trf("ui.files_open_failed", &[("name", &path.display().to_string()), ("reason", &e.to_string())]);
            app.file_tree.notice = Some((message.clone(), true));
            app.file_tree.notice_reveal = Some(path.to_path_buf());
            app.set_status(message, true);
        }
    }
}

/// Save every open design with unsaved work inside `paths` under a free name outside them, so a
/// delete can remove the originals without losing work. Each design's tab follows its copy.
/// Answers the copies made, or the failures that stopped the delete.
pub(crate) fn save_copies(app: &mut SolveApp, paths: &[PathBuf]) -> Result<usize, String> {
    let mut copies = Vec::new();
    for session in crate::documents::open_sessions(app) {
        if !session.is_dirty() {
            continue;
        }
        let Some(path) = session.path.clone() else { continue };
        if !paths.iter().any(|p| files::is_within(p, Path::new(&path))) {
            continue;
        }
        let original = PathBuf::from(&path);
        let Some(name) = original.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(mut folder) = original.parent().map(Path::to_path_buf) else { continue };
        while paths.iter().any(|p| files::is_within(p, &folder)) {
            match folder.parent() {
                Some(up) => folder = up.to_path_buf(),
                None => break,
            }
        }
        copies.push((path, folder, name.to_owned()));
    }
    for (from, folder, name) in &copies {
        let Some(session) = crate::documents::session_at_mut(app, from) else { continue };
        // The name is looked up now, after the copies before it were written: two tabs with the
        // same file name under one deleted subtree otherwise chose one free name before the
        // first save, so the second save replaced the first.
        let to = files::unique_path(folder, name);
        session
            .execute("file.save_as", &serde_json::json!({"path": to.to_string_lossy(), "cache": false}))
            .map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Ok(copies.len())
}

/// Reload a design from its file. A design with unsaved edits asks first (the panel's modal).
pub(crate) fn reload(app: &mut SolveApp, path: &str, dirty: bool) {
    if dirty {
        app.file_tree.modal = Some(Modal::DiscardReload { path: path.to_string() });
        return;
    }
    reload_now(app, path);
}

pub(crate) fn reload_now(app: &mut SolveApp, path: &str) {
    if let Some(index) = crate::documents::index_at(app, path) {
        crate::documents::close(app, index, true);
    }
    app.file_tree.dismiss_path(path);
    app.file_tree.external.retain(|e| e.path != path);
    if Path::new(path).exists() {
        crate::file_load::begin(app, path, false);
    }
}
