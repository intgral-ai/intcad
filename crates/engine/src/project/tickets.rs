//! Part tickets: what each part of a project has to meet, kept by the app in
//! `.intcad/tickets/` (the folder holds only what the app generates).
//!
//! A ticket stands for a part. The project has one top ticket that sets it up and part tickets
//! under it, one level deep. Each ticket is one JSON file, `T-<n>.json`; every step that changes a
//! ticket also appends one line to `events.jsonl`, which is never rewritten, so the record says who
//! changed what and when.
//!
//! Who may make which move is decided here, not by the caller: the agent drafts, asks for the
//! check and says when it is blocked; only a person confirms a draft, starts a ticket and marks it
//! done (the panel's buttons; there is no command for those three); the app binds the check's run.
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
/// The layout this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// Limits on what a caller may store: parameters are hostile.
pub const MAX_TICKETS: usize = 200;
pub const MAX_TITLE: usize = 200;
pub const MAX_TEXT: usize = 8000;
pub const MAX_LINES: usize = 40;
pub const MAX_LINE: usize = 400;
pub const MAX_REASON: usize = 1000;
/// The most record lines one read returns, newest last.
pub const MAX_EVENTS: usize = 2000;

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
    /// Written by the agent; a person has not confirmed it, so nobody works on it.
    Draft,
    /// Confirmed by a person; it can be started.
    Confirmed,
    /// Started by a person; the agent works on it.
    InProgress,
    /// Its check has run; it waits for a person to confirm it.
    Waiting,
    /// A person confirmed it after its check ran.
    Done,
    /// The agent stopped on it, with a reason.
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

/// The part a ticket stands for: a component of the design, or a body in a design of one part.
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
    /// The agent asked for it; the app has not started it yet.
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

/// One ticket, as its file holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub schema_version: u32,
    pub id: String,
    /// The top ticket has none; a part ticket names the top ticket.
    #[serde(default)]
    pub parent: Option<String>,
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// The requirement lines it carries, each one citable on its own.
    #[serde(default)]
    pub requirements: Vec<String>,
    #[serde(default)]
    pub part: Option<Part>,
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
    /// `drafted`, `confirmed`, `started`, `check_asked`, `check_started`, `check_ran`,
    /// `check_refused`, `blocked`, `unblocked`, `done`.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    pub revision: u64,
}

/// What a draft is made of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    pub text: String,
    pub requirements: Vec<String>,
    /// The top ticket's id; none: under the top ticket, or the top ticket itself when there is none.
    pub parent: Option<String>,
    pub part: Option<Part>,
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

fn now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    crate::validation::timestamp(ms)
}

/// Every ticket of the project, by number. A project with no tickets has an empty list.
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

/// One ticket.
pub fn load(root: &Path, id: &str) -> Result<Ticket, TicketError> {
    let path = file(root, id)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(TicketError::NoSuchTicket(id.to_string())),
        Err(error) => return Err(io(&path, &error)),
    };
    let ticket: Ticket =
        serde_json::from_slice(&bytes).map_err(|error| TicketError::Damaged { file: path.display().to_string(), message: error.to_string() })?;
    if ticket.schema_version > SCHEMA_VERSION {
        return Err(TicketError::Damaged {
            file: path.display().to_string(),
            message: format!("schema version {} is newer than this build's {SCHEMA_VERSION}", ticket.schema_version),
        });
    }
    if ticket.id != id {
        return Err(TicketError::Damaged { file: path.display().to_string(), message: format!("the file holds ticket {}", ticket.id) });
    }
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

