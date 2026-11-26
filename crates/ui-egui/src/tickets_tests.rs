//! The host side of part tickets: a person's steps, Start's message and chip, the note every
//! message carries, and one scripted agent's run from a draft to a ticket waiting for the person
//! with its DFM run bound. (No view is drawn: the person's steps are called as a view would.)

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
    let part = Some(Part { kind: "component".into(), id: "Lid".into() });
    let lid = tickets::draft(&root, Draft { title: "Lid".into(), part, ..Draft::default() }, Actor::Agent).unwrap().id;
    h.settle();
    assert_eq!(h.app.tickets.tickets.len(), 2, "the host read both");
    crate::tickets::done(&mut h.app, &lid);
    assert!(h.app.tickets.error.as_deref().is_some_and(|e| e.contains("is draft")), "a draft is not done: {:?}", h.app.tickets.error);
    assert_eq!(state(&root, &lid), TicketState::Draft);
    crate::tickets::confirm(&mut h.app, &lid);
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
    tickets::confirm(&root, &id).unwrap();
    h.settle();
    let ctx = h.ctx.clone();
    crate::tickets::start(&mut h.app, &ctx, &id);
    h.settle();
    assert_eq!(state(&root, &id), TicketState::InProgress);
    let prompts = agent.prompts();
    assert_eq!(prompts.len(), 1, "one message: {prompts:?}");
    let sent = &prompts[0];
    assert!(sent.contains("Work on part ticket T-2 alone: Bracket."), "{sent}");
    assert!(sent.contains("You are working on T-2 alone") && sent.contains("Two M4 holes, centres 40 ± 0.1 mm"), "the ticket in full: {sent}");
    assert!(sent.contains("\"action\": \"check\""), "how to ask for the check: {sent}");
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
    tickets::confirm(&root, &other).unwrap();
    crate::tickets::start(&mut h.app, &h.ctx.clone(), &other);
    assert_eq!(state(&root, &other), TicketState::Confirmed);
    assert!(h.app.tickets.error.as_deref().is_some_and(|e| e.contains("T-2")), "{:?}", h.app.tickets.error);
}

/// The loop with a scripted agent: asked in the chat, it drafts tickets; a person confirms one
/// and presses Start; the agent changes the model and asks for the check; the app runs the DFM
/// check through the Review entry (stand-in billing and journal, the real background worker) and
/// binds the run, so the ticket waits for the person, who marks it done.
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
    let drafted =
        agent_calls(&mut h, "ticket.draft", json!({"title": "Base plate", "requirements": ["40 × 30 mm"], "part": {"kind": "body", "id": "Body1"}}));
    assert_eq!(drafted["ticket"]["id"], "T-2");
    agent.end_turn();
    h.settle();
    // The agent cannot confirm its own draft.
    let refused = h.call("engine.execute", json!({"command": "ticket.update", "params": {"id": "T-2", "action": "confirm"}}));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(state(&root, "T-2"), TicketState::Draft);

    crate::tickets::confirm(&mut h.app, "T-2");
    crate::tickets::start(&mut h.app, &ctx, "T-2");
    h.settle();
    assert_eq!(state(&root, "T-2"), TicketState::InProgress);
    assert!(agent.prompts().last().is_some_and(|p| p.contains("You are working on T-2 alone")), "{:?}", agent.prompts().last());

    // The agent makes the part and asks for the check.
    agent_calls(&mut h, "solid.box", json!({"length": 40, "width": 30, "height": 1.5}));
    agent_calls(&mut h, "ticket.update", json!({"id": "T-2", "action": "check", "reason": "Plate made"}));
    agent.end_turn();

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

    crate::tickets::done(&mut h.app, "T-2");
    assert_eq!(state(&root, "T-2"), TicketState::Done);
    let steps: Vec<(String, Actor)> = tickets::events(&root, Some("T-2")).unwrap().into_iter().map(|e| (e.action, e.actor)).collect();
    let expected = [
        ("drafted", Actor::Agent),
        ("confirmed", Actor::Person),
        ("started", Actor::Person),
        ("check_asked", Actor::Agent),
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
    let id = tickets::draft(&root, Draft { title: "Plate".into(), ..Draft::default() }, Actor::Agent).unwrap().id;
    tickets::confirm(&root, &id).unwrap();
    tickets::start(&root, &id).unwrap();
    tickets::ask_check(&root, &id, Actor::Agent, None).unwrap();
    h.frames(4);
    let ticket = tickets::load(&root, &id).unwrap();
    assert_eq!(ticket.state, TicketState::InProgress);
    let check = ticket.check.unwrap();
    assert_eq!(check.state, CheckState::Refused);
    assert!(check.reason.as_deref().is_some_and(|r| r.contains("UNSAFE_TO_START") || r.contains("AUTH_UNAVAILABLE")), "{:?}", check.reason);
    crate::tickets::done(&mut h.app, &id);
    assert_eq!(state(&root, &id), TicketState::InProgress, "no run, no done");
    // The person can ask again; with no authority it is refused again, and said again.
    crate::tickets::ask_check(&mut h.app, &id);
    h.frames(4);
    let record = tickets::events(&root, Some(&id)).unwrap();
    assert_eq!(record.iter().filter(|e| e.action == "check_refused").count(), 2);
    assert_eq!(record.iter().rev().nth(1).map(|e| e.actor), Some(Actor::Person), "the second ask is the person's");
}
