//! The host side of the project's part tickets: the state the tickets view of the Assistant's page
//! is told ([`page_state`]), the person's steps its buttons ask for ([`receive`]), Start's message, the
//! note every message carries and the check bound to a ticket.
//!
//! The tickets are the project's own (`solidcraft_engine::project::tickets`). The agent drafts
//! them, sets their states and says it has finished the ticket it works through the `ticket.*`
//! commands; a person accepts or declines a draft, hands a confirmed ticket to the Assistant or
//! works it, starts a check, sets a state, and creates, edits, deletes or restores a ticket through
//! the functions here, which the view's buttons call and no command reaches. Each such step names the revision the person saw, and the store refuses it
//! when the ticket has changed since. The view's messages come in on its own channel ([`receive`]);
//! the Assistant's view refuses them. Start sends the Assistant one message about that ticket, with
//! its chip, and while the Assistant works a ticket every message carries it in full and the others
//! by id, title and state ([`note`]). When a person asks for the check, the app runs the Review
//! panel's own DFM check (the same controlled entry, so a refusal is the same refusal) and binds the
//! run to the ticket, which then waits for the person with the run's own outcome words: no tick and
//! no "passed" of its own.
//!
//! Experiment (branch `experiment/part-tickets`, 2026-11-26): the demo of the part-ticket loop.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

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
    /// A refusal the page says in the person's words (`{kind, …}`), alongside the store's `error`.
    pub refusal: Option<serde_json::Value>,
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
    /// The view's latest requests and how each went: the host's count of requests, the page's own
    /// number for it, and whether the step was taken (a refusal's reason is `error`). Several, as
    /// the page may send more than one between two states.
    pub answered: Vec<(u64, Option<u64>, bool)>,
    /// The open ticket is shown large over the window.
    pub expanded: bool,
    /// A part picked in the model: the view lists only the tickets that bear on it.
    pub filter: Option<tickets::Part>,
    /// The tickets whose 3D picture the app has tried to take, so a failed try is not repeated
    /// every frame.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    viewed: Vec<String>,
    /// The model's selection as parts when the view was last told it: a change sets `filter`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    picked: Vec<tickets::Part>,
    /// Files taken into the project for the ticket being created, by their paths there.
    pub adding: Vec<String>,
    /// The picture attachments of the tickets, by path: the key they are served by and their bytes.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pictures: BTreeMap<String, (String, Arc<Vec<u8>>)>,
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

/// A ticket is open large over the window: the Assistant's view gives way to it, as it does to
/// the picture viewer, and comes back where it was when the ticket closes or goes back to the rail.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn open_large(app: &SolveApp) -> bool {
    app.tickets.expanded && app.tickets.selected.is_some()
}

/// The host, once a frame after the app's UI: where the tickets view goes, in the window's egui
/// points, or `None` to hide it. In the rail, as for the Assistant's view
/// (`agent_chat::web::placement`): it stays off the window's resize edges and off the panel's own
/// resize grip, and it is hidden while a menu, a tooltip, a dialog or a floating panel lies over its
/// place. With a ticket open large it lies over the window below the application bar (the page dims
/// the rest), and only a menu, a popup or a dialog of the app hides it.
#[cfg(not(target_arch = "wasm32"))]
pub fn placement(app: &mut SolveApp, ctx: &egui::Context) -> Option<egui::Rect> {
    let (rect, layer) = app.tickets.web_place.take()?;
    if open_large(app) {
        let window = ctx.content_rect();
        // The rail begins under the application bar: so does the ticket opened large.
        let rect = window.with_min_y(app.shell.right_rail.map_or(window.top(), |rail| rail.top().max(window.top())));
        let rect = if app.custom_titlebar { rect.intersect(crate::titlebar::inside_resize_zones(ctx)) } else { rect };
        let popup = ctx.memory(|memory| {
            memory.layer_ids().any(|other| {
                other.order >= egui::Order::Foreground && memory.areas().is_visible(&other) && memory.area_rect(other.id).is_some_and(|area| area.intersects(rect))
            })
        });
        return (rect.is_positive() && !popup && !crate::agent_chat::gallery_open(app)).then_some(rect);
    }
    let rect = if app.custom_titlebar { rect.intersect(crate::titlebar::inside_resize_zones(ctx)) } else { rect };
    let grip = ctx.global_style().interaction.resize_grab_radius_side;
    let rect = rect.with_min_x(rect.left() + grip);
    // The picture viewer has the window and its keys while it is open, as for the Assistant's view.
    if rect.width() < 1.0 || rect.height() < 1.0 || crate::agent_chat::gallery_open(app) {
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
                // A refusal's reason stays until the person's next request, whoever writes meanwhile.
                app.tickets.tickets = list;
                read_pictures(&mut app.tickets, &root);
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
        mark_stale(app, &root);
    }
}

