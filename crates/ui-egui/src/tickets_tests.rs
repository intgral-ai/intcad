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
    assert!(sent.contains("\"action\": \"finished\"") && sent.contains("no check applies to it"), "how to say it is finished: {sent}");
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

/// The loop with a scripted agent, the project's automatic checks off: asked in the chat, it
/// drafts tickets; a person accepts one and hands it to the Assistant; the agent changes the model
/// and says it is finished, which starts nothing; the person starts the check; the app runs the DFM
/// check through the Review entry (stand-in billing and journal, the real background worker) and
/// binds the run, so the ticket waits for the person, who marks it done.
#[test]
fn a_scripted_agent_drafts_is_started_on_one_ticket_and_its_check_leaves_it_waiting() {
    let scratch = Scratch::new("loop");
    let (mut h, root) = window(&scratch);
    solidcraft_engine::project::Project::open(&root).unwrap().update_ci(Some(false), None).unwrap();
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

    // The agent makes the part and says it is finished: with automatic checks off, nothing starts.
    // A sheet-metal tray, so the check has rules to report.
    for (command, params) in [
        ("sheet.manage_rules", json!({"name": "SPCC 2.0", "thickness": "2 mm", "bend_radius": "2 mm", "k_factor": 0.44, "active": true})),
        ("sketch.create", json!({"plane": "XY", "name": "Base outline"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [100, 60]})),
        ("sketch.finish", json!({})),
        ("sheet.flange", json!({"sketch": "Base outline", "body_name": "Body1"})),
        ("sheet.flange", json!({"edges": [[50, 0, 2], [100, 30, 2], [50, 60, 2], [0, 30, 2]], "height": 30, "body": "Body1"})),
    ] {
        agent_calls(&mut h, command, params);
    }
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

    // The check's line opens to the run's rules, as the run reported them (the page in Chinese).
    h.app.preferences.language = "zh".into();
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_open", "id": "T-2"}).to_string()).unwrap();
    let page: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    let items = page["items"][&run].as_array().cloned().unwrap_or_default();
    let shaped = |item: &Value| item["rule"].is_string() && item["state"].is_string() && item["count"].is_u64() && item["parts"].is_array() && item["values"].is_array();
    assert!(!items.is_empty() && items.iter().all(shaped), "{}", page["items"]);
    assert!(items.iter().any(|item| item["state"] == "unsupported"), "the empty factory table covers nothing: {}", page["items"]);
    assert_eq!(items.len() as u64, check.counts.values().sum::<u64>(), "one item per rule the run counted");
    dump(&mut h, "check-items");

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

/// A person's comment, with a file from outside the project and a picture of the view, reaches the
/// record and the page; a sixth file is refused; the Assistant's comment and a person's reason for
/// moving a ticket back are recorded; the next hand-over carries the comments, newest last, within
/// their limits, and their files.
#[test]
fn a_comment_reaches_the_record_the_page_and_the_next_hand_over_with_its_files() {
    let scratch = Scratch::new("comments");
    let (mut h, root) = window(&scratch);
    h.app.preferences.language = "zh".into();
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    let id = tickets::create(&root, Draft { title: "底板孔位".into(), ..Draft::default() }).unwrap().id;
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_open", "id": id}).to_string()).unwrap();

    // Files for the comment being written: one chosen from outside the project, the view's picture.
    let outside = scratch.0.join("供应商回复.pdf");
    std::fs::write(&outside, b"%PDF-1.7 reply").unwrap();
    let taken = tickets::take_in(&root, &outside).unwrap();
    assert_eq!(taken, "attachments/供应商回复.pdf", "copied under attachments/");
    crate::tickets::take_for_comment(&mut h.app, &root, taken.clone());
    crate::tickets::comment_view_now(&mut h.app, &id, |_| Ok(png()));
    let view = format!("attachments/{id}-view.png");
    assert_eq!(h.app.tickets.commenting, [taken.clone(), view.clone()]);
    // Without a renderer (this test process), the view's picture is refused with its reason.
    crate::tickets::comment_view_now(&mut h.app, &id, |app| crate::control::view_png(app, 64, 40));
    assert!(h.app.tickets.error.take().is_some() && h.app.tickets.commenting.len() == 2);
    for n in 3..=5 {
        std::fs::write(root.join(format!("note-{n}.txt")), b"x").unwrap();
        crate::tickets::take_for_comment(&mut h.app, &root, format!("note-{n}.txt"));
    }
    std::fs::write(root.join("note-6.txt"), b"x").unwrap();
    crate::tickets::take_for_comment(&mut h.app, &root, "note-6.txt".into());
    assert!(h.app.tickets.error.take().is_some_and(|e| e.contains("at most 5")), "a sixth file is refused");
    assert_eq!(h.app.tickets.commenting.len(), 5);
    let page: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    assert_eq!(page["commenting"].as_array().map(Vec::len), Some(5), "the page shows what is attached before sending");

    // The person sends it with two of the files (they took the others off).
    let revision = seen(&root, &id);
    let message = json!({"type": "ticket_comment", "id": id, "text": "侧板这次不要动，孔位见截图。", "attachments": [taken, view], "ask": 7});
    crate::tickets::receive(&mut h.app, &ctx, &message.to_string()).unwrap();
    assert!(h.app.tickets.error.is_none() && h.app.tickets.commenting.is_empty(), "{:?}", h.app.tickets.error);
    assert_eq!(seen(&root, &id), revision, "a comment moves no revision");
    let page: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    let line = page["record"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((line["action"].as_str(), line["actor"].as_str(), line["note"].as_str()), (Some("comment"), Some("person"), Some("侧板这次不要动，孔位见截图。")));
    let files = line["attachments"].as_array().unwrap();
    assert_eq!((files[0]["name"].as_str(), files[0]["picture"].is_null()), (Some("供应商回复.pdf"), true));
    assert!(files[1]["picture"].is_string(), "the view's picture has a thumbnail: {}", files[1]);
    assert!(page["answered"].as_array().unwrap().iter().any(|a| a["ask"] == 7 && a["ok"] == true));

    // A person moves it back with a reason; the Assistant comments.
    tickets::set_state(&root, &id, seen(&root, &id), TicketState::Done, None).unwrap();
    h.settle();
    dump(&mut h, "comments-done");
    let back = json!({"type": "ticket_set_state", "id": id, "revision": seen(&root, &id), "to": "confirmed", "reason": "外壳改高了 2 mm"});
    crate::tickets::receive(&mut h.app, &ctx, &back.to_string()).unwrap();
    assert_eq!(tickets::events(&root, Some(&id)).unwrap().pop().and_then(|e| e.note).as_deref(), Some("外壳改高了 2 mm"));
    dump(&mut h, "comments-back");
    agent_calls(&mut h, "ticket.update", json!({"id": id, "action": "comment", "text": "收到。"}));

    // The hand-over carries them, oldest first, each with who wrote it, and the comment's files.
    crate::tickets::start(&mut h.app, &ctx, &id, seen(&root, &id));
    h.settle();
    let sent = agent.prompts().pop().unwrap_or_default();
    let (first, reason, reply) = (sent.find("侧板这次不要动"), sent.find("外壳改高了 2 mm"), sent.find("收到。"));
    assert!(sent.contains("Comments, oldest first") && first < reason && reason < reply && first.is_some(), "{sent}");
    assert!(sent.contains("- the person, ") && sent.contains("- the person, moving it to confirmed, ") && sent.contains("- the Assistant, "), "{sent}");
    let names = h.app.agent_chat.items.iter().rev().find_map(|item| match item {
        crate::agent_chat::Item::User { files, .. } => Some(files.clone()),
        _ => None,
    });
    assert!(names.unwrap_or_default().iter().any(|n| n == "供应商回复.pdf"), "the comment's file went with the message");

    // The Assistant says what it did, asking for a picture of the view; the app adds the picture
    // to that line (a stand-in picture: this process draws none); the person answers.
    let text = "改了什么：底板两个孔向板内移了 0.8 mm。\n动了的件：底板。\n没做到：法兰 7.4 mm 没改，等你定。";
    agent_calls(&mut h, "ticket.update", json!({"id": id, "action": "finished", "text": text, "view": true}));
    crate::tickets::refresh(&mut h.app);
    h.app.tickets.views_tried.clear();
    crate::tickets::attach_comment_views(&mut h.app, |_| Ok(png()));
    let answer = json!({"type": "ticket_comment", "id": id, "text": "法兰先不改，我下午问一下供应商。", "attachments": []});
    crate::tickets::receive(&mut h.app, &ctx, &answer.to_string()).unwrap();
    let page: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    let record = page["record"].as_array().unwrap();
    let finished = record.iter().find(|e| e["action"] == "finished").unwrap();
    assert_eq!(finished["note"].as_str(), Some(text));
    assert!(finished["attachments"][0]["picture"].is_string(), "the view's picture shows with the line that asked for it: {finished}");
    assert!(!record.iter().any(|e| e["action"] == "comment_view"));
    dump(&mut h, "comments-finished");
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_expand", "id": id}).to_string()).unwrap();
    dump(&mut h, "comments-finished-large");
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_collapse"}).to_string()).unwrap();
    std::fs::write(root.join("法兰尺寸草图.pdf"), b"%PDF-1.7 sketch").unwrap();
    crate::tickets::take_for_comment(&mut h.app, &root, "法兰尺寸草图.pdf".into());
    dump(&mut h, "comments-files");
    h.app.tickets.commenting.clear();

    // At most the newest 10, and 4000 characters in all: a long Chinese comment is cut, never split
    // inside a character.
    for n in 1..=12 {
        tickets::comment(&root, &id, Actor::Person, &format!("第 {n} 条"), &[], false).unwrap();
    }
    let (said, _) = crate::tickets::comments(&root, &id);
    assert_eq!(said.matches("\n- ").count(), 10, "{said}");
    assert!(said.contains("第 12 条") && !said.contains("第 2 条"), "{said}");
    tickets::comment(&root, &id, Actor::Person, &"长".repeat(tickets::MAX_COMMENT), &[], false).unwrap();
    let (said, _) = crate::tickets::comments(&root, &id);
    assert!(said.chars().count() <= 4000 + 100 && said.contains("长长长"), "{}", said.chars().count());
}

/// With `SOLIDCRAFT_TICKETS_DUMP` set to a folder, the host's state as the page is told it now, and
/// the pictures it serves, for the page's own pictures.
fn dump(h: &mut Harness, name: &str) {
    let Some(folder) = std::env::var_os("SOLIDCRAFT_TICKETS_DUMP").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(folder.join("picture")).unwrap();
    let state: Value = serde_json::from_str(&crate::tickets::page_state(&mut h.app)).unwrap();
    std::fs::write(folder.join(format!("{name}.json")), serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    for (key, bytes) in crate::tickets::pictures(&h.app) {
        std::fs::write(folder.join("picture").join(key), bytes.as_slice()).unwrap();
    }
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
    tickets::set_state(&root, &right, seen_now(&right), TicketState::Done, None).unwrap();
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
    tickets::set_state(&root, &label, seen_now(&label), TicketState::Waiting, None).unwrap();
    let paint = person(Draft { title: "Paint".into(), waits: vec![joint.clone(), label.clone()], ..Draft::default() });
    // Deleted.
    let old = person(Draft { title: "Old fixing holes".into(), ..Draft::default() });
    tickets::delete(&root, &old, seen_now(&old)).unwrap();
    // The Assistant drafts, accepts, works and finishes a ticket no check applies to.
    let agents = agent(Draft { title: "Choose the hinge supplier".into(), ..Draft::default() });
    for to in [TicketState::Confirmed, TicketState::InProgress] {
        tickets::agent_set_state(&root, &agents, to, None).unwrap();
    }
    tickets::finished(&root, &agents, Some("Supplier B: 3 mm pins in stock"), &[], false).unwrap();
    tickets::agent_set_state(&root, &agents, TicketState::Done, Some("Supplier B chosen; nothing to check")).unwrap();
    // Parts, and the check switched off by a person on the ticket (one record line).
    let rivets = person(Draft { title: "Rivet holes on the lid".into(), parts: vec![part("body", "Lid")], ..Draft::default() });
    // The Assistant's ticket, which it says is finished; a check applies.
    let flange = person(Draft { title: "Flange 8 mm".into(), parts: vec![part("body", "Base")], ..Draft::default() });
    tickets::start(&root, &flange, seen_now(&flange)).unwrap();
    tickets::finished(&root, &flange, Some("Flange raised to 8 mm"), &[], false).unwrap();
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
    assert!(refused["error"].as_str().is_some_and(|e| e.contains("check has not run")) && refused["answered"].as_array().and_then(|all| all.last()).is_some_and(|last| last["ok"] == false), "{}", refused["error"]);
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
        let answer = state["answered"].as_array().and_then(|all| all.iter().find(|a| a["ask"] == json!(ask)).cloned()).unwrap_or_else(|| panic!("{message} answered"));
        (answer["ok"] == true, state["error"].as_str().map(str::to_string))
    };
    let seen_now = |id: &str| tickets::load(&root, id).unwrap().revision;
    let parent = tickets::draft(&root, Draft { title: "Enclosure".into(), ..Draft::default() }, Actor::Agent).unwrap().id;
    let sub = tickets::draft(&root, Draft { title: "Lid".into(), parent: Some(parent.clone()), ..Draft::default() }, Actor::Agent).unwrap().id;
    h.settle();

    // Two requests before one state (a save, then a click elsewhere): both are answered.
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_edit", "id": parent, "revision": 1, "text": "From requirements.md", "ask": 901}).to_string()).unwrap();
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_open", "id": sub, "ask": 902}).to_string()).unwrap();
    let both: Vec<Value> = told(&mut h)["answered"].as_array().unwrap().iter().map(|a| a["ask"].clone()).collect();
    assert!(both.contains(&json!(901)) && both.contains(&json!(902)), "{both:?}");
    // 接受 takes the drafts under it by default.
    assert_eq!(page(&mut h, json!({"type": "ticket_confirm", "id": parent, "revision": 2})), (true, None));
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

/// A ticket's parts are selected together in the model (the 3D view highlights them, the tree
/// shows them); picking one part lists the tickets that bear on it, until the filter is cleared or
/// the pick changes. A part the model no longer has is said.
#[test]
fn a_tickets_parts_are_selected_in_the_model_and_a_picked_part_filters_the_list() {
    let scratch = Scratch::new("model");
    let (mut h, root) = window(&scratch);
    h.app.run("solid.box", json!({"length": 40, "width": 30, "height": 2})).unwrap();
    h.settle();
    let body = crate::open_meshing::session_world(&h.app.session).bodies.first().map(|b| b.name.clone()).expect("a body");
    let parts = vec![Part { kind: "body".into(), id: body.clone() }, Part { kind: "body".into(), id: "Gone".into() }];
    let id = tickets::create(&root, Draft { title: "Plate".into(), parts, ..Draft::default() }).unwrap().id;
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_show_parts", "id": id}).to_string()).unwrap();
    h.settle();
    assert_eq!(h.app.session.selection, [solidcraft_engine::Sel::Body { name: body.clone() }], "the ticket's part is selected");
    assert!(h.app.tickets.error.as_deref().is_some_and(|e| e.contains("Gone")), "{:?}", h.app.tickets.error);
    let picked = json!({"kind": "body", "id": body});
    assert_eq!(told(&mut h)["filter"], picked, "one part picked: the list shows its tickets");
    assert!(told(&mut h)["selected"].is_null(), "the list, no ticket's page");
    crate::tickets::receive(&mut h.app, &ctx, r#"{"type":"ticket_filter_clear"}"#).unwrap();
    assert!(told(&mut h)["filter"].is_null() && told(&mut h)["filter"].is_null(), "cleared, and it stays cleared while the pick is the same");
    h.settle();
    let after = told(&mut h);
    assert!(after["filter"].is_null() && after["selected"].is_null() && !after["expanded"].as_bool().unwrap_or(false), "× opens nothing: {after}");
    h.app.run("select.clear", json!({})).unwrap();
    assert!(told(&mut h)["filter"].is_null());
    h.app.session.selection = vec![solidcraft_engine::Sel::Body { name: body.clone() }];
    assert_eq!(told(&mut h)["filter"], picked, "picked again");
    // The tree's dot beside the part: the state colour of its first open ticket.
    let t = crate::theme::Tokens::get();
    assert_eq!(crate::tickets::part_dot(&h.app, "body", &body), Some(t.text_dim));
    tickets::work_myself(&root, &id, tickets::load(&root, &id).unwrap().revision).unwrap();
    h.settle();
    assert_eq!(crate::tickets::part_dot(&h.app, "body", &body), Some(t.agent_busy));
    assert!(crate::scenario::handle_at(&format!("ticket_dot:{body}")).is_some(), "the body's row in the tree has its dot");
    assert_eq!(crate::tickets::part_dot(&h.app, "component", &body), None, "a body is not a component of that name");
}

/// The Assistant's side: handing a ticket over sends its attachments with the message; a call that
/// drafted a ticket gets the ticket's card under it; a draft that asked for a picture of the 3D
/// view gets one; a done ticket whose bound run the model moved past is marked 「需重查」.
#[test]
fn the_assistant_gets_a_tickets_files_its_draft_card_and_view_and_a_changed_part_asks_for_a_recheck() {
    let scratch = Scratch::new("assistant");
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    h.app.run("solid.box", json!({"length": 40, "width": 30, "height": 2})).unwrap();
    std::fs::create_dir_all(root.join("specs")).unwrap();
    std::fs::write(root.join("specs/flange.png"), png()).unwrap();
    std::fs::write(root.join("specs/drawing.pdf"), b"%PDF-1.7\n").unwrap();
    let files = vec!["specs/flange.png".to_string(), "specs/drawing.pdf".to_string()];
    let id = tickets::create(&root, Draft { title: "Flange".into(), attachments: files, ..Draft::default() }).unwrap().id;
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::start(&mut h.app, &ctx, &id, seen(&root, &id));
    h.settle();
    let sent = h.app.agent_chat.items.iter().rev().find_map(|item| match item {
        crate::agent_chat::Item::User { files, pictures, .. } => Some((files.clone(), pictures.len())),
        _ => None,
    });
    let (names, pictures) = sent.expect("the hand-over message");
    assert!(names.iter().any(|n| n.starts_with(&id)) && names.iter().any(|n| n == "drawing.pdf"), "{names:?}");
    assert!(names.iter().any(|n| n == "flange.png") || pictures > 0, "the picture went too: {names:?} / {pictures}");

    // The agent drafts a ticket and asks for the view; the call's card names it.
    let reply = agent_calls(&mut h, "ticket.draft", json!({"title": "Hinge clashes with the lid", "view": true}));
    let drafted = reply["ticket"]["id"].as_str().unwrap().to_string();
    h.app.agent_chat.items.push(crate::agent_chat::Item::Tool {
        id: "call-7".into(),
        title: "execute".into(),
        kind: "other".into(),
        status: Some(crate::agent_chat::backend::ToolStatus::Completed),
        input: Some(json!({"command": "ticket.draft", "params": {"title": "Hinge clashes with the lid", "view": true}})),
        output: Some(serde_json::to_string(&json!({"content": [{"type": "text", "text": reply.to_string()}]})).unwrap()),
    });
    h.frames(3);
    let page: Value = serde_json::from_str(&crate::agent_chat::web::state(&mut h.app)).unwrap();
    let items = page["items"].as_array().unwrap();
    let at = items.iter().position(|item| item["id"] == "call-7").expect("the call");
    assert_eq!(items[at + 1], json!({"kind": "ticket", "id": drafted, "title": "Hinge clashes with the lid", "state": "draft"}));
    // This test process draws no pictures: the try fails once, is said in the log and not repeated.
    assert!(crate::control::view_png(&h.app, 64, 40).is_err() && tickets::load(&root, &drafted).unwrap().wants_view);
    let mut tries = 0;
    crate::tickets::attach_wanted_view(&mut h.app, |_| {
        tries += 1;
        Ok(png())
    });
    assert_eq!(tries, 0, "{drafted} was tried once already");
    // A second draft asking for the view, with a picture to take: it is attached.
    let again = agent_calls(&mut h, "ticket.draft", json!({"title": "Vent slots too close to the edge", "view": true}))["ticket"]["id"].as_str().unwrap().to_string();
    // (Frames may have tried it already without a renderer; the test takes that try back.)
    crate::tickets::refresh(&mut h.app);
    h.app.tickets.viewed.retain(|id| id != &again);
    crate::tickets::attach_wanted_view(&mut h.app, |_| Ok(png()));
    let viewed = tickets::load(&root, &again).unwrap();
    assert_eq!((viewed.wants_view, viewed.attachments.clone()), (false, vec![format!("attachments/{again}-view.png")]), "the view's picture is attached");
    assert!(std::fs::read(root.join(format!("attachments/{again}-view.png"))).unwrap().starts_with(b"\x89PNG"));

    // A done ticket whose run the model moved past: 需重查, its state kept.
    let parts = vec![Part { kind: "body".into(), id: "Body1".into() }];
    let done = tickets::create(&root, Draft { title: "Plate".into(), parts, ..Draft::default() }).unwrap().id;
    tickets::work_myself(&root, &done, 1).unwrap();
    tickets::ask_check(&root, &done, 2).unwrap();
    tickets::check_started(&root, &done, "run-9").unwrap();
    tickets::check_ran(&root, &done, tickets::RunResult { run: "run-9".into(), outcome: "done".into(), counts: [("checked".to_string(), 3)].into(), freshness: "current".into() }).unwrap();
    tickets::done(&root, &done, seen(&root, &done)).unwrap();
    h.settle();
    h.app.tickets.freshness.insert("run-9".into(), "staleModel".into());
    crate::tickets::mark_stale(&mut h.app, &root);
    let marked = tickets::load(&root, &done).unwrap();
    assert_eq!(marked.state, TicketState::Done);
    assert!(marked.recheck.as_deref().is_some_and(|why| why.contains("model changed")), "{:?}", marked.recheck);
    assert_eq!(tickets::events(&root, Some(&done)).unwrap().pop().map(|e| (e.action, e.actor)), Some(("recheck".to_string(), Actor::App)));
}

/// A stand-in run's report: `items` as (state, rule, part, found, required), counted by state, with
/// `checked` rules besides.
fn reported(items: &[(&str, &str, &str, &str, &str)], checked: u64) -> crate::tickets::Report {
    let mut counts = std::collections::BTreeMap::from([("checked".to_string(), checked)]);
    for (state, ..) in items {
        *counts.entry(state.to_string()).or_insert(0) += 1;
    }
    let items = items
        .iter()
        .map(|(state, rule, part, found, required)| crate::tickets::RunItem {
            rule: rule.to_string(),
            state: state.to_string(),
            count: 1,
            parts: vec![part.to_string()],
            values: vec![(found.to_string(), Some(required.to_string()))],
        })
        .collect();
    crate::tickets::Report { run: String::new(), done: true, outcome: "done".into(), counts, freshness: "current".into(), items }
}

const RELIEF: (&str, &str, &str, &str, &str) = ("advisory", "sm.bend_relief", "Body1", "1.2 mm", "2 mm");
const EDGE: (&str, &str, &str, &str, &str) = ("advisory", "sm.hole_to_edge", "Body1", "2.5 mm", "3 mm");
const CRACK: (&str, &str, &str, &str, &str) = ("failed", "sm.min_bend_radius", "Body1", "0.5 mm", "2 mm");

/// A window with a scripted agent and a ticket on Body1 handed to it; stand-in runs answer its
/// checks in turn.
fn handed(name: &str, answers: Vec<Result<crate::tickets::Report, String>>) -> (Scratch, Harness, std::path::PathBuf, StandIn, String) {
    let scratch = Scratch::new(name);
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    let parts = vec![Part { kind: "body".into(), id: "Body1".into() }];
    let id = tickets::draft(&root, Draft { title: "Base plate".into(), parts, ..Draft::default() }, Actor::Agent).unwrap().id;
    tickets::confirm(&root, &id, seen(&root, &id), false).unwrap();
    h.app.tickets.stand_in = Some(crate::tickets::StandInRuns { answers: answers.into(), ..Default::default() });
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::start(&mut h.app, &ctx, &id, seen(&root, &id));
    h.settle();
    (scratch, h, root, agent, id)
}

/// The scripted agent says the ticket is finished and its turn ends; the frames that follow.
fn says_finished(h: &mut Harness, agent: &StandIn, id: &str, text: &str) {
    agent_calls(h, "ticket.update", json!({"id": id, "action": "finished", "text": text}));
    agent.end_turn();
    h.frames(12);
}

/// [`dump`] with ticket `id` open, its words in Chinese, as the owner reads it.
fn dump_zh(h: &mut Harness, name: &str, id: &str) {
    let language = std::mem::replace(&mut h.app.preferences.language, "zh".into());
    let ctx = h.ctx.clone();
    crate::tickets::receive(&mut h.app, &ctx, &json!({"type": "ticket_open", "id": id}).to_string()).unwrap();
    dump(h, name);
    h.app.preferences.language = language;
}

fn started(h: &Harness) -> usize {
    h.app.tickets.stand_in.as_ref().map_or(0, |runs| runs.started)
}

fn rounds(root: &std::path::Path, id: &str) -> Option<tickets::Rounds> {
    tickets::load(root, id).unwrap().rounds
}

/// Finished → the check starts by itself → its findings reach the agent → it fixes and says
/// finished again → the second check is clean → the ticket waits for a person. The record says the
/// Assistant's work asked for each check, with the round; the page is told the rounds.
#[test]
fn the_assistants_finished_runs_its_check_by_itself_and_the_findings_come_back_until_it_is_clean() {
    let (_scratch, mut h, root, agent, id) = handed("rounds", vec![Ok(reported(&[RELIEF, EDGE], 12)), Ok(reported(&[], 14))]);
    says_finished(&mut h, &agent, &id, "Plate made, 100 × 60 mm");
    assert_eq!(started(&h), 1, "the check started without a person");
    let ticket = tickets::load(&root, &id).unwrap();
    assert_eq!((ticket.state, ticket.worker, ticket.finished), (TicketState::InProgress, Some(tickets::Worker::Assistant), false));
    assert_eq!(ticket.rounds, Some(tickets::Rounds { round: 1, of: 3, stopped: None }));
    let sent = agent.prompts().last().cloned().unwrap_or_default();
    assert!(sent.contains(&format!("{id}: its check found work to do (round 1 of 3)")), "{sent}");
    assert!(sent.contains("advisory · sm.bend_relief · Body1 · ") && sent.contains("1.2 mm") && sent.contains("2 mm"), "part, numbers and rule: {sent}");
    assert!(sent.contains("sm.hole_to_edge") && sent.contains("Round 1 of 3 of fixing"), "{sent}");
    dump_zh(&mut h, "rounds-1-findings", &id);

    agent_calls(&mut h, "sheet.manage_rules", json!({"name": "SPCC 2.0", "thickness": "2 mm", "bend_radius": "2 mm", "k_factor": 0.44, "active": true}));
    says_finished(&mut h, &agent, &id, "Bend relief 2 mm, holes 3 mm from the edge");
    assert_eq!(started(&h), 2, "a second check, by itself");
    let ticket = tickets::load(&root, &id).unwrap();
    assert_eq!(ticket.state, TicketState::Waiting);
    assert_eq!(ticket.rounds, Some(tickets::Rounds { round: 1, of: 3, stopped: Some(tickets::Stop::Clean) }));
    let asked: Vec<(Actor, Option<u32>)> =
        tickets::events(&root, Some(&id)).unwrap().iter().filter(|e| e.action == "check_asked").map(|e| (e.actor, e.rounds.as_ref().map(|r| r.round))).collect();
    assert_eq!(asked, [(Actor::Agent, Some(0)), (Actor::Agent, Some(1))], "the Assistant's work asked for both, with the round");
    let page = told(&mut h);
    assert_eq!(ticket_of(&page, &id)["rounds"], json!({"round": 1, "of": 3, "stopped": "clean"}));
    // The check's kind is carried on the ticket and its lines, and the page is told its word.
    assert_eq!((ticket_of(&page, &id)["check"]["kind"].as_str(), page["kinds"]["dfm"].is_string()), (Some("dfm"), true));
    let kinds: Vec<Option<tickets::CheckKind>> = tickets::events(&root, Some(&id)).unwrap().iter().filter(|e| e.action.starts_with("check_")).map(|e| e.kind).collect();
    assert!(kinds.len() == 6 && kinds.iter().all(|kind| *kind == Some(tickets::CheckKind::Dfm)), "{kinds:?}");
    let prompts = agent.prompts().len();
    h.frames(6);
    assert_eq!(agent.prompts().len(), prompts, "a clean end tells the Assistant nothing");
    dump_zh(&mut h, "rounds-2-clean", &id);
}

fn ticket_of<'a>(state: &'a Value, id: &str) -> &'a Value {
    ticket(state, id)
}

/// Three rounds used up: 「建议」 left → waiting; something failed left → blocked; each with the
/// Assistant asked for its comment, which it writes. Four checks for three rounds.
#[test]
fn after_the_last_round_advisories_leave_it_waiting_and_a_failure_blocks_it_each_with_a_comment() {
    for (name, left, state) in [("rounds-advisory", RELIEF, TicketState::Waiting), ("rounds-failed", CRACK, TicketState::Blocked)] {
        let answers = (0..4).map(|_| Ok(reported(&[left], 10))).collect();
        let (_scratch, mut h, root, agent, id) = handed(name, answers);
        for n in 1..=3u32 {
            says_finished(&mut h, &agent, &id, &format!("Round {n}: tried a larger relief"));
            assert_eq!(rounds(&root, &id).map(|r| (r.round, r.stopped)), Some((n, None)), "{name}");
            if n == 2 {
                assert_eq!(ticket_of(&told(&mut h), &id)["rounds"]["round"], 2);
                dump_zh(&mut h, &format!("{name}-round-2"), &id);
            }
        }
        says_finished(&mut h, &agent, &id, "Round 4: tried a larger radius");
        assert_eq!(started(&h), 4, "{name}: four checks for three rounds");
        let ticket = tickets::load(&root, &id).unwrap();
        assert_eq!(ticket.state, state, "{name}");
        let sent = agent.prompts().last().cloned().unwrap_or_default();
        assert!(sent.contains("a person takes it from here") && sent.contains(left.1), "{name}: {sent}");
        if state == TicketState::Blocked {
            assert!(ticket.blocked.as_deref().is_some_and(|why| why.contains("after 3 of 3 rounds") && why.contains("1 failed")), "{:?}", ticket.blocked);
            let refused = h.call("engine.execute", json!({"command": "ticket.update", "params": {"id": id, "action": "state", "to": "done", "text": "x"}}));
            assert_eq!(refused["ok"], false, "the Assistant does not finish what failed: {refused}");
        }
        agent_calls(&mut h, "ticket.update", json!({"id": id, "action": "comment", "text": format!("Left: {} on Body1. Tried: relief 1.5, 2 and 2.5 mm.", left.1)}));
        agent.end_turn();
        h.frames(6);
        let last = tickets::events(&root, Some(&id)).unwrap().pop().unwrap();
        assert_eq!((last.action.as_str(), last.actor), ("comment", Actor::Agent));
        assert_eq!(started(&h), 4, "{name}: nothing more starts");
        dump_zh(&mut h, &format!("{name}-stopped"), &id);
    }
}

/// With the ticket's switch off, or the project's automatic checks off, nothing starts: the ticket
/// waits for a person as before (no check applies: waiting; automatic checks off: in progress,
/// finished, for a person's 跑检查).
#[test]
fn with_either_switch_off_nothing_starts() {
    let (_scratch, mut h, root, agent, id) = handed("switch-off", vec![Ok(reported(&[RELIEF], 3))]);
    crate::tickets::edit(&mut h.app, &id, seen(&root, &id), tickets::Edit { checks: Some(false), ..tickets::Edit::default() });
    says_finished(&mut h, &agent, &id, "Plate made");
    assert_eq!((state(&root, &id), started(&h)), (TicketState::Waiting, 0), "the ticket's switch is off");

    let (_scratch, mut h, root, agent, id) = handed("project-off", vec![Ok(reported(&[RELIEF], 3))]);
    solidcraft_engine::project::Project::open(&root).unwrap().update_ci(Some(false), None).unwrap();
    says_finished(&mut h, &agent, &id, "Plate made");
    let ticket = tickets::load(&root, &id).unwrap();
    assert_eq!((ticket.state, ticket.finished, ticket.check.is_none(), started(&h)), (TicketState::InProgress, true, true, 0));
}

/// The agent's tool request as it reaches the gate in a turn of its own.
fn chat_tool(h: &mut Harness, tx: &std::sync::mpsc::Sender<crate::ControlRequest>, command: &str, params: Value) -> Value {
    let (req, reply) = crate::ControlRequest::trusted_gui("engine.execute", json!({"command": command, "params": params}));
    tx.send(req).unwrap();
    for _ in 0..200 {
        h.frames(1);
        if let Ok(answer) = reply.try_recv() {
            return answer;
        }
    }
    panic!("{command}: no answer");
}

/// While the check runs the agent's changes to the model are refused (a ticket comment is not); a
/// person's message still reaches it, its note saying to wait. A person's 跑检查 in the middle takes
/// the running check as theirs and ends the rounds; a person's move ends them too.
#[test]
fn no_change_is_taken_while_the_check_runs_and_a_persons_step_ends_the_rounds() {
    let (_scratch, mut h, root, agent, id) = handed("hold", vec![Ok(reported(&[RELIEF], 3))]);
    h.app.tickets.stand_in.as_mut().unwrap().hold = true;
    says_finished(&mut h, &agent, &id, "Plate made");
    assert_eq!(tickets::load(&root, &id).unwrap().check.map(|c| (c.state, c.by)), Some((CheckState::Running, Actor::Agent)));
    let tools = crate::agent_chat::install_test_tools(&mut h.app);
    let ctx = h.ctx.clone();
    crate::agent_chat::send(&mut h.app, &ctx, "How is it going?".into());
    h.frames(2);
    let sent = agent.prompts().last().cloned().unwrap_or_default();
    assert!(sent.contains(&format!("The check of {id} is running: do not change the model")), "{sent}");
    let before = h.app.session.doc.clone();
    let refused = chat_tool(&mut h, &tools, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(refused["error"].as_str().is_some_and(|e| e.contains("is running: wait for its result")), "{refused}");
    assert!(Arc::ptr_eq(&h.app.session.doc, &before), "the model did not change");
    let comment = chat_tool(&mut h, &tools, "ticket.update", json!({"id": id, "action": "comment", "text": "Waiting for the check."}));
    assert_eq!(comment["ok"], true, "{comment}");
    agent.end_turn();
    h.frames(4);
    // A person presses 跑检查: the running check is theirs; when it ends the ticket waits for them.
    crate::tickets::ask_check(&mut h.app, &id, seen(&root, &id));
    assert_eq!(rounds(&root, &id).and_then(|r| r.stopped), Some(tickets::Stop::Person));
    let prompts = agent.prompts().len();
    h.app.tickets.stand_in.as_mut().unwrap().hold = false;
    h.frames(8);
    let ticket = tickets::load(&root, &id).unwrap();
    assert_eq!((ticket.state, ticket.check.map(|c| c.by)), (TicketState::Waiting, Some(Actor::Person)));
    assert_eq!((agent.prompts().len(), started(&h)), (prompts, 1), "no findings to the Assistant, no second check");

    // A person moves the ticket while the Assistant fixes: the rounds end, and its finished starts
    // nothing more.
    let (_scratch, mut h, root, agent, id) = handed("moved", vec![Ok(reported(&[RELIEF], 3)), Ok(reported(&[], 3))]);
    says_finished(&mut h, &agent, &id, "Plate made");
    assert_eq!(rounds(&root, &id).map(|r| r.round), Some(1));
    let ctx = h.ctx.clone();
    let moved = json!({"type": "ticket_set_state", "id": id, "revision": seen(&root, &id), "to": "confirmed", "reason": "Wrong part"});
    crate::tickets::receive(&mut h.app, &ctx, &moved.to_string()).unwrap();
    assert_eq!(rounds(&root, &id).and_then(|r| r.stopped), Some(tickets::Stop::Person));
    let refused = h.call("engine.execute", json!({"command": "ticket.update", "params": {"id": id, "action": "finished", "text": "x"}}));
    assert_eq!(refused["ok"], false);
    agent.end_turn();
    h.frames(8);
    assert_eq!((state(&root, &id), started(&h)), (TicketState::Confirmed, 1));
}

/// The automatic check goes through the Review panel's admission, as a person's 跑检查 does: with an
/// authority it is billed once and binds its run (the generic data finds nothing to fix, so the
/// ticket waits); without one it is refused, the ticket is blocked with the refusal, and the
/// Assistant is asked for its comment.
#[test]
fn the_automatic_check_takes_the_review_entrys_admission_billed_once_or_refused() {
    let scratch = Scratch::new("auto-real");
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    let billing = crate::dfm_review::tests::CountingBilling::default();
    let ledger = crate::dfm_review::tests::ReviewLedger::default();
    crate::dfm_review::tests::install(&mut h.app, &billing, &ledger);
    let parts = vec![Part { kind: "body".into(), id: "Body1".into() }];
    let id = tickets::draft(&root, Draft { title: "Tray".into(), parts, ..Draft::default() }, Actor::Agent).unwrap().id;
    tickets::confirm(&root, &id, seen(&root, &id), false).unwrap();
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::start(&mut h.app, &ctx, &id, seen(&root, &id));
    h.settle();
    for (command, params) in [
        ("sheet.manage_rules", json!({"name": "SPCC 2.0", "thickness": "2 mm", "bend_radius": "2 mm", "k_factor": 0.44, "active": true})),
        ("sketch.create", json!({"plane": "XY", "name": "Base outline"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [100, 60]})),
        ("sketch.finish", json!({})),
        ("sheet.flange", json!({"sketch": "Base outline", "body_name": "Body1"})),
        ("sheet.flange", json!({"edges": [[50, 0, 2], [100, 30, 2], [50, 60, 2], [0, 30, 2]], "height": 30, "body": "Body1"})),
    ] {
        agent_calls(&mut h, command, params);
    }
    says_finished(&mut h, &agent, &id, "Tray made");
    let ran = |root: &std::path::Path| tickets::load(root, &id).unwrap().check.is_some_and(|c| matches!(c.state, CheckState::Ran | CheckState::Refused));
    let began = std::time::Instant::now();
    while !ran(&root) && began.elapsed() < Duration::from_secs(60) {
        h.frames(30);
        std::thread::sleep(Duration::from_millis(20));
    }
    let ticket = tickets::load(&root, &id).unwrap();
    let check = ticket.check.clone().unwrap();
    assert_eq!((check.state, check.by, billing.commits()), (CheckState::Ran, Actor::Agent, 1), "one billed Review operation: {:?}", h.app.dfm_review.error);
    assert_eq!(check.run, h.app.dfm_review.mapped_run, "the run the Review entry reserved");
    // The generic data fails nothing; what it leaves as 「建议」 goes back to the Assistant.
    let advisory = check.counts.get("advisory").copied().unwrap_or(0);
    assert_eq!(check.counts.get("failed").copied().unwrap_or(0), 0, "{:?}", check.counts);
    let expected = if advisory == 0 { (TicketState::Waiting, Some(tickets::Stop::Clean)) } else { (TicketState::InProgress, None) };
    assert_eq!((ticket.state, ticket.rounds.and_then(|r| r.stopped)), expected, "{:?}", check.counts);
    dump_zh(&mut h, "rounds-real-run", &id);

    // No authority: refused where a person's 跑检查 is refused, said on the ticket, which is blocked.
    let scratch = Scratch::new("auto-refused");
    let (mut h, root) = window(&scratch);
    let agent = StandIn::default();
    crate::agent_chat::install_test_backend(&mut h.app, Box::new(agent.clone()));
    let parts = vec![Part { kind: "body".into(), id: "Body1".into() }];
    let id = tickets::draft(&root, Draft { title: "Plate".into(), parts, ..Draft::default() }, Actor::Agent).unwrap().id;
    tickets::confirm(&root, &id, seen(&root, &id), false).unwrap();
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::start(&mut h.app, &ctx, &id, seen(&root, &id));
    h.settle();
    says_finished(&mut h, &agent, &id, "Plate made");
    let ticket = tickets::load(&root, &id).unwrap();
    assert_eq!(ticket.state, TicketState::Blocked, "{:?}", ticket.check);
    let reason = ticket.check.and_then(|c| c.reason).unwrap_or_default();
    assert!(reason.contains("UNSAFE_TO_START") || reason.contains("AUTH_UNAVAILABLE"), "{reason}");
    assert!(ticket.blocked.as_deref().is_some_and(|why| why.contains(&reason)), "the refusal is said: {:?}", ticket.blocked);
    assert_eq!(ticket.rounds.and_then(|r| r.stopped), Some(tickets::Stop::Refused));
    assert!(agent.prompts().last().is_some_and(|p| p.contains("a person takes it from here") && p.contains(&reason)), "{:?}", agent.prompts().last());
    dump_zh(&mut h, "rounds-refused", &id);
}
