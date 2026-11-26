//! The host side of part tickets: a person's steps, Start's message and chip, the note every
//! message carries, one scripted agent's run from a draft to a ticket waiting for the person with
//! its DFM run bound, and what the tickets view of the Assistant's page is told and may ask for.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use solidcraft_engine::project::tickets::{self, Actor, CheckState, Draft, Part, State as TicketState};

use crate::agent_chat::backend::{ChatBackend, Event, PromptPart};
use crate::scenario::Harness;

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("intcad-tickets-ui-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A stand-in agent: it records every prompt and answers nothing by itself; the test plays its
/// tool calls and ends its turns.
#[derive(Clone, Default)]
struct StandIn {
    prompts: Arc<Mutex<Vec<String>>>,
    events: Arc<Mutex<Vec<Event>>>,
}

impl StandIn {
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
    fn end_turn(&self) {
        self.events.lock().unwrap().push(Event::TurnEnded { reason: String::new() });
    }
}

impl ChatBackend for StandIn {
    fn poll(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
    fn prompt(&mut self, parts: &[PromptPart]) {
        let text = parts.iter().find_map(|p| if let PromptPart::Text(t) = p { Some(t.clone()) } else { None }).unwrap_or_default();
        self.prompts.lock().unwrap().push(text);
    }
    fn cancel(&mut self) {}
    fn new_session(&mut self) {}
    fn answer_permission(&mut self, _key: u64, _option: Option<&str>) {}
    fn authenticate(&mut self, _method: &str) {}
    fn shutdown(&mut self, _wait: bool) {}
}

/// A window beside a design saved in a new project.
fn window(scratch: &Scratch) -> (Harness, std::path::PathBuf) {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.resize", json!({"width": 1500, "height": 900}));
    let created = solidcraft_engine::project::create(&scratch.0, "enclosure", "enclosure").expect("a project");
    h.app.session.path = Some(created.design.to_string_lossy().into_owned());
    h.settle();
    (h, created.project.root().to_path_buf())
}

fn state(root: &std::path::Path, id: &str) -> TicketState {
    tickets::load(root, id).unwrap().state
}

/// The revision the person sees now.
fn seen(root: &std::path::Path, id: &str) -> u64 {
    tickets::load(root, id).unwrap().revision
}

/// The agent's tool call as it reaches the app: `engine.execute` with a ticket command. (In the
/// local demo copy the agent's port is admitted as free; in the product it is a charged entry.)
fn agent_calls(h: &mut Harness, command: &str, params: Value) -> Value {
    let reply = h.call("engine.execute", json!({"command": command, "params": params}));
    assert_eq!(reply["ok"], true, "{command}: {reply}");
    reply["result"].clone()
}

#[test]
fn a_persons_steps_are_refused_out_of_order_and_recorded_as_theirs() {
    let scratch = Scratch::new("steps");
    let (mut h, root) = window(&scratch);
    tickets::draft(&root, Draft { title: "Enclosure".into(), ..Draft::default() }, Actor::Agent).unwrap();
    let parts = vec![Part { kind: "component".into(), id: "Lid".into() }];
    let lid = tickets::draft(&root, Draft { title: "Lid".into(), parts, ..Draft::default() }, Actor::Agent).unwrap().id;
    h.settle();
    assert_eq!(h.app.tickets.tickets.len(), 2, "the host read both");
    crate::tickets::done(&mut h.app, &lid, seen(&root, &lid));
    assert!(h.app.tickets.error.as_deref().is_some_and(|e| e.contains("check has not run")), "a draft is not done: {:?}", h.app.tickets.error);
    assert_eq!(state(&root, &lid), TicketState::Draft);
    crate::tickets::confirm(&mut h.app, &lid, seen(&root, &lid), false);
    assert_eq!(state(&root, &lid), TicketState::Confirmed);
    assert!(h.app.tickets.error.is_none());
    let record = tickets::events(&root, Some(&lid)).unwrap();
    assert_eq!(record.last().map(|e| (e.action.as_str(), e.actor)), Some(("confirmed", Actor::Person)));
    assert_eq!(h.app.tickets.tickets.iter().find(|t| t.id == lid).map(|t| t.state), Some(TicketState::Confirmed), "read again after the step");
}

#[test]
fn start_sends_one_message_with_the_chip_and_every_message_carries_the_ticket_in_progress() {
    let scratch = Scratch::new("start");
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    tickets::draft(&root, Draft { title: "Enclosure".into(), ..Draft::default() }, Actor::Agent).unwrap();
    let lines = vec!["Two M4 holes, centres 40 ± 0.1 mm".to_string()];
    let id = tickets::draft(&root, Draft { title: "Bracket".into(), requirements: lines, ..Draft::default() }, Actor::Agent).unwrap().id;
    tickets::confirm(&root, &id, tickets::load(&root, &id).unwrap().revision, false).unwrap();
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::start(&mut h.app, &ctx, &id, seen(&root, &id));
    h.settle();
    assert_eq!(state(&root, &id), TicketState::InProgress);
    let prompts = agent.prompts();
    assert_eq!(prompts.len(), 1, "one message: {prompts:?}");
    let sent = &prompts[0];
    assert!(sent.contains("Work on part ticket T-2 alone: Bracket."), "{sent}");
    assert!(sent.contains("You are working on T-2 alone") && sent.contains("Two M4 holes, centres 40 ± 0.1 mm"), "the ticket in full: {sent}");
    assert!(sent.contains("\"action\": \"finished\"") && sent.contains("no check starts by itself"), "how to say it is finished: {sent}");
    assert!(sent.contains("Other tickets: T-1 Enclosure (draft)"), "the others by id, title and state: {sent}");
    assert!(sent.contains("Attached file `T-2 · Bracket`"), "the chip went with it: {sent}");
    let shown = h
        .app
        .agent_chat
        .items
        .iter()
        .any(|item| matches!(item, crate::agent_chat::Item::User { files, .. } if files.iter().any(|f| f.starts_with("T-2"))));
    assert!(shown, "the conversation shows the ticket's chip on the message");
    // Only one ticket at a time: a second Start is refused while T-2 is in progress.
    let other = tickets::draft(&root, Draft { title: "Lid".into(), ..Draft::default() }, Actor::Agent).unwrap().id;
    tickets::confirm(&root, &other, tickets::load(&root, &other).unwrap().revision, false).unwrap();
    let revision = seen(&root, &other);
    crate::tickets::start(&mut h.app, &h.ctx.clone(), &other, revision);
    assert_eq!(state(&root, &other), TicketState::Confirmed);
    assert!(h.app.tickets.error.as_deref().is_some_and(|e| e.contains("T-2")), "{:?}", h.app.tickets.error);
}

/// The loop with a scripted agent: asked in the chat, it drafts tickets; a person accepts one and
/// hands it to the Assistant; the agent changes the model and says it is finished, which starts
/// nothing; the person starts the check; the app runs the DFM check through the Review entry
/// (stand-in billing and journal, the real background worker) and binds the run, so the ticket
/// waits for the person, who marks it done.
#[test]
fn a_scripted_agent_drafts_is_started_on_one_ticket_and_its_check_leaves_it_waiting() {
    let scratch = Scratch::new("loop");
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    let billing = crate::dfm_review::tests::CountingBilling::default();
    let ledger = crate::dfm_review::tests::ReviewLedger::default();
    crate::dfm_review::tests::install(&mut h.app, &billing, &ledger);

    // The person says what they want; with no tickets the note says how to draft them.
    let ctx = h.ctx.clone();
    crate::agent_chat::send(&mut h.app, &ctx, "Make tickets from requirements.md".into());
    h.settle();
    assert!(agent.prompts().last().is_some_and(|p| p.contains("this project has none yet") && p.contains("ticket.draft")), "{:?}", agent.prompts());
    agent_calls(&mut h, "ticket.draft", json!({"title": "Sensor enclosure", "requirements": ["5052-H32, 1.5 mm"]}));
    let drafted = agent_calls(
        &mut h,
        "ticket.draft",
        json!({"title": "Base plate", "requirements": ["40 × 30 mm"], "parts": [{"kind": "body", "id": "Body1"}]}),
    );
    assert_eq!(drafted["ticket"]["id"], "T-2");
    agent.end_turn();
    h.settle();
    // `confirm` is no action of the agent's (it accepts a draft by setting its state).
    let refused = h.call("engine.execute", json!({"command": "ticket.update", "params": {"id": "T-2", "action": "confirm"}}));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(state(&root, "T-2"), TicketState::Draft);

    crate::tickets::confirm(&mut h.app, "T-2", seen(&root, "T-2"), false);
    crate::tickets::start(&mut h.app, &ctx, "T-2", seen(&root, "T-2"));
    h.settle();
    assert_eq!(state(&root, "T-2"), TicketState::InProgress);
    assert!(agent.prompts().last().is_some_and(|p| p.contains("You are working on T-2 alone")), "{:?}", agent.prompts().last());

    // The agent makes the part and says it is finished: no check starts by itself.
    agent_calls(&mut h, "solid.box", json!({"length": 40, "width": 30, "height": 1.5}));
    agent_calls(&mut h, "ticket.update", json!({"id": "T-2", "action": "finished", "reason": "Plate made"}));
    agent.end_turn();
    h.frames(30);
    let said = tickets::load(&root, "T-2").unwrap();
    assert_eq!((said.state, said.finished, said.check.is_none(), billing.commits()), (TicketState::InProgress, true, true, 0));
    // The person starts the check.
    crate::tickets::ask_check(&mut h.app, "T-2", seen(&root, "T-2"));

    // The app starts the check through the Review entry and binds the run once it has a report.
    let began = std::time::Instant::now();
    while state(&root, "T-2") == TicketState::InProgress && began.elapsed() < Duration::from_secs(60) {
        h.frames(30);
        std::thread::sleep(Duration::from_millis(20));
    }
    let ticket = tickets::load(&root, "T-2").unwrap();
    assert_eq!(ticket.state, TicketState::Waiting, "{:?} / review error {:?}", ticket.check, h.app.dfm_review.error);
    let check = ticket.check.clone().unwrap();
    assert_eq!(check.state, CheckState::Ran);
    let run = check.run.clone().unwrap();
    assert_eq!(Some(run.clone()), h.app.dfm_review.mapped_run, "the ticket names the run the Review entry reserved");
    assert_eq!(billing.commits(), 1, "the run was the Review entry's one billed operation");
    assert!(check.counts.get("failed").copied().unwrap_or(0) == 0, "the empty factory table fails nothing: {:?}", check.counts);

    crate::tickets::done(&mut h.app, "T-2", seen(&root, "T-2"));
    assert_eq!(state(&root, "T-2"), TicketState::Done);
    let steps: Vec<(String, Actor)> = tickets::events(&root, Some("T-2")).unwrap().into_iter().map(|e| (e.action, e.actor)).collect();
    let expected = [
        ("drafted", Actor::Agent),
        ("confirmed", Actor::Person),
        ("started", Actor::Person),
        ("finished", Actor::Agent),
        ("check_asked", Actor::Person),
        ("check_started", Actor::App),
        ("check_ran", Actor::App),
        ("done", Actor::Person),
    ];
    assert_eq!(steps, expected.map(|(a, actor)| (a.to_string(), actor)));
    // The two Spec files at the root were not touched by any of it.
    let requirements = std::fs::read_to_string(root.join("requirements.md")).unwrap();
    assert!(requirements.starts_with("# Requirements"), "{requirements}");
}

/// A refused check is said on the ticket, which stays in progress; nothing is billed.
#[test]
fn without_an_authority_the_tickets_check_is_refused_and_the_ticket_stays_in_progress() {
    let scratch = Scratch::new("refused");
    let (mut h, root) = window(&scratch);
    let parts = vec![Part { kind: "body".into(), id: "Body1".into() }];
    let id = tickets::draft(&root, Draft { title: "Plate".into(), parts, ..Draft::default() }, Actor::Agent).unwrap().id;
    tickets::confirm(&root, &id, tickets::load(&root, &id).unwrap().revision, false).unwrap();
    tickets::start(&root, &id, tickets::load(&root, &id).unwrap().revision).unwrap();
    tickets::ask_check(&root, &id, tickets::load(&root, &id).unwrap().revision).unwrap();
    h.frames(4);
    let ticket = tickets::load(&root, &id).unwrap();
    assert_eq!(ticket.state, TicketState::InProgress);
    let check = ticket.check.unwrap();
    assert_eq!(check.state, CheckState::Refused);
    assert!(check.reason.as_deref().is_some_and(|r| r.contains("UNSAFE_TO_START") || r.contains("AUTH_UNAVAILABLE")), "{:?}", check.reason);
    crate::tickets::done(&mut h.app, &id, seen(&root, &id));
    assert_eq!(state(&root, &id), TicketState::InProgress, "no run, no done");
    // The person can ask again; with no authority it is refused again, and said again.
    crate::tickets::ask_check(&mut h.app, &id, seen(&root, &id));
    h.frames(4);
    let record = tickets::events(&root, Some(&id)).unwrap();
    assert_eq!(record.iter().filter(|e| e.action == "check_refused").count(), 2);
    assert_eq!(record.iter().rev().nth(1).map(|e| e.actor), Some(Actor::Person), "the second ask is the person's");
}

/// What the tickets view is told: the tickets as the store keeps them, the open one's record,
/// and every word the page has a name for, in the app's language.
#[test]
fn the_tickets_view_is_told_the_tickets_the_open_ones_record_and_its_words() {
    let scratch = Scratch::new("page");
    let (mut h, root) = window(&scratch);
    tickets::draft(&root, Draft { title: "Enclosure".into(), ..Draft::default() }, Actor::Agent).unwrap();
    let parts = vec![Part { kind: "component".into(), id: "Lid".into() }];
    std::fs::create_dir_all(root.join("specs")).unwrap();
    std::fs::write(root.join("specs/lid.pdf"), b"%PDF-1.7").unwrap();
    let attachments = vec!["specs/lid.pdf".to_string()];
    tickets::draft(&root, Draft { title: "Lid".into(), text: "**not markdown**".into(), parts, attachments, ..Draft::default() }, Actor::Agent)
        .unwrap();
    h.settle();
    let state: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    assert_eq!((state["view"].as_str(), state["project"].as_bool()), (Some("tickets"), Some(true)));
    assert_eq!(state["tickets"].as_array().map(Vec::len), Some(2));
    let lid = &state["tickets"][1];
    assert_eq!((lid["id"].as_str(), lid["state"].as_str(), lid["parent"].as_str()), (Some("T-2"), Some("draft"), None));
    assert_eq!((lid["parts"][0]["id"].as_str(), lid["revision"].as_u64(), lid["text"].as_str()), (Some("Lid"), Some(1), Some("**not markdown**")));
    // The store's new fields, by the page's names; an attachment as the page reads one.
    assert_eq!(lid["attachments"], json!([{"path": "specs/lid.pdf", "name": "lid.pdf", "picture": null}]));
    assert_eq!(
        (lid["waits"].clone(), lid["worker"].clone(), lid["deleted"].clone(), lid["checks"].clone()),
        (json!([]), Value::Null, json!(false), json!(true))
    );
    assert!(lid["check"].is_null() && state["selected"].is_null() && state["record"].as_array().is_some_and(Vec::is_empty));

    crate::tickets::receive(&mut h.app, &h.ctx.clone(), r#"{"type":"ticket_open","id":"T-2"}"#).unwrap();
    let state: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    assert_eq!(state["selected"], "T-2");
    assert_eq!(state["record"][0]["action"], "drafted");
    assert_eq!(state["record"][0]["actor"], "agent");
    assert!(state["record"][0]["at"].as_str().is_some_and(|at| at.ends_with('Z')), "UTC, for the page to show in local time");
    crate::tickets::receive(&mut h.app, &h.ctx.clone(), r#"{"type":"ticket_close"}"#).unwrap();
    let state: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    assert!(state["selected"].is_null());

    // Every word the page has a name for is sent, and no other.
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/assistant-web/src/host.ts")).expect("the page's bridge");
    let names = page.split("export type TicketStringKey =").nth(1).and_then(|rest| rest.split(';').next()).expect("the tickets view's names");
    let mut named: Vec<&str> = names.split('|').map(|name| name.trim().trim_matches('\'')).filter(|name| !name.is_empty()).collect();
    let mut sent: Vec<&str> = state["strings"].as_object().expect("words").keys().map(String::as_str).collect();
    named.sort_unstable();
    sent.sort_unstable();
    assert_eq!(sent, named);
    assert!(
        state["strings"].as_object().is_some_and(|words| words.values().all(|w| w.as_str().is_some_and(|w| !w.is_empty() && !w.starts_with("ui."))))
    );
}

/// The tickets view's steps go through the page's own bridge, and the store checks each one against
/// the ticket's state and the revision the person saw. A person's own ticket starts confirmed.
#[test]
fn the_tickets_views_steps_are_checked_against_the_state_and_the_revision_seen() {
    let scratch = Scratch::new("page-steps");
    let (mut h, root) = window(&scratch);
    tickets::draft(&root, Draft { title: "Enclosure".into(), ..Draft::default() }, Actor::Agent).unwrap();
    let parts = vec![Part { kind: "component".into(), id: "Lid".into() }];
    tickets::draft(&root, Draft { title: "Lid".into(), parts, ..Draft::default() }, Actor::Agent).unwrap();
    h.settle();
    let ctx = h.ctx.clone();
    let page = |h: &mut Harness, message: Value| crate::tickets::receive(&mut h.app, &ctx, &message.to_string());

    // A person edits the draft they saw at revision 1; it stays a draft.
    page(&mut h, json!({"type": "ticket_edit", "id": "T-2", "revision": 1, "title": "Lid, 1.5 mm", "text": "", "requirements": ["Flange ≥ 8 mm"]}))
        .unwrap();
    assert_eq!((tickets::load(&root, "T-2").unwrap().title.as_str(), state(&root, "T-2")), ("Lid, 1.5 mm", TicketState::Draft));
    // A Confirm pressed on what was seen before that edit is refused and said; nothing changes.
    page(&mut h, json!({"type": "ticket_confirm", "id": "T-2", "revision": 1})).unwrap();
    assert!(h.app.tickets.error.as_deref().is_some_and(|e| e.contains("changed since you saw it")), "{:?}", h.app.tickets.error);
    assert_eq!(state(&root, "T-2"), TicketState::Draft);
    page(&mut h, json!({"type": "ticket_confirm", "id": "T-2", "revision": 2})).unwrap();
    assert_eq!(state(&root, "T-2"), TicketState::Confirmed);
    assert!(h.app.tickets.error.is_none());
    // Done is refused on a ticket that has not waited for confirmation.
    page(&mut h, json!({"type": "ticket_done", "id": "T-2", "revision": 3})).unwrap();
    assert_eq!(state(&root, "T-2"), TicketState::Confirmed);

    // New ticket: the person's own, confirmed from the start, recorded as theirs.
    page(&mut h, json!({"type": "ticket_create", "title": "Label area 40 × 20 mm", "text": "No holes there.", "requirements": ["40 × 20 mm flat"]}))
        .unwrap();
    let made = tickets::load(&root, "T-3").unwrap();
    assert_eq!((made.state, made.parent.as_deref(), made.requirements.len()), (TicketState::Confirmed, None, 1));
    assert_eq!(tickets::events(&root, Some("T-3")).unwrap()[0].actor, Actor::Person);
    // Delete: the open page closes, the ticket is kept marked deleted, the record keeps the line.
    page(&mut h, json!({"type": "ticket_open", "id": "T-3"})).unwrap();
    page(&mut h, json!({"type": "ticket_delete", "id": "T-3", "revision": 1})).unwrap();
    assert!(tickets::load(&root, "T-3").unwrap().deleted && h.app.tickets.selected.is_none());
    assert_eq!(tickets::events(&root, Some("T-3")).unwrap().last().map(|e| e.action.as_str()), Some("deleted"));

    // Hostile messages are refused before the store.
    for bad in [
        json!({"type": "ticket_confirm", "id": "../T-2", "revision": 3}),
        json!({"type": "ticket_confirm", "id": "T-2"}),
        json!({"type": "ticket_create", "requirements": ["x"]}),
        json!({"type": "ticket_create", "title": "x", "requirements": [1]}),
        json!({"type": "ticket_create", "title": "x", "requirements": vec!["x"; 65]}),
        json!({"type": "ticket_rename", "id": "T-2"}),
        json!({"type": "send", "text": "the Assistant's messages are not taken here"}),
    ] {
        assert!(page(&mut h, bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(tickets::list(&root).unwrap().len(), 3);
    // The Assistant's own view has no way to a ticket step.
    let confirm = json!({"type": "ticket_confirm", "id": "T-1", "revision": 1}).to_string();
    assert!(crate::agent_chat::web::act(&mut h.app, &ctx, &confirm).is_err());
    assert_eq!(state(&root, "T-1"), TicketState::Draft);
}

/// The page's request names: those of `TicketAction` in host.ts, and the conversation's card's.
fn page_requests() -> (Vec<String>, Vec<String>) {
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/assistant-web/src/host.ts")).expect("the page's bridge");
    let names = |block: &str| -> Vec<String> {
        let mut found: Vec<String> = block.split('\'').filter(|word| word.starts_with("ticket_")).map(str::to_string).collect();
        found.sort_unstable();
        found.dedup();
        found
    };
    let tickets = page.split("export type TicketAction =").nth(1).and_then(|rest| rest.split("declare global").next()).expect("TicketAction");
    let assistant = page.split("export type Action =").nth(1).and_then(|rest| rest.split("export type").next()).expect("Action");
    (names(tickets), names(assistant))
}

/// Every request the page can send has an arm in the host: a name the host does not know is
/// refused as an unknown action, any other refusal is about the message's fields.
#[test]
fn every_request_the_tickets_view_names_has_an_arm_in_the_host() {
    let scratch = Scratch::new("arms");
    let (mut h, _root) = window(&scratch);
    let ctx = h.ctx.clone();
    let (tickets_view, conversation) = page_requests();
    assert!(tickets_view.len() >= 22 && conversation == ["ticket_show"], "{tickets_view:?} / {conversation:?}");
    for name in &tickets_view {
        let answer = crate::tickets::receive(&mut h.app, &ctx, &json!({"type": name}).to_string());
        assert!(!answer.as_ref().is_err_and(|why| why.starts_with("unknown action")), "{name} has no arm: {answer:?}");
    }
    for name in &conversation {
        let answer = crate::agent_chat::web::act(&mut h.app, &ctx, &json!({"type": name}).to_string());
        assert!(!answer.as_ref().is_err_and(|why| why.starts_with("unknown action")), "{name} has no arm: {answer:?}");
    }
    let unknown = crate::tickets::receive(&mut h.app, &ctx, r#"{"type":"ticket_rename"}"#);
    assert!(unknown.is_err_and(|why| why.starts_with("unknown action")), "the check can fail");
}

/// A small real picture, for a ticket's picture attachment.
fn png() -> Vec<u8> {
    let picture = image::RgbImage::from_fn(64, 40, |x, y| image::Rgb([(x * 4) as u8, (y * 6) as u8, 160]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    picture.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn told(h: &mut Harness) -> Value {
    serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap()
}

fn ticket<'a>(state: &'a Value, id: &str) -> &'a Value {
    state["tickets"].as_array().and_then(|all| all.iter().find(|t| t["id"] == id)).unwrap_or_else(|| panic!("{id} in {state}"))
}

/// A project folder with what the page draws: a draft parent and its draft sub-tickets, a parent
/// with sub-tickets, a ticket with two parts and two attachments (a picture and a PDF), a ticket
/// waiting for a person, one waiting for other tickets, a deleted one, and the Assistant's ticket
/// it says is finished. The view is told it in the page's shape, with the store's own reasons.
/// With `SOLIDCRAFT_TICKETS_DUMP` set to a folder, the states the host produced (the list, three
/// ticket pages, one of them large) and the pictures it serves are written there for the page's
/// own checks.
#[test]
fn the_tickets_view_is_told_the_stores_tickets_in_the_pages_shape() {
    let scratch = Scratch::new("shape");
    let (mut h, root) = window(&scratch);
    h.app.preferences.language = "zh".into();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(StandIn::default()));
    let part = |kind: &str, id: &str| Part { kind: kind.into(), id: id.into() };
    let agent = |draft: Draft| tickets::draft(&root, draft, Actor::Agent).unwrap().id;
    let person = |draft: Draft| tickets::create(&root, draft).unwrap().id;
    let seen_now = |id: &str| tickets::load(&root, id).unwrap().revision;
    // The drafts the Assistant proposed: a parent with two sub-tickets, and one on its own.
    let enclosure = agent(Draft { title: "Sensor enclosure".into(), text: "From requirements.md".into(), ..Draft::default() });
    agent(Draft { title: "Base plate 1.5 mm".into(), parent: Some(enclosure.clone()), parts: vec![part("body", "Base")], ..Draft::default() });
    agent(Draft { title: "Lid with vents".into(), parent: Some(enclosure.clone()), parts: vec![part("body", "Lid")], ..Draft::default() });
    agent(Draft { title: "Hinge clearance 3 mm".into(), checks: Some(false), ..Draft::default() });
    // A parent with sub-tickets in three states.
    let bracket = person(Draft { title: "Bracket set".into(), ..Draft::default() });
    let left = person(Draft { title: "Left bracket".into(), parent: Some(bracket.clone()), parts: vec![part("component", "Left")], ..Draft::default() });
    let right = person(Draft { title: "Right bracket".into(), parent: Some(bracket.clone()), ..Draft::default() });
    tickets::work_myself(&root, &left, seen_now(&left)).unwrap();
    tickets::set_state(&root, &right, seen_now(&right), TicketState::Done).unwrap();
    // Two parts and two attachments.
    std::fs::create_dir_all(root.join("specs")).unwrap();
    std::fs::write(root.join("specs/flange.png"), png()).unwrap();
    std::fs::write(root.join("specs/drawing.pdf"), b"%PDF-1.7\n").unwrap();
    let joint = person(Draft {
        title: "Lid and base: align the hinge".into(),
        requirements: vec!["Gap 0.3 ± 0.1 mm".into(), "Hinge pin Ø3 mm".into()],
        parts: vec![part("body", "Lid"), part("body", "Base")],
        attachments: vec!["specs/flange.png".into(), "specs/drawing.pdf".into()],
        ..Draft::default()
    });
    // Waiting for a person (no check applies), and waiting for other tickets.
    let label = person(Draft { title: "Label area 40 × 20 mm".into(), ..Draft::default() });
    tickets::work_myself(&root, &label, seen_now(&label)).unwrap();
    tickets::set_state(&root, &label, seen_now(&label), TicketState::Waiting).unwrap();
    let paint = person(Draft { title: "Paint".into(), waits: vec![joint.clone(), label.clone()], ..Draft::default() });
    // Deleted.
    let old = person(Draft { title: "Old fixing holes".into(), ..Draft::default() });
    tickets::delete(&root, &old, seen_now(&old)).unwrap();
    // The Assistant drafts, accepts, works and finishes a ticket no check applies to.
    let agents = agent(Draft { title: "Choose the hinge supplier".into(), ..Draft::default() });
    for to in [TicketState::Confirmed, TicketState::InProgress] {
        tickets::agent_set_state(&root, &agents, to).unwrap();
    }
    tickets::finished(&root, &agents, Some("Supplier B: 3 mm pins in stock".into())).unwrap();
    tickets::agent_set_state(&root, &agents, TicketState::Done).unwrap();
    // Parts, and the check switched off by a person on the ticket (one record line).
    let rivets = person(Draft { title: "Rivet holes on the lid".into(), parts: vec![part("body", "Lid")], ..Draft::default() });
    // The Assistant's ticket, which it says is finished; a check applies.
    let flange = person(Draft { title: "Flange 8 mm".into(), parts: vec![part("body", "Base")], ..Draft::default() });
    tickets::start(&root, &flange, seen_now(&flange)).unwrap();
    tickets::finished(&root, &flange, Some("Flange raised to 8 mm".into())).unwrap();
    h.settle();

    let ctx = h.ctx.clone();
    let request = |h: &mut Harness, message: Value| crate::tickets::receive(&mut h.app, &ctx, &message.to_string());
    request(&mut h, json!({"type": "ticket_edit", "id": rivets, "revision": 1, "checks": false})).unwrap();
    let list = told(&mut h);
    assert_eq!(list["tickets"].as_array().map(Vec::len), Some(14));
    assert_eq!(list["strings"]["title"], "部件票", "in the app's language");
    assert_eq!((list["assistant"].as_bool(), list["expanded"].as_bool(), list["selection"].clone()), (Some(true), Some(false), json!([])));
    let parent = ticket(&list, &enclosure);
    assert_eq!((parent["state"].as_str(), parent["checks"].as_bool(), parent["unchecked"].as_bool()), (Some("draft"), Some(false), Some(true)), "no parts, no check");
    assert_eq!(parent["whyNotDone"], json!({"kind": "subs", "ids": ["T-2", "T-3"]}));
    let off = ticket(&list, "T-4");
    assert_eq!((off["checks"].as_bool(), off["checksOff"].as_bool()), (Some(false), Some(true)));
    assert_eq!(ticket(&list, &left)["whyNotDone"], json!({"kind": "check"}));
    let both = ticket(&list, &joint);
    assert_eq!(both["parts"].as_array().map(Vec::len), Some(2));
    let picture = both["attachments"][0]["picture"].as_str().expect("the picture's key");
    assert_eq!((both["attachments"][0]["name"].as_str(), both["attachments"][1]["picture"].clone()), (Some("flange.png"), Value::Null));
    let served = crate::tickets::pictures(&h.app);
    assert!(served.iter().any(|(key, bytes)| key == picture && bytes.starts_with(b"\x89PNG")), "the host serves the picture by its key");
    assert_eq!(ticket(&list, &paint)["openWaits"], json!([joint.clone(), label.clone()]));
    assert_eq!((ticket(&list, &label)["state"].as_str(), ticket(&list, &label)["whyNotDone"].clone()), (Some("waiting"), Value::Null));
    assert_eq!(ticket(&list, &old)["deleted"], true);
    let said = ticket(&list, &flange);
    assert_eq!((said["finished"].as_bool(), said["worker"].as_str(), said["whyNotDone"]["kind"].as_str()), (Some(true), Some("assistant"), Some("check")));

    let mut pages = vec![("list", list)];
    let opened = [("enclosure", &enclosure), ("joint", &joint), ("label", &label), ("rivets", &rivets), ("agents", &agents), ("flange", &flange), ("bracket", &bracket)];
    for (name, id) in opened {
        request(&mut h, json!({"type": "ticket_open", "id": id})).unwrap();
        pages.push((name, told(&mut h)));
    }
    request(&mut h, json!({"type": "ticket_expand", "id": flange})).unwrap();
    pages.push(("flange-large", told(&mut h)));
    let large = &pages.last().unwrap().1;
    assert_eq!((large["expanded"].as_bool(), large["selected"].as_str()), (Some(true), Some(flange.as_str())));
    assert!(pages[2].1["record"].as_array().is_some_and(|record| record.len() == 1 && record[0]["action"] == "created"));
    let record: Vec<(String, String)> =
        pages[5].1["record"].as_array().unwrap().iter().map(|e| (e["action"].as_str().unwrap().into(), e["actor"].as_str().unwrap().into())).collect();
    let agent_steps = ["drafted", "confirmed", "moved", "finished", "done"].map(|a| (a.to_string(), "agent".to_string()));
    assert_eq!(record, agent_steps, "every step of the Assistant's is its own line, as the Assistant's");
    assert_eq!(pages[4].1["record"].as_array().and_then(|r| r.last()).map(|e| e["action"].clone()), Some(json!("check_off")));
    // A refused step: its reason, and the answer says it was not taken.
    request(&mut h, json!({"type": "ticket_collapse"})).unwrap();
    request(&mut h, json!({"type": "ticket_open", "id": joint})).unwrap();
    request(&mut h, json!({"type": "ticket_done", "id": joint, "revision": 1, "ask": 7})).unwrap();
    let refused = told(&mut h);
    assert!(refused["error"].as_str().is_some_and(|e| e.contains("check has not run")) && refused["answered"]["ok"] == false, "{}", refused["error"]);
    pages.push(("refused-done", refused));
    // A parent deleted goes to 已删除 with its sub-tickets, and comes back with them.
    request(&mut h, json!({"type": "ticket_close"})).unwrap();
    request(&mut h, json!({"type": "ticket_delete", "id": bracket, "revision": seen_now(&bracket)})).unwrap();
    let deleted = told(&mut h);
    assert!([&bracket, &left, &right].iter().all(|id| ticket(&deleted, id)["deleted"] == true));
    pages.push(("deleted", deleted));
    request(&mut h, json!({"type": "ticket_restore", "id": bracket})).unwrap();
    let restored = told(&mut h);
    assert!([&bracket, &left, &right].iter().all(|id| ticket(&restored, id)["deleted"] == false));
    pages.push(("restored", restored));
    if let Some(folder) = std::env::var_os("SOLIDCRAFT_TICKETS_DUMP").map(std::path::PathBuf::from) {
        std::fs::create_dir_all(folder.join("picture")).unwrap();
        for (name, state) in &pages {
            std::fs::write(folder.join(format!("{name}.json")), serde_json::to_vec_pretty(state).unwrap()).unwrap();
        }
        for (key, bytes) in served {
            std::fs::write(folder.join("picture").join(key), bytes.as_slice()).unwrap();
        }
    }
}

/// The view's requests reach the store, and each is answered: the page's own number and whether
/// the step was taken. A refused step is said with its reason and answered as not taken.
#[test]
fn the_tickets_views_requests_are_answered_and_a_refusal_says_why() {
    let scratch = Scratch::new("requests");
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    let ctx = h.ctx.clone();
    let mut ask = 0;
    let mut page = |h: &mut Harness, mut message: Value| -> (bool, Option<String>) {
        ask += 1;
        message["ask"] = json!(ask);
        let _ = crate::tickets::receive(&mut h.app, &ctx, &message.to_string());
        let state = told(h);
        assert_eq!(state["answered"]["ask"], json!(ask), "{message}");
        (state["answered"]["ok"] == true, state["error"].as_str().map(str::to_string))
    };
    let seen_now = |id: &str| tickets::load(&root, id).unwrap().revision;
    let parent = tickets::draft(&root, Draft { title: "Enclosure".into(), ..Draft::default() }, Actor::Agent).unwrap().id;
    let sub = tickets::draft(&root, Draft { title: "Lid".into(), parent: Some(parent.clone()), ..Draft::default() }, Actor::Agent).unwrap().id;
    h.settle();

    // 接受 takes the drafts under it by default.
    assert_eq!(page(&mut h, json!({"type": "ticket_confirm", "id": parent, "revision": 1})), (true, None));
    assert_eq!(state(&root, &sub), TicketState::Confirmed);
    // A refused delete (seen before a change) is said, not taken; the next one is.
    let (ok, why) = page(&mut h, json!({"type": "ticket_delete", "id": parent, "revision": 1}));
    assert!(!ok && why.as_deref().is_some_and(|why| why.contains("changed since you saw it")), "{why:?}");
    assert!(page(&mut h, json!({"type": "ticket_delete", "id": parent, "revision": seen_now(&parent)})).0);
    assert!(tickets::load(&root, &sub).unwrap().deleted, "its sub-ticket went with it");
    assert!(page(&mut h, json!({"type": "ticket_restore", "id": parent})).0, "撤销");
    assert!(!tickets::load(&root, &sub).unwrap().deleted, "and came back with it");
    // A refused done says why; so does handing over a ticket that waits for another.
    let base = tickets::create(&root, Draft { title: "Base".into(), parts: vec![Part { kind: "body".into(), id: "Base".into() }], ..Draft::default() }).unwrap().id;
    let (ok, why) = page(&mut h, json!({"type": "ticket_done", "id": base, "revision": 1}));
    assert!(!ok && why.as_deref().is_some_and(|why| why.contains("check has not run")), "{why:?}");
    assert!(page(&mut h, json!({"type": "ticket_waits", "id": sub, "revision": seen_now(&sub), "waits": [base]})).0);
    let (ok, why) = page(&mut h, json!({"type": "ticket_start", "id": sub, "revision": seen_now(&sub)}));
    assert!(!ok && why.as_deref().is_some_and(|why| why.contains("waits for T-3")), "{why:?}");
    assert!(agent.prompts().is_empty(), "nothing was sent to the Assistant");
    // A partial save changes only its own field; switching the check off is one line.
    assert!(page(&mut h, json!({"type": "ticket_edit", "id": base, "revision": 1, "text": "40 × 30 mm"})).0);
    assert_eq!(tickets::load(&root, &base).unwrap().title, "Base", "the title was not sent and stays");
    assert!(page(&mut h, json!({"type": "ticket_edit", "id": base, "revision": 2, "checks": false})).0);
    assert_eq!(tickets::events(&root, Some(&base)).unwrap().pop().map(|e| e.action).as_deref(), Some("check_off"));
    assert_eq!(page(&mut h, json!({"type": "ticket_set_state", "id": base, "revision": 3, "to": "done"})), (true, None), "no check applies now");
    // 指定 takes the model's selection.
    h.app.session.selection = vec![solidcraft_engine::Sel::Body { name: "Body1".into() }];
    assert!(page(&mut h, json!({"type": "ticket_assign_parts", "id": sub, "revision": seen_now(&sub)})).0);
    assert_eq!(tickets::load(&root, &sub).unwrap().parts, [Part { kind: "body".into(), id: "Body1".into() }]);
    assert_eq!(told(&mut h)["selection"], json!([{"kind": "body", "id": "Body1"}]));

    // 加附件: the chosen file is taken into the project, for a ticket or for the form.
    let outside = scratch.0.join("photo.png");
    std::fs::write(&outside, png()).unwrap();
    let chosen = outside.to_string_lossy().into_owned();
    h.app.services.pick_file = Some(Box::new(move |_label: &str, _exts: &[&str]| {
        let path = chosen.clone();
        Box::pin(async move { Some(path) })
    }));
    page(&mut h, json!({"type": "ticket_attach", "id": sub}));
    h.frames(3);
    assert_eq!(tickets::load(&root, &sub).unwrap().attachments, ["attachments/photo.png"]);
    page(&mut h, json!({"type": "ticket_attach", "id": null}));
    h.frames(3);
    let state = told(&mut h);
    assert!(state["adding"][0]["picture"].is_string() && state["adding"][0]["path"] == "attachments/photo-2.png", "{}", state["adding"]);
    let made = page(&mut h, json!({"type": "ticket_create", "title": "Vents", "text": "", "requirements": [], "parts": [{"kind": "body", "id": "Lid"}],
                                    "attachments": ["attachments/photo-2.png"], "parent": parent}));
    assert!(made.0 && h.app.tickets.adding.is_empty());
    let vents = tickets::load(&root, "T-4").unwrap();
    assert_eq!((vents.parent.as_deref(), vents.parts.len(), vents.attachments.len()), (Some(parent.as_str()), 1, 1));
    // A picture opens in the picture viewer; any other file in the system's program; nothing else.
    assert!(page(&mut h, json!({"type": "ticket_attachment", "path": "attachments/photo.png"})).0);
    assert!(crate::agent_chat::gallery_open(&h.app));
    std::fs::write(root.join("notes.pdf"), b"%PDF-1.7\n").unwrap();
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let seen_open = opened.clone();
    h.app.services.open_file = Some(Box::new(move |path| seen_open.borrow_mut().push(path.to_string())));
    let pdf = root.join("notes.pdf").to_string_lossy().into_owned();
    h.app.services.pick_file = Some(Box::new(move |_label: &str, _exts: &[&str]| {
        let path = pdf.clone();
        Box::pin(async move { Some(path) })
    }));
    page(&mut h, json!({"type": "ticket_attach", "id": base}));
    h.frames(3);
    assert!(page(&mut h, json!({"type": "ticket_attachment", "path": "notes.pdf"})).0);
    assert!(opened.borrow().last().is_some_and(|path| path.ends_with("notes.pdf")), "{:?}", opened.borrow());
    assert!(!page(&mut h, json!({"type": "ticket_attachment", "path": "../secret.pdf"})).0);

    // 让助手起草 asks the Assistant; the card in the conversation opens the ticket.
    assert!(page(&mut h, json!({"type": "ticket_ask_draft"})).0);
    h.settle();
    assert!(agent.prompts().last().is_some_and(|p| p.contains("requirements.md")), "{:?}", agent.prompts());
    crate::agent_chat::web::act(&mut h.app, &h.ctx.clone(), r#"{"type":"ticket_show","id":"T-4"}"#).unwrap();
    assert_eq!((crate::right_rail::open(&h.app), h.app.tickets.selected.as_deref()), (Some(crate::right_rail::Item::Tickets), Some("T-4")));
}

/// Opened large, the tickets view lies over the window below the application bar and the
/// Assistant's view gives way to it, as to the picture viewer; back in the rail, the Assistant's
/// view has its place again. The Assistant's turn goes on meanwhile: nothing is cancelled.
#[test]
fn a_ticket_opened_large_lies_over_the_window_and_the_assistant_gives_way_until_it_goes_back() {
    let scratch = Scratch::new("large");
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    crate::agent_chat::send(&mut h.app, &h.ctx.clone(), "Make the lid".into());
    h.settle();
    assert_eq!(h.app.agent_chat.state(), crate::agent_chat::State::Busy, "the Assistant is in a turn");
    h.call("ui.prefs", json!({"right_rail": true}));
    h.app.preferences.right_rail_open = crate::right_rail::Item::Tickets.id().to_string();
    let id = tickets::create(&root, Draft { title: "Lid".into(), ..Draft::default() }).unwrap().id;
    h.settle();
    let ctx = h.ctx.clone();
    let rail = crate::tickets::placement(&mut h.app, &ctx).expect("the tickets view in the rail");
    // A place the Assistant's panel gives its view, on a layer nothing lies over.
    let panel = egui::Rect::from_min_size(egui::pos2(300.0, 400.0), egui::vec2(500.0, 300.0));
    let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("intcad-test-assistant-panel"));
    let assistant = |h: &mut Harness| {
        h.app.agent_chat.web_place = Some((panel, layer));
        crate::agent_chat::web::placement(&mut h.app, &ctx)
    };
    let before = assistant(&mut h);
    assert!(before.is_some(), "the Assistant's view has its place");

    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_expand", "id": id}).to_string()).unwrap();
    h.frames(1);
    let large = crate::tickets::placement(&mut h.app, &ctx).expect("the tickets view over the window");
    let window = ctx.content_rect();
    assert!(large.width() > rail.width() * 2.0 && large.left() <= window.left() + 8.0 && large.right() >= window.right() - 8.0, "{large:?} over {window:?}");
    assert!(large.top() > window.top() && large.bottom() >= window.bottom() - 8.0, "below the application bar, to the bottom: {large:?}");
    assert_eq!(assistant(&mut h), None, "the Assistant's view gives way");
    assert!(told(&mut h)["expanded"] == true);
    // Previous / Next keep it large; Esc (the page's collapse) puts it back in the rail.
    let other = tickets::create(&root, Draft { title: "Base".into(), ..Draft::default() }).unwrap().id;
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_open", "id": other}).to_string()).unwrap();
    h.frames(1);
    assert!(crate::tickets::placement(&mut h.app, &ctx).is_some_and(|rect| rect.width() > rail.width() * 2.0));
    crate::tickets::receive(&mut h.app, &ctx, r#"{"type":"ticket_collapse"}"#).unwrap();
    h.frames(1);
    assert_eq!(crate::tickets::placement(&mut h.app, &ctx), Some(rail), "back in the rail");
    assert_eq!(assistant(&mut h), before, "the Assistant's view where it was");
    assert_eq!(h.app.agent_chat.state(), crate::agent_chat::State::Busy, "its turn went on");
    assert_eq!(agent.prompts().len(), 1);
}
