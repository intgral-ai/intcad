//! Part tickets: the pieces of work a project's design has to go through, kept by the app in
//! `.intcad/tickets/` (the folder holds only what the app generates).
//!
//! A ticket is a piece of work. It may bear on no part, one or several; it may be a sub-ticket of
//! another, one level deep (there is no top ticket); it may wait for other tickets. Each ticket is
//! one JSON file, `T-<n>.json`; every step that changes a ticket also appends one line to
//! `events.jsonl`, which is never rewritten, so the record says who changed what and when. A
//! deleted ticket keeps its file, marked deleted, and can be restored.
//!
//! Who may make which move is decided here, not by the caller ([`may_move`]). The agent drafts,
//! sets a ticket's state, its own draft's acceptance included, says it has finished the ticket it
//! works and says when it is blocked; it starts no check and deletes or restores nothing. A person
//! accepts or declines a draft, hands a ticket to the Assistant or works it themself, starts its
//! check, sets any state, edits a ticket in any state, deletes and restores (the tickets view's
//! buttons; no command reaches these). For both, Done keeps its gate ([`why_not_done`]) and open
//! waits keep a ticket from the Assistant. The app binds the check's run. A person's step names
//! the revision the person saw, and is refused when the ticket has changed since.
//!
//! Attachments are ordinary files of the project, outside `.intcad/`: a ticket keeps their paths.
//! [`take_in`] copies a file from outside under the project's `attachments/`.
//!
//! Experiment (branch `experiment/part-tickets`, 2026-11-26): the demo of the part-ticket loop.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The folder inside `.intcad/`.
pub const DIR: &str = "tickets";
/// The append-only record of every step.
pub const EVENTS: &str = "events.jsonl";
/// The project's folder for files brought in from outside it (an ordinary folder of the project).
pub const ATTACHMENTS: &str = "attachments";
/// The layout this build writes. It reads 1 as well: one `part` there is a list of parts here.
pub const SCHEMA_VERSION: u32 = 2;
/// Limits on what a caller may store: parameters are hostile.
pub const MAX_TICKETS: usize = 200;
pub const MAX_TITLE: usize = 200;
pub const MAX_TEXT: usize = 8000;
pub const MAX_LINES: usize = 40;
pub const MAX_LINE: usize = 400;
pub const MAX_REASON: usize = 1000;
pub const MAX_PARTS: usize = 50;
pub const MAX_ATTACHMENTS: usize = 20;
pub const MAX_WAITS: usize = 20;
pub const MAX_PATH: usize = 400;
/// The largest file [`take_in`] copies into the project.
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
/// The most record lines one read returns, newest last.
pub const MAX_EVENTS: usize = 2000;
/// A run's finding states that mean a rule was checked: a run with none of them checked nothing.
const CHECKED_STATES: [&str; 4] = ["passed", "checked", "advisory", "failed"];

/// Bumped on every write, so a panel knows when to read the folder again.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// How many times any ticket store in this process was written.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Where a ticket stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Written by the agent; a person has not accepted it, so nobody works on it.
    Draft,
    /// Accepted (or written) by a person; it can be handed to the Assistant or worked.
    Confirmed,
    /// Somebody works it: the Assistant or a person (`worker`).
    InProgress,
    /// It waits for a person to confirm it: its check has run, or none applies.
    Waiting,
    /// A person finished it.
    Done,
    /// Stopped, with the agent's reason when the agent stopped.
    Blocked,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Draft => "draft",
            State::Confirmed => "confirmed",
            State::InProgress => "in_progress",
            State::Waiting => "waiting",
            State::Done => "done",
            State::Blocked => "blocked",
        }
    }
}

/// Who made a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    Person,
    Agent,
    App,
}

/// Who works a ticket once it is started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Worker {
    Assistant,
    Person,
}

/// A part a ticket bears on: a component of the design, or a body in a design of one part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// `component` or `body`.
    pub kind: String,
    /// The component's or body's id (or name) in the design.
    pub id: String,
}

/// Where a ticket's check stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// A person asked for it; the app has not started its run yet.
    Asked,
    /// The app started a run for it.
    Running,
    /// The run has a report.
    Ran,
    /// The run was refused or did not finish.
    Refused,
}

/// A ticket's check: today one sheet-metal DFM run, bound by its run id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    pub state: CheckState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// The run's own outcome word, as the run record has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// The run's counts by finding state (`failed`, `advisory`, `checked`, `unsupported` …).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u64>,
    /// The run's freshness when it was bound (`current`, `staleModel` …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<String>,
    /// Why a run was refused or did not finish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub at: String,
}

fn on() -> bool {
    true
}

/// One ticket, as its file holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub schema_version: u32,
    pub id: String,
    /// The ticket it is a sub-ticket of, one level deep; none for a ticket of its own.
    #[serde(default)]
    pub parent: Option<String>,
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// The requirement lines it carries, each one citable on its own; may be empty.
    #[serde(default)]
    pub requirements: Vec<String>,
    /// The parts it bears on: none, one or several.
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Files of the project it points to: paths relative to the project folder, `/` between
    /// names, never inside `.intcad/`.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// The tickets it waits for: the Assistant does not take it before they are done; a person
    /// is only reminded.
    #[serde(default)]
    pub waits: Vec<String>,
    /// Who works it: the Assistant, a person, or nobody (none).
    #[serde(default)]
    pub worker: Option<Worker>,
    /// Deleted or declined by a person: kept with its record, restorable.
    #[serde(default)]
    pub deleted: bool,
    /// The parent it was deleted together with: restoring that parent brings it back too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_with: Option<String>,
    /// The check is not switched off for it (by a person on the ticket, or by the agent's draft).
    /// Whether a check applies is [`applies`]: only to a ticket with parts.
    #[serde(default = "on")]
    pub checks: bool,
    /// 「需重查」: changed after it was finished, and why (its requirement lines or parts, or a part
    /// in the model). Its state stays; its check is out of date until a new run is bound.
    #[serde(default)]
    pub recheck: Option<String>,
    /// The Assistant said it has finished the work; a check applies, so the ticket stays in
    /// progress until a person starts that check (no check starts by itself).
    #[serde(default)]
    pub finished: bool,
    /// The agent asked, drafting it, for a picture of the 3D view; the app has not attached it yet.
    #[serde(default)]
    pub wants_view: bool,
    pub state: State,
    #[serde(default)]
    pub check: Option<Check>,
    /// Why the agent stopped on it, while it is blocked.
    #[serde(default)]
    pub blocked: Option<String>,
    /// One more on every step.
    pub revision: u64,
    pub created: String,
    pub updated: String,
}