/// A done ticket whose bound run the model has moved past (its freshness is no longer `current`)
/// is marked 「需重查」: its state stays, and its check is out of date until a new run is bound.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn mark_stale(app: &mut SolveApp, root: &std::path::Path) {
    let stale: Vec<String> = app
        .tickets
        .tickets
        .iter()
        .filter(|t| t.state == TicketState::Done && !t.deleted && t.recheck.is_none() && tickets::applies(t))
        .filter(|t| t.check.as_ref().and_then(|c| c.run.as_ref()).and_then(|run| app.tickets.freshness.get(run)).is_some_and(|now| now != "current"))
        .map(|t| t.id.clone())
        .collect();
    for id in stale {
        if let Err(error) = tickets::mark_recheck(root, &id, "the model changed after its check ran") {
            log::warn!("{id} was not marked for a re-check: {error}");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn refresh(_app: &mut SolveApp) {}

/// The largest picture attachment the view is served; a larger one shows as a file.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SERVED_PICTURE: u64 = 16 * 1024 * 1024;

/// Whether a project path names a picture the view can show, by its extension.
#[cfg(not(target_arch = "wasm32"))]
fn picture_path(path: &str) -> bool {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();
    matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
}

/// Keep the pictures among the live tickets' attachments and the files taken in for a new ticket
/// read, by path: a path read before is kept, one nothing names any more goes, and a file too large
/// or not a picture is left out (it shows as a file).
#[cfg(not(target_arch = "wasm32"))]
fn read_pictures(state: &mut State, root: &std::path::Path) {
    let live = state.tickets.iter().filter(|t| !t.deleted).flat_map(|t| t.attachments.iter());
    let named: Vec<String> = live.chain(state.adding.iter()).filter(|path| picture_path(path)).cloned().collect();
    let held = &mut state.pictures;
    held.retain(|path, _| named.contains(path));
    for path in &named {
        if held.contains_key(path) {
            continue;
        }
        let full = path.split('/').fold(root.to_path_buf(), |full, name| full.join(name));
        let small = std::fs::metadata(&full).is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_SERVED_PICTURE);
        if let Some(bytes) = small.then(|| std::fs::read(&full).ok()).flatten() {
            held.insert(path.clone(), (crate::agent_chat::web::picture_key(&bytes), Arc::new(bytes)));
        }
    }
}

/// The pictures the tickets view may show now, by the key [`page_state`] gives each; the host
/// serves the bytes itself, and only those that are pictures.
#[cfg(not(target_arch = "wasm32"))]
pub fn pictures(app: &SolveApp) -> Vec<(String, Arc<Vec<u8>>)> {
    app.tickets.pictures.values().map(|(key, bytes)| (key.clone(), Arc::clone(bytes))).collect()
}

/// The conversation's card under a tool call that drafted a part ticket (`ticket.draft`): the
/// ticket's number, title and state as they are now (`{kind: ticket, …}` in host.ts); none for
/// any other call, or a ticket deleted since.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn drafted_card(app: &SolveApp, input: Option<&serde_json::Value>, output: Option<&str>) -> Option<serde_json::Value> {
    if !input?.to_string().contains("ticket.draft") {
        return None;
    }
    // The reply names the ticket before anything else that could name one (`{"ticket": {"schemaVersion": 2,
    // "id": "T-3", "parent": …`), whether it comes as JSON or as JSON inside text.
    let output = output?;
    let at = output.find("T-")?;
    let digits: String = output[at + 2..].chars().take_while(char::is_ascii_digit).collect();
    let id = format!("T-{digits}");
    let ticket = app.tickets.tickets.iter().find(|ticket| ticket.id == id && !ticket.deleted)?;
    Some(serde_json::json!({ "kind": "ticket", "id": ticket.id, "title": ticket.title, "state": ticket.state.as_str() }))
}

/// The tree's dot for a part (`kind` component or body, by its name): the state colour of the
/// first ticket by number that bears on it and is neither done nor deleted; none without one.
pub(crate) fn part_dot(app: &SolveApp, kind: &str, name: &str) -> Option<egui::Color32> {
    let ticket = app.tickets.tickets.iter().find(|t| !t.deleted && t.state != TicketState::Done && t.parts.iter().any(|p| p.kind == kind && p.id == name))?;
    let t = crate::theme::Tokens::get();
    Some(match ticket.state {
        TicketState::InProgress => t.agent_busy,
        TicketState::Waiting => t.agent_ok,
        TicketState::Blocked => t.error,
        TicketState::Draft | TicketState::Confirmed | TicketState::Done => t.text_dim,
    })
}

/// Whether an agent is connected, so the Assistant can take a ticket (one starting counts: a
/// message waits for it).
#[cfg(not(target_arch = "wasm32"))]
fn connected(app: &SolveApp) -> bool {
    use crate::agent_chat::State;
    matches!(app.agent_chat.state(), State::Ready | State::Busy | State::Starting)
}

/// The parts selected in the model now, as a ticket names them: a body (or a face or edge of one)
/// by its name, a placed component by its component's name.
#[cfg(not(target_arch = "wasm32"))]
fn selected_parts(app: &SolveApp) -> Vec<tickets::Part> {
    use solidcraft_engine::Sel;
    let doc = &app.session.doc;
    let mut out: Vec<tickets::Part> = Vec::new();
    for sel in &app.session.selection {
        let part = match sel {
            Sel::Body { name } | Sel::Face { body: name, .. } | Sel::Edge { body: name, .. } => tickets::Part { kind: "body".into(), id: name.clone() },
            Sel::Occurrence { path } => {
                let Some(occurrence) = path.last().and_then(|id| doc.occurrences.iter().find(|o| o.id == *id)) else { continue };
                let Some(component) = doc.components.iter().find(|c| c.id == occurrence.component) else { continue };
                tickets::Part { kind: "component".into(), id: component.name.clone() }
            }
            _ => continue,
        };
        if !part.id.is_empty() && !out.contains(&part) {
            out.push(part);
        }
    }
    out
}

