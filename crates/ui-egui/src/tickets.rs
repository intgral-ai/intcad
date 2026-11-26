//! The host side of the project's part tickets: what a Tickets view reads, the person's steps it
//! calls, Start's message, the note every message carries and the check bound to a ticket.
//!
//! The tickets are the project's own (`solidcraft_engine::project::tickets`). The agent drafts
//! them and asks for a ticket's check through the `ticket.*` commands; a person confirms a draft,
//! starts a confirmed ticket and marks a ticket done through the functions here, which a view's
//! buttons call and no command reaches. (The view itself is not built: the egui drawing was
//! rejected on 2026-11-26 for a page made of Mastra's components.)
//! Start sends the Assistant one message about that ticket, with its chip, and while a ticket is
//! in progress every message carries it in full and the others by id, title and state ([`note`]).
//! When the agent asks for the check, the app runs the Review panel's own DFM check (the same
//! controlled entry, so a refusal is the same refusal) and binds the run to the ticket, which then
//! waits for the person with the run's own outcome words: no tick and no "passed" of its own.
//!
//! Experiment (branch `experiment/part-tickets`, 2026-11-26): the demo of the part-ticket loop.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use solidcraft_engine::project::tickets::{self, CheckState, Event, State as TicketState, Ticket};

use crate::SolveApp;
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use crate::i18n::{format as trf, tr};

/// What the host keeps between frames for a Tickets view.
#[derive(Default)]
pub struct State {
    /// The project's tickets, by number, as last read.
    pub tickets: Vec<Ticket>,
    /// The ticket whose details are open.
    pub selected: Option<String>,
    /// The open ticket's record, oldest first.
    pub record: Vec<Event>,
    /// Why the tickets could not be read or a button's step was refused.
    pub error: Option<String>,
    /// The ticket whose check run is in flight, and that run.
    pub checking: Option<(String, String)>,
    /// Each bound run's freshness now, read again when the design changes.
    pub freshness: BTreeMap<String, String>,
    /// The project folder the tickets were read from.
    root: Option<PathBuf>,
    /// The design file that folder was found from.
    path: Option<String>,
    /// The store's write count when the tickets were read.
    seen: Option<u64>,
    /// The design revision the freshness was read at (the browser build reads none).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    stamp: Option<(String, u64)>,
}

/// The folder of the open design's project, if it is in one.
pub fn root(app: &mut SolveApp) -> Option<PathBuf> {
    if app.tickets.path != app.session.path {
        app.tickets.path = app.session.path.clone();
        app.tickets.root = app.session.path.as_deref().and_then(|path| solidcraft_engine::project::Project::locate(std::path::Path::new(path)));
        app.tickets.seen = None;
    }
    app.tickets.root.clone()
}

/// Read the tickets again when the store was written or the project changed.
#[cfg(not(target_arch = "wasm32"))]
pub fn refresh(app: &mut SolveApp) {
    let Some(root) = root(app) else {
        app.tickets.tickets.clear();
        app.tickets.record.clear();
        return;
    };
    let generation = tickets::generation();
    if app.tickets.seen != Some(generation) {
        app.tickets.seen = Some(generation);
        match tickets::list(&root) {
            Ok(list) => {
                app.tickets.tickets = list;
                app.tickets.error = None;
            }
            Err(error) => app.tickets.error = Some(error.to_string()),
        }
        app.tickets.record = match &app.tickets.selected {
            Some(id) => tickets::events(&root, Some(id)).unwrap_or_default(),
            None => Vec::new(),
        };
        app.tickets.stamp = None;
    }
    let stamp = app.session.document_stamp();
    let stamp = (stamp.document_id, stamp.revision);
    if app.tickets.stamp.as_ref() != Some(&stamp) {
        app.tickets.stamp = Some(stamp);
        let runs: Vec<String> = app.tickets.tickets.iter().filter_map(|t| t.check.as_ref().and_then(|c| c.run.clone())).collect();
        let mut freshness = BTreeMap::new();
        for run in runs {
            // Read straight from the session: a run the project no longer holds is no toast here.
            let Ok(reply) = app.session.execute("inspect.dfm", &serde_json::json!({ "run": run })) else { continue };
            let card = crate::dfm_chat::Card::from_inspect(&reply);
            if card.run_id == run && !card.freshness.is_empty() {
                freshness.insert(run, card.freshness);
            }
        }
        app.tickets.freshness = freshness;
    }
}

#[cfg(target_arch = "wasm32")]
pub fn refresh(_app: &mut SolveApp) {}