/// One line of the record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub at: String,
    pub actor: Actor,
    pub ticket: String,
    /// `drafted`, `created`, `edited`, `confirmed`, `declined`, `deleted`, `restored`, `started`,
    /// `taken`, `moved`, `done`, `finished`, `check_asked`, `check_started`, `check_ran`,
    /// `check_refused`, `check_off`, `check_on`, `blocked`, `unblocked`, `recheck`, `view_attached`.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// Done without a check that checked anything: none applies, or its run checked nothing
    /// (「未检查」).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unchecked: bool,
    pub revision: u64,
}

/// What a new ticket is made of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    pub text: String,
    pub requirements: Vec<String>,
    /// The ticket it is a sub-ticket of; none: a ticket of its own.
    pub parent: Option<String>,
    pub parts: Vec<Part>,
    /// Paths of files already in the project ([`take_in`] brings one in from outside).
    pub attachments: Vec<String>,
    pub waits: Vec<String>,
    /// Whether a check applies; none: it does.
    pub checks: Option<bool>,
    /// Ask the app for a picture of the 3D view as it is now ([`attach_view`]).
    pub view: bool,
}

/// What a person changes in one save; none: left as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edit {
    pub title: Option<String>,
    pub text: Option<String>,
    pub requirements: Option<Vec<String>>,
    pub parts: Option<Vec<Part>>,
    pub attachments: Option<Vec<String>>,
    pub waits: Option<Vec<String>>,
    /// Switch the check off (`false`) or on again; it applies only to a ticket with parts.
    pub checks: Option<bool>,
}

/// A finished run as it is bound to a ticket.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunResult {
    pub run: String,
    pub outcome: String,
    pub counts: BTreeMap<String, u64>,
    pub freshness: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TicketError {
    #[error("there is no ticket {0}")]
    NoSuchTicket(String),
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{file}: {message}")]
    Damaged { file: String, message: String },
    #[error("{path}: {message}")]
    Io { path: String, message: String },
}

fn io(path: &Path, error: &std::io::Error) -> TicketError {
    TicketError::Io { path: path.display().to_string(), message: error.to_string() }
}

/// `<project>/.intcad/tickets`.
pub fn folder(root: &Path) -> PathBuf {
    root.join(super::FOLDER).join(DIR)
}

/// `T-` and a number from 1: the only names a ticket file may have, so an id is never a path.
pub fn number(id: &str) -> Option<u32> {
    let digits = id.strip_prefix("T-")?;
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    digits.parse().ok()
}

fn file(root: &Path, id: &str) -> Result<PathBuf, TicketError> {
    number(id).ok_or_else(|| TicketError::Invalid(format!("`{id}` is not a ticket id; ids look like T-1")))?;
    Ok(folder(root).join(format!("{id}.json")))
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    crate::validation::timestamp(ms)
}

/// The browser build keeps no project folder, so no ticket is ever written there.
#[cfg(target_arch = "wasm32")]
fn now() -> String {
    String::new()
}

/// Every ticket of the project by number, deleted ones included (marked). A project with no
/// tickets has an empty list.
pub fn list(root: &Path) -> Result<Vec<Ticket>, TicketError> {
    let dir = folder(root);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io(&dir, &error)),
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json") else { continue };
        let Some(n) = number(id) else { continue };
        if found.len() >= MAX_TICKETS {
            break;
        }
        found.push((n, load(root, id)?));
    }
    found.sort_by_key(|(n, _)| *n);
    Ok(found.into_iter().map(|(_, ticket)| ticket).collect())
}

/// One ticket. A file of layout 1 is read as this layout: its one `part` becomes the list of parts,
/// its parent link goes, a ticket somebody worked was worked by the Assistant (the only worker then), and a check the
/// agent asked for becomes the agent's "finished", so no run starts without a person.
pub fn load(root: &Path, id: &str) -> Result<Ticket, TicketError> {
    let path = file(root, id)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(TicketError::NoSuchTicket(id.to_string())),
        Err(error) => return Err(io(&path, &error)),
    };
    let damaged = |message: String| TicketError::Damaged { file: path.display().to_string(), message };
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| damaged(error.to_string()))?;
    let version = value.get("schemaVersion").and_then(serde_json::Value::as_u64).unwrap_or(0);
    if version > u64::from(SCHEMA_VERSION) {
        return Err(damaged(format!("schema version {version} is newer than this build's {SCHEMA_VERSION}")));
    }
    if version < 2
        && let Some(fields) = value.as_object_mut()
        && let Some(part) = fields.remove("part")
        && !part.is_null()
    {
        fields.insert("parts".into(), serde_json::Value::Array(vec![part]));
    }
    let mut ticket: Ticket = serde_json::from_value(value).map_err(|error| damaged(error.to_string()))?;
    if ticket.id != id {
        return Err(damaged(format!("the file holds ticket {}", ticket.id)));
    }
    if version < 2 {
        // Layout 1 made its first ticket the top ticket by rule; nobody chose those parents.
        ticket.parent = None;
        if matches!(ticket.state, State::InProgress | State::Blocked | State::Waiting | State::Done) {
            ticket.worker = Some(Worker::Assistant);
        }
        if ticket.check.as_ref().is_some_and(|check| check.state == CheckState::Asked) {
            ticket.check = None;
            ticket.finished = ticket.state == State::InProgress;
        }
    }
    ticket.schema_version = SCHEMA_VERSION;
    Ok(ticket)
}

/// The record, oldest first: of one ticket, or of all. A damaged line is skipped, never repaired.
pub fn events(root: &Path, ticket: Option<&str>) -> Result<Vec<Event>, TicketError> {
    let path = folder(root).join(EVENTS);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io(&path, &error)),
    };
    let mut out: Vec<Event> =
        text.lines().filter_map(|line| serde_json::from_str::<Event>(line).ok()).filter(|event| ticket.is_none_or(|id| event.ticket == id)).collect();
    if out.len() > MAX_EVENTS {
        out.drain(..out.len() - MAX_EVENTS);
    }
    Ok(out)
}

fn write(root: &Path, ticket: &Ticket) -> Result<(), TicketError> {
    let path = file(root, &ticket.id)?;
    let dir = folder(root);
    fs::create_dir_all(&dir).map_err(|error| io(&dir, &error))?;
    let bytes = serde_json::to_vec_pretty(ticket).map_err(|error| TicketError::Invalid(error.to_string()))?;
    solidcraft_io::safe_write::write_atomic(&path, &bytes, false).map_err(|error| io(&path, &error))
}

