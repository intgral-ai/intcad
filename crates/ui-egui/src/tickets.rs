//! The host side of the project's part tickets: the state the tickets view of the Assistant's page
//! is told ([`page_state`]), the person's steps its buttons ask for ([`receive`]), Start's message, the
//! note every message carries and the check bound to a ticket.
//!
//! The tickets are the project's own (`solidcraft_engine::project::tickets`). The agent drafts
//! them and asks for a ticket's check through the `ticket.*` commands; a person confirms a draft,
//! starts a confirmed ticket, marks a ticket done, and creates, edits or deletes one of their own
//! through the functions here, which the view's buttons call and no command reaches. Each such step
//! names the revision the person saw, and the store refuses it when the ticket has changed since.
//! The view's messages come in on its own channel ([`receive`]); the Assistant's view refuses them.
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
    /// Where the right rail's Tickets panel gives its place to the tickets view this frame.
    pub(crate) web_place: Option<(egui::Rect, egui::LayerId)>,
    /// Whether the host could make the tickets view's web view (`None`: not asked yet).
    pub(crate) web_host: Option<bool>,
}

/// The rail's Tickets panel, each frame it gives up `rect` to the tickets view's web view.
pub(crate) fn place(app: &mut SolveApp, ui: &egui::Ui, rect: egui::Rect) {
    app.tickets.web_place = Some((rect, ui.layer_id()));
}

/// The host says whether it could make the tickets view's web view; without one the panel says so.
pub fn set_host(app: &mut SolveApp, available: bool) {
    app.tickets.web_host = Some(available);
}

/// The host, once a frame after the app's UI: where the tickets view goes, in the window's egui
/// points, or `None` to hide it. As for the Assistant's view (`agent_chat::web::placement`): it
/// stays off the window's resize edges and off the panel's own resize grip, and it is hidden while
/// a menu, a tooltip, a dialog or a floating panel lies over its place.
#[cfg(not(target_arch = "wasm32"))]
pub fn placement(app: &mut SolveApp, ctx: &egui::Context) -> Option<egui::Rect> {
    let (rect, layer) = app.tickets.web_place.take()?;
    let rect = if app.custom_titlebar { rect.intersect(crate::titlebar::inside_resize_zones(ctx)) } else { rect };
    let grip = ctx.global_style().interaction.resize_grab_radius_side;
    let rect = rect.with_min_x(rect.left() + grip);
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return None;
    }
    (!crate::agent_chat::web::covered(ctx, layer, rect)).then_some(rect)
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

/// Confirm: a person's step on a draft they saw at revision `seen`.
#[cfg(not(target_arch = "wasm32"))]
pub fn confirm(app: &mut SolveApp, id: &str, seen: u64) {
    apply(app, |root| tickets::confirm(root, id, seen));
}

/// Done: a person's step on a ticket waiting for confirmation, seen at revision `seen`.
#[cfg(not(target_arch = "wasm32"))]
pub fn done(app: &mut SolveApp, id: &str, seen: u64) {
    apply(app, |root| tickets::done(root, id, seen));
}

/// New ticket: a person's own ticket, confirmed from the start.
#[cfg(not(target_arch = "wasm32"))]
pub fn create(app: &mut SolveApp, draft: tickets::Draft) -> Option<Ticket> {
    apply(app, |root| tickets::create(root, draft))
}

/// Edit: a person changes the words of a draft or a confirmed ticket seen at revision `seen`.
#[cfg(not(target_arch = "wasm32"))]
pub fn edit(app: &mut SolveApp, id: &str, seen: u64, edit: tickets::Edit) {
    apply(app, |root| tickets::edit(root, id, seen, edit));
}

/// Delete: a person removes a draft or a confirmed ticket seen at revision `seen`; its page closes.
#[cfg(not(target_arch = "wasm32"))]
pub fn delete(app: &mut SolveApp, id: &str, seen: u64) {
    if apply(app, |root| tickets::delete(root, id, seen)).is_some() && app.tickets.selected.as_deref() == Some(id) {
        app.tickets.selected = None;
        app.tickets.record.clear();
    }
}