/// Write the ticket, then its record line. The line names the revision the ticket now has.
fn commit(
    root: &Path,
    ticket: &Ticket,
    actor: Actor,
    action: &str,
    from: Option<State>,
    note: Option<String>,
    run: Option<String>,
) -> Result<(), TicketError> {
    write(root, ticket)?;
    let event = Event {
        at: ticket.updated.clone(),
        actor,
        ticket: ticket.id.clone(),
        action: action.to_string(),
        from,
        to: Some(ticket.state).filter(|to| Some(*to) != from),
        note,
        run,
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

/// Write a new draft. The agent's only way to add a ticket; nothing it drafts can be worked on
/// before a person confirms it.
pub fn draft(root: &Path, draft: Draft, actor: Actor) -> Result<Ticket, TicketError> {
    let title = clean(&draft.title, MAX_TITLE, "title")?;
    if title.is_empty() {
        return Err(TicketError::Invalid("a ticket needs a title: the part's name and what it must meet".into()));
    }
    let text = clean(&draft.text, MAX_TEXT, "text")?;
    if draft.requirements.len() > MAX_LINES {
        return Err(TicketError::Invalid(format!("a ticket carries at most {MAX_LINES} requirement lines")));
    }
    let requirements = draft
        .requirements
        .iter()
        .map(|line| clean(line, MAX_LINE, "requirement line"))
        .filter(|line| !matches!(line, Ok(l) if l.is_empty()))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(part) = &draft.part {
        if !matches!(part.kind.as_str(), "component" | "body") {
            return Err(TicketError::Invalid(format!("`{}` is not a part kind; use component or body", part.kind)));
        }
        if part.id.trim().is_empty() || part.id.chars().count() > 128 {
            return Err(TicketError::Invalid("a part needs the id or name of its component or body".into()));
        }
    }
    let existing = list(root)?;
    if existing.len() >= MAX_TICKETS {
        return Err(TicketError::Refused(format!("a project holds at most {MAX_TICKETS} tickets")));
    }
    let top = existing.iter().find(|ticket| ticket.parent.is_none()).map(|ticket| ticket.id.clone());
    let parent = match draft.parent.as_deref().map(str::trim).filter(|parent| !parent.is_empty()) {
        Some(parent) => {
            let ticket = existing.iter().find(|ticket| ticket.id == parent).ok_or_else(|| TicketError::NoSuchTicket(parent.to_string()))?;
            if ticket.parent.is_some() {
                return Err(TicketError::Refused(format!("{parent} is a part ticket; part tickets go under the top ticket, one level deep")));
            }
            Some(parent.to_string())
        }
        None => top,
    };
    let next = existing.iter().filter_map(|ticket| number(&ticket.id)).max().unwrap_or(0).saturating_add(1);
    let at = now();
    let ticket = Ticket {
        schema_version: SCHEMA_VERSION,
        id: format!("T-{next}"),
        parent,
        title,
        text,
        requirements,
        part: draft.part,
        state: State::Draft,
        check: None,
        blocked: None,
        revision: 1,
        created: at.clone(),
        updated: at,
    };
    commit(root, &ticket, actor, "drafted", None, None, None)?;
    Ok(ticket)
}

/// Load, check the move, change, write and record, in one step.
fn step(
    root: &Path,
    id: &str,
    actor: Actor,
    action: &str,
    note: Option<String>,
    run: Option<String>,
    change: impl FnOnce(&mut Ticket) -> Result<(), TicketError>,
) -> Result<Ticket, TicketError> {
    let mut ticket = load(root, id)?;
    let from = ticket.state;
    change(&mut ticket)?;
    ticket.revision = ticket.revision.saturating_add(1);
    ticket.updated = now();
    commit(root, &ticket, actor, action, Some(from), note, run)?;
    Ok(ticket)
}

fn refused(ticket: &Ticket, what: &str) -> TicketError {
    TicketError::Refused(format!("{} is {}: {what}", ticket.id, ticket.state.as_str().replace('_', " ")))
}

/// A person confirms a draft. Only the panel's Confirm button calls this.
pub fn confirm(root: &Path, id: &str) -> Result<Ticket, TicketError> {
    step(root, id, Actor::Person, "confirmed", None, None, |ticket| {
        if ticket.state != State::Draft {
            return Err(refused(ticket, "only a draft is confirmed"));
        }
        ticket.state = State::Confirmed;
        Ok(())
    })
}

/// A person starts a confirmed ticket, which sends the agent to work on it. Only the panel's Start
/// button calls this. A blocked ticket is started again the same way.
pub fn start(root: &Path, id: &str) -> Result<Ticket, TicketError> {
    step(root, id, Actor::Person, "started", None, None, |ticket| {
        if !matches!(ticket.state, State::Confirmed | State::Blocked) {
            return Err(refused(ticket, "only a confirmed or blocked ticket is started"));
        }
        ticket.state = State::InProgress;
        ticket.blocked = None;
        Ok(())
    })
}

/// The agent says it has made the ticket's changes and asks for its check (or a person asks for it
/// from the panel).
pub fn ask_check(root: &Path, id: &str, actor: Actor, note: Option<String>) -> Result<Ticket, TicketError> {
    let note = note.map(|note| clean(&note, MAX_REASON, "note")).transpose()?.filter(|note| !note.is_empty());
    step(root, id, actor, "check_asked", note, None, |ticket| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "the check is asked for on the ticket in progress"));
        }
        ticket.check =
            Some(Check { state: CheckState::Asked, run: None, outcome: None, counts: BTreeMap::new(), freshness: None, reason: None, at: now() });
        Ok(())
    })
}