fn append(root: &Path, event: &Event) -> Result<(), TicketError> {
    let dir = folder(root);
    fs::create_dir_all(&dir).map_err(|error| io(&dir, &error))?;
    let path = dir.join(EVENTS);
    let mut line = serde_json::to_string(event).map_err(|error| TicketError::Invalid(error.to_string()))?;
    line.push('\n');
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|error| io(&path, &error))?;
    file.write_all(line.as_bytes()).map_err(|error| io(&path, &error))?;
    file.sync_data().map_err(|error| io(&path, &error))
}

/// What a step's record line says besides the ticket and its states.
struct Line {
    actor: Actor,
    action: &'static str,
    note: Option<String>,
    run: Option<String>,
    unchecked: bool,
}

impl Line {
    fn new(actor: Actor, action: &'static str) -> Self {
        Self { actor, action, note: None, run: None, unchecked: false }
    }

    fn note(mut self, note: Option<String>) -> Self {
        self.note = note;
        self
    }
}

/// Write the ticket, then its record line. The line names the revision the ticket now has.
fn commit(root: &Path, ticket: &Ticket, from: Option<State>, line: Line) -> Result<(), TicketError> {
    write(root, ticket)?;
    let event = Event {
        at: ticket.updated.clone(),
        actor: line.actor,
        ticket: ticket.id.clone(),
        action: line.action.to_string(),
        from,
        to: Some(ticket.state).filter(|to| Some(*to) != from),
        note: line.note,
        run: line.run,
        unchecked: line.unchecked,
        revision: ticket.revision,
    };
    let appended = append(root, &event);
    GENERATION.fetch_add(1, Ordering::Relaxed);
    appended
}

fn clean(text: &str, max: usize, what: &str) -> Result<String, TicketError> {
    let text = text.trim();
    if text.chars().count() > max {
        return Err(TicketError::Invalid(format!("the {what} is longer than {max} characters")));
    }
    Ok(text.to_string())
}

fn note(note: Option<String>) -> Result<Option<String>, TicketError> {
    Ok(note.map(|note| clean(&note, MAX_REASON, "note")).transpose()?.filter(|note| !note.is_empty()))
}

fn title(title: &str) -> Result<String, TicketError> {
    let title = clean(title, MAX_TITLE, "title")?;
    if title.is_empty() {
        return Err(TicketError::Invalid("a ticket needs a title: what is to be done, in a few words".into()));
    }
    Ok(title)
}

/// Requirement lines, checked and trimmed, empty ones dropped.
fn lines(requirements: &[String]) -> Result<Vec<String>, TicketError> {
    if requirements.len() > MAX_LINES {
        return Err(TicketError::Invalid(format!("a ticket carries at most {MAX_LINES} requirement lines")));
    }
    requirements.iter().map(|line| clean(line, MAX_LINE, "requirement line")).filter(|line| !matches!(line, Ok(l) if l.is_empty())).collect()
}

/// The parts a ticket bears on, checked, each once.
fn parts(parts: &[Part]) -> Result<Vec<Part>, TicketError> {
    if parts.len() > MAX_PARTS {
        return Err(TicketError::Invalid(format!("a ticket bears on at most {MAX_PARTS} parts")));
    }
    let mut out: Vec<Part> = Vec::new();
    for part in parts {
        if !matches!(part.kind.as_str(), "component" | "body") {
            return Err(TicketError::Invalid(format!(
                "`{}` is not a part kind; use component or body",
                part.kind.chars().take(40).collect::<String>()
            )));
        }
        let id = part.id.trim();
        if id.is_empty() || id.chars().count() > 128 {
            return Err(TicketError::Invalid("a part needs the id or name of its component or body".into()));
        }
        let part = Part { kind: part.kind.clone(), id: id.to_string() };
        if !out.contains(&part) {
            out.push(part);
        }
    }
    Ok(out)
}

/// Whether `name`, the first name of a path in the project, is the app's own folder.
fn apps_folder(name: &str) -> bool {
    name.eq_ignore_ascii_case(super::FOLDER)
}

/// A path of a file in the project as a ticket keeps it: relative to the project folder, `/`
/// between names, the file there and inside the folder (no `..`, no link out of it), outside
/// `.intcad/`, which holds only what the app generates.
fn attachment(root: &Path, path: &str) -> Result<String, TicketError> {
    let path = path.trim();
    let shown: String = path.chars().take(80).collect();
    let refuse = |why: &str| TicketError::Invalid(format!("attachment `{shown}`: {why}"));
    if path.is_empty() || path.chars().count() > MAX_PATH {
        return Err(refuse(&format!("give the path of a file in the project, at most {MAX_PATH} characters")));
    }
    let names: Vec<&str> = path.split(['/', '\\']).collect();
    if names.iter().any(|name| name.is_empty() || *name == "." || *name == "..") || names.first().is_some_and(|name| name.ends_with(':')) {
        return Err(refuse("give a path relative to the project folder, without `..`"));
    }
    if names.first().is_some_and(|name| apps_folder(name)) {
        return Err(refuse(".intcad holds only what the app generates; attach a file of the project"));
    }
    let full = names.iter().fold(root.to_path_buf(), |path, name| path.join(name));
    if !full.is_file() {
        return Err(refuse("there is no such file in the project"));
    }
    let (Ok(real), Ok(base)) = (fs::canonicalize(&full), fs::canonicalize(root)) else {
        return Err(refuse("the file cannot be read"));
    };
    if !real.starts_with(&base) || real.starts_with(base.join(super::FOLDER)) {
        return Err(refuse("the file lies outside the project folder"));
    }
    Ok(names.join("/"))
}

fn attachments(root: &Path, paths: &[String]) -> Result<Vec<String>, TicketError> {
    if paths.len() > MAX_ATTACHMENTS {
        return Err(TicketError::Invalid(format!("a ticket carries at most {MAX_ATTACHMENTS} attachments")));
    }
    let mut out: Vec<String> = Vec::new();
    for path in paths {
        let path = attachment(root, path)?;
        if !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// A file name for the project's `attachments/`: no separators or control characters, not hidden,
/// at most about 120 characters with its extension kept.
fn file_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let clean = clean.trim().trim_start_matches('.').trim_end_matches(['.', ' ']);
    let (stem, extension) = match clean.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{}", extension.chars().take(16).collect::<String>())),
        _ => (clean, String::new()),
    };
    let stem: String = stem.chars().take(100).collect();
    format!("{}{extension}", if stem.trim().is_empty() { "attachment" } else { stem.trim() })
}