/// The ticket in progress: the one the agent works on.
pub fn current(app: &SolveApp) -> Option<&Ticket> {
    app.tickets.tickets.iter().find(|ticket| ticket.state == TicketState::InProgress)
}

/// Run a person's or the app's step on the store and read it again; a refusal is kept as the view's
/// error.
#[cfg(not(target_arch = "wasm32"))]
fn apply(app: &mut SolveApp, step: impl FnOnce(&std::path::Path) -> Result<Ticket, tickets::TicketError>) -> Option<Ticket> {
    let root = root(app)?;
    match step(&root) {
        Ok(ticket) => {
            app.tickets.error = None;
            refresh(app);
            Some(ticket)
        }
        Err(error) => {
            app.tickets.error = Some(error.to_string());
            None
        }
    }
}

/// Confirm: a person's step on a draft.
#[cfg(not(target_arch = "wasm32"))]
pub fn confirm(app: &mut SolveApp, id: &str) {
    apply(app, |root| tickets::confirm(root, id));
}

/// Done: a person's step on a ticket waiting for confirmation.
#[cfg(not(target_arch = "wasm32"))]
pub fn done(app: &mut SolveApp, id: &str) {
    apply(app, |root| tickets::done(root, id));
}

/// Run check (dev-agent's addition, 2026-11-26): the person asks for the check of the ticket in
/// progress, when the agent did not, or again after a refusal.
#[cfg(not(target_arch = "wasm32"))]
pub fn ask_check(app: &mut SolveApp, id: &str) {
    apply(app, |root| tickets::ask_check(root, id, tickets::Actor::Person, None));
}

/// Start: the ticket goes in progress and the Assistant gets one message to work on it alone,
/// with the ticket's chip. Only one ticket is worked on at a time.
#[cfg(not(target_arch = "wasm32"))]
pub fn start(app: &mut SolveApp, ctx: &egui::Context, id: &str) {
    if let Some(other) = current(app).filter(|ticket| ticket.id != id) {
        app.tickets.error = Some(trf("ui.tickets_one_at_a_time", &[("id", &other.id)]));
        return;
    }
    let Some(ticket) = apply(app, |root| tickets::start(root, id)) else { return };
    let chip = crate::agent_chat::attach::Attachment {
        id: 0,
        name: format!("{} · {}", ticket.id, ticket.title.chars().take(40).collect::<String>()),
        content: crate::agent_chat::attach::Content::Text(std::sync::Arc::new(summary(&ticket))),
        thumb: None,
    };
    if let Err(reason) = crate::agent_chat::attach_ready(app, chip) {
        crate::agent_chat::notify(app, reason, true);
    }
    let text = trf("ui.tickets_start_message", &[("id", &ticket.id), ("title", &ticket.title)]);
    if let Err(reason) = crate::agent_chat::send_with(app, ctx, text, None) {
        if !reason.is_empty() {
            app.tickets.error = Some(reason);
        }
    }
}

/// A ticket in full, as the agent reads it.
#[cfg(not(target_arch = "wasm32"))]
fn summary(ticket: &Ticket) -> String {
    let mut out = format!("{} \"{}\" ({})", ticket.id, ticket.title, ticket.state.as_str());
    if let Some(part) = &ticket.part {
        out += &format!("\nPart: {} {}", part.kind, part.id);
    }
    if let Some(parent) = &ticket.parent {
        out += &format!("\nUnder: {parent}");
    }
    if !ticket.text.is_empty() {
        out += &format!("\n{}", ticket.text);
    }
    if !ticket.requirements.is_empty() {
        out += "\nRequirement lines:";
        for line in &ticket.requirements {
            out += &format!("\n- {line}");
        }
    }
    out
}