/// Run check (dev-agent's addition, 2026-11-26): the person asks for the check of the ticket in
/// progress, when the agent did not, or again after a refusal.
#[cfg(not(target_arch = "wasm32"))]
pub fn ask_check(app: &mut SolveApp, id: &str) {
    apply(app, |root| tickets::ask_check(root, id, tickets::Actor::Person, None));
}

/// Start: the ticket (seen at revision `seen`) goes in progress and the Assistant gets one message
/// to work on it alone, with the ticket's chip. Only one ticket is worked on at a time.
#[cfg(not(target_arch = "wasm32"))]
pub fn start(app: &mut SolveApp, ctx: &egui::Context, id: &str, seen: u64) {
    if let Some(other) = current(app).filter(|ticket| ticket.id != id) {
        app.tickets.error = Some(trf("ui.tickets_one_at_a_time", &[("id", &other.id)]));
        return;
    }
    let Some(ticket) = apply(app, |root| tickets::start(root, id, seen)) else { return };
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

/// The tickets view's state as JSON (`TicketsState` in packages/assistant-web/src/host.ts): the
/// project's tickets as the store keeps them, the open ticket and its record, each bound run's
/// freshness now, the last refusal, and the view's words in the app's language. The page draws
/// the tickets view instead of the conversation when it is pushed this.
#[cfg(not(target_arch = "wasm32"))]
pub fn page_state(app: &mut SolveApp) -> String {
    let _language = crate::i18n::scope(&app.preferences.language);
    let project = root(app).is_some();
    refresh(app);
    if app.tickets.selected.as_ref().is_some_and(|id| !app.tickets.tickets.iter().any(|ticket| &ticket.id == id)) {
        app.tickets.selected = None;
        app.tickets.record.clear();
    }
    let tickets = &app.tickets;
    serde_json::json!({
        "view": "tickets",
        "dark": crate::theme::is_dark(),
        "language": app.preferences.language,
        "strings": page_strings(),
        "project": project,
        "tickets": tickets.tickets,
        "selected": tickets.selected,
        "record": tickets.record,
        "freshness": tickets.freshness,
        "error": tickets.error,
    })
    .to_string()
}

/// The longest list of requirement lines the page may send; the store's own limit is lower.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SENT_LINES: usize = 64;

/// The most a message from the tickets view may hold.
#[cfg(not(target_arch = "wasm32"))]
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// One message from the tickets view's own web view, as JSON. Only the tickets view's requests are
/// taken here ([`act`]); the Assistant's view has no way to them (its `web::act` refuses them).
#[cfg(not(target_arch = "wasm32"))]
pub fn receive(app: &mut SolveApp, ctx: &egui::Context, message: &str) -> Result<(), String> {
    let _language = crate::i18n::scope(&app.preferences.language);
    if message.len() > MAX_MESSAGE_BYTES {
        return Err("message too long".into());
    }
    let action: serde_json::Value = serde_json::from_str(message).map_err(|error| format!("not JSON: {error}"))?;
    let kind = action.get("type").and_then(serde_json::Value::as_str).ok_or("no type")?;
    if !kind.starts_with("ticket_") {
        return Err(format!("unknown action {kind}"));
    }
    act(app, ctx, kind, &action)?;
    ctx.request_repaint();
    Ok(())
}

/// A message of the tickets view, `{"type": "ticket_…"}` ([`receive`]):
/// `ticket_open` (`id`) and `ticket_close` choose the open ticket; `ticket_confirm`,
/// `ticket_start` and `ticket_done` (`id`, `revision`) are a person's steps; `ticket_create`
/// (`title`, `text`, `requirements`), `ticket_edit` (`id`, `revision`, `title`, `text`,
/// `requirements`) and `ticket_delete` (`id`, `revision`) a person's own changes. The store checks
/// each step against the ticket's state and the revision the person saw; a refusal is the view's
/// error. Anything else is refused with the reason.
#[cfg(not(target_arch = "wasm32"))]
fn act(app: &mut SolveApp, ctx: &egui::Context, kind: &str, action: &serde_json::Value) -> Result<(), String> {
    use serde_json::Value;
    let text = |name: &str| action.get(name).and_then(Value::as_str);
    let id = || text("id").map(str::trim).filter(|id| tickets::number(id).is_some()).ok_or(format!("{kind}: no ticket id"));
    let seen = || action.get("revision").and_then(Value::as_u64).ok_or(format!("{kind}: no revision"));
    let words = || -> Result<(String, String, Vec<String>), String> {
        let title = text("title").ok_or(format!("{kind}: no title"))?.to_string();
        let body = text("text").unwrap_or_default().to_string();
        let lines = match action.get("requirements") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(lines)) if lines.len() <= MAX_SENT_LINES => {
                lines.iter().map(|line| line.as_str().map(str::to_string).ok_or(format!("{kind}: a requirement line is not text"))).collect::<Result<_, _>>()?
            }
            Some(_) => return Err(format!("{kind}: requirements must be at most {MAX_SENT_LINES} lines of text")),
        };
        Ok((title, body, lines))
    };
    match kind {
        "ticket_open" => {
            app.tickets.selected = Some(id()?.to_string());
            app.tickets.error = None;
            // The open ticket's record is read with the tickets.
            app.tickets.seen = None;
            refresh(app);
        }
        "ticket_close" => {
            app.tickets.selected = None;
            app.tickets.record.clear();
            app.tickets.error = None;
        }
        "ticket_confirm" => confirm(app, id()?, seen()?),
        "ticket_start" => start(app, ctx, id()?, seen()?),
        "ticket_done" => done(app, id()?, seen()?),
        "ticket_create" => {
            let (title, text, requirements) = words()?;
            create(app, tickets::Draft { title, text, requirements, parent: None, part: None });
        }
        "ticket_edit" => {
            let (title, text, requirements) = words()?;
            edit(app, id()?, seen()?, tickets::Edit { title, text, requirements });
        }
        "ticket_delete" => delete(app, id()?, seen()?),
        other => return Err(format!("unknown action {other}")),
    }
    Ok(())
}