/// Write `bytes` as a new file under the project's `attachments/`, named after `name`: `name`, or
/// `stem-2.ext` and on while that is taken, so no file there is ever overwritten. Its path as a
/// ticket keeps it.
fn store_file(root: &Path, name: &str, bytes: &[u8]) -> Result<String, TicketError> {
    let name = file_name(name);
    let dir = root.join(ATTACHMENTS);
    fs::create_dir_all(&dir).map_err(|error| io(&dir, &error))?;
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    for n in 1..=1000 {
        let candidate = if n == 1 { name.clone() } else { format!("{stem}-{n}{extension}") };
        let path = dir.join(&candidate);
        if fs::symlink_metadata(&path).is_ok() {
            continue;
        }
        solidcraft_io::safe_write::write_atomic(&path, bytes, false).map_err(|error| io(&path, &error))?;
        return Ok(format!("{ATTACHMENTS}/{candidate}"));
    }
    Err(TicketError::Refused(format!("{ATTACHMENTS}/ has too many files called {name}")))
}

/// Bring a file in for a ticket: a file already in the project (outside `.intcad/`) stays where it
/// is; any other is copied under the project's `attachments/` with a name of its own, never into
/// `.intcad/`. The path a ticket keeps for it, to pass to [`create`] or [`edit`].
pub fn take_in(root: &Path, source: &Path) -> Result<String, TicketError> {
    let meta = fs::metadata(source).map_err(|error| io(source, &error))?;
    if !meta.is_file() {
        return Err(TicketError::Invalid(format!("{}: only a file can be attached", source.display())));
    }
    let base = fs::canonicalize(root).map_err(|error| io(root, &error))?;
    let real = fs::canonicalize(source).map_err(|error| io(source, &error))?;
    if let Ok(inside) = real.strip_prefix(&base) {
        let names: Vec<String> = inside.components().map(|name| name.as_os_str().to_string_lossy().into_owned()).collect();
        if !names.first().is_some_and(|name| apps_folder(name)) {
            return attachment(root, &names.join("/"));
        }
    }
    if meta.len() > MAX_ATTACHMENT_BYTES {
        return Err(TicketError::Refused(format!("{}: a file brought in is at most {} MB", source.display(), MAX_ATTACHMENT_BYTES / (1024 * 1024))));
    }
    let bytes = fs::read(&real).map_err(|error| io(&real, &error))?;
    let name = real.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    store_file(root, &name, &bytes)
}

/// The tickets `id` waits for, checked: each exists, is not deleted, is not `id`, is named once,
/// and none of them waits for `id` already, directly or through others. A parent counts as
/// waiting for each of its sub-tickets (it is done only after them), so a sub-ticket cannot wait
/// for its parent. `parent`: the parent of a ticket not written yet.
fn waits(id: &str, parent: Option<&str>, wanted: &[String], all: &[Ticket]) -> Result<Vec<String>, TicketError> {
    if wanted.len() > MAX_WAITS {
        return Err(TicketError::Invalid(format!("a ticket waits for at most {MAX_WAITS} tickets")));
    }
    let mut out: Vec<String> = Vec::new();
    for other in wanted {
        let other = other.trim();
        number(other)
            .ok_or_else(|| TicketError::Invalid(format!("`{}` is not a ticket id; ids look like T-1", other.chars().take(40).collect::<String>())))?;
        if other == id {
            return Err(TicketError::Invalid(format!("{id} cannot wait for itself")));
        }
        let ticket = all.iter().find(|ticket| ticket.id == other).ok_or_else(|| TicketError::NoSuchTicket(other.to_string()))?;
        if ticket.deleted {
            return Err(TicketError::Refused(format!("{other} is deleted: restore it first, or wait for another ticket")));
        }
        if !out.iter().any(|known| known == other) {
            out.push(other.to_string());
        }
    }
    let edges = |at: &str| -> Vec<String> {
        let mut next: Vec<String> = all.iter().find(|ticket| ticket.id == at).map(|ticket| ticket.waits.clone()).unwrap_or_default();
        next.extend(all.iter().filter(|ticket| ticket.parent.as_deref() == Some(at)).map(|ticket| ticket.id.clone()));
        if parent == Some(at) {
            next.push(id.to_string());
        }
        next
    };
    for start in &out {
        let mut stack = vec![start.clone()];
        let mut seen: Vec<String> = Vec::new();
        while let Some(at) = stack.pop() {
            if at == id {
                return Err(TicketError::Refused(format!("{id} cannot wait for {start}: {start} already waits for {id}, or is its parent")));
            }
            if seen.contains(&at) || seen.len() > MAX_TICKETS {
                continue;
            }
            stack.extend(edges(&at));
            seen.push(at);
        }
    }
    Ok(out)
}

/// The tickets `ticket` waits for that are not done yet (deleted ones included: they will not be).
pub fn open_waits(ticket: &Ticket, all: &[Ticket]) -> Vec<String> {
    ticket.waits.iter().filter(|other| all.iter().find(|t| &t.id == *other).is_none_or(|t| t.state != State::Done)).cloned().collect()
}

/// One more than the highest number the project has ever given, in its files or its record.
fn next_number(root: &Path, existing: &[Ticket]) -> Result<u32, TicketError> {
    let path = folder(root).join(EVENTS);
    let recorded = match fs::read_to_string(&path) {
        Ok(text) => text.lines().filter_map(|line| serde_json::from_str::<Event>(line).ok()).filter_map(|event| number(&event.ticket)).max(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io(&path, &error)),
    };
    let filed = existing.iter().filter_map(|ticket| number(&ticket.id)).max();
    Ok(filed.max(recorded).unwrap_or(0).saturating_add(1))
}

/// Write a new draft. The agent's only way to add a ticket; nothing it drafts can be worked on
/// before a person accepts it. A draft goes under a parent only when it names one.
pub fn draft(root: &Path, draft: Draft, actor: Actor) -> Result<Ticket, TicketError> {
    new_ticket(root, draft, actor, State::Draft, "drafted")
}

/// A person writes a ticket of their own. It starts confirmed: the person who wrote it needs no
/// second look. Only the tickets view's New ticket calls this.
pub fn create(root: &Path, draft: Draft) -> Result<Ticket, TicketError> {
    new_ticket(root, draft, Actor::Person, State::Confirmed, "created")
}