/// The ticket in progress with the Assistant: the one the agent works on (not one a person works,
/// nor a deleted one).
pub fn current(app: &SolveApp) -> Option<&Ticket> {
    app.tickets
        .tickets
        .iter()
        .find(|ticket| ticket.state == TicketState::InProgress && ticket.worker == Some(tickets::Worker::Assistant) && !ticket.deleted)
}

/// Run a person's or the app's step on the store and read it again; a refusal is kept as the view's
/// error.
#[cfg(not(target_arch = "wasm32"))]
fn apply(app: &mut SolveApp, step: impl FnOnce(&std::path::Path) -> Result<Ticket, tickets::TicketError>) -> Option<Ticket> {
    let root = root(app)?;
    match step(&root) {
        Ok(ticket) => {
            app.tickets.error = None;
            app.tickets.refusal = None;
            refresh(app);
            Some(ticket)
        }
        Err(error) => {
            app.tickets.refusal = refusal_of(&error);
            app.tickets.error = Some(error.to_string());
            None
        }
    }
}

/// A refusal a person can meet, as the page words it: its kind and what it names. Any other keeps
/// only the store's words (`error`).
#[cfg(not(target_arch = "wasm32"))]
fn refusal_of(error: &tickets::TicketError) -> Option<serde_json::Value> {
    use serde_json::json;
    use tickets::{NotDone, TicketError};
    Some(match error {
        TicketError::Stale { id, .. } => json!({ "kind": "stale", "id": id }),
        TicketError::NotDone { why: NotDone::Subs(ids), .. } => json!({ "kind": "subs", "ids": ids }),
        TicketError::NotDone { why: NotDone::NoRun, .. } => json!({ "kind": "check" }),
        TicketError::NotDone { why: NotDone::Recheck(why), .. } => json!({ "kind": "recheck", "why": why }),
        TicketError::Waits { open, .. } => json!({ "kind": "waits", "ids": open }),
        _ => return None,
    })
}

/// Confirm (接受): a person's step on a draft they saw at revision `seen`; `with_subs`: its
/// sub-tickets that are still drafts go with it.
#[cfg(not(target_arch = "wasm32"))]
pub fn confirm(app: &mut SolveApp, id: &str, seen: u64, with_subs: bool) {
    apply(app, |root| tickets::confirm(root, id, seen, with_subs));
}

/// Done: a person finishes a ticket seen at revision `seen`, past the store's gate.
#[cfg(not(target_arch = "wasm32"))]
pub fn done(app: &mut SolveApp, id: &str, seen: u64) {
    apply(app, |root| tickets::done(root, id, seen));
}

/// New ticket: a person's own ticket, confirmed from the start. The files taken in for it are its
/// attachments; they are let go once it is made.
#[cfg(not(target_arch = "wasm32"))]
pub fn create(app: &mut SolveApp, draft: tickets::Draft) -> Option<Ticket> {
    let made = apply(app, |root| tickets::create(root, draft));
    if made.is_some() {
        app.tickets.adding.clear();
    }
    made
}

/// Edit: a person changes a ticket seen at revision `seen`, in any state.
#[cfg(not(target_arch = "wasm32"))]
pub fn edit(app: &mut SolveApp, id: &str, seen: u64, edit: tickets::Edit) {
    apply(app, |root| tickets::edit(root, id, seen, edit));
}

/// Delete: a person deletes a ticket seen at revision `seen`, its sub-tickets with it (kept,
/// restorable); its page closes.
#[cfg(not(target_arch = "wasm32"))]
pub fn delete(app: &mut SolveApp, id: &str, seen: u64) {
    if apply(app, |root| tickets::delete(root, id, seen)).is_some() && app.tickets.selected.as_deref() == Some(id) {
        app.tickets.selected = None;
        app.tickets.expanded = false;
        app.tickets.record.clear();
    }
}

/// Run check: the person starts the check of a ticket in progress, seen at revision `seen`; only a
/// person starts one. [`tick`] then starts its run.
#[cfg(not(target_arch = "wasm32"))]
pub fn ask_check(app: &mut SolveApp, id: &str, seen: u64) {
    apply(app, |root| tickets::ask_check(root, id, seen));
}

