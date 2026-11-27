//! The part-ticket store and its commands: who may make which move, the gates, and the record of
//! each step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

fn part(kind: &str, id: &str) -> Part {
    Part { kind: kind.into(), id: id.into() }
}

fn bracket() -> Draft {
    Draft {
        title: "Bracket: 2 mm 5052, two M4 holes 40 mm apart".into(),
        text: "The bracket that holds the board.".into(),
        requirements: vec!["Thickness 2 mm".into(), "Hole centres 40 ± 0.1 mm".into()],
        parts: vec![part("component", "Bracket")],
        ..Draft::default()
    }
}

fn titled(title: &str) -> Draft {
    Draft { title: title.into(), ..Draft::default() }
}

/// The revision a person sees now.
fn seen(root: &Path, id: &str) -> u64 {
    load(root, id).unwrap().revision
}

fn ran(run: &str) -> RunResult {
    RunResult {
        run: run.into(),
        outcome: "complete".into(),
        counts: BTreeMap::from([("checked".into(), 3), ("unsupported".into(), 5)]),
        freshness: "current".into(),
    }
}

/// A confirmed ticket handed to the Assistant.
fn handed(root: &Path, draft_of: Draft) -> String {
    let id = draft(root, draft_of, Actor::Agent).unwrap().id;
    confirm(root, &id, seen(root, &id), false).unwrap();
    start(root, &id, seen(root, &id)).unwrap();
    id
}

/// The check of a ticket in progress, started by a person and bound by the app.
fn checked(root: &Path, id: &str, result: RunResult) {
    ask_check(root, id, seen(root, id)).unwrap();
    check_started(root, id, &result.run).unwrap();
    check_ran(root, id, result).unwrap();
}

fn actions(root: &Path, id: &str) -> Vec<(String, Actor)> {
    events(root, Some(id)).unwrap().into_iter().map(|e| (e.action, e.actor)).collect()
}