/// The tickets view's words, by the names the page gives them (`TicketStringKey` in
/// packages/assistant-web/src/host.ts; a test keeps the two equal).
#[cfg(not(target_arch = "wasm32"))]
fn page_strings() -> serde_json::Map<String, serde_json::Value> {
    [
        ("title", "ui.tickets_title"),
        ("newTicket", "ui.tickets_new"),
        ("empty", "ui.tickets_empty"),
        ("noProject", "ui.tickets_no_project"),
        ("previous", "ui.tickets_previous"),
        ("next", "ui.tickets_next"),
        ("more", "ui.tickets_more"),
        ("stateDraft", "ui.tickets_state_draft"),
        ("stateConfirmed", "ui.tickets_state_confirmed"),
        ("stateInProgress", "ui.tickets_state_in_progress"),
        ("stateWaiting", "ui.tickets_state_waiting"),
        ("stateDone", "ui.tickets_state_done"),
        ("stateBlocked", "ui.tickets_state_blocked"),
        ("properties", "ui.tickets_properties"),
        ("part", "ui.tickets_part"),
        ("noPart", "ui.tickets_no_part"),
        ("check", "ui.tickets_check"),
        ("noCheck", "ui.tickets_no_check"),
        ("checkAsked", "ui.tickets_check_asked"),
        ("checkRunning", "ui.tickets_check_running"),
        ("checkRefused", "ui.tickets_check_refused"),
        // The run's own words: the DFM check's names for its outcome and counts.
        ("checkTitle", "ui.dfm_check.title"),
        ("checkDone", "ui.dfm_check.done"),
        ("checkStale", "ui.dfm_check.stale"),
        ("failed", "ui.dfm_check.failed"),
        ("advisory", "ui.dfm_check.advisory"),
        ("checked", "ui.dfm_check.checked"),
        ("notCovered", "ui.dfm_check.not_covered"),
        ("requirements", "ui.tickets_requirements"),
        ("activity", "ui.tickets_activity"),
        ("actorPerson", "ui.tickets_actor_person"),
        ("actorAgent", "ui.tickets_actor_agent"),
        ("actorApp", "ui.tickets_actor_app"),
        ("didDrafted", "ui.tickets_did_drafted"),
        ("didCreated", "ui.tickets_did_created"),
        ("didEdited", "ui.tickets_did_edited"),
        ("didConfirmed", "ui.tickets_did_confirmed"),
        ("didStarted", "ui.tickets_did_started"),
        ("didCheckAsked", "ui.tickets_did_check_asked"),
        ("didCheckStarted", "ui.tickets_did_check_started"),
        ("didCheckRan", "ui.tickets_did_check_ran"),
        ("didCheckRefused", "ui.tickets_did_check_refused"),
        ("didBlocked", "ui.tickets_did_blocked"),
        ("didUnblocked", "ui.tickets_did_unblocked"),
        ("didDone", "ui.tickets_did_done"),
        ("confirm", "ui.tickets_confirm"),
        ("start", "ui.tickets_start"),
        ("done", "ui.tickets_done"),
        ("delete", "ui.delete"),
        ("cancel", "ui.cancel"),
        ("create", "ui.tickets_create"),
        ("fieldTitle", "ui.tickets_field_title"),
        ("fieldText", "ui.tickets_field_text"),
        ("fieldLines", "ui.tickets_field_lines"),
        ("decline", "ui.tickets_decline"),
        ("proposed", "ui.tickets_proposed"),
        ("state", "ui.tickets_state"),
        ("waitsFor", "ui.tickets_waits_for"),
        ("setWaits", "ui.tickets_set_waits"),
        ("worker", "ui.tickets_worker"),
        ("workerNone", "ui.tickets_worker_none"),
        ("workMyself", "ui.tickets_work_myself"),
        ("askCheck", "ui.tickets_ask_check"),
        ("whyOffline", "ui.tickets_why_offline"),
        ("whyBusy", "ui.tickets_why_busy"),
        ("whyWaits", "ui.tickets_why_waits"),
        ("stillWaits", "ui.tickets_still_waits"),
        ("whySubs", "ui.tickets_why_subs"),
        ("whyCheck", "ui.tickets_why_check"),
        ("notChecked", "ui.tickets_not_checked"),
        ("noApplicableCheck", "ui.tickets_no_applicable_check"),
        ("attachments", "ui.tickets_attachments"),
        ("attach", "ui.tickets_attach"),
        ("subTickets", "ui.tickets_sub_tickets"),
        ("addSub", "ui.tickets_add_sub"),
        ("parent", "ui.tickets_parent"),
        ("assign", "ui.tickets_assign"),
        ("useSelection", "ui.tickets_use_selection"),
        ("selectFirst", "ui.tickets_select_first"),
        ("filterPart", "ui.tickets_filter_part"),
        ("clear", "ui.clear"),
        ("askDraft", "ui.tickets_ask_draft"),
        ("deleted", "ui.tickets_deleted"),
        ("noDeleted", "ui.tickets_no_deleted"),
        ("restore", "ui.tickets_restore"),
        ("deletedToast", "ui.tickets_deleted_toast"),
        ("undo", "ui.undo"),
        ("expand", "ui.tickets_expand"),
        ("collapse", "ui.tickets_collapse"),
        ("addText", "ui.tickets_add_text"),
    ]
    .into_iter()
    .map(|(name, key)| (name.to_string(), serde_json::Value::from(tr(key))))
    .collect()
}

#[cfg(test)]
#[path = "tickets_tests.rs"]
mod tests;
