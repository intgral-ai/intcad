//! Launch-owned startup bookkeeping.
//!
//! One versioned ledger plus one lifetime-locked entry per launch replaces the
//! shared numeric marker for new code. The legacy marker is imported once and
//! left byte-unchanged. This module is std-only and performs no CEF call; it is
//! the ownership/history half of controlled startup cancellation.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Same bound the numeric marker used.
pub const HISTORY_CAP: u32 = 1000;
const LEDGER: &str = "startup-ledger";
const COORDINATOR: &str = "startup-ledger.lock";
const ENTRY_PREFIX: &str = "startup-entry-";
const LEGACY_MARKER: &str = "starting";
const MAX_ENTRIES: usize = 4096;
const MAX_RECORD: u64 = 4096;
const MAX_ID_ATTEMPTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupError {
    /// Another launch holds the coordinator or an entry lock.
    Busy,
    Io,
    /// A ledger or entry did not parse; history is never invented.
    InvalidRecord,
    /// This ticket's own entry could not be claimed/bound.
    Ownership,
    HistoryCapacity,
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "another launch is updating startup history",
            Self::Io => "startup history could not be read or written",
            Self::InvalidRecord => "startup history is damaged",
            Self::Ownership => "this launch could not claim its startup entry",
            Self::HistoryCapacity => "startup history is at capacity",
        })
    }
}

fn io() -> StartupError {
    StartupError::Io
}

fn map_lock(error: std::fs::TryLockError) -> StartupError {
    match error {
        std::fs::TryLockError::WouldBlock => StartupError::Busy,
        std::fs::TryLockError::Error(_) => StartupError::Io,
    }
}

struct Coordinator {
    _file: File,
}

fn coordinator(dir: &Path) -> Result<Coordinator, StartupError> {
    let path = dir.join(COORDINATOR);
    let file = OpenOptions::new().create(true).read(true).write(true).open(&path).map_err(|_| io())?;
    file.try_lock().map_err(map_lock)?;
    Ok(Coordinator { _file: file })
}

fn legacy_count(dir: &Path) -> u32 {
    std::fs::read_to_string(dir.join(LEGACY_MARKER))
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(0)
        .min(HISTORY_CAP)
}

fn read_ledger(dir: &Path) -> Result<Option<u32>, StartupError> {
    let text = match std::fs::read_to_string(dir.join(LEDGER)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(io()),
    };
    let mut version = None;
    let mut baseline = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("version=") {
            version = value.parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix("baseline=") {
            baseline = value.parse::<u32>().ok();
        } else if line.is_empty() || line.starts_with("imported=") {
            continue;
        } else {
            return Err(StartupError::InvalidRecord);
        }
    }
    match (version, baseline) {
        (Some(1), Some(baseline)) if baseline <= HISTORY_CAP => Ok(Some(baseline)),
        _ => Err(StartupError::InvalidRecord),
    }
}

fn write_ledger(dir: &Path, baseline: u32) -> Result<(), StartupError> {
    let body = format!("version=1\nbaseline={}\nimported=1\n", baseline.min(HISTORY_CAP));
    let tmp = dir.join("startup-ledger.tmp");
    {
        let mut file = File::create(&tmp).map_err(|_| io())?;
        file.write_all(body.as_bytes()).map_err(|_| io())?;
        file.sync_all().map_err(|_| io())?;
    }
    std::fs::rename(&tmp, dir.join(LEDGER)).map_err(|_| io())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryState {
    Unresolved,
    Started,
    Cancelled,
    NoWindow,
}

impl EntryState {
    fn terminal(self) -> bool {
        !matches!(self, Self::Unresolved)
    }

    fn text(self) -> &'static str {
        match self {
            Self::Unresolved => "unresolved",
            Self::Started => "started",
            Self::Cancelled => "cancelled",
            Self::NoWindow => "no_window",
        }
    }
}

fn read_entry(file: &mut File) -> Result<EntryState, StartupError> {
    file.seek(SeekFrom::Start(0)).map_err(|_| io())?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD + 1).read_to_end(&mut bytes).map_err(|_| io())?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(StartupError::InvalidRecord);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| StartupError::InvalidRecord)?;
    let mut version = None;
    let mut state = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("version=") {
            version = value.parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix("state=") {
            state = Some(match value {
                "unresolved" => EntryState::Unresolved,
                "started" => EntryState::Started,
                "cancelled" => EntryState::Cancelled,
                "no_window" => EntryState::NoWindow,
                _ => return Err(StartupError::InvalidRecord),
            });
        } else if line.is_empty() || line.starts_with("launch=") {
            continue;
        } else {
            return Err(StartupError::InvalidRecord);
        }
    }
    match (version, state) {
        (Some(1), Some(state)) => Ok(state),
        _ => Err(StartupError::InvalidRecord),
    }
}

fn write_entry(file: &mut File, id: &str, state: EntryState) -> Result<(), StartupError> {
    let body = format!("version=1\nlaunch={id}\nstate={}\n", state.text());
    file.set_len(0).and_then(|()| file.seek(SeekFrom::Start(0))).map_err(|_| io())?;
    file.write_all(body.as_bytes()).map_err(|_| io())?;
    file.sync_all().map_err(|_| io())
}

