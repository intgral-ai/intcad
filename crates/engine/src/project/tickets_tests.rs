//! The part-ticket store and its commands: who may make which move, and the record of each step.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;
use crate::Session;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("intcad-tickets-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A project made the way New Project makes one, with the open design in it.
fn project(scratch: &Scratch) -> (Session, PathBuf) {
    let created = crate::project::create(&scratch.0, "enclosure", "enclosure").expect("a project");
    let session = Session { path: Some(created.design.to_string_lossy().into_owned()), ..Session::default() };
    (session, created.project.root().to_path_buf())
}

fn bracket() -> Draft {
    Draft {
        title: "Bracket: 2 mm 5052, two M4 holes 40 mm apart".into(),
        text: "The bracket that holds the board.".into(),
        requirements: vec!["Thickness 2 mm".into(), "Hole centres 40 ± 0.1 mm".into()],
        parent: None,
        part: Some(Part { kind: "component".into(), id: "Bracket".into() }),
    }
}

fn ran(run: &str) -> RunResult {
    RunResult {
        run: run.into(),
        outcome: "complete".into(),
        counts: BTreeMap::from([("checked".into(), 3), ("unsupported".into(), 5)]),
        freshness: "current".into(),
    }
}

#[test]
fn the_first_draft_is_the_top_ticket_and_later_ones_go_under_it() {
    let scratch = Scratch::new("tree");
    let (_, root) = project(&scratch);
    let top = draft(&root, Draft { title: "Enclosure for the sensor board".into(), ..Draft::default() }, Actor::Agent).unwrap();
    assert_eq!((top.id.as_str(), top.parent.as_deref(), top.state), ("T-1", None, State::Draft));
    let part = draft(&root, bracket(), Actor::Agent).unwrap();
    assert_eq!((part.id.as_str(), part.parent.as_deref()), ("T-2", Some("T-1")));
    assert_eq!(part.part, Some(Part { kind: "component".into(), id: "Bracket".into() }));
    assert_eq!(part.requirements.len(), 2);
    // One level of children: a part ticket cannot be a parent.
    let error = draft(&root, Draft { title: "Nut".into(), parent: Some("T-2".into()), ..Draft::default() }, Actor::Agent).unwrap_err();
    assert!(error.to_string().contains("one level deep"), "{error}");
    let listed: Vec<String> = list(&root).unwrap().into_iter().map(|t| t.id).collect();
    assert_eq!(listed, ["T-1", "T-2"]);
    // The Spec files at the root are left as they were.
    assert!(root.join("requirements.md").is_file() && root.join("design-intent.md").is_file());
    assert!(folder(&root).join("T-2.json").is_file() && folder(&root).join(EVENTS).is_file());
}

#[test]
fn a_ticket_goes_from_draft_to_done_only_by_the_allowed_moves_and_each_step_is_recorded() {
    let scratch = Scratch::new("moves");
    let (_, root) = project(&scratch);
    draft(&root, Draft { title: "Enclosure".into(), ..Draft::default() }, Actor::Agent).unwrap();
    let id = draft(&root, bracket(), Actor::Agent).unwrap().id;

    // A draft cannot be started, asked for its check, or done.
    for refused in [start(&root, &id).err(), ask_check(&root, &id, Actor::Agent, None).err(), done(&root, &id).err()] {
        assert!(matches!(refused, Some(TicketError::Refused(_))), "{refused:?}");
    }
    assert_eq!(load(&root, &id).unwrap().revision, 1, "a refused move changes nothing");

    assert_eq!(confirm(&root, &id).unwrap().state, State::Confirmed);
    assert!(confirm(&root, &id).is_err(), "a confirmed ticket is not confirmed again");
    assert_eq!(start(&root, &id).unwrap().state, State::InProgress);
    // Done before the check has run is refused.
    assert!(done(&root, &id).is_err());
    let asked = ask_check(&root, &id, Actor::Agent, Some("Holes moved to 40 mm".into())).unwrap();
    assert_eq!((asked.state, asked.check.as_ref().map(|c| c.state)), (State::InProgress, Some(CheckState::Asked)));
    check_started(&root, &id, "run-1").unwrap();
    let waiting = check_ran(&root, &id, ran("run-1")).unwrap();
    assert_eq!(waiting.state, State::Waiting);
    let check = waiting.check.clone().unwrap();
    assert_eq!((check.run.as_deref(), check.outcome.as_deref(), check.freshness.as_deref()), (Some("run-1"), Some("complete"), Some("current")));
    let finished = done(&root, &id).unwrap();
    assert_eq!((finished.state, finished.revision), (State::Done, 7));

    let record = events(&root, Some(&id)).unwrap();
    let steps: Vec<(&str, Actor)> = record.iter().map(|e| (e.action.as_str(), e.actor)).collect();
    assert_eq!(
        steps,
        [
            ("drafted", Actor::Agent),
            ("confirmed", Actor::Person),
            ("started", Actor::Person),
            ("check_asked", Actor::Agent),
            ("check_started", Actor::App),
            ("check_ran", Actor::App),
            ("done", Actor::Person),
        ]
    );
    assert_eq!(record.iter().map(|e| e.revision).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7]);
    let bound = record.iter().find(|e| e.action == "check_ran").unwrap();
    assert_eq!(bound.run.as_deref(), Some("run-1"));
    assert_eq!(bound.note.as_deref(), Some("complete · 3 checked · 5 not covered"));
    assert_eq!(record.last().and_then(|e| e.run.as_deref()), Some("run-1"), "done names the run it was confirmed on");
    // The record is only appended to: the file starts with the first line written.
    let text = std::fs::read_to_string(folder(&root).join(EVENTS)).unwrap();
    assert!(text.lines().next().is_some_and(|line| line.contains("\"drafted\"") && line.contains("\"T-1\"")), "{text}");
}