/// The agent stops on a ticket in progress and says why.
pub fn block(root: &Path, id: &str, reason: &str) -> Result<Ticket, TicketError> {
    let reason = clean(reason, MAX_REASON, "reason")?;
    if reason.is_empty() {
        return Err(TicketError::Invalid("say why the ticket is blocked".into()));
    }
    step(root, id, Actor::Agent, "blocked", Some(reason.clone()), None, |ticket| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "only the ticket in progress can be blocked"));
        }
        ticket.state = State::Blocked;
        ticket.blocked = Some(reason);
        Ok(())
    })
}

/// The agent takes a blocked ticket up again.
pub fn unblock(root: &Path, id: &str, note: Option<String>) -> Result<Ticket, TicketError> {
    let note = note.map(|note| clean(&note, MAX_REASON, "note")).transpose()?.filter(|note| !note.is_empty());
    step(root, id, Actor::Agent, "unblocked", note, None, |ticket| {
        if ticket.state != State::Blocked {
            return Err(refused(ticket, "only a blocked ticket is unblocked"));
        }
        ticket.state = State::InProgress;
        ticket.blocked = None;
        Ok(())
    })
}

/// The app started the run of a ticket's asked-for check.
pub fn check_started(root: &Path, id: &str, run: &str) -> Result<Ticket, TicketError> {
    let run = run.to_string();
    step(root, id, Actor::App, "check_started", None, Some(run.clone()), |ticket| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "a check runs for the ticket in progress"));
        }
        ticket.check = Some(Check {
            state: CheckState::Running,
            run: Some(run),
            outcome: None,
            counts: BTreeMap::new(),
            freshness: None,
            reason: None,
            at: now(),
        });
        Ok(())
    })
}

/// The run of a ticket's check was refused or did not finish: the ticket stays in progress, with
/// the reason on it.
pub fn check_refused(root: &Path, id: &str, reason: &str) -> Result<Ticket, TicketError> {
    let reason: String = reason.trim().chars().take(MAX_REASON).collect();
    step(root, id, Actor::App, "check_refused", Some(reason.clone()), None, |ticket| {
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
/// person's confirmation. The run's own outcome and counts are kept as they are.
pub fn check_ran(root: &Path, id: &str, result: RunResult) -> Result<Ticket, TicketError> {
    let note = format!("{} · {}", result.outcome, counts_text(&result.counts));
    let run = result.run.clone();
    step(root, id, Actor::App, "check_ran", Some(note), Some(run), |ticket| {
        if ticket.state != State::InProgress {
            return Err(refused(ticket, "a check runs for the ticket in progress"));
        }
        ticket.state = State::Waiting;
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

/// A person marks a ticket done once its check has run. Only the panel's Done button calls this.
pub fn done(root: &Path, id: &str) -> Result<Ticket, TicketError> {
    let ticket = load(root, id)?;
    let run = ticket.check.as_ref().and_then(|check| check.run.clone());
    step(root, id, Actor::Person, "done", None, run, |ticket| {
        if ticket.state != State::Waiting {
            return Err(refused(ticket, "a ticket is done after its check has run"));
        }
        if !ticket.check.as_ref().is_some_and(|check| check.state == CheckState::Ran && check.run.is_some()) {
            return Err(refused(ticket, "it has no check run bound to it"));
        }
        ticket.state = State::Done;
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