#[derive(Debug)]
enum Seen {
    Active,
    Dead(PathBuf),
    Terminal,
}

fn classify(path: &Path) -> Result<Seen, StartupError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path).map_err(|_| io())?;
    let state = read_entry(&mut file)?;
    if state.terminal() {
        return Ok(Seen::Terminal);
    }
    match file.try_lock() {
        Ok(()) => Ok(Seen::Dead(path.to_path_buf())),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Seen::Active),
        Err(std::fs::TryLockError::Error(_)) => Err(io()),
    }
}

fn entry_paths(dir: &Path) -> Result<Vec<PathBuf>, StartupError> {
    let mut paths = Vec::new();
    let Some(entries) = std::fs::read_dir(dir).ok() else {
        // A missing directory is an empty history here; callers create it before use.
        return Ok(paths);
    };
    for entry in entries {
        let path = entry.map_err(|_| io())?.path();
        if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(ENTRY_PREFIX)) {
            paths.push(path);
            if paths.len() > MAX_ENTRIES {
                return Err(StartupError::HistoryCapacity);
            }
        }
    }
    Ok(paths)
}

fn unique_id() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{nanos}-{seq}", std::process::id())
}

/// One launch's owned startup entry, holding its lifetime lock.
///
/// Dropping it releases the lock without marking success or cancellation, so a
/// genuine premature death still counts on the next launch.
#[derive(Debug)]
pub struct StartupTicket {
    dir: PathBuf,
    id: String,
    path: PathBuf,
    entry: File,
    prior: u32,
    dead_seen: Vec<PathBuf>,
    active: bool,
}

impl StartupTicket {
    /// How many earlier launches died before their window was up.
    pub fn prior_failed_starts(&self) -> u32 {
        self.prior
    }

    /// Mark only this launch's entry cancelled; never baseline or another launch.
    /// Consumed by the pre-document CEF cancellation seam when that gate is enabled.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cancel_controlled(&mut self) -> Result<(), StartupError> {
        if !self.active {
            return Ok(());
        }
        let _guard = coordinator(&self.dir)?;
        write_entry(&mut self.entry, &self.id, EntryState::Cancelled)?;
        // A durable terminal record is the acknowledgement; a failed unlink keeps it.
        let _ = std::fs::remove_file(&self.path);
        self.active = false;
        Ok(())
    }

    /// The real second-frame success: reset history from this launch's observation.
    pub fn mark_window_started(&mut self) -> Result<(), StartupError> {
        self.retire(EntryState::Started)
    }

    /// The handled renderer failure: retire the attempt like the old no_window path,
    /// without claiming a window opened.
    pub fn retire_no_window_attempt(&mut self) -> Result<(), StartupError> {
        self.retire(EntryState::NoWindow)
    }

    fn retire(&mut self, state: EntryState) -> Result<(), StartupError> {
        if !self.active {
            return Ok(());
        }
        let _guard = coordinator(&self.dir)?;
        write_entry(&mut self.entry, &self.id, state)?;
        let _ = std::fs::remove_file(&self.path);
        // Only failures this launch actually observed may be retired; a later or
        // still-active launch's entry is never touched.
        for path in std::mem::take(&mut self.dead_seen) {
            if let Ok(file) = OpenOptions::new().read(true).write(true).open(&path)
                && file.try_lock().is_ok()
            {
                let _ = std::fs::remove_file(&path);
            }
        }
        write_ledger(&self.dir, 0)?;
        self.active = false;
        Ok(())
    }
}