#[test]
fn a_refused_check_keeps_the_ticket_in_progress_with_the_reason_and_blocking_needs_one() {
    let scratch = Scratch::new("refused");
    let (_, root) = project(&scratch);
    let id = draft(&root, bracket(), Actor::Agent).unwrap().id;
    confirm(&root, &id).unwrap();
    start(&root, &id).unwrap();
    ask_check(&root, &id, Actor::Agent, None).unwrap();
    let refused = check_refused(&root, &id, "AUTH_UNAVAILABLE: no host billing authority").unwrap();
    assert_eq!(refused.state, State::InProgress);
    assert_eq!(refused.check.as_ref().map(|c| c.state), Some(CheckState::Refused));
    assert!(done(&root, &id).is_err(), "no run, no done");
    assert!(block(&root, &id, "  ").is_err(), "a block needs its reason");
    let blocked = block(&root, &id, "Which flange height? requirements.md has none").unwrap();
    assert_eq!((blocked.state, blocked.blocked.as_deref()), (State::Blocked, Some("Which flange height? requirements.md has none")));
    assert_eq!(start(&root, &id).unwrap().state, State::InProgress, "a person starts a blocked ticket again");
}

#[test]
fn ids_are_never_paths_and_hostile_text_is_capped() {
    let scratch = Scratch::new("hostile");
    let (_, root) = project(&scratch);
    for id in ["../T-1", "T-0", "T-01", "T-", "T-1/../x", "t-1", "T-1234567"] {
        assert!(matches!(load(&root, id), Err(TicketError::Invalid(_))), "{id}");
    }
    let long = "x".repeat(MAX_TITLE + 1);
    assert!(draft(&root, Draft { title: long, ..Draft::default() }, Actor::Agent).is_err());
    assert!(draft(&root, Draft { title: "  ".into(), ..Draft::default() }, Actor::Agent).is_err());
    let part = Some(Part { kind: "assembly".into(), id: "x".into() });
    assert!(draft(&root, Draft { title: "A".into(), part, ..Draft::default() }, Actor::Agent).is_err());
    assert!(list(&root).unwrap().is_empty(), "nothing refused was written");
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn the_agents_commands_draft_and_ask_for_the_check_but_cannot_confirm_or_finish() {
    let scratch = Scratch::new("commands");
    let (mut s, root) = project(&scratch);
    let top = run(&mut s, "ticket.draft", json!({"title": "Enclosure", "requirements": ["5052-H32, 1.5 mm"]}));
    assert_eq!(top["ticket"]["id"], "T-1");
    assert_eq!(top["ticket"]["state"], "draft");
    assert_eq!(top["events"][0]["actor"], "agent");
    let part = run(&mut s, "ticket.draft", json!({"title": "Lid", "part": {"kind": "component", "id": "Lid"}}));
    assert_eq!(part["ticket"]["parent"], "T-1");
    assert_eq!(run(&mut s, "ticket.list", json!({}))["tickets"].as_array().map(Vec::len), Some(2));

    // No command confirms, starts or finishes; update refuses those words.
    for missing in ["ticket.confirm", "ticket.done", "ticket.start", "ticket.complete"] {
        assert!(crate::cmd::find_command(missing).is_none(), "{missing} must not exist");
    }
    for action in ["confirm", "done", "start"] {
        let error = s.execute("ticket.update", &json!({"id": "T-2", "action": action})).unwrap_err().to_string();
        assert!(error.contains("only a person"), "{action}: {error}");
    }
    // A draft is not worked on: asking for its check is refused.
    let error = s.execute("ticket.update", &json!({"id": "T-2", "action": "check"})).unwrap_err().to_string();
    assert!(error.contains("T-2 is draft"), "{error}");

    confirm(&root, "T-2").unwrap();
    start(&root, "T-2").unwrap();
    let asked = run(&mut s, "ticket.update", json!({"id": "T-2", "action": "check", "reason": "Lid flange added"}));
    assert_eq!(asked["ticket"]["check"]["state"], "asked");
    let inspected = run(&mut s, "ticket.inspect", json!({"id": "T-2"}));
    let actions: Vec<&str> = inspected["events"].as_array().unwrap().iter().filter_map(|e| e["action"].as_str()).collect();
    assert_eq!(actions, ["drafted", "confirmed", "started", "check_asked"]);
    // The commands are no undo steps: the design's history is untouched.
    assert!(s.undo.is_empty());
}

#[test]
fn a_design_outside_a_project_has_no_tickets() {
    let mut s = Session::default();
    let error = s.execute("ticket.list", &json!({})).unwrap_err().to_string();
    assert!(error.contains("project"), "{error}");
}