fn new_ticket(root: &Path, draft: Draft, actor: Actor, state: State, action: &'static str) -> Result<Ticket, TicketError> {
    let title = title(&draft.title)?;
    let text = clean(&draft.text, MAX_TEXT, "text")?;
    let requirements = lines(&draft.requirements)?;
    let parts = parts(&draft.parts)?;
    let attachments = attachments(root, &draft.attachments)?;
    let existing = list(root)?;
    if existing.len() >= MAX_TICKETS {
        return Err(TicketError::Refused(format!("a project holds at most {MAX_TICKETS} tickets")));
    }
    let parent = match draft.parent.as_deref().map(str::trim).filter(|parent| !parent.is_empty()) {
        Some(parent) => {
            let ticket = existing.iter().find(|ticket| ticket.id == parent).ok_or_else(|| TicketError::NoSuchTicket(parent.to_string()))?;
            if ticket.deleted {
                return Err(TicketError::Refused(format!("{parent} is deleted: restore it before adding sub-tickets")));
            }
            if ticket.parent.is_some() {
                return Err(TicketError::Refused(format!("{parent} is a sub-ticket; sub-tickets go one level deep")));
            }
            Some(parent.to_string())
        }
        None => None,
    };
    let id = format!("T-{}", next_number(root, &existing)?);
    let waits = waits(&id, parent.as_deref(), &draft.waits, &existing)?;
    let at = now();
    let ticket = Ticket {
        schema_version: SCHEMA_VERSION,
        id,
        parent,
        title,
        text,
        requirements,
        parts,
        attachments,
        waits,
        worker: None,
        deleted: false,
        deleted_with: None,
        checks: draft.checks.unwrap_or(true),
        recheck: None,
        finished: false,
        wants_view: draft.view,
        state,
        check: None,
        blocked: None,
        revision: 1,
        created: at.clone(),
        updated: at,
    };
    commit(root, &ticket, None, Line::new(actor, action))?;
    Ok(ticket)
}

/// Load, check the move, change, write and record, in one step. `seen`: the revision a person saw
/// when they pressed; a ticket that has changed since is refused, so nobody acts on what they did
/// not read. A deleted ticket takes no step but its restoring.
fn step(
    root: &Path,
    id: &str,
    mut line: Line,
    seen: Option<u64>,
    change: impl FnOnce(&mut Ticket, &mut Line) -> Result<(), TicketError>,
) -> Result<Ticket, TicketError> {
    let mut ticket = load(root, id)?;
    if let Some(seen) = seen {
        unchanged(&ticket, seen)?;
    }
    if ticket.deleted && line.action != "restored" {
        return Err(TicketError::Refused(format!("{} is deleted: restore it first", ticket.id)));
    }
    let from = ticket.state;
    change(&mut ticket, &mut line)?;
    ticket.revision = ticket.revision.saturating_add(1);
    ticket.updated = now();
    commit(root, &ticket, Some(from), line)?;
    Ok(ticket)
}

/// Refused when the ticket is not at the revision a person saw.
fn unchanged(ticket: &Ticket, seen: u64) -> Result<(), TicketError> {
    if ticket.revision == seen {
        return Ok(());
    }
    Err(TicketError::Refused(format!("{} has changed since you saw it (revision {seen}, now {}): look at it again", ticket.id, ticket.revision)))
}

fn refused(ticket: &Ticket, what: &str) -> TicketError {
    TicketError::Refused(format!("{} is {}: {what}", ticket.id, ticket.state.as_str().replace('_', " ")))
}

/// Who may move a ticket from one state to another. This is the one place the rule lives: every
/// step that changes a state asks it ([`move_to`]).
///
/// - A person and the agent: any move, back as well as forward, a draft's acceptance included.
///   Done keeps its gate for both ([`why_not_done`]); open waits keep a ticket from the Assistant
///   ([`start`], [`agent_set_state`]).
/// - The app: in progress to waiting, when it binds a check's run.
///
/// Starting a check, deleting and restoring are not moves: only a person does those.
pub fn may_move(actor: Actor, from: State, to: State) -> bool {
    match actor {
        Actor::Person | Actor::Agent => true,
        Actor::App => matches!((from, to), (State::InProgress, State::Waiting)),
    }
}

/// Move `ticket` to `to` as `actor`, or say why not. With the move: nobody works a draft or a
/// confirmed ticket, a block's reason goes when the block ends, and the Assistant's "finished"
/// goes with the state it was said in.
fn move_to(ticket: &mut Ticket, actor: Actor, to: State) -> Result<(), TicketError> {
    let from = ticket.state;
    if from == to {
        return Ok(());
    }
    if !may_move(actor, from, to) {
        return Err(TicketError::Refused(format!(
            "{} is {}: the app cannot move it to {}",
            ticket.id,
            from.as_str().replace('_', " "),
            to.as_str().replace('_', " ")
        )));
    }
    ticket.state = to;
    ticket.finished = false;
    if from == State::Blocked {
        ticket.blocked = None;
    }
    if matches!(to, State::Draft | State::Confirmed) {
        ticket.worker = None;
    }
    Ok(())
}

/// Whether a check applies to `ticket`: it bears on parts and the check is not switched off for it.
/// A ticket with no parts is finished without one, recorded as unchecked. The one place this rule
/// lives.
pub fn applies(ticket: &Ticket) -> bool {
    ticket.checks && !ticket.parts.is_empty()
}

/// Why a ticket cannot be finished now.
#[derive(Clone, Debug, PartialEq)]
pub enum NotDone {
    /// Its sub-tickets that are not deleted and not done.
    Subs(Vec<String>),
    /// A check applies and no run is bound to it.
    NoRun,
    /// A check applies and the ticket changed after its run (「需重查」), and why.
    Recheck(String),
}

impl NotDone {
    /// The reason in the store's own words, naming the ticket.
    pub fn say(&self, id: &str) -> String {
        match self {
            NotDone::Subs(open) => format!("{id} has sub-tickets not done yet ({})", open.join(", ")),
            NotDone::NoRun => format!("{id}'s check has not run"),
            NotDone::Recheck(why) => format!("{id} was changed after its check ran ({why}): run its check again"),
        }
    }
}

/// Why `ticket` cannot be finished now, by a person or the agent, or none: every sub-ticket of it
/// that is not deleted is done, and where a check applies ([`applies`]) a run is bound to it and
/// nothing changed since that asks for another (「需重查」). A run that checked nothing, or no
/// check at all, still lets it finish, recorded as unchecked ([`unchecked`]).
pub fn why_not_done(ticket: &Ticket, all: &[Ticket]) -> Option<NotDone> {
    let open: Vec<String> = all
        .iter()
        .filter(|t| t.parent.as_deref() == Some(ticket.id.as_str()) && !t.deleted && t.state != State::Done)
        .map(|t| t.id.clone())
        .collect();
    if !open.is_empty() {
        return Some(NotDone::Subs(open));
    }
    if applies(ticket) {
        if !ticket.check.as_ref().is_some_and(|check| check.state == CheckState::Ran && check.run.is_some()) {
            return Some(NotDone::NoRun);
        }
        if let Some(why) = &ticket.recheck {
            return Some(NotDone::Recheck(why.clone()));
        }
    }
    None
}