/// Start: the ticket (seen at revision `seen`) goes in progress and the Assistant gets one message
/// to work on it alone, with the ticket's chip. Only one ticket is worked on at a time, and only
/// while an agent is connected: the ticket is not moved for nobody to work it.
#[cfg(not(target_arch = "wasm32"))]
pub fn start(app: &mut SolveApp, ctx: &egui::Context, id: &str, seen: u64) {
    if !connected(app) {
        app.tickets.error = Some(tr("ui.tickets_why_offline").to_string());
        app.tickets.refusal = Some(serde_json::json!({ "kind": "offline" }));
        return;
    }
    if let Some(other) = current(app).filter(|ticket| ticket.id != id).map(|ticket| ticket.id.clone()) {
        app.tickets.refusal = Some(serde_json::json!({ "kind": "busy", "id": other }));
        app.tickets.error = Some(trf("ui.tickets_one_at_a_time", &[("id", &other)]));
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
    // Its attachments go with the message, as files dropped on the Assistant go: pictures and
    // PDFs whole where the agent takes them, any other file pointed to.
    if let Some(root) = root(app) {
        for path in &ticket.attachments {
            let full = path.split('/').fold(root.clone(), |full, name| full.join(name));
            if let Err(reason) = crate::agent_chat::attach::read(&full).and_then(|file| crate::agent_chat::attach_ready(app, file)) {
                crate::agent_chat::notify(app, reason, true);
            }
        }
    }
    let text = trf("ui.tickets_start_message", &[("id", &ticket.id), ("title", &ticket.title)]);
    if let Err(reason) = crate::agent_chat::send_with(app, ctx, text, None)
        && !reason.is_empty()
    {
        app.tickets.error = Some(reason);
    }
}

/// The conversation's card of a ticket the agent drafted: the right rail's Tickets panel opens on
/// that ticket.
pub fn show(app: &mut SolveApp, id: &str) {
    app.preferences.right_rail_open = crate::right_rail::Item::Tickets.id().to_string();
    app.tickets.selected = Some(id.to_string());
    app.tickets.expanded = false;
    app.tickets.error = None;
    app.tickets.seen = None;
}

/// A ticket in full, as the agent reads it.
#[cfg(not(target_arch = "wasm32"))]
fn summary(ticket: &Ticket) -> String {
    let mut out = format!("{} \"{}\" ({})", ticket.id, ticket.title, ticket.state.as_str());
    if !ticket.parts.is_empty() {
        let parts: Vec<String> = ticket.parts.iter().map(|part| format!("{} {}", part.kind, part.id)).collect();
        out += &format!("\nParts: {}", parts.join(", "));
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
                 execute tool, command ticket.draft: one ticket per piece of work, with no top ticket (title, text, requirement lines \
                 with numbers and units, parts: [{kind: component | body, id}], parent for a sub-ticket, waits for the tickets it waits \
                 for). When the person asked for that work, accept your drafts with ticket.update {\"action\": \"state\", \"to\": \
                 \"confirmed\"}; a ticket you propose unasked stays a draft for the person to accept or decline in the Tickets panel."
                    .into(),
            );
        }
        let mut out = String::from(
            "Part tickets of this project. You may set a ticket's state with ticket.update {\"action\": \"state\", \"to\": …}: done only \
             when its sub-tickets are done and, where a check applies, its check ran; one ticket in progress with you at a time; a ticket \
             waiting for others is not started. Accept your own drafts only when the person asked for that work. No command starts a \
             check, edits, deletes or restores a ticket: a person does those in the Tickets panel.",
        );
        match current(app) {
            Some(ticket) => {
                out += &format!(
                    "\nYou are working on {} alone. Change only what it asks for.\n{}\nWhen its changes are made, call execute with \
                     ticket.update {{\"id\": \"{}\", \"action\": \"finished\"}}: where a check applies a person then starts it (no \
                     check starts by itself); where none applies you may set it done. If you cannot go on, use action block with the reason.",
                    ticket.id,
                    summary(ticket),
                    ticket.id
                );
            }
            None => out += "\nNo ticket is in progress: do not change the model for a ticket until a person starts it.",
        }
        let others: Vec<String> = all
            .iter()
            .filter(|ticket| !ticket.deleted && current(app).is_none_or(|working| working.id != ticket.id))
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

/// Each frame: when a person asked for a ticket's check, start its run; when that run has a report,
/// bind it and the ticket waits for the person; a refusal or a run that did not finish stays on
/// the ticket with its reason.
#[cfg(not(target_arch = "wasm32"))]
pub fn tick(app: &mut SolveApp) {
    if root(app).is_none() {
        return;
    }
    refresh(app);
    attach_wanted_view(app, |app| crate::control::view_png(app, VIEW_PICTURE.0, VIEW_PICTURE.1));
    match app.tickets.checking.clone() {
        None => {
            let asked = app
                .tickets
                .tickets
                .iter()
                .find(|t| t.state == TicketState::InProgress && !t.deleted && t.check.as_ref().is_some_and(|c| c.state == CheckState::Asked))
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

/// The agent asked, drafting a ticket, for a picture of the 3D view: `picture` takes it as the view
/// is now, once per ticket (a picture that cannot be taken is not tried again every frame), and the
/// store attaches it under the project's `attachments/`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn attach_wanted_view(app: &mut SolveApp, picture: impl FnOnce(&SolveApp) -> Result<Vec<u8>, String>) {
    let wanted = app.tickets.tickets.iter().find(|t| t.wants_view && !t.deleted && !app.tickets.viewed.contains(&t.id)).map(|t| t.id.clone());
    let Some(id) = wanted else { return };
    app.tickets.viewed.push(id.clone());
    match picture(app) {
        Ok(png) => {
            apply(app, |root| tickets::attach_view(root, &id, &png));
        }
        Err(error) => log::warn!("the 3D view's picture for {id} was not taken: {error}"),
    }
}

/// The size of the 3D view's picture a ticket gets, in pixels.
#[cfg(not(target_arch = "wasm32"))]
const VIEW_PICTURE: (u32, u32) = (1280, 800);

/// The tickets view's state as JSON (`TicketsState` in packages/assistant-web/src/host.ts): the
/// project's tickets as the store keeps them, with what the store says of each (whether a check
/// applies, why it cannot be finished, the tickets it still waits for, whether finishing leaves it
/// unchecked), the open ticket and its record, each bound run's freshness now, the last refusal
/// and how the view's last request went, the model's selection and the picked part, and the view's
/// words in the app's language. The page draws the tickets view instead of the conversation when
/// it is pushed this.
#[cfg(not(target_arch = "wasm32"))]
pub fn page_state(app: &mut SolveApp) -> String {
    let _language = crate::i18n::scope(&app.preferences.language);
    let project = root(app).is_some();
    refresh(app);
    if app.tickets.selected.as_ref().is_some_and(|id| !app.tickets.tickets.iter().any(|ticket| &ticket.id == id)) {
        app.tickets.selected = None;
        app.tickets.expanded = false;
        app.tickets.record.clear();
    }
    let selection = selected_parts(app);
    // Picking one part in the 3D view or the tree lists the tickets that bear on it; picking
    // something else, or nothing, lists them all again. Clearing the filter holds until the next pick.
    if selection != app.tickets.picked {
        app.tickets.picked = selection.clone();
        app.tickets.filter = match selection.as_slice() {
            [one] => Some(one.clone()),
            _ => None,
        };
    }
    let assistant = connected(app);
    let tickets = &app.tickets;
    let file = |path: &String| attachment(path, &tickets.pictures);
    serde_json::json!({
        "view": "tickets",
        "dark": crate::theme::is_dark(),
        "language": app.preferences.language,
        "strings": page_strings(),
        "project": project,
        "tickets": tickets.tickets.iter().map(|ticket| page_ticket(ticket, &tickets.tickets, &tickets.pictures)).collect::<Vec<_>>(),
        "selected": tickets.selected,
        "record": tickets.record,
        "freshness": tickets.freshness,
        "error": tickets.error,
        "refusal": tickets.refusal,
        "answered": tickets.answered.iter().map(|(n, ask, ok)| serde_json::json!({"n": n, "ask": ask, "ok": ok})).collect::<Vec<_>>(),
        "expanded": tickets.expanded && tickets.selected.is_some(),
        "filter": tickets.filter,
        "selection": selection,
        "adding": tickets.adding.iter().map(file).collect::<Vec<_>>(),
        "assistant": assistant,
    })
    .to_string()
}

/// A file of the project as the page reads one (`TicketAttachment` in host.ts): its path, its
/// name, and the key its picture is served by (none for any other file).
#[cfg(not(target_arch = "wasm32"))]
fn attachment(path: &String, pictures: &BTreeMap<String, (String, Arc<Vec<u8>>)>) -> serde_json::Value {
    let name = path.rsplit('/').next().unwrap_or(path);
    serde_json::json!({ "path": path, "name": name, "picture": pictures.get(path).map(|(key, _)| key) })
}

/// A ticket as the page reads it (`Ticket` in host.ts): the store's own fields, each attachment as
/// `{path, name, picture}`, `checks` as whether a check applies (`checksOff`: switched off on a
/// ticket with parts), and the store's reasons: `whyNotDone` (`{kind: subs | check | recheck,
/// ids?, why?}`), `openWaits` and `unchecked`.
#[cfg(not(target_arch = "wasm32"))]
fn page_ticket(ticket: &Ticket, all: &[Ticket], pictures: &BTreeMap<String, (String, Arc<Vec<u8>>)>) -> serde_json::Value {
    use serde_json::{Value, json};
    let mut value = serde_json::to_value(ticket).unwrap_or_default();
    let why = tickets::why_not_done(ticket, all).map(|why| match why {
        tickets::NotDone::Subs(ids) => json!({ "kind": "subs", "ids": ids }),
        tickets::NotDone::NoRun => json!({ "kind": "check" }),
        tickets::NotDone::Recheck(why) => json!({ "kind": "recheck", "why": why }),
    });
    if let Some(fields) = value.as_object_mut() {
        fields.insert("attachments".into(), Value::Array(ticket.attachments.iter().map(|path| attachment(path, pictures)).collect()));
        fields.insert("checks".into(), Value::Bool(tickets::applies(ticket)));
        fields.insert("checksOff".into(), Value::Bool(!ticket.checks));
        fields.insert("whyNotDone".into(), why.unwrap_or(Value::Null));
        fields.insert("openWaits".into(), json!(tickets::open_waits(ticket, all)));
        fields.insert("unchecked".into(), Value::Bool(tickets::unchecked(ticket)));
    }
    value
}

/// The longest list the page may send (requirement lines, parts, attachments, waits); the store's
/// own limits are lower.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SENT_LINES: usize = 64;

/// The most a message from the tickets view may hold.
#[cfg(not(target_arch = "wasm32"))]
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// One message from the tickets view's own web view, as JSON. Only the tickets view's requests are
/// taken here ([`act`]); the Assistant's view has no way to them (its `web::act` refuses them).
/// Every message is answered in the next state (`answered`, the latest few): the page's own number for it (`ask`)
/// and whether the step was taken, so the page shows a success only for a step that was.
#[cfg(not(target_arch = "wasm32"))]
pub fn receive(app: &mut SolveApp, ctx: &egui::Context, message: &str) -> Result<(), String> {
    let _language = crate::i18n::scope(&app.preferences.language);
    let n = app.tickets.answered.last().map_or(1, |(n, _, _)| n.saturating_add(1));
    if message.len() > MAX_MESSAGE_BYTES {
        answer(app, (n, None, false));
        return Err("message too long".into());
    }
    let action: serde_json::Value = serde_json::from_str(message).map_err(|error| format!("not JSON: {error}"))?;
    let ask = action.get("ask").and_then(serde_json::Value::as_u64);
    let kind = action.get("type").and_then(serde_json::Value::as_str).ok_or("no type")?;
    app.tickets.error = None;
    app.tickets.refusal = None;
    let done = if kind.starts_with("ticket_") { act(app, ctx, kind, &action) } else { Err(format!("unknown action {kind}")) };
    answer(app, (n, ask, done.is_ok() && app.tickets.error.is_none()));
    ctx.request_repaint();
    done
}

/// The most answers the view is told at once: more than the page sends between two states.
#[cfg(not(target_arch = "wasm32"))]
const MAX_ANSWERS: usize = 16;

/// Keep a request's answer among the latest ones the view is told.
#[cfg(not(target_arch = "wasm32"))]
fn answer(app: &mut SolveApp, answered: (u64, Option<u64>, bool)) {
    app.tickets.answered.push(answered);
    if app.tickets.answered.len() > MAX_ANSWERS {
        app.tickets.answered.remove(0);
    }
}

/// A list of text sent by the page, at most [`MAX_SENT_LINES`] long.
#[cfg(not(target_arch = "wasm32"))]
fn texts(kind: &str, name: &str, value: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    use serde_json::Value;
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) if items.len() <= MAX_SENT_LINES => {
            items.iter().map(|item| item.as_str().map(str::to_string).ok_or(format!("{kind}: an item of {name} is not text"))).collect()
        }
        Some(_) => Err(format!("{kind}: {name} must be at most {MAX_SENT_LINES} items of text")),
    }
}

/// A message of the tickets view, `{"type": "ticket_…", "ask"?: n}` ([`receive`]). Each name in
/// `TicketAction` (host.ts) has an arm here, and a test keeps the two equal:
/// - choosing what is shown: `ticket_open` (`id`), `ticket_close`, `ticket_expand` (`id`),
///   `ticket_collapse`, `ticket_filter_clear`;
/// - a person's steps on a ticket seen at `revision`: `ticket_confirm` (`with_subs`, default
///   true), `ticket_decline`, `ticket_start`, `ticket_work_myself`, `ticket_ask_check`,
///   `ticket_done`, `ticket_set_state` (`to`), `ticket_delete`, and `ticket_restore` (no revision);
/// - a person's changes: `ticket_create` (`title`, `text`, `requirements`, `parts`, `attachments`,
///   `parent`), `ticket_edit` (any of `title`, `text`, `requirements`, `checks`), `ticket_waits`
///   (`waits`), `ticket_assign_parts` (the model's selection), `ticket_attach` (`id`, or null for
///   the ticket being created: the app asks for the file);
/// - the app: `ticket_attachment` (`path`: a picture in the picture viewer, any other file in the
///   system's program), `ticket_show_parts` (`id`), `ticket_ask_draft` (a message to the Assistant).
///
/// The store checks each step against the ticket's state and the revision the person saw; a
/// refusal is the view's error. Anything else is refused with the reason.
#[cfg(not(target_arch = "wasm32"))]
fn act(app: &mut SolveApp, ctx: &egui::Context, kind: &str, action: &serde_json::Value) -> Result<(), String> {
    use serde_json::Value;
    let text = |name: &str| action.get(name).and_then(Value::as_str);
    let id = || text("id").map(str::trim).filter(|id| tickets::number(id).is_some()).ok_or(format!("{kind}: no ticket id"));
    let seen = || action.get("revision").and_then(Value::as_u64).ok_or(format!("{kind}: no revision"));
    let list = |name: &str| texts(kind, name, action.get(name));
    let parts = || -> Result<Vec<tickets::Part>, String> {
        match action.get("parts") {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) if items.len() <= MAX_SENT_LINES => {
                serde_json::from_value(Value::Array(items.clone())).map_err(|error| format!("{kind}: parts: {error}"))
            }
            Some(_) => Err(format!("{kind}: parts must be at most {MAX_SENT_LINES} parts")),
        }
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
            app.tickets.expanded = false;
            app.tickets.record.clear();
            app.tickets.error = None;
        }
        "ticket_expand" => {
            app.tickets.selected = Some(id()?.to_string());
            app.tickets.expanded = true;
            app.tickets.seen = None;
            refresh(app);
        }
        "ticket_collapse" => app.tickets.expanded = false,
        "ticket_filter_clear" => app.tickets.filter = None,
        "ticket_confirm" => {
            let with_subs = action.get("with_subs").and_then(Value::as_bool).unwrap_or(true);
            confirm(app, id()?, seen()?, with_subs);
        }
        "ticket_decline" => {
            let (id, seen) = (id()?, seen()?);
            apply(app, |root| tickets::decline(root, id, seen));
        }
        "ticket_start" => start(app, ctx, id()?, seen()?),
        "ticket_work_myself" => {
            let (id, seen) = (id()?, seen()?);
            apply(app, |root| tickets::work_myself(root, id, seen));
        }
        "ticket_ask_check" => ask_check(app, id()?, seen()?),
        "ticket_done" => done(app, id()?, seen()?),
        "ticket_set_state" => {
            let to = text("to").ok_or(format!("{kind}: no state"))?;
            let to: TicketState = serde_json::from_value(Value::from(to)).map_err(|_| format!("{kind}: `{to}` is not a state"))?;
            let (id, seen) = (id()?, seen()?);
            apply(app, |root| tickets::set_state(root, id, seen, to));
        }
        "ticket_delete" => delete(app, id()?, seen()?),
        "ticket_restore" => {
            let id = id()?;
            apply(app, |root| tickets::restore(root, id));
        }
        "ticket_create" => {
            let title = text("title").ok_or(format!("{kind}: no title"))?.to_string();
            let parent = text("parent").map(str::to_string);
            let draft = tickets::Draft {
                title,
                text: text("text").unwrap_or_default().to_string(),
                requirements: list("requirements")?,
                parts: parts()?,
                attachments: list("attachments")?,
                parent,
                ..tickets::Draft::default()
            };
            create(app, draft);
        }
        "ticket_edit" => {
            let requirements = match action.get("requirements") {
                None => None,
                lines => Some(texts(kind, "requirements", lines)?),
            };
            let change = tickets::Edit {
                title: text("title").map(str::to_string),
                text: text("text").map(str::to_string),
                requirements,
                checks: action.get("checks").and_then(Value::as_bool),
                ..tickets::Edit::default()
            };
            if change == tickets::Edit::default() {
                return Err(format!("{kind}: nothing to save"));
            }
            edit(app, id()?, seen()?, change);
        }
        "ticket_waits" => {
            let change = tickets::Edit { waits: Some(list("waits")?), ..tickets::Edit::default() };
            edit(app, id()?, seen()?, change);
        }
        "ticket_assign_parts" => {
            let (id, seen) = (id()?, seen()?);
            let selection = selected_parts(app);
            if selection.is_empty() {
                app.tickets.error = Some(tr("ui.tickets_select_first").to_string());
                return Ok(());
            }
            // 指定 adds the selection to the parts the ticket bears on.
            let mut parts = app.tickets.tickets.iter().find(|ticket| ticket.id == id).map(|ticket| ticket.parts.clone()).unwrap_or_default();
            parts.extend(selection.into_iter().filter(|part| !parts.contains(part)).collect::<Vec<_>>());
            edit(app, id, seen, tickets::Edit { parts: Some(parts), ..tickets::Edit::default() });
        }
        "ticket_attach" => {
            let to = match action.get("id") {
                None | Some(Value::Null) => None,
                Some(_) => Some(id()?.to_string()),
            };
            attach(app, to);
        }
        "ticket_attachment" => open_attachment(app, text("path").ok_or(format!("{kind}: no path"))?)?,
        "ticket_show_parts" => show_parts(app, id()?),
        "ticket_ask_draft" => {
            if !connected(app) {
                app.tickets.error = Some(tr("ui.tickets_why_offline").to_string());
                app.tickets.refusal = Some(serde_json::json!({ "kind": "offline" }));
                return Ok(());
            }
            if let Err(reason) = crate::agent_chat::send_with(app, ctx, tr("ui.tickets_ask_draft_message").to_string(), None)
                && !reason.is_empty()
            {
                app.tickets.error = Some(reason);
            }
        }
        other => return Err(format!("unknown action {other}")),
    }
    Ok(())
}