/// The note every message carries while the project has tickets: the ticket in progress in full
/// and what to do when its changes are made; the others by id, title and state. With no tickets,
/// how to draft them. Outside a project, nothing.
pub fn note(app: &mut SolveApp) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        root(app)?;
        refresh(app);
        let all = &app.tickets.tickets;
        if all.is_empty() {
            return Some(
                "Part tickets: this project has none yet. When asked to make tickets, or to read requirements.md, draft them with the \
                 execute tool, command ticket.draft: first one top ticket for the project, then one per part (title, text, requirement \
                 lines with numbers and units, part: {kind: component | body, id}). A person confirms each draft in the Tickets panel."
                    .into(),
            );
        }
        let mut out = String::from(
            "Part tickets of this project (a person confirms drafts, starts tickets and marks them done in the Tickets panel; no command does those).",
        );
        match current(app) {
            Some(ticket) => {
                out += &format!(
                    "\nYou are working on {} alone. Change only what it asks for.\n{}\nWhen its changes are made, call execute with \
                     ticket.update {{\"id\": \"{}\", \"action\": \"check\"}}: the app runs its check and the ticket waits for the person. \
                     If you cannot go on, use action block with the reason.",
                    ticket.id,
                    summary(ticket),
                    ticket.id
                );
            }
            None => out += "\nNo ticket is in progress: do not change the model for a ticket until a person starts it.",
        }
        let others: Vec<String> = all
            .iter()
            .filter(|ticket| current(app).is_none_or(|working| working.id != ticket.id))
            .map(|ticket| format!("{} {} ({})", ticket.id, ticket.title, ticket.state.as_str()))
            .collect();
        if !others.is_empty() {
            out += &format!("\nOther tickets: {}.", others.join("; "));
        }
        Some(out)
    }
}

/// Start the run of a ticket's check through the Review panel's own controlled entry: the run it
/// reserved, or why it did not start (the entry's own refusal).
#[cfg(not(target_arch = "wasm32"))]
fn start_run(app: &mut SolveApp) -> Result<String, String> {
    crate::dfm_review::start(app);
    match (app.dfm_review.running, app.dfm_review.mapped_run.clone()) {
        (true, Some(run)) => Ok(run),
        _ => Err(app.dfm_review.error.clone().unwrap_or_else(|| tr("ui.tickets_check_not_started").to_string())),
    }
}

/// Where the run of a ticket's check stands: `None` while it runs; its report once it has one;
/// why it did not finish otherwise. The Review panel's own poll reads the run each frame.
#[cfg(not(target_arch = "wasm32"))]
fn run_result(app: &mut SolveApp, run: &str) -> Option<Result<crate::dfm_chat::Card, String>> {
    if app.dfm_review.running {
        return None;
    }
    let card = app.dfm_review.card.clone().filter(|card| !run.is_empty() && card.run_id == run);
    Some(match (app.dfm_review.error.clone(), card) {
        (None, Some(card)) => Ok(card),
        (error, _) => Err(error.unwrap_or_else(|| tr("ui.dfm_review_did_not_finish").to_string())),
    })
}

/// Each frame: when a ticket's check was asked for, start its run; when that run has a report,
/// bind it and the ticket waits for the person; a refusal or a run that did not finish stays on
/// the ticket with its reason.
#[cfg(not(target_arch = "wasm32"))]
pub fn tick(app: &mut SolveApp) {
    if root(app).is_none() {
        return;
    }
    refresh(app);
    match app.tickets.checking.clone() {
        None => {
            let asked = app
                .tickets
                .tickets
                .iter()
                .find(|t| t.state == TicketState::InProgress && t.check.as_ref().is_some_and(|c| c.state == CheckState::Asked))
                .map(|t| t.id.clone());
            let Some(id) = asked else { return };
            if app.dfm_review.running {
                // A check started by hand is still running: wait for it to end.
                return;
            }
            match start_run(app) {
                Ok(run) => {
                    app.tickets.checking = Some((id.clone(), run.clone()));
                    apply(app, |root| tickets::check_started(root, &id, &run));
                }
                Err(reason) => {
                    apply(app, |root| tickets::check_refused(root, &id, &reason));
                }
            }
        }
        Some((id, run)) => match run_result(app, &run) {
            None => {}
            Some(Ok(card)) => {
                app.tickets.checking = None;
                let counts = card
                    .counts()
                    .as_object()
                    .map(|counts| counts.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect())
                    .unwrap_or_default();
                // The run's own words: its card state (`done`), or its outcome when that is not the
                // newest run's (`superseded`, `cancelled`, `failed`).
                let outcome = card.outcome.clone().filter(|outcome| outcome != "current").unwrap_or_else(|| card.status.clone());
                let result = tickets::RunResult { run: card.run_id.clone(), outcome, counts, freshness: card.freshness.clone() };
                apply(app, |root| tickets::check_ran(root, &id, result));
            }
            Some(Err(reason)) => {
                app.tickets.checking = None;
                apply(app, |root| tickets::check_refused(root, &id, &reason));
            }
        },
    }
}

#[cfg(target_arch = "wasm32")]
pub fn tick(_app: &mut SolveApp) {}

#[cfg(test)]
#[path = "tickets_tests.rs"]
mod tests;