/// Whether finishing `ticket` now leaves it unchecked: no check applies, or its run checked nothing.
pub fn unchecked(ticket: &Ticket) -> bool {
    !applies(ticket) || ticket.check.as_ref().is_none_or(|check| CHECKED_STATES.iter().all(|state| check.counts.get(*state).copied().unwrap_or(0) == 0))
}

/// A person accepts a draft they saw at revision `seen`; `with_subs`: its sub-tickets that are
/// still drafts with it, each with its own record line. Only the tickets view's 接受 calls this
/// (the agent accepts its own draft by [`agent_set_state`]).
pub fn confirm(root: &Path, id: &str, seen: u64, with_subs: bool) -> Result<Ticket, TicketError> {
    let ticket = step(root, id, Line::new(Actor::Person, "confirmed"), Some(seen), |ticket, _| {
        if ticket.state != State::Draft {
            return Err(refused(ticket, "only a draft is accepted"));
        }
        move_to(ticket, Actor::Person, State::Confirmed)
    })?;
    if with_subs {
        let subs = list(root)?.into_iter().filter(|sub| sub.parent.as_deref() == Some(id) && sub.state == State::Draft && !sub.deleted);
        for sub in subs {
            let line = Line::new(Actor::Person, "confirmed").note(Some(format!("with {id}")));
            step(root, &sub.id, line, None, |sub, _| move_to(sub, Actor::Person, State::Confirmed))?;
        }
    }
    Ok(ticket)
}

/// The sub-tickets of `id` that are not deleted: they go to 已删除 with it.
fn live_subs(root: &Path, id: &str) -> Result<Vec<String>, TicketError> {
    Ok(list(root)?.into_iter().filter(|other| other.parent.as_deref() == Some(id) && !other.deleted).map(|other| other.id).collect())
}

/// Delete `id`'s live sub-tickets with it, each with its own line, marked as gone with it.
fn delete_subs(root: &Path, id: &str, subs: &[String]) -> Result<(), TicketError> {
    for sub in subs {
        let line = Line::new(Actor::Person, "deleted").note(Some(format!("with {id}")));
        step(root, sub, line, None, |sub, _| {
            sub.deleted = true;
            sub.deleted_with = Some(id.to_string());
            Ok(())
        })?;
    }
    Ok(())
}

/// A person declines a draft (不要) they saw at revision `seen`: it is deleted, its record kept,
/// and can be restored as a draft; its sub-tickets go with it (the page asks first). Only the
/// tickets view calls this.
pub fn decline(root: &Path, id: &str, seen: u64) -> Result<Ticket, TicketError> {
    let subs = live_subs(root, id)?;
    let ticket = step(root, id, Line::new(Actor::Person, "declined"), Some(seen), |ticket, _| {
        if ticket.state != State::Draft {
            return Err(refused(ticket, "only a draft is declined; delete it instead"));
        }
        ticket.deleted = true;
        Ok(())
    })?;
    delete_subs(root, id, &subs)?;
    Ok(ticket)
}

/// A person deletes a ticket in any state, seen at revision `seen`: it is marked deleted, its file
/// and record kept, and it can be restored; its sub-tickets go with it (the page asks first). Only
/// the tickets view's Delete calls this.
pub fn delete(root: &Path, id: &str, seen: u64) -> Result<Ticket, TicketError> {
    let subs = live_subs(root, id)?;
    let ticket = step(root, id, Line::new(Actor::Person, "deleted"), Some(seen), |ticket, _| {
        ticket.deleted = true;
        Ok(())
    })?;
    delete_subs(root, id, &subs)?;
    Ok(ticket)
}

/// A person restores a deleted or declined ticket, in the state it had, with the sub-tickets that
/// were deleted together with it. Only the tickets view's Restore (and the toast's Undo) calls this.
pub fn restore(root: &Path, id: &str) -> Result<Ticket, TicketError> {
    let ticket = step(root, id, Line::new(Actor::Person, "restored"), None, |ticket, _| {
        if !ticket.deleted {
            return Err(TicketError::Refused(format!("{} is not deleted", ticket.id)));
        }
        ticket.deleted = false;
        ticket.deleted_with = None;
        Ok(())
    })?;
    let with: Vec<String> = list(root)?.into_iter().filter(|sub| sub.deleted && sub.deleted_with.as_deref() == Some(id)).map(|sub| sub.id).collect();
    for sub in with {
        let line = Line::new(Actor::Person, "restored").note(Some(format!("with {id}")));
        step(root, &sub, line, None, |sub, _| {
            sub.deleted = false;
            sub.deleted_with = None;
            Ok(())
        })?;
    }
    Ok(ticket)
}

/// A person hands a confirmed or blocked ticket, seen at revision `seen`, to the Assistant (交给助手).
/// Refused while it waits for tickets not done yet. Only the tickets view calls this; the host
/// then sends the Assistant its message.
pub fn start(root: &Path, id: &str, seen: u64) -> Result<Ticket, TicketError> {
    let all = list(root)?;
    step(root, id, Line::new(Actor::Person, "started"), Some(seen), |ticket, _| {
        if !matches!(ticket.state, State::Confirmed | State::Blocked) {
            return Err(refused(ticket, "only a confirmed or blocked ticket is handed to the Assistant"));
        }
        let open = open_waits(ticket, &all);
        if !open.is_empty() {
            return Err(TicketError::Refused(format!("{} waits for {}: the Assistant takes it once those are done", ticket.id, open.join(", "))));
        }
        move_to(ticket, Actor::Person, State::InProgress)?;
        ticket.worker = Some(Worker::Assistant);
        Ok(())
    })
}

/// A person takes a confirmed or blocked ticket, seen at revision `seen`, to work it themself
/// (我自己做). Tickets it waits for do not stop a person (the page reminds them).
pub fn work_myself(root: &Path, id: &str, seen: u64) -> Result<Ticket, TicketError> {
    step(root, id, Line::new(Actor::Person, "taken"), Some(seen), |ticket, _| {
        if !matches!(ticket.state, State::Confirmed | State::Blocked) {
            return Err(refused(ticket, "only a confirmed or blocked ticket is taken up"));
        }
        move_to(ticket, Actor::Person, State::InProgress)?;
        ticket.worker = Some(Worker::Person);
        Ok(())
    })
}

/// A person sets a ticket they saw at revision `seen` to any state (the state menu): Done only
/// past its gate ([`why_not_done`]), recorded as unchecked where nothing was checked. Setting the
/// state it already has changes nothing. Tickets it waits for do not stop a person.
pub fn set_state(root: &Path, id: &str, seen: u64, to: State) -> Result<Ticket, TicketError> {
    move_state(root, id, Some(seen), to, Actor::Person)
}