/// 加附件: the app asks for a file, takes it into the project ([`tickets::take_in`]: a file of the
/// project stays where it is, any other is copied under `attachments/`) and adds it to ticket `to`
/// as it is when the file is chosen, or to the files of the ticket being created (`None`).
#[cfg(not(target_arch = "wasm32"))]
fn attach(app: &mut SolveApp, to: Option<String>) {
    app.pick_file(tr("ui.tickets_attachments"), &["pdf", "png", "jpg", "jpeg", "gif", "webp", "step", "stp", "dxf", "txt", "md"], move |app, path| {
        let (Some(path), Some(root)) = (path, root(app)) else { return };
        let taken = match tickets::take_in(&root, std::path::Path::new(&path)) {
            Ok(taken) => taken,
            Err(error) => {
                app.tickets.error = Some(error.to_string());
                return;
            }
        };
        let Some(id) = to else {
            if !app.tickets.adding.contains(&taken) {
                app.tickets.adding.push(taken);
            }
            read_pictures(&mut app.tickets, &root);
            return;
        };
        let Ok(ticket) = tickets::load(&root, &id) else { return };
        let mut files = ticket.attachments.clone();
        if !files.contains(&taken) {
            files.push(taken);
        }
        edit(app, &id, ticket.revision, tickets::Edit { attachments: Some(files), ..tickets::Edit::default() });
    });
}