/// Begin one launch: migrate legacy history once, observe provably dead launches,
/// then create and lock this launch's unique entry.
pub fn begin_owned(dir: &Path) -> Result<StartupTicket, StartupError> {
    std::fs::create_dir_all(dir).map_err(|_| io())?;
    let _guard = coordinator(dir)?;

    let baseline = match read_ledger(dir)? {
        Some(baseline) => baseline,
        None => {
            let imported = legacy_count(dir);
            write_ledger(dir, imported)?;
            imported
        }
    };

    let mut dead = 0u32;
    let mut dead_seen = Vec::new();
    for path in entry_paths(dir)? {
        match classify(&path)? {
            Seen::Active | Seen::Terminal => {}
            Seen::Dead(path) => {
                dead = dead.saturating_add(1);
                dead_seen.push(path);
            }
        }
    }
    // Saturation at the existing bound is the historical behavior.
    let prior = baseline.saturating_add(dead).min(HISTORY_CAP);

    let id = unique_id();
    let mut last = StartupError::Ownership;
    for attempt in 0..MAX_ID_ATTEMPTS {
        let id = if attempt == 0 { id.clone() } else { format!("{id}-{attempt}") };
        let path = dir.join(format!("{ENTRY_PREFIX}{id}"));
        let mut entry = match OpenOptions::new().create_new(true).read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last = StartupError::Ownership;
                continue;
            }
            Err(_) => return Err(io()),
        };
        entry.try_lock().map_err(map_lock)?;
        write_entry(&mut entry, &id, EntryState::Unresolved)?;
        return Ok(StartupTicket { dir: dir.to_path_buf(), id, path, entry, prior, dead_seen, active: true });
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("intcad-startup-{name}-{}-{}", std::process::id(), unique_id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn legacy(dir: &Path, value: &str) {
        let mut file = File::create(dir.join(LEGACY_MARKER)).unwrap();
        file.write_all(value.as_bytes()).unwrap();
    }

    #[test]
    fn legacy_count_is_imported_once_and_bytes_are_kept() {
        let temp = Temp::new("migrate");
        legacy(temp.path(), "7");
        let ticket = begin_owned(temp.path()).unwrap();
        assert_eq!(ticket.prior_failed_starts(), 7);
        assert_eq!(std::fs::read_to_string(temp.path().join(LEGACY_MARKER)).unwrap(), "7");
        drop(ticket);
        // The ledger, not the legacy file, is authoritative from here on.
        legacy(temp.path(), "7");
        let next = begin_owned(temp.path()).unwrap();
        assert_eq!(next.prior_failed_starts(), 8, "the dropped unresolved launch counts once");
    }

    #[test]
    fn legacy_count_is_clamped_to_the_history_bound() {
        let temp = Temp::new("clamp");
        legacy(temp.path(), "5000");
        let ticket = begin_owned(temp.path()).unwrap();
        assert_eq!(ticket.prior_failed_starts(), HISTORY_CAP);
    }

    #[test]
    fn dropping_an_unresolved_launch_counts_it_on_the_next_launch() {
        let temp = Temp::new("death");
        let first = begin_owned(temp.path()).unwrap();
        assert_eq!(first.prior_failed_starts(), 0);
        drop(first);
        let second = begin_owned(temp.path()).unwrap();
        assert_eq!(second.prior_failed_starts(), 1);
    }

    #[test]
    fn controlled_cancel_adds_zero_and_keeps_prior_history() {
        let temp = Temp::new("cancel");
        legacy(temp.path(), "2");
        let mut first = begin_owned(temp.path()).unwrap();
        assert_eq!(first.prior_failed_starts(), 2);
        first.cancel_controlled().unwrap();
        first.cancel_controlled().unwrap();
        drop(first);
        let second = begin_owned(temp.path()).unwrap();
        assert_eq!(second.prior_failed_starts(), 2, "a controlled cancel is not a failure");
    }

    #[test]
    fn window_started_resets_history_but_not_a_later_live_entry() {
        let temp = Temp::new("started");
        legacy(temp.path(), "3");
        let mut first = begin_owned(temp.path()).unwrap();
        let live = begin_owned(temp.path()).unwrap();
        assert_eq!(live.prior_failed_starts(), 3, "an active launch is not dead");
        first.mark_window_started().unwrap();
        drop(live);
        let next = begin_owned(temp.path()).unwrap();
        assert_eq!(next.prior_failed_starts(), 1, "the later launch was not erased by the first");
    }

    #[test]
    fn two_controlled_cancels_never_erase_each_other() {
        let temp = Temp::new("two");
        let mut a = begin_owned(temp.path()).unwrap();
        let mut b = begin_owned(temp.path()).unwrap();
        b.cancel_controlled().unwrap();
        a.cancel_controlled().unwrap();
        drop((a, b));
        let next = begin_owned(temp.path()).unwrap();
        assert_eq!(next.prior_failed_starts(), 0);
    }

    #[test]
    fn damaged_ledger_is_an_error_not_invented_zero_history() {
        let temp = Temp::new("damaged");
        std::fs::write(temp.path().join(LEDGER), "version=1\nbaseline=not-a-number\n").unwrap();
        assert_eq!(begin_owned(temp.path()).map(|_| ()), Err(StartupError::InvalidRecord));
    }

    #[test]
    fn terminal_records_left_by_a_failed_unlink_are_not_counted() {
        let temp = Temp::new("terminal");
        std::fs::write(temp.path().join(format!("{ENTRY_PREFIX}old")), "version=1\nlaunch=old\nstate=cancelled\n").unwrap();
        let ticket = begin_owned(temp.path()).unwrap();
        assert_eq!(ticket.prior_failed_starts(), 0);
    }

    #[test]
    fn damaged_entry_is_an_error() {
        let temp = Temp::new("entry");
        std::fs::write(temp.path().join(format!("{ENTRY_PREFIX}bad")), "not a record").unwrap();
        assert_eq!(begin_owned(temp.path()).map(|_| ()), Err(StartupError::InvalidRecord));
    }

    #[test]
    fn no_window_retirement_resets_like_the_old_handled_failure_path() {
        let temp = Temp::new("nowindow");
        legacy(temp.path(), "4");
        let mut ticket = begin_owned(temp.path()).unwrap();
        ticket.retire_no_window_attempt().unwrap();
        drop(ticket);
        let next = begin_owned(temp.path()).unwrap();
        assert_eq!(next.prior_failed_starts(), 0);
    }
}