fn refusal<T: std::fmt::Debug>(result: Result<T, TicketError>) -> String {
    match result {
        Err(TicketError::Refused(why)) | Err(TicketError::Invalid(why)) => why,
        Err(error @ (TicketError::Stale { .. } | TicketError::NotDone { .. } | TicketError::Waits { .. })) => error.to_string(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_ticket_stands_on_its_own_and_sub_tickets_go_one_level_deep() {
    let scratch = Scratch::new("tree");
    let (_, root) = project(&scratch);
    // No top ticket: the first draft is a ticket of its own, and so is the next.
    let first = draft(&root, bracket(), Actor::Agent).unwrap();
    assert_eq!((first.id.as_str(), first.parent.as_deref(), first.state, first.worker), ("T-1", None, State::Draft, None));
    let second = draft(&root, titled("Lid"), Actor::Agent).unwrap();
    assert_eq!(second.parent, None, "nothing goes under a ticket by itself");
    // Several parts, each once; a joint adjustment of two components.
    let joint = Draft {
        title: "Lid and base: align the hinge".into(),
        parts: vec![part("component", "Lid"), part("component", "Base"), part("component", "Lid")],
        parent: Some("T-1".into()),
        ..Draft::default()
    };
    let sub = draft(&root, joint, Actor::Agent).unwrap();
    assert_eq!((sub.parent.as_deref(), sub.parts.len()), (Some("T-1"), 2));
    // One level: a sub-ticket is no parent; a parent must exist.
    let error = refusal(draft(&root, Draft { parent: Some(sub.id.clone()), ..titled("Pin") }, Actor::Agent));
    assert!(error.contains("one level deep"), "{error}");
    assert!(matches!(draft(&root, Draft { parent: Some("T-9".into()), ..titled("Pin") }, Actor::Agent), Err(TicketError::NoSuchTicket(_))));
    // Any ticket may be a parent: a person's own ticket too.
    let mine = create(&root, titled("Label area")).unwrap();
    assert_eq!(create(&root, Draft { parent: Some(mine.id.clone()), ..titled("Label text") }).unwrap().parent, Some(mine.id));
    let listed: Vec<String> = list(&root).unwrap().into_iter().map(|t| t.id).collect();
    assert_eq!(listed, ["T-1", "T-2", "T-3", "T-4", "T-5"]);
    // The Spec files at the root are left as they were.
    assert!(root.join("requirements.md").is_file() && root.join("design-intent.md").is_file());
    let text = std::fs::read_to_string(folder(&root).join("T-3.json")).unwrap();
    assert!(text.contains("\"schemaVersion\": 2") && text.contains("\"parts\"") && !text.contains("\"part\""), "{text}");
}

#[test]
fn the_assistants_ticket_goes_from_draft_to_done_by_the_allowed_moves_and_each_step_is_recorded() {
    let scratch = Scratch::new("moves");
    let (_, root) = project(&scratch);
    let id = draft(&root, bracket(), Actor::Agent).unwrap().id;

    // A draft cannot be handed over, checked, or finished by the agent.
    for refused in [
        start(&root, &id, seen(&root, &id)).err(),
        ask_check(&root, &id, seen(&root, &id)).err(),
        finished(&root, &id, Some("Changed the part as asked."), &[], false).err(),
    ] {
        assert!(matches!(refused, Some(TicketError::Refused(_))), "{refused:?}");
    }
    assert_eq!(seen(&root, &id), 1, "a refused move changes nothing");

    assert_eq!(confirm(&root, &id, 1, false).unwrap().state, State::Confirmed);
    assert!(confirm(&root, &id, 2, false).is_err(), "a confirmed ticket is not accepted again");
    let started = start(&root, &id, 2).unwrap();
    assert_eq!((started.state, started.worker), (State::InProgress, Some(Worker::Assistant)));
    // The agent says it is finished: a check applies, so nothing starts and the ticket stays in
    // progress until a person starts the check.
    let said = finished(&root, &id, Some("Holes moved to 40 mm"), &[], false).unwrap();
    assert_eq!((said.state, said.finished, said.check.clone()), (State::InProgress, true, None));
    assert!(refusal(done(&root, &id, seen(&root, &id))).contains("check has not run"));
    let asked = ask_check(&root, &id, seen(&root, &id)).unwrap();
    assert_eq!(asked.check.as_ref().map(|c| c.state), Some(CheckState::Asked));
    check_started(&root, &id, "run-1").unwrap();
    let waiting = check_ran(&root, &id, ran("run-1")).unwrap();
    assert_eq!((waiting.state, waiting.finished), (State::Waiting, false));
    let check = waiting.check.clone().unwrap();
    assert_eq!((check.run.as_deref(), check.outcome.as_deref(), check.freshness.as_deref()), (Some("run-1"), Some("complete"), Some("current")));
    let finished_ticket = done(&root, &id, seen(&root, &id)).unwrap();
    assert_eq!((finished_ticket.state, finished_ticket.revision), (State::Done, 8));

    let record = events(&root, Some(&id)).unwrap();
    let steps: Vec<(&str, Actor)> = record.iter().map(|e| (e.action.as_str(), e.actor)).collect();
    assert_eq!(
        steps,
        [
            ("drafted", Actor::Agent),
            ("confirmed", Actor::Person),
            ("started", Actor::Person),
            ("finished", Actor::Agent),
            ("check_asked", Actor::Person),
            ("check_started", Actor::App),
            ("check_ran", Actor::App),
            ("done", Actor::Person),
        ]
    );
    assert_eq!(record.iter().map(|e| e.revision).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(record.get(3).and_then(|e| e.note.as_deref()), Some("Holes moved to 40 mm"));
    let bound = record.iter().find(|e| e.action == "check_ran").unwrap();
    assert_eq!((bound.run.as_deref(), bound.note.as_deref()), (Some("run-1"), Some("complete · 3 checked · 5 not covered")));
    let last = record.last().unwrap();
    assert_eq!((last.run.as_deref(), last.unchecked), (Some("run-1"), false), "done names the run, and it checked something");
    // The record is only appended to: the file starts with the first line written.
    let text = std::fs::read_to_string(folder(&root).join(EVENTS)).unwrap();
    assert!(text.lines().next().is_some_and(|line| line.contains("\"drafted\"") && line.contains("\"T-1\"")), "{text}");
}

#[test]
fn a_ticket_no_check_applies_to_waits_once_the_assistant_finishes_and_is_done_unchecked() {
    let scratch = Scratch::new("no-check");
    let (_, root) = project(&scratch);
    let talk = Draft { title: "How to meet the 3 mm clearance at the hinge".into(), checks: Some(false), ..Draft::default() };
    let id = handed(&root, talk);
    assert!(!load(&root, &id).unwrap().checks);
    assert!(refusal(ask_check(&root, &id, seen(&root, &id))).contains("no check applies"));
    let waiting = finished(&root, &id, Some("Changed the part as asked."), &[], false).unwrap();
    assert_eq!((waiting.state, waiting.finished), (State::Waiting, false));
    done(&root, &id, seen(&root, &id)).unwrap();
    let last = events(&root, Some(&id)).unwrap().pop().unwrap();
    assert_eq!((last.action.as_str(), last.unchecked, last.run), ("done", true, None));
    let text = std::fs::read_to_string(folder(&root).join(EVENTS)).unwrap();
    assert_eq!(text.matches("\"unchecked\":true").count(), 1, "only the unchecked done line says so: {text}");

    // A run that checked nothing lets a ticket finish too, recorded as unchecked.
    let id = handed(&root, bracket());
    let nothing = RunResult { counts: BTreeMap::from([("unsupported".into(), 5), ("not_applicable".into(), 2)]), ..ran("run-2") };
    checked(&root, &id, nothing);
    done(&root, &id, seen(&root, &id)).unwrap();
    let last = events(&root, Some(&id)).unwrap().pop().unwrap();
    assert_eq!((last.unchecked, last.run.as_deref()), (true, Some("run-2")));
    // A ticket with no parts has no check to run: it finishes unchecked, whatever its switch says.
    let loose = handed(&root, titled("Choose the hinge supplier"));
    assert!(refusal(ask_check(&root, &loose, seen(&root, &loose))).contains("no check applies"));
    finished(&root, &loose, Some("Changed the part as asked."), &[], false).unwrap();
    done(&root, &loose, seen(&root, &loose)).unwrap();
    assert!(events(&root, Some(&loose)).unwrap().pop().unwrap().unchecked);
    // A ticket with parts needs its check until a person switches it off on the ticket, in one line.
    let parted = handed(&root, bracket());
    assert!(refusal(set_state(&root, &parted, seen(&root, &parted), State::Done, None)).contains("check has not run"));
    let off = edit(&root, &parted, seen(&root, &parted), Edit { checks: Some(false), ..Edit::default() }).unwrap();
    assert!(!applies(&off) && !off.checks);
    assert_eq!(actions(&root, &parted).pop(), Some(("check_off".to_string(), Actor::Person)));
    let on = edit(&root, &parted, seen(&root, &parted), Edit { checks: Some(true), ..Edit::default() }).unwrap();
    assert!(applies(&on));
    assert_eq!(actions(&root, &parted).pop().map(|(a, _)| a).as_deref(), Some("check_on"));
    edit(&root, &parted, seen(&root, &parted), Edit { checks: Some(false), ..Edit::default() }).unwrap();
    assert_eq!(set_state(&root, &parted, seen(&root, &parted), State::Done, None).unwrap().state, State::Done);
    assert!(events(&root, Some(&parted)).unwrap().pop().unwrap().unchecked);

    // One failure is something checked.
    let failing = Ticket {
        check: Some(Check { counts: BTreeMap::from([("failed".into(), 1)]), ..load(&root, &id).unwrap().check.unwrap() }),
        ..load(&root, &id).unwrap()
    };
    assert!(!unchecked(&failing));
}

#[test]
fn a_person_sets_any_state_and_done_waits_for_the_sub_tickets_and_the_check() {
    let scratch = Scratch::new("set-state");
    let (_, root) = project(&scratch);
    let parent = create(&root, Draft { checks: Some(false), ..titled("Enclosure fits the board") }).unwrap().id;
    let child = create(&root, Draft { parent: Some(parent.clone()), checks: Some(false), ..titled("Standoffs 6 mm") }).unwrap().id;
    let other = create(&root, Draft { parent: Some(parent.clone()), parts: vec![part("component", "Vents")], ..titled("Vent slots") }).unwrap().id;

    // Back and forth, by a person, as they like; the same state again changes nothing.
    let moved = set_state(&root, &child, 1, State::InProgress, None).unwrap();
    assert_eq!((moved.state, moved.worker), (State::InProgress, Some(Worker::Person)), "a person who moves it to in progress works it");
    assert_eq!(set_state(&root, &child, 2, State::InProgress, None).unwrap().revision, 2);
    assert_eq!(set_state(&root, &child, 2, State::Blocked, None).unwrap().state, State::Blocked);
    let back = set_state(&root, &child, 3, State::Confirmed, None).unwrap();
    assert_eq!((back.state, back.worker), (State::Confirmed, None), "nobody works a confirmed ticket");
    assert_eq!(set_state(&root, &child, 4, State::Done, None).unwrap().state, State::Done, "no check applies: done straight from confirmed");
    assert_eq!(
        events(&root, Some(&child)).unwrap().into_iter().map(|e| e.action).collect::<Vec<_>>(),
        ["created", "moved", "moved", "moved", "done"]
    );
    assert_eq!(set_state(&root, &child, 5, State::InProgress, None).unwrap().state, State::InProgress, "and back from done");
    set_state(&root, &child, 6, State::Done, None).unwrap();

    // The parent waits for its sub-tickets that are not deleted; it is not finished by itself.
    let why = refusal(done(&root, &parent, 1));
    assert!(why.contains("sub-tickets not done yet (T-3)"), "{why}");
    assert!(refusal(set_state(&root, &other, 1, State::Done, None)).contains("check has not run"), "a check applies to the vent slots");
    delete(&root, &other, 1).unwrap();
    assert_eq!(load(&root, &parent).unwrap().state, State::Confirmed, "nothing finished the parent");
    assert_eq!(done(&root, &parent, 1).unwrap().state, State::Done);
    // A stale press is refused.
    assert!(refusal(set_state(&root, &child, 1, State::Blocked, None)).contains("changed since you saw it"));
}

#[test]
fn a_refused_check_keeps_the_ticket_in_progress_and_the_agent_acts_only_on_the_ticket_it_works() {
    let scratch = Scratch::new("refused");
    let (_, root) = project(&scratch);
    let id = handed(&root, bracket());
    ask_check(&root, &id, seen(&root, &id)).unwrap();
    let refused = check_refused(&root, &id, "AUTH_UNAVAILABLE: no host billing authority").unwrap();
    assert_eq!((refused.state, refused.check.as_ref().map(|c| c.state)), (State::InProgress, Some(CheckState::Refused)));
    assert!(done(&root, &id, seen(&root, &id)).is_err(), "no run, no done");
    assert!(block(&root, &id, "  ").is_err(), "a block needs its reason");
    let blocked = block(&root, &id, "Which flange height? requirements.md has none").unwrap();
    assert_eq!((blocked.state, blocked.blocked.as_deref()), (State::Blocked, Some("Which flange height? requirements.md has none")));
    let resumed = unblock(&root, &id, Some("Flange 8 mm from the person".into())).unwrap();
    assert_eq!((resumed.state, resumed.blocked), (State::InProgress, None));
    block(&root, &id, "Again").unwrap();
    assert_eq!(start(&root, &id, seen(&root, &id)).unwrap().state, State::InProgress, "a person hands a blocked ticket over again");

    // A ticket a person works is not the agent's: it can neither finish nor block it.
    let mine = create(&root, titled("Countersink the M3 holes")).unwrap().id;
    work_myself(&root, &mine, 1).unwrap();
    assert_eq!(load(&root, &mine).unwrap().worker, Some(Worker::Person));
    for refused in [finished(&root, &mine, Some("Changed the part as asked."), &[], false).err(), block(&root, &mine, "x").err()] {
        assert!(matches!(&refused, Some(TicketError::Refused(why)) if why.contains("not handed to the Assistant")), "{refused:?}");
    }
    assert_eq!(seen(&root, &mine), 2);
}

#[test]
fn who_may_move_a_ticket_is_one_rule() {
    use State::{Blocked, Confirmed, Done, InProgress, Waiting};
    let states = [State::Draft, Confirmed, InProgress, Waiting, Done, Blocked];
    let allowed = |actor: Actor| -> Vec<(State, State)> {
        states
            .iter()
            .flat_map(|from| states.iter().map(move |to| (*from, *to)))
            .filter(|(from, to)| from != to && may_move(actor, *from, *to))
            .collect()
    };
    assert_eq!(allowed(Actor::Person).len(), 30, "a person: every move");
    assert_eq!(allowed(Actor::Agent).len(), 30, "the agent: every move, a draft's acceptance and finishing included");
    assert_eq!(allowed(Actor::App), [(InProgress, Waiting), (InProgress, Blocked)], "binding a run, and ending the Assistant's rounds");
    assert!(may_move(Actor::Agent, Done, InProgress) && may_move(Actor::Agent, State::Draft, Confirmed));
}

#[test]
fn waits_are_checked_cycles_refused_and_only_the_assistant_is_held_by_them() {
    let scratch = Scratch::new("waits");
    let (_, root) = project(&scratch);
    let base = create(&root, Draft { checks: Some(false), ..titled("Base plate") }).unwrap().id;
    let lid = create(&root, Draft { parts: vec![part("component", "Lid")], ..titled("Lid") }).unwrap().id;
    let joint = create(&root, Draft { waits: vec![base.clone(), lid.clone(), base.clone()], ..titled("Join lid and base") }).unwrap();
    assert_eq!(joint.waits, [base.clone(), lid.clone()], "each once");

    for (wanted, why) in [(vec!["T-9"], "no ticket T-9"), (vec!["../T-1"], "not a ticket id"), (vec!["T-3"], "cannot wait for itself")] {
        let error = edit(&root, &joint.id, 1, Edit { waits: Some(wanted.iter().map(|w| w.to_string()).collect()), ..Edit::default() })
            .unwrap_err()
            .to_string();
        assert!(error.contains(why), "{wanted:?}: {error}");
    }
    // A cycle, directly or through another ticket, is refused.
    let error = refusal(edit(&root, &base, 1, Edit { waits: Some(vec![joint.id.clone()]), ..Edit::default() }));
    assert!(error.contains("T-3 already waits for T-1"), "{error}");
    edit(&root, &lid, 1, Edit { waits: Some(vec![base.clone()]), ..Edit::default() }).unwrap();
    assert!(refusal(edit(&root, &base, 1, Edit { waits: Some(vec![lid.clone()]), ..Edit::default() })).contains("already waits for"));
    // A sub-ticket cannot wait for its parent: the parent is done only after it.
    let hinge = create(&root, Draft { parent: Some(joint.id.clone()), ..titled("Hinge pin") }).unwrap().id;
    assert!(refusal(edit(&root, &hinge, 1, Edit { waits: Some(vec![joint.id.clone()]), ..Edit::default() })).contains("or is its parent"));
    assert!(refusal(create(&root, Draft { parent: Some(base.clone()), waits: vec![base.clone()], ..titled("x") })).contains("or is its parent"));

    // Waits hold the Assistant, not a person.
    let error = refusal(start(&root, &joint.id, 1));
    assert!(error.contains("T-3 waits for T-1, T-2"), "{error}");
    set_state(&root, &base, 1, State::Done, None).unwrap();
    set_state(&root, &lid, 2, State::Done, None).unwrap_err();
    assert!(refusal(start(&root, &joint.id, 1)).contains("waits for T-2"));
    assert_eq!(work_myself(&root, &joint.id, 1).unwrap().worker, Some(Worker::Person), "a person is only reminded");
    // A deleted ticket cannot be waited for.
    delete(&root, &hinge, 1).unwrap();
    assert!(refusal(edit(&root, &lid, 2, Edit { waits: Some(vec![hinge]), ..Edit::default() })).contains("is deleted"));
}

#[test]
fn a_person_edits_a_ticket_in_any_state_in_one_line_and_a_done_tickets_change_asks_for_a_recheck() {
    let scratch = Scratch::new("edit");
    let (_, root) = project(&scratch);
    let id = handed(&root, bracket());
    let revision = seen(&root, &id);
    // In progress with the Assistant: title, text, lines, parts in one save, one record line.
    let lines = vec!["Thickness 2 mm".to_string(), String::new(), "Bend radius ≥ 2 mm".to_string()];
    let change = Edit {
        title: Some("Bracket, 2 mm".into()),
        text: Some("  ".into()),
        requirements: Some(lines),
        parts: Some(vec![part("component", "Bracket"), part("body", "Body2")]),
        ..Edit::default()
    };
    let edited = edit(&root, &id, revision, change).unwrap();
    assert_eq!((edited.title.as_str(), edited.text.as_str(), edited.state, edited.revision), ("Bracket, 2 mm", "", State::InProgress, revision + 1));
    assert_eq!(edited.requirements, ["Thickness 2 mm", "Bend radius ≥ 2 mm"], "empty lines are dropped");
    assert_eq!(edited.recheck, None, "only a done ticket asks for a re-check");
    // A save that changes nothing writes nothing; a stale one is refused; a title is needed.
    assert_eq!(edit(&root, &id, revision + 1, Edit { title: Some("Bracket, 2 mm".into()), ..Edit::default() }).unwrap().revision, revision + 1);
    assert!(refusal(edit(&root, &id, revision, Edit { title: Some("Other".into()), ..Edit::default() })).contains("changed since you saw it"));
    assert!(matches!(edit(&root, &id, revision + 1, Edit { title: Some(" ".into()), ..Edit::default() }), Err(TicketError::Invalid(_))));
    assert_eq!(actions(&root, &id).iter().filter(|(a, _)| a == "edited").count(), 1);

    // Done, then its requirement lines change: its state stays, it is marked 需重查 in the same line.
    finished(&root, &id, Some("Changed the part as asked."), &[], false).unwrap();
    checked(&root, &id, ran("run-1"));
    done(&root, &id, seen(&root, &id)).unwrap();
    let revision = seen(&root, &id);
    edit(&root, &id, revision, Edit { title: Some("Bracket, 2 mm, 5052".into()), ..Edit::default() }).unwrap();
    assert_eq!(load(&root, &id).unwrap().recheck, None, "a title is no reason to check again");
    let lines = Some(vec!["Thickness 2.5 mm".to_string()]);
    let marked = edit(&root, &id, revision + 1, Edit { requirements: lines, ..Edit::default() }).unwrap();
    assert_eq!((marked.state, marked.recheck.as_deref()), (State::Done, Some("requirement lines changed")));
    let line = events(&root, Some(&id)).unwrap().pop().unwrap();
    assert_eq!((line.action.as_str(), line.note.as_deref()), ("edited", Some("needs a re-check: requirement lines changed")));
    // Moved back, it cannot be done on the old run; a new run clears the mark.
    set_state(&root, &id, marked.revision, State::InProgress, None).unwrap();
    assert!(refusal(done(&root, &id, seen(&root, &id))).contains("changed after its check ran"));
    checked(&root, &id, ran("run-2"));
    assert_eq!(load(&root, &id).unwrap().recheck, None);
    done(&root, &id, seen(&root, &id)).unwrap();

    // The app marks a done ticket whose part changed in the model; once.
    let marked = mark_recheck(&root, &id, "Bracket changed in the model").unwrap();
    assert_eq!((marked.state, marked.recheck.as_deref()), (State::Done, Some("Bracket changed in the model")));
    assert_eq!(mark_recheck(&root, &id, "again").unwrap().revision, marked.revision);
    assert_eq!(events(&root, Some(&id)).unwrap().pop().map(|e| (e.action, e.actor)), Some(("recheck".to_string(), Actor::App)));
    let open = create(&root, titled("Lid")).unwrap().id;
    assert!(refusal(mark_recheck(&root, &open, "Lid changed")).contains("only a done ticket"));
}

#[test]
fn deleting_and_declining_keep_the_ticket_and_its_record_and_restoring_brings_it_back() {
    let scratch = Scratch::new("delete");
    let (_, root) = project(&scratch);
    let id = handed(&root, bracket());
    // Any state: a ticket in progress with the Assistant too.
    let gone = delete(&root, &id, seen(&root, &id)).unwrap();
    assert_eq!((gone.deleted, gone.state), (true, State::InProgress), "the state is kept");
    assert!(load(&root, &id).unwrap().deleted && list(&root).unwrap().len() == 1, "its file stays");
    // A deleted ticket takes no step but its restoring.
    for refused in [
        finished(&root, &id, Some("Changed the part as asked."), &[], false).err(),
        edit(&root, &id, seen(&root, &id), Edit { title: Some("x".into()), ..Edit::default() }).err(),
        delete(&root, &id, seen(&root, &id)).err(),
        set_state(&root, &id, seen(&root, &id), State::Done, None).err(),
    ] {
        assert!(matches!(&refused, Some(TicketError::Refused(why)) if why.contains("deleted")), "{refused:?}");
    }
    let back = restore(&root, &id).unwrap();
    assert_eq!((back.deleted, back.state, back.worker), (false, State::InProgress, Some(Worker::Assistant)));
    assert!(refusal(restore(&root, &id)).contains("is not deleted"));
    assert_eq!(actions(&root, &id).into_iter().rev().take(2).map(|(a, _)| a).collect::<Vec<_>>(), ["restored", "deleted"]);

    // Declining a draft deletes it; only a draft is declined.
    let other = draft(&root, titled("Lid"), Actor::Agent).unwrap().id;
    let declined = decline(&root, &other, 1).unwrap();
    assert_eq!((declined.deleted, declined.state), (true, State::Draft));
    assert!(refusal(decline(&root, &id, seen(&root, &id))).contains("only a draft is declined"));
    assert_eq!(restore(&root, &other).unwrap().state, State::Draft, "back among the drafts");

    // A parent goes to 已删除 with its live sub-tickets and comes back with them; a sub-ticket
    // deleted on its own before stays deleted.
    let subs: Vec<String> = ["Lid hinge", "Lid seal", "Lid label"]
        .iter()
        .map(|t| draft(&root, Draft { parent: Some(other.clone()), ..titled(t) }, Actor::Agent).unwrap().id)
        .collect();
    decline(&root, &subs[2], 1).unwrap();
    decline(&root, &other, seen(&root, &other)).unwrap();
    let gone: Vec<(bool, Option<String>)> = subs.iter().map(|id| load(&root, id).map(|t| (t.deleted, t.deleted_with)).unwrap()).collect();
    assert_eq!(gone, [(true, Some(other.clone())), (true, Some(other.clone())), (true, None)]);
    let line = events(&root, Some(&subs[0])).unwrap().pop().unwrap();
    assert_eq!((line.action.as_str(), line.note.as_deref()), ("deleted", Some("with T-2")));
    restore(&root, &other).unwrap();
    let back: Vec<bool> = subs.iter().map(|id| load(&root, id).unwrap().deleted).collect();
    assert_eq!(back, [false, false, true], "only those that went with it come back");
    assert_eq!(load(&root, &subs[0]).unwrap().deleted_with, None);
    let parent = create(&root, titled("Base")).unwrap().id;
    let under = create(&root, Draft { parent: Some(parent.clone()), ..titled("Base feet") }).unwrap().id;
    delete(&root, &parent, 1).unwrap();
    assert!(load(&root, &under).unwrap().deleted);
    // Ids are never given again.
    assert_eq!(draft(&root, titled("Base"), Actor::Agent).unwrap().id, "T-8");
}

#[test]
fn accepting_a_draft_takes_its_sub_tickets_with_it_or_leaves_them_to_be_picked() {
    let scratch = Scratch::new("accept");
    let (_, root) = project(&scratch);
    let parent = draft(&root, titled("Enclosure for the sensor board"), Actor::Agent).unwrap().id;
    let subs: Vec<String> = ["Base", "Lid", "Vents"]
        .iter()
        .map(|t| draft(&root, Draft { parent: Some(parent.clone()), ..titled(t) }, Actor::Agent).unwrap().id)
        .collect();
    decline(&root, &subs[2], 1).unwrap();
    confirm(&root, &parent, 1, true).unwrap();
    let states: Vec<(State, bool)> = subs.iter().map(|id| load(&root, id).map(|t| (t.state, t.deleted)).unwrap()).collect();
    assert_eq!(states, [(State::Confirmed, false), (State::Confirmed, false), (State::Draft, true)], "the declined one stays declined");
    let line = events(&root, Some(&subs[0])).unwrap().pop().unwrap();
    assert_eq!((line.action.as_str(), line.actor, line.note.as_deref()), ("confirmed", Actor::Person, Some("with T-1")));

    // Without them: the parent alone; each sub-ticket is picked on its own.
    let parent = draft(&root, titled("Bracket set"), Actor::Agent).unwrap().id;
    let sub = draft(&root, Draft { parent: Some(parent.clone()), ..titled("Left bracket") }, Actor::Agent).unwrap().id;
    confirm(&root, &parent, 1, false).unwrap();
    assert_eq!(load(&root, &sub).unwrap().state, State::Draft);
    assert_eq!(confirm(&root, &sub, 1, false).unwrap().state, State::Confirmed);
}

#[test]
fn attachments_are_files_in_the_project_and_one_from_outside_is_copied_under_attachments() {
    let scratch = Scratch::new("attach");
    let (_, root) = project(&scratch);
    std::fs::create_dir_all(root.join("specs")).unwrap();
    std::fs::write(root.join("specs/flange.pdf"), b"%PDF-1.7").unwrap();
    let ticket = create(&root, Draft { attachments: vec!["specs/flange.pdf".into(), "specs\\flange.pdf".into()], ..titled("Flange") }).unwrap();
    assert_eq!(ticket.attachments, ["specs/flange.pdf"], "a path in the project, `/` between names, once");

    // Paths are hostile: outside the project, `..`, absolute, missing, or inside .intcad are refused.
    let outside = scratch.0.join("outside.png");
    std::fs::write(&outside, b"\x89PNG outside").unwrap();
    std::fs::write(root.join(".intcad").join("note.txt"), b"app's own").unwrap();
    for (path, why) in [
        ("../outside.png", "without `..`"),
        (outside.to_str().unwrap(), "relative to the project folder"),
        ("C:\\x.png", "relative to the project folder"),
        ("specs/none.pdf", "no such file"),
        (".intcad/note.txt", ".intcad holds only"),
        (".INTCAD/note.txt", ".intcad holds only"),
        ("specs", "no such file"),
        ("", "give the path"),
    ] {
        let error = refusal(create(&root, Draft { attachments: vec![path.into()], ..titled("x") }));
        assert!(error.contains(why), "{path}: {error}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("specs/link.png")).unwrap();
        let error = refusal(create(&root, Draft { attachments: vec!["specs/link.png".into()], ..titled("x") }));
        assert!(error.contains("outside the project folder"), "{error}");
    }
    let too_many: Vec<String> = (0..=MAX_ATTACHMENTS).map(|_| "specs/flange.pdf".to_string()).collect();
    assert!(refusal(create(&root, Draft { attachments: too_many, ..titled("x") })).contains("at most"));

    // Taking a file in: one already in the project stays where it is; one from outside is copied
    // under attachments/, never over a file there, never into .intcad.
    assert_eq!(take_in(&root, &root.join("specs/flange.pdf")).unwrap(), "specs/flange.pdf");
    let first = take_in(&root, &outside).unwrap();
    assert_eq!(first, "attachments/outside.png");
    std::fs::write(&outside, b"\x89PNG second").unwrap();
    let second = take_in(&root, &outside).unwrap();
    assert_eq!(second, "attachments/outside-2.png");
    assert_eq!(std::fs::read(root.join(&first)).unwrap(), b"\x89PNG outside", "the first copy is untouched");
    assert_eq!(std::fs::read(root.join(&second)).unwrap(), b"\x89PNG second");
    assert_eq!(take_in(&root, &root.join(".intcad/note.txt")).unwrap(), "attachments/note.txt", "the app's folder is no place to point into");
    assert!(take_in(&root, &root.join("specs")).is_err(), "a folder is no attachment");
    let hidden = scratch.0.join(".env");
    std::fs::write(&hidden, b"x").unwrap();
    assert_eq!(take_in(&root, &hidden).unwrap(), "attachments/env");
    let added = edit(&root, &ticket.id, 1, Edit { attachments: Some(vec!["specs/flange.pdf".into(), first.clone()]), ..Edit::default() }).unwrap();
    assert_eq!(added.attachments, ["specs/flange.pdf", "attachments/outside.png"]);
    assert!(!folder(&root).join("outside.png").exists() && !root.join(".intcad").join("attachments").exists());

    // The seam for the host: the agent asks for a picture of the 3D view, the app attaches it.
    let asked = draft(&root, Draft { view: true, ..titled("Hinge clashes with the lid") }, Actor::Agent).unwrap();
    assert!(asked.wants_view);
    let attached = attach_view(&root, &asked.id, b"\x89PNG view").unwrap();
    assert_eq!((attached.attachments.clone(), attached.wants_view), (vec![format!("attachments/{}-view.png", asked.id)], false));
    assert_eq!(actions(&root, &asked.id).pop(), Some(("view_attached".to_string(), Actor::App)));
}

/// A project folder as layout 1 wrote it, by hand: one `part` per ticket, a top ticket the
/// others are under, and a check the agent asked for.
#[test]
fn comments_by_a_person_and_the_agent_are_recorded_in_order_and_leave_the_revision_alone() {
    let scratch = Scratch::new("comments");
    let (_, root) = project(&scratch);
    let id = handed(&root, bracket());
    let revision = seen(&root, &id);
    let mine = comment(&root, &id, Actor::Person, "  Keep the side plates as they are.  ", &[], false).unwrap();
    assert_eq!((mine.action.as_str(), mine.actor, mine.note.as_deref()), ("comment", Actor::Person, Some("Keep the side plates as they are.")));
    comment(&root, &id, Actor::Agent, "孔位按 5 mm 改。", &[], false).unwrap();
    let said: Vec<(Actor, Option<String>)> =
        events(&root, Some(&id)).unwrap().into_iter().filter(|e| e.action == "comment").map(|e| (e.actor, e.note)).collect();
    assert_eq!(
        said,
        [(Actor::Person, Some("Keep the side plates as they are.".into())), (Actor::Agent, Some("孔位按 5 mm 改。".into()))],
        "in the order written"
    );
    assert_eq!(seen(&root, &id), revision, "a comment writes no ticket and moves no revision");

    // A save made on the revision a person saw is still taken after a comment, and so is the next.
    let titled_now = edit(&root, &id, revision, Edit { title: Some("Bracket, holes moved".into()), ..Edit::default() }).unwrap();
    comment(&root, &id, Actor::Person, "Between two saves.", &[], false).unwrap();
    edit(&root, &id, titled_now.revision, Edit { text: Some("Moved by 0.8 mm.".into()), ..Edit::default() }).unwrap();

    // Refused: the app as author, nothing to say, too long, a deleted ticket, no such ticket.
    assert!(refusal(comment(&root, &id, Actor::App, "x", &[], false)).contains("a person or the Assistant"));
    assert!(refusal(comment(&root, &id, Actor::Person, "   ", &[], false)).contains("needs text"));
    let long = "字".repeat(MAX_COMMENT + 1);
    assert!(refusal(comment(&root, &id, Actor::Person, &long, &[], false)).contains("longer than 4000"));
    assert!(comment(&root, &id, Actor::Person, &"字".repeat(MAX_COMMENT), &[], false).is_ok(), "4000 characters, not bytes");
    let gone = create(&root, titled("Label")).unwrap().id;
    delete(&root, &gone, seen(&root, &gone)).unwrap();
    assert!(refusal(comment(&root, &gone, Actor::Person, "x", &[], false)).contains("deleted"));
    assert!(matches!(comment(&root, "T-99", Actor::Person, "x", &[], false), Err(TicketError::NoSuchTicket(_))));
}

#[test]
fn finished_and_blocked_need_a_text_that_says_what_was_done_or_why() {
    let scratch = Scratch::new("said");
    let (_, root) = project(&scratch);
    let id = handed(&root, bracket());
    for text in [None, Some(""), Some("   ")] {
        let why = refusal(finished(&root, &id, text, &[], false));
        assert!(why.contains("what you changed, on which parts, and what you did not do"), "{why}");
    }
    let why = refusal(block(&root, &id, " "));
    assert!(why.contains("what stops you"), "{why}");
    assert_eq!(load(&root, &id).unwrap().state, State::InProgress, "nothing moved");
    let text = "Changed: holes moved 0.8 mm inwards.\nParts: Bracket.\nNot done: flange stays 7.4 mm.";
    finished(&root, &id, Some(text), &[], false).unwrap();
    let line = events(&root, Some(&id)).unwrap().pop().unwrap();
    assert_eq!((line.action.as_str(), line.note.as_deref()), ("finished", Some(text)));
}

#[test]
fn the_agents_own_move_to_waiting_blocked_or_done_takes_a_text_kept_as_its_comment() {
    // As finished and block do: no text, no move; the text is the move line's note. A person's
    // moves need none.
    let scratch = Scratch::new("move-text");
    let (_, root) = project(&scratch);
    let id = handed(&root, titled("Constraint, no parts"));
    for to in [State::Waiting, State::Blocked, State::Done] {
        let why = refusal(agent_set_state(&root, &id, to, None));
        assert!(why.contains("why you move the ticket"), "{to:?}: {why}");
        assert!(refusal(agent_set_state(&root, &id, to, Some("  "))).contains("why you move the ticket"));
    }
    assert_eq!(load(&root, &id).unwrap().state, State::InProgress, "nothing moved without a text");
    let moved = agent_set_state(&root, &id, State::Blocked, Some("The hinge needs a part we do not have.")).unwrap();
    assert_eq!((moved.state, moved.blocked.as_deref()), (State::Blocked, Some("The hinge needs a part we do not have.")));
    let line = events(&root, Some(&id)).unwrap().pop().unwrap();
    assert_eq!((line.action.as_str(), line.actor, line.note.as_deref()), ("moved", Actor::Agent, Some("The hinge needs a part we do not have.")));
    // Back in progress needs none; a person's moves need none either.
    agent_set_state(&root, &id, State::InProgress, None).unwrap();
    set_state(&root, &id, seen(&root, &id), State::Waiting, None).unwrap();
    assert_eq!(events(&root, Some(&id)).unwrap().pop().unwrap().note, None);
}

/// A run's result with these counts.
fn counted(run: &str, counts: &[(&str, u64)]) -> RunResult {
    RunResult {
        run: run.into(),
        outcome: "done".into(),
        counts: counts.iter().map(|(k, n)| (k.to_string(), *n)).collect(),
        freshness: "current".into(),
    }
}

/// The Assistant says it is finished, and the app asks for the check and runs it with `result`.
fn round(root: &Path, id: &str, result: RunResult) -> Ticket {
    finished(root, id, Some("Fixed what the check found"), &[], false).unwrap();
    let asked = auto_check(root, id).unwrap();
    assert_eq!(asked.check.as_ref().map(|c| (c.state, c.by, c.kind)), Some((CheckState::Asked, Actor::Agent, CheckKind::Dfm)));
    check_started(root, id, &result.run).unwrap();
    check_ran(root, id, result).unwrap()
}

#[test]
fn the_assistants_finished_asks_for_its_check_and_the_findings_keep_it_fixing_for_a_few_rounds() {
    let scratch = Scratch::new("rounds");
    let (_, root) = project(&scratch);
    // Clean on the second check: one round of fixing, then waiting for a person.
    let id = handed(&root, bracket());
    let first = round(&root, &id, counted("run-1", &[("advisory", 2), ("checked", 9)]));
    assert_eq!((first.state, first.worker, first.finished), (State::InProgress, Some(Worker::Assistant), false), "it stays the Assistant's to fix");
    assert_eq!(first.rounds, Some(Rounds { round: 1, of: DEFAULT_ROUNDS, stopped: None }));
    assert_eq!(first.check.as_ref().map(|c| c.state), Some(CheckState::Ran), "the run is bound");
    let second = round(&root, &id, counted("run-2", &[("checked", 11)]));
    assert_eq!(second.state, State::Waiting);
    assert_eq!(second.rounds, Some(Rounds { round: 1, of: DEFAULT_ROUNDS, stopped: Some(Stop::Clean) }));
    let record = events(&root, Some(&id)).unwrap();
    let steps: Vec<(&str, Actor, Option<(u32, Option<Stop>)>)> =
        record.iter().skip(3).map(|e| (e.action.as_str(), e.actor, e.rounds.as_ref().map(|r| (r.round, r.stopped)))).collect();
    assert_eq!(
        steps,
        [
            ("finished", Actor::Agent, None),
            ("check_asked", Actor::Agent, Some((0, None))),
            ("check_started", Actor::App, Some((0, None))),
            ("check_ran", Actor::App, Some((1, None))),
            ("finished", Actor::Agent, None),
            ("check_asked", Actor::Agent, Some((1, None))),
            ("check_started", Actor::App, Some((1, None))),
            ("check_ran", Actor::App, Some((1, Some(Stop::Clean)))),
        ]
    );
    // The Assistant may finish it itself: nothing failed.
    let done = agent_set_state(&root, &id, State::Done, Some("Checked clean in round 1")).unwrap();
    assert_eq!(done.state, State::Done);

    // 「建议」 left after the last round: four checks for three rounds, then waiting for a person.
    let id = handed(&root, bracket());
    for n in 0..DEFAULT_ROUNDS {
        let ticket = round(&root, &id, counted(&format!("run-a{n}"), &[("advisory", 1)]));
        assert_eq!((ticket.state, ticket.rounds.map(|r| r.round)), (State::InProgress, Some(n + 1)));
    }
    let last = round(&root, &id, counted("run-a-last", &[("advisory", 1)]));
    assert_eq!((last.state, last.rounds.as_ref().and_then(|r| r.stopped)), (State::Waiting, Some(Stop::Advisory)));
    assert_eq!(events(&root, Some(&id)).unwrap().iter().filter(|e| e.action == "check_ran").count() as u32, DEFAULT_ROUNDS + 1);
    assert!(refusal(auto_check(&root, &id)).contains("T-2 is waiting"), "no check after the rounds stopped");

    // Something failed after the last round: blocked, with what is left and what was tried.
    let id = handed(&root, bracket());
    for n in 0..DEFAULT_ROUNDS {
        round(&root, &id, counted(&format!("run-f{n}"), &[("failed", 1), ("advisory", 2)]));
    }
    let blocked = round(&root, &id, counted("run-f-last", &[("failed", 1), ("advisory", 2)]));
    assert_eq!((blocked.state, blocked.rounds.as_ref().and_then(|r| r.stopped)), (State::Blocked, Some(Stop::Failed)));
    let why = blocked.blocked.clone().unwrap();
    assert!(why.contains("after 3 of 3 rounds") && why.contains("1 failed · 2 advisory"), "{why}");
    // The Assistant does not finish a ticket whose last check left something failed; a person may.
    assert!(refusal(agent_set_state(&root, &id, State::Done, Some("done"))).contains("left 1 failed"));
    set_state(&root, &id, seen(&root, &id), State::Done, None).unwrap();
}

#[test]
fn the_rounds_stop_when_the_check_cannot_run_a_person_steps_in_or_automatic_checks_are_off() {
    let scratch = Scratch::new("rounds-stop");
    let (_, root) = project(&scratch);
    // A refused check stops the rounds: blocked, with the reason.
    let id = handed(&root, bracket());
    finished(&root, &id, Some("Holes moved"), &[], false).unwrap();
    auto_check(&root, &id).unwrap();
    assert!(refusal(finished(&root, &id, Some("again"), &[], false)).contains("check is running"), "no finished while a check is asked for");
    let refused = check_refused(&root, &id, "AUTH_REQUIRED: sign in").unwrap();
    assert_eq!((refused.state, refused.rounds.as_ref().and_then(|r| r.stopped)), (State::Blocked, Some(Stop::Refused)));
    assert!(refused.blocked.as_deref().is_some_and(|why| why.contains("AUTH_REQUIRED")), "{:?}", refused.blocked);
    // The Assistant cannot buy itself more rounds: taken up again by itself, its finished asks for none.
    unblock(&root, &id, None).unwrap();
    finished(&root, &id, Some("Holes moved again"), &[], false).unwrap();
    assert!(refusal(auto_check(&root, &id)).contains("rounds on T-1 stopped (refused)"));
    set_state(&root, &id, seen(&root, &id), State::Blocked, None).unwrap();
    // Handed again by a person, it starts over.
    start(&root, &id, seen(&root, &id)).unwrap();
    assert_eq!(load(&root, &id).unwrap().rounds, None);

    // A person's 跑检查 while the Assistant's check runs: the run's result is the person's, the ticket
    // waits for them, and the rounds say who stopped them.
    finished(&root, &id, Some("Holes moved"), &[], false).unwrap();
    auto_check(&root, &id).unwrap();
    check_started(&root, &id, "run-p").unwrap();
    ask_check(&root, &id, seen(&root, &id)).unwrap();
    let ticket = check_ran(&root, &id, counted("run-p", &[("advisory", 2)])).unwrap();
    assert_eq!((ticket.state, ticket.rounds.and_then(|r| r.stopped)), (State::Waiting, Some(Stop::Person)));
    assert_eq!(ticket.check.map(|c| c.by), Some(Actor::Person));

    // A person's move while the Assistant fixes stops the rounds; no further check is asked for.
    let id = handed(&root, bracket());
    round(&root, &id, counted("run-m", &[("advisory", 1)]));
    set_state(&root, &id, seen(&root, &id), State::Confirmed, Some("Wrong part".into())).unwrap();
    assert_eq!(load(&root, &id).unwrap().rounds.and_then(|r| r.stopped), Some(Stop::Person));
    // A move out of progress ends a running check; a person's move back in progress gives the
    // rounds back.
    start(&root, &id, seen(&root, &id)).unwrap();
    finished(&root, &id, Some("Again"), &[], false).unwrap();
    auto_check(&root, &id).unwrap();
    check_started(&root, &id, "run-gone").unwrap();
    let moved = set_state(&root, &id, seen(&root, &id), State::Waiting, None).unwrap();
    assert_eq!(moved.check.as_ref().map(|c| c.state), Some(CheckState::Refused));
    assert!(!checking(&moved) && moved.rounds.as_ref().and_then(|r| r.stopped) == Some(Stop::Person), "no check is left going on");
    let back = set_state(&root, &id, seen(&root, &id), State::InProgress, Some("One more look".into())).unwrap();
    assert_eq!((back.worker, back.rounds), (Some(Worker::Assistant), None));

    // With the project's automatic checks off, nothing is asked for: a person starts the check.
    let id = handed(&root, bracket());
    crate::project::Project::open(&root).unwrap().update_ci(Some(false), None).unwrap();
    finished(&root, &id, Some("Holes moved"), &[], false).unwrap();
    assert!(refusal(auto_check(&root, &id)).contains("automatic checks are off"));
    assert_eq!(load(&root, &id).unwrap().check, None);
    // A ticket the agent does not work, or not finished, is refused too.
    crate::project::Project::open(&root).unwrap().update_ci(Some(true), None).unwrap();
    let other = draft(&root, bracket(), Actor::Agent).unwrap().id;
    assert!(refusal(auto_check(&root, &other)).contains("is draft"));
}

#[test]
fn the_projects_ticket_settings_set_a_new_tickets_check_and_the_rounds_and_old_files_read_the_defaults() {
    let scratch = Scratch::new("ticket-settings");
    let (mut s, root) = project(&scratch);
    // A ci.toml written before the two settings existed reads as checked by default, three rounds.
    let path = root.join(crate::project::FOLDER).join(crate::project::CI);
    let old: String = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|line| !["new_tickets", "rounds", "# Part tickets", "# the Assistant"].iter().any(|start| line.starts_with(start)))
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(!old.contains("rounds"), "{old}");
    std::fs::write(&path, &old).unwrap();
    let project = crate::project::Project::open(&root).unwrap();
    assert_eq!((project.ci().checks.new_tickets, project.ci().checks.rounds), (true, DEFAULT_ROUNDS));
    assert_eq!(automatic_rounds(&root), Some(DEFAULT_ROUNDS));
    assert!(draft(&root, bracket(), Actor::Agent).unwrap().checks, "a new ticket is checked by default");

    // Set and written: read back the same, and a new ticket follows the project.
    let mut project = crate::project::Project::open(&root).unwrap();
    project.update_checks(None, None, Some(false), Some(0)).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("new_tickets = false") && text.contains("rounds = 0"), "{text}");
    let reread = crate::project::Project::open(&root).unwrap();
    assert_eq!((reread.ci().checks.new_tickets, reread.ci().checks.rounds), (false, 0));
    let unchecked = draft(&root, bracket(), Actor::Agent).unwrap();
    assert!(!unchecked.checks && !applies(&unchecked), "the project's default: not checked");
    // A person's switch on the ticket overrides the project's default.
    let turned = edit(&root, &unchecked.id, unchecked.revision, Edit { checks: Some(true), ..Edit::default() }).unwrap();
    assert!(applies(&turned));
    // More rounds than allowed are refused; the file keeps what it had.
    let refused = crate::project::Project::open(&root).unwrap().update_checks(None, None, None, Some(crate::project::MAX_ROUNDS + 1));
    assert!(refused.unwrap_err().to_string().contains("at most"), "bounded");
    assert_eq!(crate::project::Project::open(&root).unwrap().ci().checks.rounds, 0);

    // Through the command (a person's, in the Project Settings window), and as project.inspect says.
    let reply = s.execute("project.ci_set", &json!({"newTickets": true, "rounds": 0})).unwrap();
    assert_eq!((reply["ci"]["newTickets"].as_bool(), reply["ci"]["rounds"].as_u64()), (Some(true), Some(0)));
    let inspected = s.execute("project.inspect", &json!({})).unwrap();
    assert_eq!((inspected["ci"]["newTickets"].as_bool(), inspected["ci"]["rounds"].as_u64()), (Some(true), Some(0)));
    assert!(s.execute("project.ci_set", &json!({"rounds": 11})).is_err(), "more than ten rounds are refused");
    // Zero rounds is zero: the first check's 「建议」 go to a person at once.
    let id = handed(&root, bracket());
    let ticket = round(&root, &id, counted("run-z", &[("advisory", 1)]));
    assert_eq!((ticket.state, ticket.rounds.map(|r| (r.round, r.of, r.stopped))), (State::Waiting, Some((0, 0, Some(Stop::Advisory)))));
}

#[test]
fn a_persons_move_back_keeps_its_reason_on_the_line() {
    let scratch = Scratch::new("reason");
    let (_, root) = project(&scratch);
    let id = create(&root, titled("Cover")).unwrap().id;
    set_state(&root, &id, seen(&root, &id), State::Done, None).unwrap();
    set_state(&root, &id, seen(&root, &id), State::InProgress, Some(" The housing grew 2 mm. ".into())).unwrap();
    let line = events(&root, Some(&id)).unwrap().pop().unwrap();
    assert_eq!(
        (line.action.as_str(), line.from, line.to, line.note.as_deref()),
        ("moved", Some(State::Done), Some(State::InProgress), Some("The housing grew 2 mm."))
    );
    let long = "x".repeat(MAX_REASON + 1);
    assert!(refusal(set_state(&root, &id, seen(&root, &id), State::Done, Some(long))).contains("longer than"));
}

#[test]
fn a_comment_carries_files_of_the_project_one_from_outside_copied_and_at_most_five() {
    let scratch = Scratch::new("comment-files");
    let (_, root) = project(&scratch);
    let id = create(&root, titled("Flange")).unwrap().id;
    let outside = scratch.0.join("supplier list.pdf");
    std::fs::write(&outside, b"%PDF-1.7 outside").unwrap();
    let taken = take_in(&root, &outside).unwrap();
    assert_eq!(taken, "attachments/supplier list.pdf", "copied under attachments/");
    assert!(root.join("attachments/supplier list.pdf").is_file());
    let line = comment(&root, &id, Actor::Person, "From the supplier.", std::slice::from_ref(&taken), false).unwrap();
    assert_eq!(line.attachments, std::slice::from_ref(&taken));
    assert_eq!(comment(&root, &id, Actor::Person, "", std::slice::from_ref(&taken), false).unwrap().note, None, "files alone are a comment");

    let six: Vec<String> = (1..=6)
        .map(|n| {
            std::fs::write(root.join(format!("view-{n}.png")), b"\x89PNG").unwrap();
            format!("view-{n}.png")
        })
        .collect();
    assert!(refusal(comment(&root, &id, Actor::Person, "six", &six, false)).contains("at most 5"));
    assert!(comment(&root, &id, Actor::Person, "five", &six[..5], false).is_ok());
    for path in ["../outside.png", ".intcad/tickets/events.jsonl", "none.pdf"] {
        assert!(comment(&root, &id, Actor::Person, "x", &[path.to_string()], false).is_err(), "{path} is refused");
    }
}

#[test]
fn the_view_the_agent_asks_for_is_joined_to_its_line() {
    let scratch = Scratch::new("comment-view");
    let (_, root) = project(&scratch);
    let id = handed(&root, bracket());
    finished(&root, &id, Some("Changed the holes on Bracket; nothing left undone."), &[], true).unwrap();
    let wanted = views_wanted(&root, None).unwrap();
    assert_eq!(wanted.len(), 1);
    let (ticket, at) = wanted[0].clone();
    assert_eq!(ticket, id);
    let revision = seen(&root, &id);
    let added = comment_view(&root, &id, &at, b"\x89PNG view").unwrap();
    assert_eq!(added.attachments, [format!("attachments/{id}-view.png")]);
    assert_eq!(seen(&root, &id), revision, "the picture moves no revision");
    assert!(views_wanted(&root, Some(&id)).unwrap().is_empty());
    let shown = record(&root, &id).unwrap();
    let finished_line = shown.iter().find(|e| e.action == "finished").unwrap();
    assert_eq!(finished_line.attachments, [format!("attachments/{id}-view.png")], "shown with the line that asked for it");
    assert!(!shown.iter().any(|e| e.action == "comment_view"));
    assert!(refusal(comment_view(&root, &id, &at, b"\x89PNG again")).contains("waits for a picture"), "once only");
}

fn write_version_1(root: &Path) {
    let dir = folder(root);
    std::fs::create_dir_all(&dir).unwrap();
    let at = "2026-11-26T10:00:00.000Z";
    let files = [
        json!({"schemaVersion": 1, "id": "T-1", "parent": null, "title": "temp", "text": "", "requirements": [], "part": null,
               "state": "draft", "check": null, "blocked": null, "revision": 1, "created": at, "updated": at}),
        json!({"schemaVersion": 1, "id": "T-2", "parent": "T-1", "title": "Bracket", "text": "Two M4 holes",
               "requirements": ["Hole centres 40 ± 0.1 mm"], "part": {"kind": "component", "id": "Bracket"},
               "state": "in_progress", "check": {"state": "asked", "at": at}, "blocked": null, "revision": 4, "created": at, "updated": at}),
        json!({"schemaVersion": 1, "id": "T-3", "parent": "T-1", "title": "Base plate", "part": {"kind": "body", "id": "Body1"},
               "state": "done", "check": {"state": "ran", "run": "run-7", "outcome": "done", "counts": {"unsupported": 5}, "freshness": "current", "at": at},
               "blocked": null, "revision": 7, "created": at, "updated": at}),
    ];
    for ticket in files {
        let id = ticket["id"].as_str().unwrap().to_string();
        std::fs::write(dir.join(format!("{id}.json")), serde_json::to_vec_pretty(&ticket).unwrap()).unwrap();
    }
    let lines = [
        json!({"at": at, "actor": "agent", "ticket": "T-1", "action": "drafted", "to": "draft", "revision": 1}),
        json!({"at": at, "actor": "agent", "ticket": "T-2", "action": "drafted", "to": "draft", "revision": 1}),
        json!({"at": at, "actor": "agent", "ticket": "T-2", "action": "check_asked", "note": "Holes moved", "revision": 4}),
        json!({"at": at, "actor": "person", "ticket": "T-3", "action": "done", "from": "waiting", "to": "done", "run": "run-7", "revision": 7}),
    ];
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(dir.join(EVENTS), text).unwrap();
}

#[test]
fn a_version_1_folder_is_read_and_a_later_step_writes_version_2() {
    let scratch = Scratch::new("v1");
    let (_, root) = project(&scratch);
    write_version_1(&root);
    let tickets = list(&root).unwrap();
    let shape: Vec<(&str, Option<&str>, usize, Option<Worker>, bool, u32)> =
        tickets.iter().map(|t| (t.id.as_str(), t.parent.as_deref(), t.parts.len(), t.worker, t.checks, t.schema_version)).collect();
    assert_eq!(
        shape,
        [("T-1", None, 0, None, true, 2), ("T-2", None, 1, Some(Worker::Assistant), true, 2), ("T-3", None, 1, Some(Worker::Assistant), true, 2),]
    );
    let bracket = &tickets[1];
    assert_eq!(bracket.parts, [part("component", "Bracket")]);
    assert_eq!((bracket.check.clone(), bracket.finished), (None, true), "the agent's ask became its 'finished': no run starts without a person");
    assert_eq!(tickets[2].check.as_ref().and_then(|c| c.run.as_deref()), Some("run-7"));
    assert_eq!(
        tickets[2].check.as_ref().map(|c| (c.kind, c.by)),
        Some((CheckKind::Dfm, Actor::Person)),
        "a check without a kind is a DFM check, a person's"
    );
    assert!(tickets.iter().all(|t| !t.deleted && t.waits.is_empty() && t.attachments.is_empty() && t.recheck.is_none()));
    assert_eq!(events(&root, None).unwrap().len(), 4, "the record of layout 1 reads as it is");
    // The file is left as it was until a step changes the ticket; then it is layout 2.
    let before = std::fs::read_to_string(folder(&root).join("T-2.json")).unwrap();
    assert!(before.contains("\"schemaVersion\": 1") && before.contains("\"part\""), "{before}");
    ask_check(&root, "T-2", 4).unwrap();
    let after = std::fs::read_to_string(folder(&root).join("T-2.json")).unwrap();
    assert!(after.contains("\"schemaVersion\": 2") && after.contains("\"parts\"") && !after.contains("\"part\":"), "{after}");
    assert_eq!(load(&root, "T-2").unwrap().worker, Some(Worker::Assistant));
    // Nobody chose layout 1's parents (its first ticket was the top one by rule): they are dropped,
    // so its first ticket is a draft of its own and is declined alone.
    let file = std::fs::read_to_string(folder(&root).join("T-3.json")).unwrap();
    assert!(file.contains("\"parent\": \"T-1\""), "the file is left as it was: {file}");
    decline(&root, "T-1", 1).unwrap();
    assert!(!load(&root, "T-2").unwrap().deleted && !load(&root, "T-3").unwrap().deleted);
    assert_eq!(draft(&root, titled("New"), Actor::Agent).unwrap().id, "T-4");
    // A newer layout than this build's is refused, not guessed at.
    std::fs::write(folder(&root).join("T-9.json"), br#"{"schemaVersion": 3, "id": "T-9"}"#).unwrap();
    assert!(matches!(load(&root, "T-9"), Err(TicketError::Damaged { .. })));
}

#[test]
fn ids_are_never_paths_and_hostile_text_is_capped() {
    let scratch = Scratch::new("hostile");
    let (_, root) = project(&scratch);
    for id in ["../T-1", "T-0", "T-01", "T-", "T-1/../x", "t-1", "T-1234567"] {
        assert!(matches!(load(&root, id), Err(TicketError::Invalid(_))), "{id}");
    }
    let long = "x".repeat(MAX_TITLE + 1);
    assert!(draft(&root, titled(&long), Actor::Agent).is_err());
    assert!(draft(&root, titled("  "), Actor::Agent).is_err());
    assert!(draft(&root, Draft { parts: vec![part("assembly", "x")], ..titled("A") }, Actor::Agent).is_err());
    assert!(draft(&root, Draft { parts: vec![part("body", " ")], ..titled("A") }, Actor::Agent).is_err());
    assert!(draft(&root, Draft { parts: (0..=MAX_PARTS).map(|n| part("body", &format!("B{n}"))).collect(), ..titled("A") }, Actor::Agent).is_err());
    assert!(draft(&root, Draft { waits: (0..=MAX_WAITS).map(|n| format!("T-{}", n + 1)).collect(), ..titled("A") }, Actor::Agent).is_err());
    assert!(list(&root).unwrap().is_empty(), "nothing refused was written");
    assert_eq!(file_name("../../a:b?.png"), "_.._a_b_.png");
    assert_eq!(file_name("...."), "attachment");
    assert_eq!(file_name(&format!("{}.pdf", "y".repeat(300))).chars().count(), 104);
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn the_agents_commands_draft_set_states_and_say_finished_but_start_no_check_and_delete_nothing() {
    let scratch = Scratch::new("commands");
    let (mut s, root) = project(&scratch);
    std::fs::write(root.join("sketch.png"), b"\x89PNG").unwrap();
    let first = run(&mut s, "ticket.draft", json!({"title": "Enclosure", "requirements": ["5052-H32, 1.5 mm"]}));
    assert_eq!((first["ticket"]["id"].as_str(), first["ticket"]["parent"].as_str()), (Some("T-1"), None), "no top ticket by itself");
    assert_eq!((first["ticket"]["state"].as_str(), first["events"][0]["actor"].as_str()), (Some("draft"), Some("agent")));
    let joint = run(
        &mut s,
        "ticket.draft",
        json!({"title": "Lid and base: hinge clearance 3 mm", "parent": "T-1", "waits": [], "view": true,
               "parts": [{"kind": "component", "id": "Lid"}, {"kind": "component", "id": "Base"}], "attachments": ["sketch.png"]}),
    );
    let ticket = &joint["ticket"];
    assert_eq!((ticket["parent"].as_str(), ticket["parts"].as_array().map(Vec::len), ticket["checks"].as_bool()), (Some("T-1"), Some(2), Some(true)));
    // Only a person switches a ticket's check off: the agent's draft does not carry the switch.
    for checks in [json!(false), json!(true)] {
        let error = s.execute("ticket.draft", &json!({"title": "Unchecked", "checks": checks})).unwrap_err().to_string();
        assert!(error.contains("a person switches a ticket's check off"), "{error}");
    }
    assert_eq!((ticket["attachments"][0].as_str(), ticket["wantsView"].as_bool()), (Some("sketch.png"), Some(true)));
    let waiting = run(&mut s, "ticket.draft", json!({"title": "Paint", "waits": ["T-2"]}));
    assert_eq!(waiting["ticket"]["waits"][0], "T-2");
    assert_eq!(run(&mut s, "ticket.list", json!({}))["tickets"].as_array().map(Vec::len), Some(3));
    // Refused before anything is written: a path out of the project, a cycle, the old one-part shape.
    for bad in [
        json!({"title": "x", "attachments": ["../escape.png"]}),
        json!({"title": "x", "attachments": [".intcad/tickets/T-1.json"]}),
        json!({"title": "x", "parent": "T-2"}),
        json!({"title": "x", "part": {"kind": "body", "id": "Body1"}}),
    ] {
        assert!(s.execute("ticket.draft", &bad).is_err(), "{bad}");
    }
    assert_eq!(list(&root).unwrap().len(), 3);

    // No command starts a check, edits, deletes or restores: the agent's commands are these four,
    // and update refuses those words.
    let mut commands: Vec<&str> = crate::cmd::command_specs().iter().map(|spec| spec.id).filter(|id| id.starts_with("ticket.")).collect();
    commands.sort_unstable();
    assert_eq!(commands, ["ticket.draft", "ticket.inspect", "ticket.list", "ticket.update"]);
    for missing in [
        "ticket.confirm",
        "ticket.done",
        "ticket.start",
        "ticket.complete",
        "ticket.create",
        "ticket.edit",
        "ticket.delete",
        "ticket.restore",
        "ticket.state",
    ] {
        assert!(crate::cmd::find_command(missing).is_none(), "{missing} must not exist");
    }
    for action in ["decline", "delete", "restore", "edit"] {
        let error = s.execute("ticket.update", &json!({"id": "T-2", "action": action})).unwrap_err().to_string();
        assert!(error.contains("only a person"), "{action}: {error}");
    }
    for to in [json!({}), json!({"to": "finished"}), json!({"to": "../done"})] {
        let mut params = json!({"id": "T-2", "action": "state"});
        params.as_object_mut().unwrap().extend(to.as_object().unwrap().clone());
        let error = s.execute("ticket.update", &params).unwrap_err().to_string();
        assert!(error.contains("is not a state"), "{params}: {error}");
    }
    let error = s.execute("ticket.update", &json!({"id": "T-2", "action": "check"})).unwrap_err().to_string();
    assert!(error.contains("no command asks for a check") && error.contains("finished"), "{error}");
    for action in ["create", "move", "confirm"] {
        let error = s.execute("ticket.update", &json!({"id": "T-2", "action": action, "reason": "x"})).unwrap_err().to_string();
        assert!(error.contains("is not an action"), "{action}: {error}");
    }
    // A draft is not worked on, nor a ticket a person works.
    let error = s.execute("ticket.update", &json!({"id": "T-2", "action": "finished", "text": "x"})).unwrap_err().to_string();
    assert!(error.contains("T-2 is draft"), "{error}");
    assert_eq!(seen(&root, "T-2"), 1, "nothing the agent asked for changed the draft");
    confirm(&root, "T-1", 1, false).unwrap();
    work_myself(&root, "T-1", 2).unwrap();
    let error = s.execute("ticket.update", &json!({"id": "T-1", "action": "finished", "text": "x"})).unwrap_err().to_string();
    assert!(error.contains("not handed to the Assistant"), "{error}");
    // A comment from the agent, on any ticket, with files of the project.
    let commented = run(&mut s, "ticket.update", json!({"id": "T-1", "action": "comment", "text": "Seen.", "attachments": ["sketch.png"]}));
    let line = commented["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((line["action"].as_str(), line["actor"].as_str(), line["note"].as_str()), (Some("comment"), Some("agent"), Some("Seen.")));
    assert_eq!(line["attachments"], json!(["sketch.png"]));

    // On the ticket handed to the Assistant: finished, with no check applying (a person switched it
    // off; the agent cannot), waits for a person.
    edit(&root, "T-2", 1, Edit { checks: Some(false), ..Edit::default() }).unwrap();
    confirm(&root, "T-2", 2, false).unwrap();
    start(&root, "T-2", 3).unwrap();
    let error = s.execute("ticket.update", &json!({"id": "T-2", "action": "finished"})).unwrap_err().to_string();
    assert!(error.contains("what you changed, on which parts, and what you did not do"), "the refusal says what to write: {error}");
    let said = run(&mut s, "ticket.update", json!({"id": "T-2", "action": "finished", "reason": "Hinge moved 1 mm"}));
    assert_eq!((said["ticket"]["state"].as_str(), said["ticket"]["check"].is_null()), (Some("waiting"), true));
    let error = s.execute("ticket.update", &json!({"id": "T-2", "action": "block", "reason": "x"})).unwrap_err().to_string();
    assert!(error.contains("T-2 is waiting"), "{error}");
    let inspected = run(&mut s, "ticket.inspect", json!({"id": "T-2"}));
    let steps: Vec<&str> = inspected["events"].as_array().unwrap().iter().filter_map(|e| e["action"].as_str()).collect();
    assert_eq!(steps, ["drafted", "check_off", "confirmed", "started", "finished"]);
    assert_eq!(inspected["events"][4]["actor"], "agent");
    // The agent sets states as a person does, under the same conditions, each step its own line.
    let accepted = run(&mut s, "ticket.update", json!({"id": "T-3", "action": "state", "to": "confirmed"}));
    assert_eq!(accepted["ticket"]["state"], "confirmed", "the agent accepts its own draft");
    let line = accepted["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((line["action"].as_str(), line["actor"].as_str()), (Some("confirmed"), Some("agent")));
    let error = s.execute("ticket.update", &json!({"id": "T-3", "action": "state", "to": "in_progress"})).unwrap_err().to_string();
    assert!(error.contains("T-3 waits for T-2"), "open waits keep it from the Assistant: {error}");
    let error = s.execute("ticket.update", &json!({"id": "T-2", "action": "state", "to": "done"})).unwrap_err().to_string();
    assert!(error.contains("why you move the ticket"), "a move to done needs a text: {error}");
    let done_line = run(&mut s, "ticket.update", json!({"id": "T-2", "action": "state", "to": "done", "text": "Hinge moved; nothing to check"}));
    let line = done_line["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((line["action"].as_str(), line["actor"].as_str(), line["unchecked"].as_bool()), (Some("done"), Some("agent"), Some(true)));
    let taken = run(&mut s, "ticket.update", json!({"id": "T-3", "action": "state", "to": "in_progress"}));
    assert_eq!((taken["ticket"]["state"].as_str(), taken["ticket"]["worker"].as_str()), (Some("in_progress"), Some("assistant")));
    run(&mut s, "ticket.draft", json!({"title": "Flange", "parts": [{"kind": "component", "id": "Base"}]}));
    run(&mut s, "ticket.update", json!({"id": "T-4", "action": "state", "to": "confirmed"}));
    let error = s.execute("ticket.update", &json!({"id": "T-4", "action": "state", "to": "in_progress"})).unwrap_err().to_string();
    assert!(error.contains("one ticket at a time"), "{error}");
    let error = s.execute("ticket.update", &json!({"id": "T-4", "action": "state", "to": "done", "text": "x"})).unwrap_err().to_string();
    assert!(error.contains("check has not run"), "the done gate holds for the agent: {error}");
    let reopened = run(&mut s, "ticket.update", json!({"id": "T-2", "action": "state", "to": "confirmed"}));
    assert_eq!(reopened["ticket"]["state"], "confirmed", "and back from done");
    // The commands are no undo steps: the design's history is untouched.
    assert!(s.undo.is_empty());
}

#[test]
fn a_design_outside_a_project_has_no_tickets() {
    let mut s = Session::default();
    let error = s.execute("ticket.list", &json!({})).unwrap_err().to_string();
    assert!(error.contains("project"), "{error}");
}