/// Open an attachment the view names: only a path one of the live tickets carries. A picture opens
/// in the picture viewer over the app with that ticket's other pictures; any other file in the
/// system's program for it.
#[cfg(not(target_arch = "wasm32"))]
fn open_attachment(app: &mut SolveApp, path: &str) -> Result<(), String> {
    let root = root(app).ok_or("ticket_attachment: no project")?;
    let ticket = app
        .tickets
        .tickets
        .iter()
        .find(|ticket| !ticket.deleted && ticket.attachments.iter().any(|file| file == path))
        .ok_or("ticket_attachment: no ticket carries that file")?;
    let shown: Vec<(Arc<Vec<u8>>, String)> = ticket
        .attachments
        .iter()
        .filter_map(|file| app.tickets.pictures.get(file).map(|(_, bytes)| (Arc::clone(bytes), file.rsplit('/').next().unwrap_or(file).to_string())))
        .collect();
    if app.tickets.pictures.contains_key(path) {
        let at = ticket.attachments.iter().filter(|file| app.tickets.pictures.contains_key(*file)).position(|file| file == path).unwrap_or(0);
        crate::agent_chat::show_pictures(app, shown, at);
        return Ok(());
    }
    // The file as it is now: still in the project folder, not a link out of it.
    let full = path.split('/').fold(root.clone(), |full, name| full.join(name));
    let inside = match (std::fs::canonicalize(&full), std::fs::canonicalize(&root)) {
        (Ok(real), Ok(base)) => real.starts_with(&base) && real.is_file(),
        _ => false,
    };
    if !inside {
        app.tickets.error = Some(format!("{path}: the file is no longer in the project"));
        return Ok(());
    }
    match &app.services.open_file {
        Some(open) => open(&full.to_string_lossy()),
        None => app.tickets.error = Some(format!("{path}: no program opens files here")),
    }
    Ok(())
}