/// The agent sets a ticket to any state (`ticket.update`, action `state`), under the conditions a
/// person meets: Done only past its gate. Moving a ticket in progress makes it the Assistant's, and
/// is refused while it waits for tickets not done yet or while the Assistant works another ticket
/// (one at a time). Accepting a draft is a move from draft to confirmed.
pub fn agent_set_state(root: &Path, id: &str, to: State) -> Result<Ticket, TicketError> {
    move_state(root, id, None, to, Actor::Agent)
}

fn move_state(root: &Path, id: &str, seen: Option<u64>, to: State, actor: Actor) -> Result<Ticket, TicketError> {
    let current = load(root, id)?;
    if let Some(seen) = seen {
        unchanged(&current, seen)?;
    }
    if current.state == to && !current.deleted {
        return Ok(current);
    }
    let all = list(root)?;
    if actor == Actor::Agent && to == State::InProgress {
        let open = open_waits(&current, &all);
        if !open.is_empty() {
            return Err(TicketError::Refused(format!("{id} waits for {}: the Assistant takes it once those are done", open.join(", "))));
        }
        if let Some(other) = all.iter().find(|t| t.id != id && !t.deleted && t.state == State::InProgress && t.worker == Some(Worker::Assistant)) {
            return Err(TicketError::Refused(format!("the Assistant works {} now: one ticket at a time", other.id)));
        }
    }
    step(root, id, Line::new(actor, "moved"), seen, |ticket, line| {
        if to == State::Done {
            if let Some(why) = why_not_done(ticket, &all) {
                return Err(TicketError::Refused(why.say(&ticket.id)));
            }
            line.action = "done";
            line.unchecked = unchecked(ticket);
            line.run = ticket.check.as_ref().and_then(|check| check.run.clone());
            ticket.recheck = None;
        } else if ticket.state == State::Draft && to == State::Confirmed {
            line.action = "confirmed";
        }
        move_to(ticket, actor, to)?;
        if to == State::InProgress {
            match actor {
                Actor::Agent => ticket.worker = Some(Worker::Assistant),
                _ if ticket.worker.is_none() => ticket.worker = Some(Worker::Person),
                _ => {}
            }
        }
        Ok(())
    })
}

/// A person marks a ticket done (完成): [`set_state`] to Done.
pub fn done(root: &Path, id: &str, seen: u64) -> Result<Ticket, TicketError> {
    set_state(root, id, seen, State::Done)
}

/// A person changes, in one save, any of a ticket's title, text, requirement lines, parts,
/// attachments, waits and whether its check is switched off, in any state, seen at revision
/// `seen`: one record line (`check_off` or `check_on` when the switch moved, `edited` otherwise). A
/// draft stays a draft. Changing the requirement lines or parts of a done ticket marks it 「需重查」
/// in the same line. A save that changes nothing writes nothing. Only the tickets view calls this.
pub fn edit(root: &Path, id: &str, seen: u64, edit: Edit) -> Result<Ticket, TicketError> {
    let current = load(root, id)?;
    unchanged(&current, seen)?;
    if current.deleted {
        return Err(TicketError::Refused(format!("{id} is deleted: restore it first")));
    }
    let mut next = current.clone();
    if let Some(words) = &edit.title {
        next.title = title(words)?;
    }
    if let Some(words) = &edit.text {
        next.text = clean(words, MAX_TEXT, "text")?;
    }
    if let Some(requirements) = &edit.requirements {
        next.requirements = lines(requirements)?;
    }
    if let Some(wanted) = &edit.parts {
        next.parts = parts(wanted)?;
    }
    if let Some(paths) = &edit.attachments {
        next.attachments = attachments(root, paths)?;
    }
    if let Some(wanted) = &edit.waits {
        next.waits = waits(id, None, wanted, &list(root)?)?;
    }
    if let Some(on) = edit.checks {
        next.checks = on;
    }
    if next == current {
        return Ok(current);
    }
    let changed: Vec<&str> = [("requirement lines", next.requirements != current.requirements), ("parts", next.parts != current.parts)]
        .into_iter()
        .filter(|(_, c)| *c)
        .map(|(what, _)| what)
        .collect();
    let recheck = (current.state == State::Done && !changed.is_empty()).then(|| format!("{} changed", changed.join(" and ")));
    let action = match (current.checks, next.checks) {
        (true, false) => "check_off",
        (false, true) => "check_on",
        _ => "edited",
    };
    let line = Line::new(Actor::Person, action).note(recheck.as_ref().map(|why| format!("needs a re-check: {why}")));
    step(root, id, line, Some(seen), move |ticket, _| {
        ticket.title = next.title;
        ticket.text = next.text;
        ticket.requirements = next.requirements;
        ticket.parts = next.parts;
        ticket.attachments = next.attachments;
        ticket.waits = next.waits;
        ticket.checks = next.checks;
        if recheck.is_some() {
            ticket.recheck = recheck;
        }
        Ok(())
    })
}

/// 「需重查」 on a done ticket changed afterwards: its state stays, and its check is out of date
/// until a new run is bound. The app calls this when a part the ticket bears on changed in the
/// model (the store marks a person's edit itself, [`edit`]). A ticket already marked keeps its
/// first reason and no line is written.
pub fn mark_recheck(root: &Path, id: &str, why: &str) -> Result<Ticket, TicketError> {
    let why = clean(why, MAX_REASON, "reason")?;
    if why.is_empty() {
        return Err(TicketError::Invalid("say what changed".into()));
    }
    let current = load(root, id)?;
    if current.recheck.is_some() && !current.deleted {
        return Ok(current);
    }
    step(root, id, Line::new(Actor::App, "recheck").note(Some(why.clone())), None, |ticket, _| {
        if ticket.state != State::Done {
            return Err(refused(ticket, "only a done ticket is marked for a re-check"));
        }
        ticket.recheck = Some(why);
        Ok(())
    })
}

/// A person starts the check of a ticket in progress (跑检查, 请求检查), seen at revision `seen`;
/// the app then starts its run. Only a person starts a check, which costs credits.
pub fn ask_check(root: &Path, id: &str, seen: u64) -> Result<Ticket, TicketError> {
    step(root, id, Line::new(Actor::Person, "check_asked"), Some(seen), |ticket, _| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "the check is started on a ticket in progress"));
        }
        if !applies(ticket) {
            return Err(TicketError::Refused(format!("no check applies to {}: it is finished without one", ticket.id)));
        }
        ticket.check =
            Some(Check { state: CheckState::Asked, run: None, outcome: None, counts: BTreeMap::new(), freshness: None, reason: None, at: now() });
        Ok(())
    })
}

/// Refused unless `ticket` is in `state` and worked by the Assistant: the agent acts only on the
/// ticket handed to it.
fn agents_ticket(ticket: &Ticket, state: State, what: &str) -> Result<(), TicketError> {
    if ticket.state != state {
        return Err(refused(ticket, what));
    }
    if ticket.worker != Some(Worker::Assistant) {
        return Err(TicketError::Refused(format!("{} is not handed to the Assistant: the agent acts only on the ticket it works", ticket.id)));
    }
    Ok(())
}

/// The agent says it has finished the ticket it works (做完了). No check starts by itself:
/// where a check applies, the ticket stays in progress, marked finished, until a person starts
/// the check; where none applies, it goes to waiting for a person's confirmation.
pub fn finished(root: &Path, id: &str, note_text: Option<String>) -> Result<Ticket, TicketError> {
    let note_text = note(note_text)?;
    step(root, id, Line::new(Actor::Agent, "finished").note(note_text), None, |ticket, _| {
        agents_ticket(ticket, State::InProgress, "only the ticket in progress is finished")?;
        if applies(ticket) {
            ticket.finished = true;
            Ok(())
        } else {
            move_to(ticket, Actor::Agent, State::Waiting)
        }
    })
}

/// The agent stops on the ticket it works and says why.
pub fn block(root: &Path, id: &str, reason: &str) -> Result<Ticket, TicketError> {
    let reason = clean(reason, MAX_REASON, "reason")?;
    if reason.is_empty() {
        return Err(TicketError::Invalid("say why the ticket is blocked".into()));
    }
    step(root, id, Line::new(Actor::Agent, "blocked").note(Some(reason.clone())), None, |ticket, _| {
        agents_ticket(ticket, State::InProgress, "only the ticket in progress can be blocked")?;
        move_to(ticket, Actor::Agent, State::Blocked)?;
        ticket.blocked = Some(reason);
        Ok(())
    })
}

/// The agent takes the blocked ticket it works up again.
pub fn unblock(root: &Path, id: &str, note_text: Option<String>) -> Result<Ticket, TicketError> {
    let note_text = note(note_text)?;
    step(root, id, Line::new(Actor::Agent, "unblocked").note(note_text), None, |ticket, _| {
        agents_ticket(ticket, State::Blocked, "only a blocked ticket is unblocked")?;
        move_to(ticket, Actor::Agent, State::InProgress)
    })
}

/// The app attaches the picture of the 3D view the agent asked for when it drafted (`png`, as the
/// host rendered the view): saved under `attachments/` as `<id>-view.png`, added to the ticket, the
/// ask cleared. The seam for the host, which takes the picture; the engine has no view.
pub fn attach_view(root: &Path, id: &str, png: &[u8]) -> Result<Ticket, TicketError> {
    if png.len() as u64 > MAX_ATTACHMENT_BYTES || png.is_empty() {
        return Err(TicketError::Invalid("the picture is empty or too large".into()));
    }
    let ticket = load(root, id)?;
    if ticket.deleted {
        return Err(TicketError::Refused(format!("{id} is deleted: restore it first")));
    }
    if ticket.attachments.len() >= MAX_ATTACHMENTS {
        return Err(TicketError::Refused(format!("{id} carries {MAX_ATTACHMENTS} attachments already")));
    }
    let path = store_file(root, &format!("{id}-view.png"), png)?;
    step(root, id, Line::new(Actor::App, "view_attached").note(Some(path.clone())), None, |ticket, _| {
        if !ticket.attachments.contains(&path) {
            ticket.attachments.push(path);
        }
        ticket.wants_view = false;
        Ok(())
    })
}

/// The app started the run of a ticket's asked-for check (`run` empty: its id is known only once
/// it has a report).
pub fn check_started(root: &Path, id: &str, run: &str) -> Result<Ticket, TicketError> {
    let run = Some(run.to_string()).filter(|run| !run.is_empty());
    let mut line = Line::new(Actor::App, "check_started");
    line.run = run.clone();
    step(root, id, line, None, |ticket, _| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "a check runs for the ticket in progress"));
        }
        ticket.check =
            Some(Check { state: CheckState::Running, run, outcome: None, counts: BTreeMap::new(), freshness: None, reason: None, at: now() });
        Ok(())
    })
}

/// The run of a ticket's check was refused or did not finish: the ticket stays in progress, with
/// the reason on it.
pub fn check_refused(root: &Path, id: &str, reason: &str) -> Result<Ticket, TicketError> {
    let reason: String = reason.trim().chars().take(MAX_REASON).collect();
    step(root, id, Line::new(Actor::App, "check_refused").note(Some(reason.clone())), None, |ticket, _| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "a check runs for the ticket in progress"));
        }
        let run = ticket.check.as_ref().and_then(|check| check.run.clone());
        ticket.check =
            Some(Check { state: CheckState::Refused, run, outcome: None, counts: BTreeMap::new(), freshness: None, reason: Some(reason), at: now() });
        Ok(())
    })
}

/// The run of a ticket's check has a report: it is bound to the ticket, which now waits for a
/// person's confirmation, and a 「需重查」 mark goes. The run's own outcome and counts are kept as
/// they are.
pub fn check_ran(root: &Path, id: &str, result: RunResult) -> Result<Ticket, TicketError> {
    let mut line = Line::new(Actor::App, "check_ran").note(Some(format!("{} · {}", result.outcome, counts_text(&result.counts))));
    line.run = Some(result.run.clone());
    step(root, id, line, None, |ticket, _| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "a check runs for the ticket in progress"));
        }
        move_to(ticket, Actor::App, State::Waiting)?;
        ticket.recheck = None;
        ticket.check = Some(Check {
            state: CheckState::Ran,
            run: Some(result.run),
            outcome: Some(result.outcome),
            counts: result.counts,
            freshness: Some(result.freshness),
            reason: None,
            at: now(),
        });
        Ok(())
    })
}

/// `3 failed · 1 advisory · 12 checked · 4 not covered`, from a run's counts.
pub fn counts_text(counts: &BTreeMap<String, u64>) -> String {
    let word = |key: &str| match key {
        "unsupported" => "not covered".to_string(),
        other => other.replace('_', " "),
    };
    let order = ["failed", "advisory", "checked", "passed", "unsupported"];
    let mut parts: Vec<String> = order.iter().filter_map(|key| counts.get(*key).map(|n| format!("{n} {}", word(key)))).collect();
    parts.extend(counts.iter().filter(|(key, _)| !order.contains(&key.as_str())).map(|(key, n)| format!("{n} {}", word(key))));
    if parts.is_empty() { "no findings".to_string() } else { parts.join(" · ") }
}

#[cfg(test)]
#[path = "tickets_tests.rs"]
mod tests;