/// A placement's path from the root: its own id after those of the placements it sits in.
#[cfg(not(target_arch = "wasm32"))]
fn placement_path(doc: &solidcraft_engine::doc::Document, id: u64) -> Option<Vec<u64>> {
    let mut path = vec![id];
    let mut at = doc.occurrences.iter().find(|o| o.id == id)?;
    for _ in 0..16 {
        if at.parent == 0 {
            return Some(path);
        }
        at = doc.occurrences.iter().find(|outer| outer.component == at.parent)?;
        path.insert(0, at.id);
    }
    None
}

/// 点票上的件: the parts ticket `id` bears on, selected together in the model, so the 3D view
/// highlights them and the tree shows them. A part the model no longer has is said, not guessed.
#[cfg(not(target_arch = "wasm32"))]
fn show_parts(app: &mut SolveApp, id: &str) {
    use solidcraft_engine::Sel;
    let Some(parts) = app.tickets.tickets.iter().find(|ticket| ticket.id == id && !ticket.deleted).map(|ticket| ticket.parts.clone()) else { return };
    let doc = &app.session.doc;
    let world = crate::open_meshing::session_world(&app.session);
    let mut items: Vec<Sel> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for part in &parts {
        let found: Vec<Sel> = match part.kind.as_str() {
            "body" => world.body(&part.id).is_some().then(|| Sel::Body { name: part.id.clone() }).into_iter().collect(),
            _ => {
                let components: Vec<u64> = doc.components.iter().filter(|c| c.name == part.id).map(|c| c.id).collect();
                doc.occurrences.iter().filter(|o| components.contains(&o.component)).filter_map(|o| placement_path(doc, o.id)).map(|path| Sel::Occurrence { path }).collect()
            }
        };
        if found.is_empty() {
            missing.push(part.id.clone());
        }
        items.extend(found);
    }
    if !items.is_empty() {
        let _ = app.run("select.set", serde_json::json!({ "items": items }));
        crate::browser::reveal_pick(app);
    }
    if !missing.is_empty() {
        app.tickets.error = Some(trf("ui.tickets_parts_missing", &[("parts", &missing.join(", "))]));
    }
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
        ("didDeclined", "ui.tickets_did_declined"),
        ("didDeleted", "ui.tickets_did_deleted"),
        ("didRestored", "ui.tickets_did_restored"),
        ("didTaken", "ui.tickets_did_taken"),
        ("didMoved", "ui.tickets_did_moved"),
        ("didFinished", "ui.tickets_did_finished"),
        ("didCheckOff", "ui.tickets_did_check_off"),
        ("didCheckOn", "ui.tickets_did_check_on"),
        ("didRecheck", "ui.tickets_did_recheck"),
        ("didViewAttached", "ui.tickets_did_view_attached"),
        ("runCheck", "ui.tickets_run_check"),
        ("recheck", "ui.tickets_recheck"),
        ("finishedNote", "ui.tickets_finished_note"),
        ("needsCheck", "ui.tickets_needs_check"),
        ("deleteWithSubs", "ui.tickets_delete_with_subs"),
        ("confirmWithSubs", "ui.tickets_confirm_with_subs"),
        ("together", "ui.tickets_together"),
        ("onlyThis", "ui.tickets_only_this"),
        ("whyRecheck", "ui.tickets_why_recheck"),
        ("withTicket", "ui.tickets_with_ticket"),
        ("whyStale", "ui.tickets_why_stale"),
        ("checkOff", "ui.tickets_check_off"),
    ]
    .into_iter()
    .map(|(name, key)| (name.to_string(), serde_json::Value::from(tr(key))))
    .collect()
}

#[cfg(test)]
#[path = "tickets_tests.rs"]
mod tests;
