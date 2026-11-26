//! The ghost of a next-step suggestion (INT-1458): the two questions, the app's checks, the ghost
//! over the design and Tab. Case numbers are the approved list's (docs/design/autocomplete-tests.md).
//! The engine and its copy are real; the agent is the recording backend and the clock is scripted.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use solidcraft_engine::Session;

use super::tests::*;
use super::*;

/// An app on a 100 × 60 × 6 plate, with the setting on.
pub(super) fn plate() -> (SolveApp, Fake, Fake, egui::Context) {
    let (mut app, chat, guesser, ctx) = setup(true);
    app.session
        .execute("solid.box", &json!({"length": 100, "width": 60, "height": 6, "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"}))
        .unwrap();
    (app, chat, guesser, ctx)
}

pub(super) fn holes() -> Value {
    let hole =
        |x: f64, y: f64, n: u32| json!({"command": "solid.hole", "params": {"position": [x, y, 6], "diameter": 5, "name": format!("Hole{n}")}});
    json!([hole(10.0, 10.0, 1), hole(90.0, 10.0, 2), hole(90.0, 50.0, 3), hole(10.0, 50.0, 4)])
}

pub(super) const HOLES: &str = r#"{"step": "Drill four 5 mm through holes near the corners of Plate", "reason": "A plate is usually screwed down."}"#;

/// From input time `t`: the pause, the first question and its answer `step`. The line is shown and
/// the second question is out, 0.2 s after the first was asked.
pub(super) fn line(app: &mut SolveApp, ctx: &egui::Context, guesser: &Fake, t: f64, step: &str) {
    frame(app, ctx, t, vec![]);
    frame(app, ctx, t + 1.5, vec![]);
    guesser.ready();
    frame(app, ctx, t + 1.5, vec![]);
    guesser.answer(step);
    frame(app, ctx, t + 1.6, vec![]);
    assert!(app.agent_chat.predict.shown().is_some(), "{}", status(app));
    frame(app, ctx, t + 1.7, vec![]);
}

pub(super) fn sessions_opened(guesser: &Fake) -> usize {
    guesser.calls().iter().filter(|c| *c == "new_session").count()
}

/// The answer to the second question, at `t`, and one frame more so that its ghost is on screen.
pub(super) fn answer_commands(app: &mut SolveApp, ctx: &egui::Context, guesser: &Fake, t: f64, commands: &Value) {
    guesser.answer(&json!({"commands": commands}).to_string());
    frame(app, ctx, t, vec![]);
    frame(app, ctx, t + 0.01, vec![]);
}

pub(super) fn ghost_drawn(app: &mut SolveApp, ctx: &egui::Context, guesser: &Fake) {
    line(app, ctx, guesser, 0.0, HOLES);
    answer_commands(app, ctx, guesser, 2.0, &holes());
    assert_eq!(status(app)["state"], "ghost", "{}", status(app));
}

/// What a withdrawn suggestion leaves alone.
pub(super) struct Before {
    stamp: solidcraft_engine::DocumentStamp,
    doc: Arc<solidcraft_engine::doc::Document>,
    undo: usize,
}

pub(super) fn before(app: &mut SolveApp) -> Before {
    Before { stamp: app.session.document_stamp(), doc: app.session.doc.clone(), undo: app.session.undo.len() }
}

/// The suggestion went with this outcome and the design is as it was.
pub(super) fn withdrawn(app: &mut SolveApp, chat: &Fake, was: &Before, what: &str) {
    let p = &app.agent_chat.predict;
    assert!(p.shown().is_none() && p.ghost.is_none() && p.work.is_none(), "{}", status(app));
    assert_eq!(p.log().last().unwrap().outcome, what, "{}", status(app));
    assert!(chat.prompts().is_empty(), "{:?}", chat.calls());
    assert_eq!(app.session.document_stamp(), was.stamp);
    assert!(Arc::ptr_eq(&app.session.doc, &was.doc));
    assert_eq!(app.session.undo.len(), was.undo);
}

fn volumes(session: &Session) -> Vec<(String, f64, usize)> {
    let state = session.world_state().unwrap();
    let mut out: Vec<_> = state.bodies.iter().map(|b| (b.name.clone(), b.body.volume_at(0.01).unwrap(), b.body.face_count())).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ---- A. Asking -------------------------------------------------------------------------------------

/// G2: the pause is 1.5 s from the last edit; the setting keeps what it accepts.
#[test]
fn g2_the_pause_is_one_and_a_half_seconds_after_the_last_edit() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    edit(&mut app, 0.0);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 1.4, vec![]);
    assert!(guesser.prompts().is_empty(), "{:?}", guesser.calls());
    frame(&mut app, &ctx, 1.5, vec![]);
    assert_eq!(guesser.prompts().len(), 1, "{:?}", guesser.calls());
    // A second edit counts again.
    let (mut app, _chat, guesser, ctx) = setup(true);
    edit(&mut app, 0.0);
    frame(&mut app, &ctx, 0.0, vec![]);
    edit(&mut app, 20.0);
    frame(&mut app, &ctx, 1.0, vec![]);
    frame(&mut app, &ctx, 2.4, vec![]);
    assert!(guesser.prompts().is_empty(), "{:?}", guesser.calls());
    frame(&mut app, &ctx, 2.5, vec![]);
    assert_eq!(guesser.prompts().len(), 1);
    for (set, is) in [(4.0, 4.0), (0.2, 1.5), (500.0, 1.5)] {
        app.preferences.agent_next_step_pause_s = set;
        assert_eq!(pause(&app), is, "{set}");
    }
}

/// G3: while the chat's preview runs or waits, nothing is asked; once it is gone, one question.
/// (The agent busy, a dialog, a sketch, a tool, a skill, Ask Here: the cases in `predict_tests`.)
#[test]
fn g3_no_question_while_a_chat_preview_is_there() {
    let (mut app, _chat, guesser, ctx) = plate();
    app.preferences.agent_chat_preview = true;
    crate::agent_chat::review::begin(&mut app);
    for t in [0.0, 2.0, 60.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert!(guesser.prompts().is_empty(), "{:?}", guesser.calls());
    crate::agent_chat::review::forget(&mut app.agent_chat);
    frame(&mut app, &ctx, 61.0, vec![]);
    frame(&mut app, &ctx, 63.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1);
}

/// G4: History's lines say what a step does to the bodies, and where.
#[test]
fn g4_history_names_the_operation_and_the_place() {
    let (mut app, _chat, _guesser, _ctx) = plate();
    let cylinder = |op: &str, name: &str| {
        let mut p = json!({"radius": 2.5, "height": 8, "base": [10, 10, -1], "operation": op, "name": name});
        if op == "cut" {
            p["targets"] = json!(["Plate"]);
        }
        p
    };
    app.session.execute("solid.cylinder", &cylinder("cut", "Hole")).unwrap();
    app.session.execute("solid.cylinder", &cylinder("new", "Pin")).unwrap();
    app.session
        .execute(
            "solid.fillet",
            &json!({"edges": [[0, 0, 3], [100, 0, 3], [100, 60, 3], [0, 60, 3]], "radius": 5, "body": "Plate", "name": "Corners"}),
        )
        .unwrap();
    let q = prompt(&mut app);
    let of = |name: &str| q.lines().find(|l| l.contains(&format!(". {name} ("))).unwrap_or_else(|| panic!("{name}: {q}")).to_string();
    let hole = of("Hole");
    assert!(hole.contains("cut from Plate") && hole.contains("base (10, 10, -1)") && hole.contains("diameter 5,"), "{hole}");
    let pin = of("Pin");
    assert!(pin.contains("new body") && !pin.contains("cut"), "{pin}");
    let corners = of("Corners");
    assert!(corners.contains("radius 5") && corners.contains("4 edges"), "{corners}");
    assert!(of("Plate").contains("corner (0, 0, 0)"), "{q}");
}

/// G5: an empty or unreadable answer shows nothing and asks nothing more; of two steps the first.
#[test]
fn g5_an_empty_answer_shows_nothing_and_of_two_steps_the_first_is_taken() {
    for reply in [r#"{"step": "", "reason": "nothing"}"#, ""] {
        let (mut app, _chat, guesser, ctx) = plate();
        asked(&mut app, &ctx, &guesser);
        guesser.answer(reply);
        for t in [6.0, 60.0] {
            frame(&mut app, &ctx, t, vec![]);
        }
        assert!(app.agent_chat.predict.shown().is_none(), "{reply}");
        assert_eq!(guesser.prompts().len(), 1);
        assert_eq!(outcome(&app, 0), "empty", "{reply}");
        assert_eq!(sessions_opened(&guesser), 1, "{reply}: the next one's, {:?}", guesser.calls());
    }
    let (mut app, _chat, guesser, ctx) = plate();
    asked(&mut app, &ctx, &guesser);
    guesser.answer(&format!("{HOLES}\n{GUESS}"));
    frame(&mut app, &ctx, 6.0, vec![]);
    let g = app.agent_chat.predict.shown().cloned().unwrap();
    assert_eq!((g.step.as_str(), g.reason.as_str()), ("Drill four 5 mm through holes near the corners of Plate", "A plate is usually screwed down."));
}

/// G6: the second question goes after the first answer, in the same session, with the step as it
/// was given, the commands that may run and the body's faces and edges. The next suggestion's
/// session is opened once both are answered, and its question does not wait for one.
#[test]
fn g6_the_second_question_asks_for_the_commands_of_the_step_shown() {
    let (mut app, _chat, guesser, ctx) = plate();
    app.session.execute("solid.box", &json!({"length": 5, "width": 5, "height": 5, "corner": [200, 0, 0], "body_name": "Other"})).unwrap();
    asked(&mut app, &ctx, &guesser);
    guesser.answer(HOLES);
    frame(&mut app, &ctx, 6.0, vec![]);
    assert!(chip(&app).unwrap().0.contains("Drill four 5 mm"), "the line shows before the commands");
    assert_eq!(sessions_opened(&guesser), 0, "the second goes in the first's session: {:?}", guesser.calls());
    let prompts = guesser.prompts();
    assert_eq!(prompts.len(), 2);
    let q = &prompts[1];
    assert!(q.contains("Step: Drill four 5 mm through holes near the corners of Plate\n"), "{q}");
    for command in ghost::solid_commands() {
        assert!(q.contains(&format!("- {command} {{")), "{command}: {q}");
    }
    assert!(!q.contains("sheet."), "sheet metal is off: {q}");
    assert!(q.contains("Body Plate:") && !q.contains("Body Other:"), "only the body the step names: {q}");
    let top = q.lines().find(|l| l.contains("plane normal [0, 0, 1] at [")).unwrap_or_else(|| panic!("{q}"));
    assert!(top.ends_with(", 6]"), "a point on the top face: {top}");
    assert_eq!(q.matches("- edge mid").count(), 12, "{q}");
    assert!(q.contains("Do not call any tools"), "{q}");
    for t in [7.0, 60.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!((guesser.prompts().len(), sessions_opened(&guesser)), (2, 0), "two questions in all, in one session");
    // A step that names no body: the last step's.
    let q = ghost::question(&mut app.session, "Round the corners", "", false);
    assert!(q.contains("Body Other:") && !q.contains("Body Plate:"), "{q}");
    // Answered: the next one's session is opened, once, and waits for the next edit.
    answer_commands(&mut app, &ctx, &guesser, 61.0, &holes());
    assert_eq!(sessions_opened(&guesser), 1);
    guesser.ready();
    edit(&mut app, 300.0);
    frame(&mut app, &ctx, 70.0, vec![]);
    frame(&mut app, &ctx, 71.5, vec![]);
    assert_eq!((guesser.prompts().len(), sessions_opened(&guesser)), (3, 1), "the third question waits for no session");
}

/// G7: a body with many edges stays small in the question, which says what it leaves out.
#[test]
fn g7_a_large_body_is_cut_short_in_the_question() {
    let (mut app, _chat, _guesser, _ctx) = plate();
    app.session.execute("solid.hole", &json!({"position": [5, 5, 6], "diameter": 1, "name": "Hole"})).unwrap();
    let grid = json!({"features": ["Hole"], "dir1": [1, 0, 0], "count1": 30, "spacing1": 3, "dir2": [0, 1, 0], "count2": 9, "spacing2": 6});
    app.session.execute("solid.pattern.rectangular", &grid).unwrap();
    let edges = app.session.execute("model.edges", &json!({"body": "Plate"})).unwrap()["edges"].as_array().unwrap().len();
    assert!(edges > 500, "{edges}");
    let history = history_block(&app.session.doc);
    let q = ghost::question(&mut app.session, "Chamfer Plate", &history, false);
    assert!(q.chars().count() <= ghost::MAX_QUESTION, "{}", q.chars().count());
    let left: usize = q.lines().last().unwrap().trim_start_matches('(').split(' ').next().unwrap().parse().unwrap();
    assert!(left > 400, "{}", q.lines().last().unwrap());
}

/// Where a suggestion can be when something happens to it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Asking,
    Line,
    Computing,
    Drawn,
}

const STAGES: [Stage; 4] = [Stage::Asking, Stage::Line, Stage::Computing, Stage::Drawn];

/// A suggestion on the plate at `stage`, by input time 3.
fn at_stage(stage: Stage) -> (SolveApp, Fake, Fake, egui::Context) {
    let (mut app, chat, guesser, ctx) = plate();
    if stage == Stage::Asking {
        frame(&mut app, &ctx, 0.0, vec![]);
        frame(&mut app, &ctx, 1.5, vec![]);
        assert_eq!(guesser.prompts().len(), 1);
        return (app, chat, guesser, ctx);
    }
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    match stage {
        Stage::Computing => {
            let (work, tx) = Work::held(2.0);
            std::mem::forget(tx);
            app.agent_chat.predict.ask = None;
            app.agent_chat.predict.work = Some(work);
        }
        Stage::Drawn => answer_commands(&mut app, &ctx, &guesser, 2.0, &holes()),
        _ => {}
    }
    (app, chat, guesser, ctx)
}

/// G8: an edit takes the suggestion away at every stage; a late answer shows nothing and asks
/// nothing; one new question comes 1.5 s after the edit.
#[test]
fn g8_an_edit_takes_the_suggestion_away_at_every_stage() {
    for stage in STAGES {
        let (mut app, chat, guesser, ctx) = at_stage(stage);
        let asked_before = guesser.prompts().len();
        edit(&mut app, 300.0);
        let was = before(&mut app);
        frame(&mut app, &ctx, 3.0, vec![]);
        withdrawn(&mut app, &chat, &was, "stale");
        // What was out answers late.
        guesser.answer(if stage == Stage::Asking {
            HOLES
        } else {
            r#"{"commands": [{"command": "solid.box", "params": {"length": 1, "width": 1, "height": 1}}]}"#
        });
        frame(&mut app, &ctx, 3.2, vec![]);
        withdrawn(&mut app, &chat, &was, "stale");
        frame(&mut app, &ctx, 4.4, vec![]);
        assert_eq!(guesser.prompts().len(), asked_before, "{stage:?}: not before the pause");
        frame(&mut app, &ctx, 4.5, vec![]);
        guesser.ready();
        frame(&mut app, &ctx, 4.6, vec![]);
        assert_eq!(guesser.prompts().len(), asked_before + 1, "{stage:?}: {:?}", guesser.calls());
        assert!(guesser.prompts().last().unwrap().contains("Guess the one modelling step"), "{stage:?}: a first question");
    }
}

/// G9: Esc takes it away at every stage and is used up, and the step is not offered again; another
/// key takes away a line or a ghost and is not used up. (A key while the first question is out
/// keeps it: `a_late_answer_is_shown_if_the_design_and_selection_held`.)
#[test]
fn g9_esc_turns_it_down_at_every_stage_and_another_key_dismisses_it() {
    for stage in STAGES {
        let (mut app, chat, guesser, ctx) = at_stage(stage);
        let was = before(&mut app);
        app.esc_handled = false;
        frame(&mut app, &ctx, 2.5, vec![key(egui::Key::Escape)]);
        assert!(app.esc_handled, "{stage:?}: used up");
        withdrawn(&mut app, &chat, &was, "dismissed");
        assert_eq!(app.agent_chat.predict.dismissed.len(), usize::from(stage != Stage::Asking), "{stage:?}");
        let questions = guesser.prompts().len();
        for t in [5.0, 60.0] {
            frame(&mut app, &ctx, t, vec![]);
        }
        assert_eq!(guesser.prompts().len(), questions, "{stage:?}: none until an edit");
        if stage != Stage::Asking {
            let (mut app, chat, _guesser, ctx) = at_stage(stage);
            let was = before(&mut app);
            frame(&mut app, &ctx, 2.5, vec![key(egui::Key::A)]);
            withdrawn(&mut app, &chat, &was, "dismissed");
            assert!(app.agent_chat.predict.dismissed.is_empty(), "{stage:?}: not turned down");
        }
    }
}

/// G10: looking around and clicking in the view keep the line and the ghost.
#[test]
fn g10_looking_around_keeps_the_line_and_the_ghost() {
    for stage in [Stage::Line, Stage::Drawn] {
        let (mut app, _chat, guesser, ctx) = at_stage(stage);
        frame(&mut app, &ctx, 2.5, look_around());
        frame(&mut app, &ctx, 2.6, click_in_view());
        assert!(app.agent_chat.predict.shown().is_some(), "{stage:?}");
        assert_eq!(app.agent_chat.predict.ghost.is_some(), stage == Stage::Drawn, "{stage:?}");
        // The click picked something.
        app.session.selection = vec![solidcraft_engine::Sel::Body { name: "Plate".into() }];
        frame(&mut app, &ctx, 2.7, vec![]);
        if stage == Stage::Line {
            answer_commands(&mut app, &ctx, &guesser, 2.8, &holes());
        }
        assert!(app.agent_chat.predict.ghost.is_some(), "{stage:?}: {}", status(&app));
    }
}

// ---- B. The app's checks ---------------------------------------------------------------------------

/// The plate's suggestion answered with these commands.
fn answered(commands: &Value) -> (SolveApp, Fake, Fake, egui::Context, Before) {
    let (mut app, chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let was = before(&mut app);
    answer_commands(&mut app, &ctx, &guesser, 2.0, commands);
    (app, chat, guesser, ctx, was)
}

fn one(command: &str, params: Value) -> Value {
    json!([{"command": command, "params": params}])
}

/// G11 and G12: a command that is not on the list, however it is written, and beside good ones:
/// nothing runs, not even on the copy, and no file appears.
#[test]
fn g11_g12_only_listed_commands_run_and_one_other_drops_them_all() {
    let dir = std::env::temp_dir().join(format!("intcad-ghost-list-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.step").to_string_lossy().into_owned();
    let to = json!({"path": path});
    let cube = json!({"length": 1, "width": 1, "height": 1});
    let mut cases: Vec<Value> = [
        "file.export",
        "file.save",
        "file.new",
        "doc.open",
        "file.send_to_slicer",
        "view.pick",
        "parameters.change",
        "edit.undo",
        "sheet.export_dxf",
        "sheet.export_bend_table",
        "sheet.manage_rules",
        "sheet.flange",
        "no.such_command",
        "engine.batch",
        "batch",
        " FILE.Export ",
        "",
    ]
    .iter()
    .map(|c| one(c, to.clone()))
    .collect();
    // Both keys: the engine reads `command`.
    cases.push(json!([{"id": "solid.box", "command": "file.export", "params": to}]));
    // G12: a good one first.
    cases.push(json!([{"command": "solid.box", "params": cube}, {"command": "file.export", "params": to}]));
    for commands in &cases {
        let (mut app, chat, _guesser, _ctx, was) = answered(commands);
        withdrawn(&mut app, &chat, &was, "not allowed");
        assert_eq!(app.agent_chat.predict.log().last().unwrap().ran, 0, "{commands}");
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no file");
    let _ = std::fs::remove_dir_all(&dir);
    // Read by the same rule, `id` alone names a command too, in any case and spacing.
    for id in ["solid.box", " Solid.BOX "] {
        let (app, ..) = answered(&json!([{"id": id, "params": cube}]));
        assert!(app.agent_chat.predict.ghost.is_some(), "{id}: {}", status(&app));
    }
}

/// G13: every command on the list is one the engine takes, and draws a ghost.
#[test]
fn g13_every_listed_command_draws_a_ghost() {
    let mut base = Session::default();
    let run = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"));
    run(&mut base, "solid.box", json!({"length": 100, "width": 60, "height": 20, "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"}));
    run(&mut base, "solid.hole", json!({"position": [20, 20, 20], "diameter": 5, "name": "Hole"}));
    run(
        &mut base,
        "solid.box",
        json!({"length": 10, "width": 10, "height": 10, "corner": [45, 25, 15], "operation": "new", "body_name": "Boss", "name": "Boss"}),
    );
    run(&mut base, "sketch.create", json!({"plane": "XY", "offset": "40", "name": "Outline"}));
    run(&mut base, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [10, 10]}));
    run(&mut base, "sketch.finish", json!({}));
    let sketch = base.doc.features.iter().find(|f| f.name == "Outline").expect("the sketch").id;
    let calls = [
        ("solid.box", json!({"length": 5, "width": 5, "height": 5, "corner": [200, 0, 0], "operation": "new"})),
        ("solid.cylinder", json!({"radius": 3, "height": 40, "base": [70, 30, -1], "operation": "cut", "targets": ["Plate"]})),
        ("solid.extrude", json!({"sketch": sketch, "distance": 5, "operation": "new"})),
        ("solid.fillet", json!({"edges": [[0, 0, 10]], "radius": 3, "body": "Plate"})),
        ("solid.chamfer", json!({"edges": [[100, 60, 10]], "distance": 2, "body": "Plate"})),
        ("solid.shell", json!({"faces": [[50, 30, 0]], "thickness": 2, "body": "Plate"})),
        ("solid.hole", json!({"position": [80, 40, 20], "diameter": 6})),
        ("solid.pattern.rectangular", json!({"features": ["Hole"], "dir1": [1, 0, 0], "count1": 2, "spacing1": 60})),
        ("solid.pattern.circular", json!({"features": ["Boss"], "axis": "Z", "count": 3})),
        ("solid.mirror", json!({"bodies": ["Boss"], "plane": "YZ"})),
        ("solid.combine", json!({"target": "Plate", "tools": ["Boss"], "operation": "join"})),
    ];
    assert_eq!(calls.iter().map(|(c, _)| *c).collect::<Vec<_>>(), ghost::solid_commands(), "one call for each");
    for (command, params) in calls {
        let (mut app, _chat, _guesser, _ctx) = setup(true);
        app.session = base.scratch();
        let steps = ghost::allowed(vec![(command.to_string(), params)], false).unwrap();
        let work = ghost::start(&mut app, steps, || {});
        match work.poll(0.0) {
            Some(Ok(drawn)) => assert_eq!(drawn.steps.len(), 1),
            Some(Err(why)) => panic!("{command}: {why:?}"),
            None => panic!("{command}: not done"),
        }
        assert_eq!(work.ran.load(Ordering::Relaxed), 1, "{command}");
    }
}

/// G14: a command that fails, alone or after a good one.
#[test]
fn g14_a_failing_command_withdraws_the_suggestion() {
    let fillet = json!({"command": "solid.fillet", "params": {"edges": [[0, 0, 3]], "radius": 500, "body": "Plate"}});
    let good = json!({"command": "solid.box", "params": {"length": 1, "width": 1, "height": 1, "corner": [200, 0, 0]}});
    for commands in [json!([fillet]), json!([good, fillet])] {
        let (mut app, chat, _guesser, _ctx, was) = answered(&commands);
        withdrawn(&mut app, &chat, &was, "failed");
        assert!(crate::agent_chat::review::overlay(&mut app).is_none(), "nothing of it is drawn");
    }
}

/// G15: a cut that misses: the engine reports success, and nothing changed.
#[test]
fn g15_a_step_that_changes_nothing_is_withdrawn() {
    let miss = one("solid.cylinder", json!({"radius": 2.5, "height": 8, "base": [500, 500, -1], "operation": "cut", "targets": ["Plate"]}));
    let (mut app, chat, _guesser, _ctx, was) = answered(&miss);
    assert_eq!(app.agent_chat.predict.log().last().unwrap().ran, 1);
    withdrawn(&mut app, &chat, &was, "no change");
}

/// G16: a body that is open afterwards, and was not before, is refused (the rule itself: no listed
/// command is known to open a body).
#[test]
fn g16_a_body_left_open_is_refused() {
    assert_eq!(ghost::verdict_of(true, Some(true), Some(false)), Err(Refused::NotClosed));
    assert_eq!(ghost::verdict_of(true, None, Some(false)), Err(Refused::NotClosed));
    assert_eq!(ghost::verdict_of(true, Some(false), Some(false)), Ok(()), "it was open before");
    assert_eq!(ghost::verdict_of(true, Some(true), Some(true)), Ok(()));
    assert_eq!(ghost::verdict_of(true, Some(true), None), Ok(()), "removed");
    assert_eq!(ghost::verdict_of(false, Some(true), Some(true)), Err(Refused::NoChange));
}

/// G17: the last History step again, and a step turned down before however it is written, are not
/// offered; the same step with another size is.
#[test]
fn g17_repeats_are_not_offered() {
    let again = |h: f64| one("solid.box", json!({"length": 100, "width": 60, "height": h, "corner": [0, 0, 0], "operation": "new", "name": "Again"}));
    let (mut app, chat, _guesser, _ctx, was) = answered(&again(6.0));
    withdrawn(&mut app, &chat, &was, "repeat");
    let (app, ..) = answered(&again(7.0));
    assert!(app.agent_chat.predict.ghost.is_some(), "{}", status(&app));
    // Turned down before, in other case and spacing: no line, and no second question.
    let (mut app, _chat, guesser, ctx) = plate();
    app.agent_chat.predict.turned_down("drill FOUR 5 mm through-holes near  the corners of plate.".into());
    asked(&mut app, &ctx, &guesser);
    guesser.answer(HOLES);
    frame(&mut app, &ctx, 6.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_none());
    assert_eq!(outcome(&app, 0), "repeat");
    assert_eq!(guesser.prompts().len(), 1);
    guesser.answer("");
    let (mut app, _chat, guesser, ctx) = plate();
    app.agent_chat.predict.turned_down("Drill four 5 mm through holes near the corners of Plate".into());
    asked(&mut app, &ctx, &guesser);
    guesser.answer(&HOLES.replace("5 mm", "4 mm"));
    frame(&mut app, &ctx, 6.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_some(), "another size is another step");
}

/// G18: the design moves between question and answer, answer and ghost, ghost and Tab (here by an
/// undo, as the control channel would): nothing lands.
#[test]
fn g18_nothing_lands_on_a_design_that_moved() {
    for stage in [Stage::Line, Stage::Computing, Stage::Drawn] {
        let (mut app, chat, guesser, ctx) = at_stage(stage);
        app.session.execute("edit.undo", &json!({})).unwrap();
        let was = before(&mut app);
        if stage == Stage::Line {
            guesser.answer(&json!({"commands": holes()}).to_string());
        }
        frame(&mut app, &ctx, 2.5, vec![key(egui::Key::Tab)]);
        withdrawn(&mut app, &chat, &was, "stale");
    }
}

/// G19: a copy that takes 3 s is given up: its own work is stopped and the design's is not, so a
/// command the user runs then completes. Closing the window's chat stops it too.
#[test]
fn g19_a_slow_ghost_is_given_up_without_stopping_the_users_own_work() {
    let (mut app, chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let was = before(&mut app);
    let (work, _tx) = Work::held(2.0);
    let token = work.stop_token();
    app.agent_chat.predict.ask = None;
    app.agent_chat.predict.work = Some(work);
    frame(&mut app, &ctx, 4.9, vec![]);
    assert!(app.agent_chat.predict.work.is_some() && !token.is_stopped(), "still computing at 2.9 s");
    frame(&mut app, &ctx, 5.0, vec![]);
    withdrawn(&mut app, &chat, &was, "too slow");
    assert!(token.is_stopped());
    assert!(!app.session.stop_token().is_stopped(), "the design's own work goes on");
    edit(&mut app, 300.0);
    assert_eq!(app.session.world_state().unwrap().bodies.len(), 2);
    // A real copy, stopped while it computes: the design then does the same step with no trace of it.
    let (mut app, chat, guesser, ctx) = plate();
    app.session
        .execute("solid.cylinder", &json!({"radius": 1, "height": 8, "base": [3, 3, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"}))
        .unwrap();
    let _ = app.session.world_state();
    app.apply_off_thread = Some(std::time::Duration::ZERO);
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let was = before(&mut app);
    let pattern = json!({"features": ["Hole"], "dir1": [1, 0, 0], "count1": 20, "spacing1": 4.5, "dir2": [0, 1, 0], "count2": 12, "spacing2": 4.5});
    guesser.answer(&json!({"commands": one("solid.pattern.rectangular", pattern.clone())}).to_string());
    frame(&mut app, &ctx, 2.0, vec![]);
    let ran = app.agent_chat.predict.work.as_ref().expect("computing").ran.clone();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while ran.load(Ordering::Relaxed) == 0 {
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    std::thread::sleep(std::time::Duration::from_millis(5));
    app.agent_chat.predict.work.as_mut().unwrap().expire_at_input_time(2.0);
    frame(&mut app, &ctx, 5.0, vec![]);
    withdrawn(&mut app, &chat, &was, "too slow");
    // The worker has let go of its counter once it ended.
    while Arc::strong_count(&ran) > 1 {
        assert!(std::time::Instant::now() < until, "the worker ends");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.session.execute("solid.pattern.rectangular", &pattern).expect("the user's own pattern");
    assert_eq!(volumes(&app.session)[0].2, 6 + 240);
    // Closing the design, and the predictor going away, stop what computes.
    let (work, _tx) = Work::held(0.0);
    let token = work.stop_token();
    app.agent_chat.predict.work = Some(work);
    app.session = Session::new(solidcraft_engine::doc::Document::default());
    frame(&mut app, &ctx, 6.0, vec![]);
    assert!(token.is_stopped(), "another design is open");
    let (work, _tx) = Work::held(0.0);
    let token = work.stop_token();
    app.agent_chat.predict.work = Some(work);
    drop(app);
    assert!(token.is_stopped());
}

/// G19 and G20: on a real worker the ghost comes once it is done; without threads it comes in the
/// same frame.
#[test]
fn g20_the_ghost_comes_from_a_worker_or_in_the_same_frame() {
    let (mut app, _chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    app.apply_off_thread = Some(std::time::Duration::ZERO);
    guesser.answer(&json!({"commands": holes()}).to_string());
    frame(&mut app, &ctx, 2.0, vec![]);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.agent_chat.predict.ghost.is_none() {
        assert!(std::time::Instant::now() < until && app.agent_chat.predict.shown().is_some(), "{}", status(&app));
        std::thread::sleep(std::time::Duration::from_millis(5));
        frame(&mut app, &ctx, 2.1, vec![]);
    }
    let (mut app, _chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    guesser.answer(&json!({"commands": holes()}).to_string());
    frame(&mut app, &ctx, 2.0, vec![]);
    assert!(app.agent_chat.predict.ghost.is_some(), "{}", status(&app));
}

/// G21: too many commands, too long an answer, not JSON, no commands in it.
#[test]
fn g21_an_answer_that_cannot_be_read_is_withdrawn() {
    let cube = json!({"command": "solid.box", "params": {"length": 1, "width": 1, "height": 1}});
    let many = json!({"commands": vec![cube.clone(); 33]}).to_string();
    let long = format!("{}{}", json!({"commands": [cube]}), " ".repeat(8_000));
    for reply in [
        many.as_str(),
        long.as_str(),
        "Sure, I will drill them.",
        r#"{"step": "x"}"#,
        r#"{"commands": "solid.box"}"#,
        r#"{"commands": [{"params": {}}]}"#,
        r#"{"commands": [{"command": "solid.box", "params": [1]}]}"#,
    ] {
        let (mut app, chat, guesser, ctx) = plate();
        line(&mut app, &ctx, &guesser, 0.0, HOLES);
        let was = before(&mut app);
        guesser.answer(reply);
        frame(&mut app, &ctx, 2.0, vec![]);
        withdrawn(&mut app, &chat, &was, "unreadable");
        assert_eq!(app.agent_chat.predict.log().last().unwrap().ran, 0);
    }
    let (app, ..) = answered(&json!(vec![json!({"command": "solid.box", "params": {"length": 1, "width": 1, "height": 1, "operation": "new"}}); 32]));
    assert!(app.agent_chat.predict.ghost.is_some(), "32 are taken: {}", status(&app));
    let (mut app, chat, _guesser, _ctx, was) = answered(&json!([]));
    withdrawn(&mut app, &chat, &was, "no commands");
}

// ---- C. Ghost and Tab ------------------------------------------------------------------------------

#[test]
fn final_review_preferences_owns_tab_and_blocks_all_prediction_acceptance() {
    for route in ["Tab", "History click", "WebView"] {
        let (mut app, _chat, guesser, ctx) = plate();
        ghost_drawn(&mut app, &ctx, &guesser);
        assert_eq!(page(&app)["ready"], true);
        let was = before(&mut app);
        if route == "History click" {
            let at = crate::scenario::rect_of("history:next_step").unwrap().center();
            frame(&mut app, &ctx, 2.1, vec![egui::Event::PointerMoved(at), press(at, true)]);
            frame(&mut app, &ctx, 2.2, vec![press(at, false)]);
        }
        // Preferences opens (e.g. from a mouse click) without a predictor tick.
        crate::prefs::open_at(&mut app, crate::prefs::PAGE_ASSISTANT);
        assert!(app.prefs_window.open);
        match route {
            "WebView" => page_act(&mut app, &ctx, 1, true),
            "Tab" => {
                let raw = egui::RawInput { time: Some(3.0), events: vec![key(egui::Key::Tab)], ..Default::default() };
                let _ = ctx.run_ui(raw, |ui| {
                    tick(&mut app, ui.ctx());
                    assert_eq!(app.session.document_stamp(), was.stamp, "Tab in Preferences changed CAD");
                    assert!(ui.ctx().input(|i| i.key_pressed(egui::Key::Tab)), "Tab belongs to Preferences");
                });
            }
            _ => frame(&mut app, &ctx, 3.0, vec![]),
        }
        assert_eq!(app.session.document_stamp(), was.stamp, "{route}");
        assert_eq!(app.session.undo.len(), was.undo, "{route}");
        assert!(app.prefs_window.open, "{route}");
        assert_ne!(page(&app)["ready"], true, "{route}");
    }
}

#[test]
fn review_round_one_a_chip_click_must_start_on_a_ready_ghost() {
    let (mut app, _chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let was = before(&mut app);
    assert_eq!(page(&app)["ready"], false);
    let words = crate::scenario::rect_of("history:next_step").unwrap();
    let at = words.center();
    frame(&mut app, &ctx, 1.8, vec![egui::Event::PointerMoved(at), press(at, true)]);
    answer_commands(&mut app, &ctx, &guesser, 1.9, &holes());
    assert_eq!(page(&app)["ready"], true);
    let ghost_chip = crate::scenario::rect_of("history:next_step").unwrap();
    assert!(ghost_chip.contains(at), "release is inside both chip positions");
    frame(&mut app, &ctx, 2.0, vec![press(at, false)]);
    frame(&mut app, &ctx, 2.01, vec![]);
    assert_eq!(app.session.document_stamp(), was.stamp, "an early pointer-down is not consent");

    frame(&mut app, &ctx, 2.3, vec![press(at, true)]);
    frame(&mut app, &ctx, 2.4, vec![press(at, false)]);
    frame(&mut app, &ctx, 2.5, vec![]);
    assert_eq!(outcome(&app, 0), "accepted", "a fresh click on the visible ghost still lands it");
}

#[test]
fn review_round_one_short_web_turn_between_frames_retires_each_prediction_stage() {
    for stage in [Stage::Asking, Stage::Drawn] {
        let (mut app, chat, guesser, ctx) = at_stage(stage);
        let was = before(&mut app);
        let prediction_prompts = guesser.prompts().len();
        app.preferences.agent_chat_preview = false;
        app.agent_chat.open = true;
        app.agent_chat.web_host = true;
        crate::agent_chat::web::act(&mut app, &ctx, r#"{"type":"send","text":"What is the plate for?"}"#).unwrap();
        assert_eq!(chat.prompts().len(), 1);
        chat.answer("A mounting plate.");
        crate::agent_chat::web::advance(&mut app, &ctx);
        assert_eq!(app.agent_chat.state(), State::Ready, "{stage:?}: the turn already ended");
        if stage == Stage::Asking {
            guesser.answer(HOLES);
        }
        frame(&mut app, &ctx, 3.0, vec![]);
        assert_eq!(guesser.prompts().len(), prediction_prompts, "{stage:?}: no further prediction question");
        assert!(app.agent_chat.predict.shown().is_none(), "{stage:?}: no stale suggestion");
        assert_eq!(app.session.document_stamp(), was.stamp);
    }
}

#[test]
fn review_round_one_enqueue_retires_prediction_even_if_the_message_is_removed_before_a_frame() {
    let (mut app, chat, _guesser, ctx) = at_stage(Stage::Drawn);
    let was = before(&mut app);
    // Put the recording backend in Busy without polling a predictor frame.
    app.agent_chat.state = Some(State::Busy);
    app.agent_chat.open = true;
    app.agent_chat.web_host = true;
    crate::agent_chat::web::act(&mut app, &ctx, r#"{"type":"send","text":"A follow-up"}"#).unwrap();
    assert_eq!(app.agent_chat.pending.len(), 1);
    assert!(chat.prompts().is_empty(), "the message was queued, not sent");
    crate::agent_chat::remove_pending(&mut app, 0);
    app.agent_chat.state = Some(State::Ready);
    frame(&mut app, &ctx, 3.0, vec![]);
    withdrawn(&mut app, &chat, &was, "dismissed");
}

/// Querying a new overlay does not make it the scene the viewport has already applied.
#[test]
fn review_fix_tab_refuses_a_ghost_when_the_applied_scene_is_for_another_ghost() {
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| crate::preview::update(&mut app, ui.ctx()));
    assert!(app.preview.active);
    let old = app.agent_chat.predict.ghost.take().unwrap();
    app.agent_chat.predict.shown.as_mut().unwrap().id = 999;
    app.agent_chat.predict.ghost = Some(Ghost::new(999, old.drawn));
    assert!(crate::agent_chat::review::overlay(&mut app).is_some());
    let was = before(&mut app);
    assert_eq!(page(&app)["ready"], false, "the old scene is still applied");
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    assert_eq!(app.session.document_stamp(), was.stamp);
}

#[test]
fn review_fix_repeated_tab_cannot_accept_a_ghost_that_arrived_while_held() {
    let (mut app, _chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let was = before(&mut app);
    frame(&mut app, &ctx, 1.8, vec![key(egui::Key::Tab)]);
    answer_commands(&mut app, &ctx, &guesser, 2.0, &holes());
    // egui marks the second press as a repeat because there was no release.
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    assert_eq!(app.session.document_stamp(), was.stamp);
}

#[test]
fn review_fix_modified_tab_cannot_accept_the_visible_ghost() {
    for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::ALT] {
        let (mut app, _chat, guesser, ctx) = plate();
        ghost_drawn(&mut app, &ctx, &guesser);
        let was = before(&mut app);
        let mut tab = key(egui::Key::Tab);
        if let egui::Event::Key { modifiers: m, .. } = &mut tab {
            *m = modifiers;
        }
        frame(&mut app, &ctx, 3.0, vec![tab]);
        assert_eq!(app.session.document_stamp(), was.stamp, "{modifiers:?}");
    }
}

#[test]
fn review_fix_disabled_prediction_cannot_land_from_any_entry_point_before_the_next_frame() {
    for route in ["tab", "click", "page"] {
        let (mut app, _chat, guesser, ctx) = plate();
        ghost_drawn(&mut app, &ctx, &guesser);
        let was = before(&mut app);
        app.preferences.agent_next_step = false;
        match route {
            "page" => page_act(&mut app, &ctx, 1, true),
            "click" => {
                app.agent_chat.predict.clicked = Some(1);
                frame(&mut app, &ctx, 3.0, vec![]);
            }
            _ => frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]),
        }
        assert_eq!(app.session.document_stamp(), was.stamp, "{route}");
        assert_eq!(app.session.undo.len(), was.undo, "{route}");
    }
}

#[test]
fn review_fix_busy_chat_cannot_land_from_either_key_route_before_the_next_frame() {
    for web in [false, true] {
        let (mut app, chat, guesser, ctx) = plate();
        ghost_drawn(&mut app, &ctx, &guesser);
        let was = before(&mut app);
        crate::agent_chat::send(&mut app, &ctx, "now a lid".into());
        assert_eq!(chat.prompts().len(), 1);
        if web {
            page_act(&mut app, &ctx, 1, true);
        } else {
            frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
        }
        assert_eq!(app.session.document_stamp(), was.stamp, "{web}");
    }
}

#[test]
fn review_fix_first_reply_while_chat_starts_cannot_bill_for_commands_or_show_a_line() {
    let (mut app, chat, guesser, ctx) = plate();
    asked(&mut app, &ctx, &guesser);
    guesser.answer(HOLES);
    crate::agent_chat::send(&mut app, &ctx, "now a lid".into());
    assert_eq!(chat.prompts().len(), 1);
    frame(&mut app, &ctx, 6.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1, "{:?}", guesser.calls());
    assert!(app.agent_chat.predict.shown().is_none());
}

fn page_act(app: &mut SolveApp, ctx: &egui::Context, id: u64, accept: bool) {
    app.agent_chat.open = true;
    app.agent_chat.web_host = true;
    crate::agent_chat::web::act(app, ctx, &json!({"type": "suggestion", "id": id.to_string(), "accept": accept}).to_string()).unwrap();
}

#[test]
fn review_fix_web_action_for_a_previous_guess_cannot_accept_the_current_ghost() {
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    assert_eq!(page(&app)["ready"], true);
    let was = before(&mut app);
    page_act(&mut app, &ctx, 999, true);
    assert_eq!(app.session.document_stamp(), was.stamp);
    page_act(&mut app, &ctx, 1, true);
    assert_eq!(outcome(&app, 0), "accepted");
}

#[test]
fn review_fix_completion_queued_at_the_cutoff_cannot_become_a_ghost() {
    let (mut app, chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    let drawn = app.agent_chat.predict.ghost.take().unwrap().drawn;
    let was = before(&mut app);
    let (work, tx) = Work::held(2.0);
    tx.send((work.deadline(), Ok(drawn))).unwrap();
    app.agent_chat.predict.work = Some(work);
    frame(&mut app, &ctx, 5.1, vec![]);
    withdrawn(&mut app, &chat, &was, "too slow");
}

/// G22: the ghost is drawn over the design and the design does not move; it is not a chat preview,
/// and nothing else is asked while it is there.
#[test]
fn g22_the_ghost_is_drawn_without_touching_the_design() {
    let (mut app, chat, guesser, ctx) = plate();
    let was = before(&mut app);
    ghost_drawn(&mut app, &ctx, &guesser);
    let (_, design, copy) = crate::agent_chat::review::overlay(&mut app).expect("the view draws it");
    let faces = |s: &solidcraft_engine::doc::ModelState| s.bodies.iter().map(|b| b.body.face_count()).sum::<usize>();
    assert_eq!((faces(&design), faces(&copy)), (6, 10));
    assert!(app.agent_chat.preview.is_none());
    assert_eq!(app.session.document_stamp(), was.stamp);
    assert!(Arc::ptr_eq(&app.session.doc, &was.doc));
    assert_eq!(app.session.undo.len(), was.undo);
    for t in [10.0, 600.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!(guesser.prompts().len(), 2, "{:?}", guesser.calls());
    assert!(chat.calls().is_empty(), "{:?}", chat.calls());
}

/// G23: Tab puts exactly the ghost into the design as one undo step: no message, no card.
#[test]
fn g23_tab_lands_what_was_drawn_as_one_undo_step() {
    let (mut app, chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    let doc = app.session.doc.clone();
    let undo = app.session.undo.len();
    let mut ghosts: Vec<_> = app
        .agent_chat
        .predict
        .ghost
        .as_ref()
        .unwrap()
        .after()
        .bodies
        .iter()
        .map(|b| (b.name.clone(), b.body.volume_at(0.01).unwrap(), b.body.face_count()))
        .collect();
    ghosts.sort_by(|a, b| a.0.cmp(&b.0));
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    assert_eq!(app.session.doc.features.len(), doc.features.len() + 4);
    let now = volumes(&app.session);
    assert_eq!(now.len(), ghosts.len());
    for (a, b) in now.iter().zip(&ghosts) {
        assert_eq!((&a.0, a.2), (&b.0, b.2));
        assert!((a.1 - b.1).abs() <= 1e-6 * b.1.abs(), "{a:?} {b:?}");
    }
    assert_eq!(app.session.undo.len(), undo + 1, "one undo step");
    assert_eq!(app.session.undo.last().unwrap().label, "Drill four 5 mm through holes near the corners of Plate");
    assert!(chat.calls().is_empty(), "{:?}", chat.calls());
    assert!(!app.agent_chat.items.iter().any(|i| matches!(i, Item::Review(_) | Item::User { .. })));
    assert_eq!(outcome(&app, 0), "accepted");
    assert!(app.agent_chat.predict.ghost.is_none() && app.agent_chat.predict.shown().is_none());
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&*app.session.doc).unwrap(), serde_json::to_value(&*doc).unwrap(), "undo gives back the document");
    assert_eq!(volumes(&app.session)[0].2, 6);
}

fn step_text(session: &mut Session, name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("intcad-ghost-step-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.step");
    session.execute("file.export", &json!({"path": path.to_string_lossy()})).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    text.lines().filter(|l| !l.starts_with("FILE_NAME")).collect::<Vec<_>>().join("\n")
}

/// G24: the design after Tab is the design the same commands make when they are run directly.
#[test]
fn g24_tab_makes_what_running_the_commands_makes() {
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    let (mut twin, ..) = plate();
    for step in holes().as_array().unwrap() {
        twin.session.execute(step["command"].as_str().unwrap(), &step["params"]).unwrap();
    }
    assert_eq!(step_text(&mut app.session, "tab"), step_text(&mut twin.session, "direct"));
}

/// G25: Tab before the ghost is drawn is used up and does nothing; the next one lands it.
#[test]
fn g25_tab_before_the_ghost_does_nothing() {
    let (mut app, chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let was = before(&mut app);
    frame(&mut app, &ctx, 1.8, vec![key(egui::Key::Tab)]);
    assert!(app.agent_chat.predict.shown().is_some(), "the line stays: {}", status(&app));
    assert_eq!(app.session.document_stamp(), was.stamp);
    assert!(ctx.memory(|m| m.focused().is_none()));
    answer_commands(&mut app, &ctx, &guesser, 2.0, &holes());
    assert_eq!(app.session.document_stamp(), was.stamp, "the early Tab was not kept for later");
    assert_eq!(status(&app)["state"], "ghost");
    frame(&mut app, &ctx, 3.0, vec![release(egui::Key::Tab), key(egui::Key::Tab)]);
    assert_eq!(outcome(&app, 0), "accepted");
    assert!(chat.calls().is_empty());
}

/// G26: "Preview the agent's changes" makes no difference to Tab.
#[test]
fn g26_tab_lands_the_ghost_whether_previews_are_on_or_off() {
    for on in [false, true] {
        let (mut app, chat, guesser, ctx) = plate();
        app.preferences.agent_chat_preview = on;
        ghost_drawn(&mut app, &ctx, &guesser);
        frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
        assert_eq!(outcome(&app, 0), "accepted", "previews {on}");
        assert_eq!(volumes(&app.session)[0].2, 10, "previews {on}");
        assert!(app.agent_chat.preview.is_none() && chat.calls().is_empty(), "previews {on}");
    }
}

/// G27: after Tab the next first question goes at once; after Esc or an empty answer none does.
#[test]
fn g27_after_tab_the_next_question_goes_at_once() {
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    frame(&mut app, &ctx, 3.02, vec![]);
    guesser.ready();
    frame(&mut app, &ctx, 3.04, vec![]);
    assert_eq!(guesser.prompts().len(), 3, "{:?}", guesser.calls());
    assert!(guesser.prompts()[2].contains("Hole4"), "about the design with the holes");
    // An empty answer ends the chain.
    guesser.answer(r#"{"step": "", "reason": ""}"#);
    for t in [4.0, 600.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!(guesser.prompts().len(), 3);
    // A later edit waits for the pause again.
    edit(&mut app, 300.0);
    frame(&mut app, &ctx, 601.0, vec![]);
    frame(&mut app, &ctx, 602.4, vec![]);
    guesser.ready();
    frame(&mut app, &ctx, 602.45, vec![]);
    assert_eq!(guesser.prompts().len(), 3, "{:?}", guesser.calls());
    frame(&mut app, &ctx, 602.5, vec![]);
    assert_eq!(guesser.prompts().len(), 4);
}

/// G28: a landing that fails half way leaves the design as it was.
#[test]
fn g28_a_landing_that_fails_leaves_the_design_as_it_was() {
    let (mut app, chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    let was = before(&mut app);
    // The design refuses the second step.
    app.agent_chat.predict.ghost.as_mut().unwrap().drawn.steps[1] =
        ("solid.fillet".into(), json!({"edges": [[0, 0, 3]], "radius": 500, "body": "Plate"}));
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    assert!(app.agent_chat.predict.ghost.is_none() && app.agent_chat.predict.shown().is_none());
    assert_eq!(outcome(&app, 0), "failed");
    assert!(chat.calls().is_empty());
    assert!(Arc::ptr_eq(&app.session.doc, &was.doc), "the document is the one before");
    assert_eq!(app.session.undo.len(), was.undo);
    assert_eq!(volumes(&app.session)[0].2, 6);
}

/// G29: a chat message takes the ghost away, and a ghost and a chat preview are never both drawn.
#[test]
fn g29_a_chat_message_takes_the_ghost_away() {
    for previews in [false, true] {
        let (mut app, chat, guesser, ctx) = plate();
        app.preferences.agent_chat_preview = previews;
        ghost_drawn(&mut app, &ctx, &guesser);
        crate::agent_chat::send(&mut app, &ctx, "now a lid".into());
        assert_eq!(chat.prompts().len(), 1);
        assert_eq!(app.agent_chat.preview.is_some(), previews);
        assert!(
            !previews || crate::agent_chat::review::overlay(&mut app).is_none(),
            "the chat's preview has drawn nothing yet, and the ghost is not drawn for it"
        );
        frame(&mut app, &ctx, 3.0, vec![]);
        assert!(app.agent_chat.predict.ghost.is_none() && app.agent_chat.predict.shown().is_none(), "{}", status(&app));
        assert_eq!(outcome(&app, 0), "dismissed");
    }
}

// ---- D. Which numbers have a source ----------------------------------------------------------------

/// G30: each number is known, with its source, or guessed, whatever its spelling; counts are
/// neither.
#[test]
fn g30_each_number_is_known_with_its_source_or_guessed() {
    let (mut app, ..) = plate();
    let run = |app: &mut SolveApp, c: &str, p: Value| app.session.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"));
    run(&mut app, "parameters.add", json!({"name": "w", "expression": "40 mm"}));
    run(&mut app, "solid.fillet", json!({"edges": [[0, 0, 3]], "radius": 2, "body": "Plate", "name": "Round"}));
    run(&mut app, "solid.hole", json!({"position": [10, 12, 6], "diameter": 7, "name": "Old"}));
    let steps = vec![
        ("solid.fillet".to_string(), json!({"radius": 2, "edges": [[9, 9, 9]]})),
        ("solid.fillet".to_string(), json!({"radius": "2 mm"})),
        ("solid.fillet".to_string(), json!({"radius": 2.0})),
        ("solid.box".to_string(), json!({"length": "w/2", "width": 12, "height": 3.4, "name": "9.9"})),
        ("solid.hole".to_string(), json!({"position": [10, 33.3, 6], "diameter": 3.6, "depth": 2.5})),
        ("solid.pattern.circular".to_string(), json!({"features": ["Old"], "axis": "Z", "count": 4})),
    ];
    let marks = ghost::marks(
        &app.session,
        &steps,
        "Faces:\n- Top of Plate: plane, 12 mm wide",
        "use 3.4 mm screws",
        "Inserts\nBoard inserts take 3.6 mm holes.",
    );
    let of =
        |key: &str, text: &str| marks.iter().find(|m| m.key == key && m.text == text).unwrap_or_else(|| panic!("{key} {text}: {marks:?}")).source;
    for text in ["2", "2 mm"] {
        assert_eq!(of("radius", text), Some("ui.next_step_src_design"), "{text}");
    }
    assert_eq!(marks.iter().filter(|m| m.key == "radius").count(), 3);
    assert!(marks.iter().filter(|m| m.key == "radius").all(|m| m.source.is_some()), "{marks:?}");
    assert_eq!(of("length", "w/2"), Some("ui.next_step_src_parameter"));
    assert_eq!(of("width", "12"), Some("ui.next_step_src_selection"));
    assert_eq!(of("height", "3.4"), Some("ui.next_step_src_said"));
    assert_eq!(of("diameter", "3.6"), Some("ui.next_step_src_notes"));
    assert_eq!(of("depth", "2.5"), None, "guessed");
    assert_eq!(of("position.x", "10"), Some("ui.next_step_src_design"), "the hole's x");
    assert_eq!(of("position.y", "33.3"), None, "guessed");
    // A size is known only as the same thing: the fillet's 2 is no hole, and the 7 mm hole is a
    // cylinder of radius 3.5 too.
    let of_one = |command: &str, params: Value| {
        let key = params.as_object().unwrap().keys().next().unwrap().clone();
        let marks = ghost::marks(&app.session, &[(command.to_string(), params)], "", "", "");
        marks.iter().find(|m| m.key == key).unwrap().source.is_some()
    };
    assert!(!of_one("solid.hole", json!({"diameter": 2})), "a fillet's radius is not a hole's size");
    assert!(!of_one("solid.cylinder", json!({"radius": 2})));
    assert!(of_one("solid.cylinder", json!({"radius": 3.5})) && of_one("solid.hole", json!({"diameter": 7})));
    assert!(!of_one("solid.cylinder", json!({"radius": 7})));
    assert!(
        !marks.iter().any(|m| m.key.contains("count") || m.key.contains("edges") || m.text == "9.9"),
        "counts, picks and names are not numbers: {marks:?}"
    );
}

/// G31 and G32: the line says in words what was guessed, its tooltip where the rest comes from, in
/// the app's language; what the agent claims changes nothing.
#[test]
fn g31_g32_the_line_says_what_was_guessed_whatever_the_agent_claims() {
    for claim in ["", r#""guessed": [], "#] {
        let (mut app, _chat, guesser, ctx) = plate();
        app.session.execute("solid.hole", &json!({"position": [10, 12, 6], "diameter": 7, "name": "Old"})).unwrap();
        line(&mut app, &ctx, &guesser, 0.0, &HOLES.replace("5 mm", "7 mm"));
        assert!(!chip(&app).unwrap().0.contains("guessed"), "nothing is known of the numbers before the commands");
        guesser.answer(&format!(
            r#"{{{claim}"commands": [{{"command": "solid.hole", "params": {{"position": [10, 33, 6], "diameter": 7, "depth": 2.5}}}}]}}"#
        ));
        frame(&mut app, &ctx, 2.0, vec![]);
        let (text, tip, _) = chip(&app).unwrap();
        assert!(text.contains("guessed:") && text.contains("2.5") && text.contains("position"), "{text}");
        let guessed = text.split("guessed:").nth(1).unwrap();
        assert!(!guessed.contains('7'), "the design's own diameter is not guessed: {text}");
        assert!(tip.unwrap().contains("7: already used in this design"));
        let _zh = crate::i18n::scope("zh");
        let (text, ..) = chip(&app).unwrap();
        assert!(text.contains("猜的：") && text.contains("位置"), "{text}");
    }
}

// ---- F. Agents and sessions ------------------------------------------------------------------------

#[cfg(unix)]
fn first_session(name: &str, kinds: &[&str]) -> Value {
    let (dir, argv) = signed_in(name, CLAUDE_INIT, "", kinds);
    let ctx = crate::fonts::test_context();
    let mut b = start_backend(&argv, &ctx, None).unwrap();
    poll_until(b.as_mut(), &dir, &|d| !sent(d, "session/new").is_empty());
    let params = sent(&dir, "session/new")[0]["params"].clone();
    b.shutdown(true);
    let _ = std::fs::remove_dir_all(&dir);
    params
}

#[cfg(unix)]
fn lean(params: &Value) -> bool {
    params["systemPrompt"]["mode"] == "override" && params["_meta"]["claudeCode"]["options"]["tools"] == json!([])
}

/// G38: of the six things an agent may say of its sign-in, only an API key makes a lean session.
#[cfg(unix)]
#[test]
fn g38_only_an_api_key_makes_a_lean_session() {
    for (kind, expected) in [("account", false), ("api_key", true), ("gateway", false), ("external", false), ("none", false), ("", false)] {
        let params = first_session(&format!("kind-{kind}"), &[kind]);
        assert_eq!(lean(&params), expected, "{kind}: {params}");
    }
}

/// G39: the first session opens before the agent said anything, and the next ones follow what it
/// said last.
#[cfg(unix)]
#[test]
fn g39_sessions_follow_what_the_agent_last_said() {
    let (dir, argv) = signed_in("late", CLAUDE_INIT, "", &["", "api_key", "account"]);
    let ctx = crate::fonts::test_context();
    let mut b = start_backend(&argv, &ctx, None).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !b.key_sign_in() {
        assert!(std::time::Instant::now() < deadline);
        b.poll();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    b.new_session();
    while b.key_sign_in() {
        assert!(std::time::Instant::now() < deadline);
        b.poll();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    b.new_session();
    poll_until(b.as_mut(), &dir, &|d| sent(d, "session/new").len() >= 3);
    let sessions = sent(&dir, "session/new");
    assert_eq!(sessions.iter().map(|s| lean(&s["params"])).collect::<Vec<_>>(), [false, true, false], "{sessions:?}");
    b.shutdown(true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// G40: a full guessing session is the agent's own, with the guessing note, none of the user's MCP
/// servers and no MCP tool to call. (The chat's session is unchanged: L2.)
#[cfg(unix)]
#[test]
fn g40_a_full_guessing_session_may_call_no_mcp_tool() {
    let params = first_session("full", &["account"]);
    assert_eq!(params["systemPrompt"]["mode"], "append", "{params}");
    assert_eq!(params["_meta"]["systemPrompt"]["append"], crate::agent_chat::acp::system_note(SESSION_NOTE));
    let options = &params["_meta"]["claudeCode"]["options"];
    for key in ["tools", "settingSources"] {
        assert!(options.get(key).is_none(), "{key}: {params}");
    }
    assert_eq!(options["strictMcpConfig"], true, "{params}");
    let mut off: Vec<&str> = crate::agent_chat::acp::CLAUDE_DISALLOWED_TOOLS.to_vec();
    off.push("mcp__*");
    assert_eq!(options["disallowedTools"], json!(off));
    assert_eq!(params["mcpServers"], json!([]));
}

/// L8: a lean session keeps the user's MCP servers out, and a chat session says nothing of them.
#[cfg(unix)]
#[test]
fn l8_a_lean_session_loads_none_of_the_users_mcp_servers() {
    let params = first_session("strict", &["api_key"]);
    assert_eq!(params["_meta"]["claudeCode"]["options"]["strictMcpConfig"], true, "{params}");
}

/// G41: the switch for lean sessions on a plan is in the code, and off.
#[cfg(unix)]
#[test]
fn g41_lean_on_a_plan_is_a_switch_in_the_code_and_off() {
    for (on, expected) in [(false, false), (true, true)] {
        let (dir, argv) = signed_in(&format!("switch-{on}"), CLAUDE_INIT, "", &["account", "account"]);
        let start = crate::agent_chat::backend::StartContext { cwd: dir.clone(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut b = crate::agent_chat::acp_client::AcpBackend::start(&argv, start).unwrap();
        b.set_instructions(SESSION_NOTE);
        b.lean_sessions(SESSION_NOTE);
        if on {
            b.lean_on_plans();
        }
        poll_until(&mut b, &dir, &|d| !sent(d, "session/new").is_empty());
        assert_eq!(lean(&sent(&dir, "session/new")[0]["params"]), expected, "on: {on}");
        b.shutdown(true);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// G42: no background process or prompt for Codex until its tool isolation is verified.
#[test]
fn review_fix_g42_codex_prediction_is_unavailable_but_chat_works() {
    let (mut app, chat, guesser, ctx) = plate();
    app.agent_chat.running_id = "codex".into();
    for t in [0.0, 5.0, 60.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert!(guesser.prompts().is_empty());
    assert!(app.agent_chat.predict.backend.is_none());
    assert_eq!(status(&app)["state"], "no agent");
    crate::agent_chat::send(&mut app, &ctx, "now a lid".into());
    assert_eq!(chat.prompts().len(), 1);
}

/// G43: any other agent, an API endpoint or a Mastra workflow is asked nothing, and the line says so.
#[test]
fn g43_other_agents_are_asked_nothing() {
    let cases: [fn(&mut SolveApp); 4] = [
        |app| app.agent_chat.running_id = "opencode".into(),
        |app| app.agent_chat.running_id = "intcad".into(),
        |app| app.agent_chat.running_endpoint = "https://api.example.com".into(),
        |app| app.agent_chat.mastra = true,
    ];
    for (i, case) in cases.into_iter().enumerate() {
        let (mut app, _chat, guesser, ctx) = plate();
        case(&mut app);
        for t in [0.0, 5.0, 60.0] {
            frame(&mut app, &ctx, t, vec![]);
        }
        assert!(guesser.prompts().is_empty(), "case {i}: {:?}", guesser.calls());
        assert_eq!(status(&app)["state"], "no agent", "case {i}");
    }
    let (mut app, ..) = plate();
    app.agent_chat.running_id = "opencode".into();
    assert_eq!(chip(&app).unwrap().0, "Next step: not available with this agent yet");
}

/// G44: what the agent asks to do during either question is declined.
#[test]
fn g44_permission_requests_are_declined_in_both_questions() {
    let (mut app, _chat, guesser, ctx) = plate();
    let ask = |guesser: &Fake, key: u64| guesser.say(Event::Permission { key, title: "intcad_execute".into(), kind: None, options: Vec::new() });
    asked(&mut app, &ctx, &guesser);
    ask(&guesser, 1);
    frame(&mut app, &ctx, 5.5, vec![]);
    guesser.answer(HOLES);
    frame(&mut app, &ctx, 6.0, vec![]);
    guesser.ready();
    frame(&mut app, &ctx, 6.1, vec![]);
    ask(&guesser, 2);
    frame(&mut app, &ctx, 6.2, vec![]);
    for key in [1, 2] {
        assert!(guesser.calls().contains(&format!("permission: {key} None")), "{:?}", guesser.calls());
    }
}

// ---- G. Limits and the log -------------------------------------------------------------------------

/// Edit and wait, asking one first question and having it answered with nothing, at `t`.
fn one_question(app: &mut SolveApp, ctx: &egui::Context, guesser: &Fake, t: f64, n: usize) {
    edit(app, 20.0 * (n as f64 + 10.0));
    frame(app, ctx, t, vec![]);
    frame(app, ctx, t + 1.5, vec![]);
    guesser.ready();
    frame(app, ctx, t + 1.6, vec![]);
    guesser.answer(r#"{"step": "", "reason": ""}"#);
    frame(app, ctx, t + 1.7, vec![]);
}

/// G45: on a plan 20 suggestions an hour, told once; on an API key no limit. A suggestion counts
/// once, and so does each one after a Tab.
#[test]
fn g45_twenty_suggestions_an_hour_on_a_plan() {
    let (mut app, _chat, guesser, ctx) = plate();
    for n in 0..20 {
        one_question(&mut app, &ctx, &guesser, 10.0 + 80.0 * n as f64, n);
    }
    assert_eq!(guesser.prompts().len(), 20, "{}", status(&app));
    app.set_status("", false);
    one_question(&mut app, &ctx, &guesser, 1700.0, 20);
    assert_eq!(guesser.prompts().len(), 20, "the 21st is not asked");
    let said = |app: &SolveApp| app.status.as_ref().is_some_and(|s| s.0.contains("this hour's 20 suggestions are used up"));
    assert!(said(&app), "{:?}", app.status);
    app.set_status("", false);
    frame(&mut app, &ctx, 1800.0, vec![]);
    assert!(!said(&app), "said once: {:?}", app.status);
    // An hour after the first one, one is free again.
    frame(&mut app, &ctx, 3612.0, vec![]);
    guesser.ready();
    frame(&mut app, &ctx, 3612.1, vec![]);
    assert_eq!(guesser.prompts().len(), 21, "{}", status(&app));
    // The setting.
    let (mut app, _chat, guesser, ctx) = plate();
    app.preferences.agent_next_step_per_hour = 5;
    for n in 0..8 {
        one_question(&mut app, &ctx, &guesser, 10.0 + 10.0 * n as f64, n);
    }
    assert_eq!(guesser.prompts().len(), 5);
    // An API key.
    let (mut app, _chat, guesser, ctx) = plate();
    guesser.key.store(true, Ordering::Relaxed);
    for n in 0..60 {
        one_question(&mut app, &ctx, &guesser, 10.0 + 10.0 * n as f64, n);
    }
    assert_eq!(guesser.prompts().len(), 60);
    // Both questions of a suggestion, and the one after Tab.
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    assert_eq!(status(&app)["hourUsed"], 1, "two questions, one suggestion");
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    frame(&mut app, &ctx, 3.1, vec![]);
    guesser.ready();
    frame(&mut app, &ctx, 3.2, vec![]);
    assert_eq!(status(&app)["hourUsed"], 2);
}

/// G46: each suggestion's entry has what both questions took, as reported, and one outcome.
#[test]
fn g46_the_log_has_both_questions_and_one_outcome() {
    let (mut app, _chat, guesser, ctx) = plate();
    asked(&mut app, &ctx, &guesser);
    let spent = |n: u64, cost: Option<f64>| Usage {
        input_tokens: Some(n),
        output_tokens: Some(n + 1),
        cost: cost.map(|c| (c, "USD".into())),
        ..Default::default()
    };
    *guesser.usage.lock().unwrap() = Some(spent(100, Some(0.01)));
    guesser.answer(HOLES);
    frame(&mut app, &ctx, 6.0, vec![]);
    guesser.ready();
    frame(&mut app, &ctx, 6.1, vec![]);
    // The agent reports the session's cost so far, and each question's own tokens.
    *guesser.usage.lock().unwrap() = Some(spent(200, Some(0.03)));
    guesser.answer(&json!({"commands": holes()}).to_string());
    frame(&mut app, &ctx, 7.6, vec![]);
    frame(&mut app, &ctx, 7.7, vec![key(egui::Key::Tab)]);
    let l = app.agent_chat.predict.log()[0].clone();
    assert_eq!((l.input_tokens, l.output_tokens, l.cost, l.ms), (Some(100), Some(101), Some(0.01), Some(1000)));
    let c = l.commands.clone().unwrap();
    assert_eq!((c.input_tokens, c.output_tokens, c.ms, c.billed), (Some(200), Some(201), Some(1600), true));
    assert!((c.cost.unwrap() - 0.02).abs() < 1e-9, "{:?}", c.cost);
    assert!(c.prompt_chars > 0 && c.reply_chars > 0);
    assert_eq!((l.outcome.as_str(), l.ran), ("accepted", 4));
    assert!(l.engine_ms.is_some() && l.line_ms == Some(1000));
    assert!((status(&app)["totalCost"].as_f64().unwrap() - 0.03).abs() < 1e-9);
    // Not reported: not invented.
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    let l = app.agent_chat.predict.log()[0].clone();
    let c = l.commands.unwrap();
    assert_eq!((l.cost, l.input_tokens, c.cost, c.input_tokens), (None, None, None, None));
    // At most 200 entries.
    guesser.key.store(true, Ordering::Relaxed);
    for n in 0..210 {
        one_question(&mut app, &ctx, &guesser, 10.0 + 10.0 * n as f64, n);
    }
    assert_eq!(app.agent_chat.predict.log().len(), MAX_LOG);
}

/// G47: lines the app took back are counted; with the line-first switch off, the line comes only
/// with its ghost.
#[test]
fn g47_lines_taken_back_are_counted_and_line_first_can_be_switched_off() {
    let (mut app, _chat, guesser, ctx) = plate();
    guesser.key.store(true, Ordering::Relaxed);
    for n in 0..40 {
        let t = 10.0 + 10.0 * f64::from(n);
        edit(&mut app, 20.0 * f64::from(n + 10));
        frame(&mut app, &ctx, t, vec![]);
        frame(&mut app, &ctx, t + 1.5, vec![]);
        guesser.ready();
        frame(&mut app, &ctx, t + 1.6, vec![]);
        guesser.answer(&HOLES.replace("near", &format!("{} mm from", n + 1)));
        frame(&mut app, &ctx, t + 1.7, vec![]);
        guesser.ready();
        frame(&mut app, &ctx, t + 1.8, vec![]);
        let reply = if n < 3 { "no".to_string() } else { json!({"commands": holes()}).to_string() };
        guesser.answer(&reply);
        frame(&mut app, &ctx, t + 1.9, vec![]);
    }
    assert_eq!((status(&app)["lines"].clone(), status(&app)["withdrawn"].clone()), (json!(40), json!(3)), "{}", status(&app)["log"]);
    let (mut app, _chat, guesser, ctx) = plate();
    app.agent_chat.predict.ghost_first = true;
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    assert!(chip(&app).is_none_or(|(_, _, guess)| !guess), "{:?}", chip(&app));
    answer_commands(&mut app, &ctx, &guesser, 2.0, &holes());
    assert!(chip(&app).unwrap().0.contains("Drill four"));
}

// ---- A, E, G, I: said, places, the panel, sheet metal and the account -------------------------------

/// G1: off by default, and where it is turned on it says whose agent and plan it uses.
#[test]
fn g1_the_setting_says_whose_agent_and_plan_it_uses() {
    assert!(!crate::prefs::Prefs::default().agent_next_step);
    for (language, agent, plan) in [("en", "your own agent", "plan"), ("zh", "你自己的 agent", "套餐")] {
        let _scope = crate::i18n::scope(language);
        for key in ["ui.next_step_setting_tip", "ui.next_step_cost"] {
            let text = tr(key);
            assert!(text.contains(agent) && text.contains(plan), "{language} {key}: {text}");
        }
    }
}

/// G33: with a ghost drawn, Tab and Esc are ours in the view and the field's, the dialog's or the
/// Ask Here box's elsewhere; there the ghost stays.
#[test]
fn g33_tab_and_esc_are_ours_only_in_the_view() {
    for place in ["field", "dialog", "ask here"] {
        let (mut app, _chat, guesser, ctx) = plate();
        ghost_drawn(&mut app, &ctx, &guesser);
        let was = before(&mut app);
        match place {
            "dialog" => app.dialog = Some(crate::dialogs::Dialog::chat_context(&Vec::new())),
            "ask here" => app.agent_chat.ask_box.open = true,
            _ => {}
        }
        assert!(place != "dialog" || app.dialog.is_some());
        let mut text = String::new();
        let mut run = |app: &mut SolveApp, t: f64, events: Vec<egui::Event>| {
            let raw = egui::RawInput { time: Some(t), events, ..Default::default() };
            let _ = ctx.run_ui(raw, |ui| {
                tick(app, ui.ctx());
                if place == "field" {
                    ui.text_edit_singleline(&mut text).request_focus();
                }
            });
        };
        run(&mut app, 3.0, vec![]);
        app.esc_handled = false;
        for (t, k) in [(3.1, egui::Key::Tab), (3.2, egui::Key::Escape)] {
            run(&mut app, t, vec![key(k)]);
            assert!(!app.esc_handled, "{place}: Esc is its own");
            assert!(app.agent_chat.predict.ghost.is_some(), "{place}: the ghost stays, {}", status(&app));
            assert_eq!(app.session.document_stamp(), was.stamp, "{place}: nothing landed");
        }
    }
    // The view: ours.
    for (k, what) in [(egui::Key::Tab, "accepted"), (egui::Key::Escape, "dismissed")] {
        let (mut app, _chat, guesser, ctx) = plate();
        ghost_drawn(&mut app, &ctx, &guesser);
        frame(&mut app, &ctx, 3.0, vec![key(k)]);
        assert_eq!(outcome(&app, 0), what);
        assert!(app.agent_chat.predict.ghost.is_none());
    }
}

/// G36 (the app's side): what the page is told at each stage, in the app's language.
#[test]
fn g36_the_page_is_told_the_line_and_whether_its_ghost_is_drawn() {
    let (mut app, _chat, guesser, ctx) = plate();
    assert!(page(&app).is_null(), "nothing during the pause");
    asked(&mut app, &ctx, &guesser);
    assert_eq!(page(&app), json!({"label": null, "text": "Next step: thinking…", "guess": false, "ready": false}));
    guesser.answer(HOLES);
    frame(&mut app, &ctx, 6.0, vec![]);
    let text = "Drill four 5 mm through holes near the corners of Plate · A plate is usually screwed down.";
    assert_eq!(page(&app), json!({"label": "Next step", "text": text, "guess": true, "ready": false, "id": "1"}));
    guesser.ready();
    frame(&mut app, &ctx, 6.1, vec![]);
    answer_commands(&mut app, &ctx, &guesser, 6.2, &holes());
    let drawn = page(&app);
    assert_eq!((drawn["ready"].clone(), drawn["label"].clone()), (json!(true), json!("Next step (Tab)")));
    assert!(drawn["text"].as_str().unwrap().contains("(guessed: 5, position)"), "{drawn}");
    {
        let _zh = crate::i18n::scope("zh");
        assert_eq!(page(&app)["label"], "下一步（Tab）");
    }
    // Esc and Tab in the page's empty box.
    page_act(&mut app, &ctx, 1, true);
    assert_eq!(outcome(&app, 0), "accepted");
    let (mut app, _chat, guesser, ctx) = plate();
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let was = before(&mut app);
    page_act(&mut app, &ctx, 1, true);
    assert_eq!(app.session.document_stamp(), was.stamp, "no ghost yet: nothing");
    assert!(app.agent_chat.predict.shown().is_some());
    page_act(&mut app, &ctx, 1, false);
    assert_eq!(outcome(&app, 0), "dismissed");
    assert_eq!(app.agent_chat.predict.dismissed.len(), 1);
}

/// G37: each reason for no suggestion is said for a while, once, and nothing is asked.
#[test]
fn g37_each_reason_is_said_once() {
    let cases: [(fn(&mut SolveApp), &str); 2] = [
        (|app| app.agent_chat.running_id = "opencode".into(), "Next step: not available with this agent yet"),
        (|app| app.agent_chat.predict.account = Account::Denied, "Next step: sign in to Intcad to get suggestions"),
    ];
    for (case, words) in cases {
        let (mut app, _chat, guesser, ctx) = plate();
        case(&mut app);
        frame(&mut app, &ctx, 0.0, vec![]);
        assert_eq!(chip(&app).unwrap().0, words);
        assert_eq!(page(&app)["text"], words);
        frame(&mut app, &ctx, 9.0, vec![]);
        assert!(chip(&app).is_some(), "{words}");
        for t in [11.0, 60.0, 600.0] {
            frame(&mut app, &ctx, t, vec![]);
            assert!(chip(&app).is_none() && page(&app).is_null(), "{words} at {t}");
        }
        assert!(guesser.prompts().is_empty());
    }
    // The agent's sign-in.
    let (mut app, _chat, guesser, ctx) = plate();
    asked(&mut app, &ctx, &guesser);
    guesser.say(Event::AuthRequired { methods: Vec::new() });
    frame(&mut app, &ctx, 6.0, vec![]);
    frame(&mut app, &ctx, 6.1, vec![]);
    assert_eq!(chip(&app).unwrap().0, "Next step paused: sign in to the agent");
    frame(&mut app, &ctx, 20.0, vec![]);
    assert!(chip(&app).is_none(), "{:?}", chip(&app));
}

/// G48: the rail's panel lists what became of each suggestion, newest first, and has the switch.
#[test]
fn g48_the_panel_lists_suggestions_and_has_the_switch() {
    let (mut app, _chat, guesser, ctx) = plate();
    assert!(panel::rows(&app).is_empty());
    ghost_drawn(&mut app, &ctx, &guesser);
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    app.agent_chat.predict.at_once = false;
    for (n, stage) in ["dismissed", "withdrawn"].into_iter().enumerate() {
        let t = 100.0 * (n as f64 + 1.0);
        guesser.answer("");
        edit(&mut app, 300.0 + 20.0 * n as f64);
        frame(&mut app, &ctx, t, vec![]);
        frame(&mut app, &ctx, t + 1.5, vec![]);
        guesser.ready();
        frame(&mut app, &ctx, t + 1.6, vec![]);
        guesser.answer(&HOLES.replace("5 mm", &format!("{stage} 6 mm")));
        frame(&mut app, &ctx, t + 1.7, vec![]);
        guesser.ready();
        frame(&mut app, &ctx, t + 1.8, vec![]);
        if stage == "dismissed" {
            frame(&mut app, &ctx, t + 1.9, vec![key(egui::Key::Escape)]);
        } else {
            guesser.answer("no");
            frame(&mut app, &ctx, t + 1.9, vec![]);
        }
    }
    let rows = panel::rows(&app);
    assert_eq!(rows.len(), 3, "{rows:?}");
    for (row, (step, what)) in rows.iter().zip([("withdrawn 6 mm", "Taken back"), ("dismissed 6 mm", "Turned down"), ("four 5 mm", "Accepted")]) {
        assert!(row.0.contains(step) && row.1.starts_with(what) && row.1.contains("just now"), "{row:?}");
    }
}

fn sheet() -> Session {
    let mut s = Session::default();
    for (c, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Base"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [100, 60]})),
        ("sketch.finish", json!({})),
        ("sheet.flange", json!({"sketch": "Base"})),
    ] {
        s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"));
    }
    s
}

/// G54: sheet-metal commands are refused while they are off, and draw and land while they are on.
#[test]
fn g54_sheet_metal_is_a_switch_in_the_code() {
    let flange = one("sheet.flange", json!({"edges": [[50, 0, 2.5]], "height": 20}));
    for on in [false, true] {
        let (mut app, chat, guesser, ctx) = setup(true);
        app.session = sheet();
        app.agent_chat.predict.sheet = on;
        line(&mut app, &ctx, &guesser, 0.0, r#"{"step": "Add a 20 mm flange on the front edge", "reason": "A lip."}"#);
        assert_eq!(guesser.prompts()[1].contains("sheet.flange"), on);
        let was = before(&mut app);
        answer_commands(&mut app, &ctx, &guesser, 2.0, &flange);
        if on {
            assert_eq!(status(&app)["state"], "ghost", "{}", status(&app));
            frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
            assert_eq!(outcome(&app, 0), "accepted");
        } else {
            withdrawn(&mut app, &chat, &was, "not allowed");
            assert_eq!(app.agent_chat.predict.log()[0].ran, 0);
        }
    }
    const { assert!(!ghost::SHEET_METAL, "off until it is measured") };
}

/// G55: every sheet-metal command on the list is one the engine takes, and draws a ghost.
#[test]
fn g55_every_listed_sheet_command_draws_a_ghost() {
    let run = |s: &mut Session, c: &str, p: Value| drop(s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}")));
    let flanged = || {
        let mut s = sheet();
        run(&mut s, "sheet.flange", json!({"edges": [[50, 0, 2.5]], "height": 20}));
        s
    };
    let unfolded = || {
        let mut s = flanged();
        run(&mut s, "sheet.unfold", json!({}));
        s
    };
    let calls: [(&str, Session, Value); 5] = [
        ("sheet.flange", sheet(), json!({"edges": [[50, 0, 2.5]], "height": 20})),
        ("sheet.hem", flanged(), json!({"edges": [[50, 0, 20]], "length": 8})),
        ("sheet.fold", sheet(), json!({"points": [[50, 0, 2.5], [50, 60, 2.5]]})),
        ("sheet.unfold", flanged(), json!({})),
        ("sheet.refold", unfolded(), json!({})),
    ];
    assert_eq!(calls.iter().map(|(c, ..)| *c).collect::<Vec<_>>(), ghost::sheet_commands());
    for (command, session, params) in calls {
        let (mut app, ..) = setup(true);
        app.session = session;
        let steps = ghost::allowed(vec![(command.to_string(), params)], true).unwrap();
        match ghost::start(&mut app, steps, || {}).poll(0.0) {
            Some(Ok(drawn)) => assert_eq!(drawn.steps.len(), 1),
            Some(Err(why)) => panic!("{command}: {why:?}"),
            None => panic!("{command}: not done"),
        }
    }
}

/// G56: asked only while the account allows it; no longer allowed once the ghost is drawn, Tab lands
/// nothing.
#[test]
fn g56_suggestions_follow_the_account() {
    for (account, asked_for) in [(Account::NotNeeded, 1), (Account::Allowed, 1), (Account::Denied, 0), (Account::Unknown, 0)] {
        let (mut app, _chat, guesser, ctx) = plate();
        app.agent_chat.predict.account = account;
        for t in [0.0, 5.0, 60.0] {
            frame(&mut app, &ctx, t, vec![]);
        }
        assert_eq!(guesser.prompts().len(), asked_for, "{account:?}");
    }
    let (mut app, _chat, guesser, ctx) = plate();
    app.agent_chat.predict.account = Account::Allowed;
    ghost_drawn(&mut app, &ctx, &guesser);
    let was = before(&mut app);
    app.agent_chat.predict.account = Account::Denied;
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    assert_eq!(app.session.document_stamp(), was.stamp);
    assert_eq!(outcome(&app, 0), "not authorised");
}

// ---- J. After the code review, and with one session for both questions (G57–G64) --------------------

/// G57: a ghost drawn for one component, or before a sketch was opened, does not land, and nothing
/// is asked again until an edit.
#[test]
fn g57_a_ghost_does_not_land_in_another_component_or_a_sketch() {
    let run = |app: &mut SolveApp, c: &str, p: Value| app.session.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"));
    for place in ["component", "sketch"] {
        let (mut app, chat, guesser, ctx) = plate();
        run(&mut app, "component.create", json!({"name": "Part", "activate": false}));
        run(&mut app, "sketch.create", json!({"plane": "XY", "name": "Outline"}));
        run(&mut app, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [10, 10]}));
        run(&mut app, "sketch.finish", json!({}));
        ghost_drawn(&mut app, &ctx, &guesser);
        let was = before(&mut app);
        match place {
            "component" => {
                run(&mut app, "component.activate", json!({"component": "Part"}));
                assert_ne!(app.session.active_component, 0);
            }
            _ => {
                run(&mut app, "sketch.edit", json!({"sketch": "Outline"}));
                assert!(app.session.active_sketch.is_some());
            }
        }
        assert_eq!(app.session.document_stamp(), was.stamp, "{place}: no edit");
        frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
        withdrawn(&mut app, &chat, &was, "stale");
        for t in [5.0, 60.0, 600.0] {
            frame(&mut app, &ctx, t, vec![]);
        }
        assert_eq!(guesser.firsts().len(), 1, "{place}: {:?}", guesser.calls());
        withdrawn(&mut app, &chat, &was, "stale");
    }
}

/// G58: with History rolled back a good step is drawn, even one like a step after the marker, and
/// the step just before the marker is a repeat.
#[test]
fn g58_with_history_rolled_back_the_step_before_the_marker_is_the_last_one() {
    let hole = |name: &str, x: f64| {
        one("solid.cylinder", json!({"radius": 2.5, "height": 8, "base": [x, 10, -1], "operation": "cut", "targets": ["Plate"], "name": name}))
    };
    let plate_again = one("solid.box", json!({"length": 100, "width": 60, "height": 6, "corner": [0, 0, 0], "operation": "new", "name": "Again"}));
    for (what, commands, expected) in [
        ("another", hole("Other", 50.0), "ghost"),
        ("the one after the marker", hole("Hole2", 10.0), "ghost"),
        ("the one before", plate_again, "repeat"),
    ] {
        let (mut app, chat, guesser, ctx) = plate();
        app.session.execute("solid.cylinder", &hole("Hole", 10.0)[0]["params"]).unwrap();
        app.session.execute("timeline.roll_to", &json!({"position": 1})).unwrap();
        assert_eq!((app.session.doc.marker, app.session.doc.features.len()), (Some(1), 2));
        line(&mut app, &ctx, &guesser, 0.0, HOLES);
        let was = before(&mut app);
        answer_commands(&mut app, &ctx, &guesser, 2.0, &commands);
        if expected == "ghost" {
            assert_eq!(status(&app)["state"], "ghost", "{what}: {}", status(&app));
        } else {
            withdrawn(&mut app, &chat, &was, expected);
        }
    }
}

/// G59: in the Render workspace Tab and Esc are its own; the ghost is still there after it.
#[test]
fn g59_tab_and_esc_are_the_render_workspaces_own() {
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    let was = before(&mut app);
    app.ui.tab = "RENDER".into();
    assert!(crate::render_workspace::active(&app));
    app.esc_handled = false;
    for (t, k) in [(3.0, egui::Key::Tab), (3.1, egui::Key::Escape)] {
        let raw = egui::RawInput { time: Some(t), events: vec![key(k)], ..Default::default() };
        let mut left = false;
        let _ = ctx.run_ui(raw, |ui| {
            tick(&mut app, ui.ctx());
            left = ui.ctx().input(|i| i.key_pressed(k));
        });
        assert!(left, "{k:?} is not used up");
        // The page's keys in its message box do nothing either.
        page_act(&mut app, &ctx, 1, k == egui::Key::Tab);
        assert!(!app.esc_handled && app.agent_chat.predict.ghost.is_some(), "{k:?}: {}", status(&app));
        assert_eq!(app.session.document_stamp(), was.stamp, "{k:?}: nothing landed");
    }
    app.ui.tab = "MODEL".into();
    frame(&mut app, &ctx, 4.0, vec![]);
    assert_eq!(status(&app)["state"], "ghost");
    assert!(app.agent_chat.predict.dismissed.is_empty());
    frame(&mut app, &ctx, 4.1, vec![release(egui::Key::Tab), key(egui::Key::Tab)]);
    assert_eq!(outcome(&app, 0), "accepted");
}

/// The guessing session is there again after a failure (its process cannot be started in these tests).
fn back(app: &mut SolveApp, guesser: &Fake) {
    app.agent_chat.predict.backend = Some(Box::new(guesser.clone()));
    app.agent_chat.predict.link = Link::Starting;
    guesser.ready();
}

/// G60: the agent ends while it is asked for the commands, also after Esc: the line goes, and the
/// first question is not asked (and paid for) again until an edit.
#[test]
fn g60_an_agent_that_dies_on_the_second_question_is_not_asked_the_first_again() {
    for esc in [false, true] {
        let (mut app, chat, guesser, ctx) = plate();
        line(&mut app, &ctx, &guesser, 0.0, HOLES);
        let was = before(&mut app);
        assert_eq!(guesser.prompts().len(), 2, "the second question is out");
        if esc {
            frame(&mut app, &ctx, 1.8, vec![key(egui::Key::Escape)]);
        }
        guesser.say(Event::Exited { tail: vec!["boom".into()] });
        frame(&mut app, &ctx, 2.0, vec![]);
        withdrawn(&mut app, &chat, &was, if esc { "dismissed" } else { "failed" });
        // Back each time its wait is over, for ten minutes.
        for t in (1..=60).map(|n| 2.0 + 10.0 * f64::from(n)) {
            frame(&mut app, &ctx, t, vec![]);
            if app.agent_chat.predict.backend.is_none() && t > 32.0 {
                back(&mut app, &guesser);
            }
        }
        assert_eq!(app.agent_chat.predict.link, Link::Fresh, "it is back");
        assert_eq!((guesser.prompts().len(), guesser.firsts().len()), (2, 1), "esc {esc}: {:?}", guesser.calls());
        edit(&mut app, 300.0);
        frame(&mut app, &ctx, 700.0, vec![]);
        frame(&mut app, &ctx, 701.5, vec![]);
        assert_eq!(guesser.firsts().len(), 2, "esc {esc}: after an edit");
    }
}

/// G61: a session the agent refuses to open: paused and tried again, nothing shown, the hour not
/// used. One opened ahead and refused leaves the ghost that is drawn, and Tab lands it.
#[test]
fn g61_a_session_that_cannot_be_opened_pauses_and_keeps_a_drawn_ghost() {
    let (mut app, _chat, guesser, ctx) = plate();
    // Not ready yet: the question waits for the session.
    let _ = app.agent_chat.predict.backend.as_mut().unwrap().poll();
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 1.5, vec![]);
    assert!(app.agent_chat.predict.ask.is_some() && guesser.prompts().is_empty());
    guesser.say(Event::Error("Invalid params".into()));
    for t in (0..=600).map(|n| 2.0 + f64::from(n)) {
        frame(&mut app, &ctx, t, vec![]);
        assert!(app.agent_chat.predict.shown().is_none(), "{t}");
    }
    // Tried at 32, 92, 212 and 452 s (it cannot be started here).
    assert_eq!((status(&app)["state"].clone(), app.agent_chat.predict.failures), (json!("paused"), 5));
    assert!(guesser.prompts().is_empty());
    assert_eq!(status(&app)["hourUsed"], 0);
    back(&mut app, &guesser);
    frame(&mut app, &ctx, 603.0, vec![]);
    frame(&mut app, &ctx, 603.1, vec![]);
    assert_eq!((guesser.prompts().len(), status(&app)["hourUsed"].clone()), (1, json!(1)), "asked once it is back");
    // The one opened ahead.
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    assert_eq!((sessions_opened(&guesser), &app.agent_chat.predict.link), (1, &Link::Starting));
    guesser.say(Event::Error("Invalid params".into()));
    for t in (0..=600).map(|n| 3.0 + f64::from(n)) {
        frame(&mut app, &ctx, t, vec![]);
        assert_eq!(status(&app)["state"], "ghost", "{t}");
    }
    assert!(matches!(app.agent_chat.predict.link, Link::Failed { .. }));
    assert_eq!(app.agent_chat.predict.failures, 5);
    assert_eq!(status(&app)["hourUsed"], 1);
    frame(&mut app, &ctx, 604.0, vec![key(egui::Key::Tab)]);
    assert_eq!(outcome(&app, 0), "accepted");
    assert_eq!(volumes(&app.session)[0].2, 10);
}

/// G62: a value that defines a parameter is not one of a listed command's.
#[test]
fn g62_a_value_that_adds_a_parameter_is_not_allowed() {
    let cube = one("solid.box", json!({"length": "w = 40", "width": 1, "height": 1, "corner": [200, 0, 0]}));
    let (mut app, chat, _guesser, _ctx, was) = answered(&cube);
    assert_eq!(app.agent_chat.predict.log().last().unwrap().ran, 1, "the engine takes it");
    withdrawn(&mut app, &chat, &was, "not allowed");
    assert!(app.session.doc.params.iter().all(|p| p.name != "w"));
}

/// What Claude Code's adapter says of itself, offering `session/close` too.
#[cfg(unix)]
const CLOSING_INIT: &str = r#"{"agentCapabilities":{"sessionCapabilities":{"close":{},"systemPrompt":{"mode":{"override":{}}}}},"agentInfo":{"name":"@agentclientprotocol/claude-agent-acp","title":"Claude Agent","version":"0.107.0"}}"#;

/// A stand-in agent as [`signed_in`]'s that also answers: a step to a first question, a hole to a
/// second. With `mid` it says it is signed in with that before its first answer ends.
#[cfg(unix)]
fn answering(name: &str, init: &str, kinds: &[&str], mid: &str) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("intcad-ghost-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = r#"seen="$0"; init="$1"; kinds="$2"; mid="$3"; step="$4"; commands="$5"; n=0; k=0
signed() {
  if [ -n "$1" ]; then printf '{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"%s","label":"test"}}}\n' "$1"; fi
}
say() { k=$((k+1)); signed "$(printf '%s' "$kinds" | cut -d'|' -f$k)"; }
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$seen"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) say; printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$init" ;;
    *'"method":"session/new"'*) n=$((n+1)); printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"s%s"}}\n' "$id" "$n"; say ;;
    *'"method":"session/close"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
    *'"method":"session/prompt"'*)
      sid=$(printf '%s' "$line" | sed -n 's/.*"sessionId":"\([^"]*\)".*/\1/p')
      case "$line" in *'Write the Intcad commands'*) text="$commands" ;; *) text="$step"; signed "$mid"; mid= ;; esac
      printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":%s}}}}\n' "$sid" "$text"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id" ;;
  esac
done"#;
    let seen = dir.join("seen").to_string_lossy().into_owned();
    let kinds = format!("{}|", kinds.join("|"));
    let hole = json!({"commands": one("solid.hole", json!({"position": [50, 30, 6], "diameter": 5}))}).to_string();
    let (step, hole) = (json!(HOLES).to_string(), json!(hole).to_string());
    let argv = ["sh", "-c", script, &seen, init, &kinds, mid, &step, &hole].map(str::to_string).to_vec();
    (dir, argv)
}

/// Frames at input time `t` until `done` (30 s at most): for what a real process answers.
#[cfg(unix)]
fn frames_until(app: &mut SolveApp, ctx: &egui::Context, t: f64, done: &dyn Fn(&SolveApp) -> bool) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !done(app) {
        assert!(std::time::Instant::now() < until, "timed out: {}", status(app));
        frame(app, ctx, t, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// An edit at `t`, the pause, and its suggestion drawn with the next one's session open (the `n`th
/// suggestion, with `sessions` asked for in all).
#[cfg(unix)]
fn suggestion(app: &mut SolveApp, ctx: &egui::Context, dir: &std::path::Path, t: f64, n: usize, sessions: usize) {
    edit(app, 200.0 + 20.0 * n as f64);
    frame(app, ctx, t, vec![]);
    frame(app, ctx, t + 1.5, vec![]);
    frames_until(app, ctx, t + 1.6, &|app| {
        let p = &app.agent_chat.predict;
        p.log.len() == n && p.ghost.is_some() && p.link == Link::Fresh && sent(dir, "session/new").len() == sessions
    });
}

#[cfg(unix)]
fn asked_in(dir: &std::path::Path) -> Vec<String> {
    sent(dir, "session/prompt").iter().map(|p| p["params"]["sessionId"].as_str().unwrap().to_string()).collect()
}

/// G63: the session of a suggestion is closed when the next one's is opened, where the agent offers
/// it; the chat's sessions are not.
#[cfg(unix)]
#[test]
fn g63_a_suggestions_session_is_closed_when_the_next_is_opened() {
    for (offered, init) in [(true, CLOSING_INIT), (false, CLAUDE_INIT)] {
        let (dir, argv) = answering(&format!("close-{offered}"), init, &["account"], "");
        let (mut app, _chat, _guesser, ctx) = plate();
        app.agent_chat.argv = argv;
        app.agent_chat.predict.stop();
        frames_until(&mut app, &ctx, 0.0, &|app| app.agent_chat.predict.link == Link::Fresh);
        suggestion(&mut app, &ctx, &dir, 10.0, 1, 2);
        suggestion(&mut app, &ctx, &dir, 20.0, 2, 3);
        assert_eq!(asked_in(&dir), ["s1", "s1", "s2", "s2"]);
        // What went over the wire, in order.
        let wire: Vec<String> = std::fs::read_to_string(dir.join("seen"))
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| match v["method"].as_str() {
                Some("session/new") => Some("new".to_string()),
                Some("session/close") => Some(format!("close {}", v["params"]["sessionId"].as_str().unwrap())),
                _ => None,
            })
            .collect();
        let expected: &[&str] = if offered { &["new", "close s1", "new", "close s2", "new"] } else { &["new", "new", "new"] };
        assert_eq!(wire, expected, "offered: {offered}");
        app.agent_chat.predict.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
    // The chat's New chat.
    let (dir, argv) = answering("close-chat", CLOSING_INIT, &["account"], "");
    let start = crate::agent_chat::backend::StartContext { cwd: dir.clone(), bridge: None, tools: None, wake: Box::new(|| {}) };
    let mut b = crate::agent_chat::acp_client::AcpBackend::start(&argv, start).unwrap();
    let mut ready = false;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !ready {
        assert!(std::time::Instant::now() < until);
        ready = b.poll().iter().any(|e| matches!(e, Event::Ready { .. }));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    ChatBackend::new_session(&mut b);
    poll_until(&mut b, &dir, &|d| sent(d, "session/new").len() == 2);
    assert!(sent(&dir, "session/close").is_empty());
    b.shutdown(true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// G64: a session opened ahead is used only on the sign-in and for the model preference it was
/// opened for; one opened lean on an API key is asked nothing on a plan, the second question either.
#[cfg(unix)]
#[test]
fn g64_a_session_opened_ahead_is_replaced_when_the_sign_in_changed() {
    let news = |dir: &std::path::Path| sent(dir, "session/new").iter().map(|s| lean(&s["params"])).collect::<Vec<_>>();
    // `account` once the second session is open.
    let (dir, argv) = answering("ahead", CLAUDE_INIT, &["api_key", "api_key", "account"], "");
    let (mut app, _chat, _guesser, ctx) = plate();
    app.agent_chat.argv = argv;
    app.agent_chat.predict.stop();
    frames_until(&mut app, &ctx, 0.0, &|app| app.agent_chat.predict.link == Link::Fresh);
    suggestion(&mut app, &ctx, &dir, 10.0, 1, 2);
    frames_until(&mut app, &ctx, 12.0, &|app| !app.agent_chat.predict.backend.as_ref().unwrap().key_sign_in());
    assert_eq!(news(&dir), [true, true], "both lean");
    suggestion(&mut app, &ctx, &dir, 20.0, 2, 4);
    assert_eq!(news(&dir), [true, true, false, false]);
    assert_eq!(asked_in(&dir), ["s1", "s1", "s3", "s3"], "nothing is asked in the lean one opened ahead");
    app.agent_chat.predict.stop();
    let _ = std::fs::remove_dir_all(&dir);
    // `account` between the two questions.
    let (dir, argv) = answering("between", CLAUDE_INIT, &["api_key"], "account");
    let (mut app, _chat, _guesser, ctx) = plate();
    app.agent_chat.argv = argv;
    app.agent_chat.predict.stop();
    frames_until(&mut app, &ctx, 0.0, &|app| app.agent_chat.predict.link == Link::Fresh);
    suggestion(&mut app, &ctx, &dir, 10.0, 1, 3);
    assert_eq!(news(&dir), [true, false, false]);
    assert_eq!(asked_in(&dir), ["s1", "s2"], "the second question in a full session of its own");
    assert_eq!(app.agent_chat.predict.lines, 1, "asked once");
    app.agent_chat.predict.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// G64: and when the model preference changed after it was opened.
#[test]
fn g64_a_session_opened_ahead_is_replaced_when_the_model_preference_changed() {
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    let options = mastra_options("build", SONNET, "low");
    let opens = |app: &mut SolveApp, t: f64, model: &str| {
        let before = configs(&guesser).len();
        guesser.say(Event::ConfigOptions(options.clone()));
        guesser.ready();
        frame(app, &ctx, t, vec![]);
        answer_settings(app, &ctx, &guesser, options.clone(), before, t);
        assert!(configs(&guesser)[before..].contains(&format!("config: model={model}")), "{:?}", guesser.calls());
    };
    opens(&mut app, 3.0, HAIKU);
    assert_eq!(app.agent_chat.predict.link, Link::Fresh, "opened ahead");
    app.preferences.agent_next_step_model = MINI.into();
    edit(&mut app, 300.0);
    frame(&mut app, &ctx, 20.0, vec![]);
    frame(&mut app, &ctx, 21.5, vec![]);
    assert_eq!((guesser.prompts().len(), sessions_opened(&guesser)), (2, 2), "a new session first");
    opens(&mut app, 22.0, MINI);
    frame(&mut app, &ctx, 30.0, vec![]);
    assert_eq!(guesser.firsts().len(), 2);
    let calls = guesser.calls();
    let at = |what: &str| calls.iter().rposition(|c| c.starts_with(what)).unwrap();
    assert!(at("new_session") < at(&format!("config: model={MINI}")) && at(&format!("config: model={MINI}")) < at("prompt: "));
    // Unchanged, the session opened ahead is used.
    let (mut app, _chat, guesser, ctx) = plate();
    ghost_drawn(&mut app, &ctx, &guesser);
    guesser.ready();
    frame(&mut app, &ctx, 3.0, vec![]);
    edit(&mut app, 300.0);
    frame(&mut app, &ctx, 20.0, vec![]);
    frame(&mut app, &ctx, 21.5, vec![]);
    assert_eq!((guesser.firsts().len(), sessions_opened(&guesser)), (2, 1));
}

// ---- The words and the ghost -----------------------------------------------------------------------

/// A hole of radius 2.5 cut by a cylinder, as the measured plate's second state is answered.
fn bored() -> Value {
    one("solid.cylinder", json!({"radius": 2.5, "height": 8, "base": [90, 50, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole2"}))
}

fn hole_of(diameter: f64, at: [f64; 2]) -> Value {
    one("solid.hole", json!({"position": [at[0], at[1], 6], "diameter": diameter, "name": "Hole2"}))
}

type Start = (SolveApp, Fake, Fake, egui::Context);

/// A suggestion on this design in these words, answered with these commands.
fn worded((mut app, chat, guesser, ctx): Start, step: &str, commands: &Value) -> (SolveApp, Fake, Fake, egui::Context, Before) {
    line(&mut app, &ctx, &guesser, 0.0, &json!({"step": step, "reason": ""}).to_string());
    let was = before(&mut app);
    answer_commands(&mut app, &ctx, &guesser, 2.0, commands);
    (app, chat, guesser, ctx, was)
}

/// Its ghost is drawn.
fn drawn(start: Start, step: &str, commands: &Value) -> SolveApp {
    let (app, ..) = worded(start, step, commands);
    assert_eq!(status(&app)["state"], "ghost", "{step} {commands}: {}", status(&app));
    app
}

/// The suggestion is withdrawn with this outcome before anything runs, for good.
fn not_run(start: Start, step: &str, commands: &Value, what: &str) {
    let (mut app, chat, guesser, ctx, was) = worded(start, step, commands);
    for t in [3.0, 10.0, 60.0, 600.0] {
        frame(&mut app, &ctx, t, vec![]);
        assert!(app.agent_chat.predict.ghost.is_none() && ghost::overlay(&mut app).is_none(), "{step} {commands} at {t}");
    }
    assert_eq!(app.agent_chat.predict.log().last().unwrap().outcome, what, "{step} {commands}");
    withdrawn(&mut app, &chat, &was, what);
    let p = &app.agent_chat.predict;
    assert_eq!(p.log().last().unwrap().ran, 0, "{step}: no command ran on the copy");
    assert!(p.dismissed.is_empty(), "{step}: not turned down");
    assert_eq!(guesser.prompts().len(), 2, "{step}: no new question without an edit");
}

/// The words and the ghost agree: it is drawn.
fn agrees(step: &str, commands: &Value) {
    drawn(plate(), step, commands);
}

/// They disagree: nothing runs.
fn disagrees(step: &str, commands: &Value) {
    not_run(plate(), step, commands, "words differ");
}

/// G66: a hole's size is its diameter in both questions, however the hole was cut.
#[test]
fn g66_the_questions_give_a_hole_its_diameter() {
    let (mut app, _chat, guesser, ctx) = plate();
    let cut = |at: f64, size: Value, name: &str| {
        let mut p = json!({"height": 8, "base": [at, 10, -1], "operation": "cut", "targets": ["Plate"], "name": name});
        p.as_object_mut().unwrap().extend(size.as_object().unwrap().clone());
        ("solid.cylinder", p)
    };
    let edges = json!([[0, 0, 3], [100, 0, 3], [100, 60, 3], [0, 60, 3]]);
    for (command, params) in [
        ("parameters.add", json!({"name": "w", "expression": "6 mm"})),
        ("solid.fillet", json!({"edges": edges, "radius": 5, "body": "Plate", "name": "Corners"})),
        cut(10.0, json!({"radius": 2.5}), "ByRadius"),
        cut(30.0, json!({"radius": "w/2"}), "ByParameter"),
        cut(50.0, json!({"diameter": 7}), "ByDiameter"),
        ("solid.hole", json!({"position": [70, 30, 6], "diameter": 6.6, "name": "Drilled"})),
        ("solid.cylinder", json!({"radius": 4, "height": 10, "base": [200, 0, 0], "operation": "new", "name": "Pin"})),
    ] {
        app.session.execute(command, &params).unwrap_or_else(|e| panic!("{command} {params}: {e}"));
    }
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let prompts = guesser.prompts();
    assert_eq!(prompts.len(), 2);
    for q in &prompts {
        let of = |name: &str| q.lines().find(|l| l.contains(&format!(". {name} ("))).unwrap_or_else(|| panic!("{name}: {q}")).to_string();
        for (name, size) in [("ByRadius", "diameter 5,"), ("ByParameter", "diameter 6,"), ("ByDiameter", "diameter 7,"), ("Drilled", "diameter 6.6,")]
        {
            let hole = of(name);
            assert!(hole.contains(size) && !hole.contains("radius"), "{hole}");
        }
        assert!(of("Pin").contains("radius 4,"), "{q}");
        assert!(of("Corners").contains("radius 5,"), "{q}");
    }
    assert!(prompts[0].contains("hole diameter: 5, 6, 7"), "{}", prompts[0]);
    assert_eq!(prompts[0].matches("A hole's size is its diameter").count(), 1, "{}", prompts[0]);
    // Counted with the holes, whichever way each was made; a cylinder that does not cut is none.
    let (mut app, _chat, _guesser, _ctx) = plate();
    for (command, params) in [
        ("solid.hole", json!({"position": [70, 30, 6], "diameter": "6.6 mm"})),
        cut(10.0, json!({"radius": 2.5}), "A"),
        cut(30.0, json!({"radius": 3.3}), "B"),
        ("solid.cylinder", json!({"radius": 4, "height": 10, "base": [200, 0, 0], "operation": "new", "name": "Pin"})),
    ] {
        app.session.execute(command, &params).unwrap();
    }
    let q = prompt(&mut app);
    assert!(q.contains("hole diameter: 6.6 (×2), 5\n"), "{q}");
}

/// G67: words that agree with the ghost's holes, and steps that make none, are drawn.
#[test]
fn g67_words_that_agree_are_drawn() {
    for commands in [bored(), hole_of(5.0, [90.0, 50.0])] {
        for step in [
            "Add a second 5 mm hole",
            "Add a second 5 mm diameter hole",
            "Add a second 2.5 mm radius hole",
            "Add a second hole of radius 2.5 mm",
            "Add a second R2.5 hole",
            "Add a second ⌀5 hole",
        ] {
            agrees(step, &commands);
        }
    }
    let four: Vec<Value> = [(10, 10), (90, 10), (90, 50), (10, 50)]
        .iter()
        .map(|(x, y)| json!({"command": "solid.hole", "params": {"position": [x, y, 6], "diameter": 12}}))
        .collect();
    agrees("Cut a 6 mm radius through-hole near each corner of Base.", &json!(four));
    agrees("Drill a 20 mm hole 12 mm deep", &one("solid.hole", json!({"position": [50, 30, 6], "diameter": 20, "depth": 12})));
    agrees("Drill a hole 12 mm deep", &one("solid.hole", json!({"position": [50, 30, 6], "diameter": 20, "depth": 12})));
    // Half the hole's size, but its depth.
    agrees("Drill a hole 10 mm deep", &one("solid.hole", json!({"position": [50, 30, 6], "diameter": 20, "depth": 10})));
    agrees("Add a hole 10 mm from each edge", &hole_of(6.0, [10.0, 10.0]));
    // A length said as neither radius nor diameter, more than 5% from the hole's size, twice it and half it,
    // is not about the hole.
    agrees("Add a hole 10 mm from each edge", &hole_of(6.0, [90.0, 50.0]));
    agrees("Drill a through hole in the 6 mm plate", &bored());
    for step in ["Add a 0.25 in hole", "Add a .25 in hole", "Add a 5.3 mm hole", "Add a 10.6 mm hole", "Add a 2.65 mm hole"] {
        agrees(step, &hole_of(5.0, [50.0, 30.0]));
    }
    drawn(inch_plate(), "Add a 0.05 ft hole", &one("solid.hole", json!({"position": [20, 40, 6.35], "diameter": "0.25 in"})));
    agrees("Add a hole 2.5 mm in radius", &bored());
    agrees("Add a hole 5 mm in diameter", &bored());
    let wide = json!({"position": [50, 30, 6], "diameter": 5, "type": "counterbore", "cb_diameter": 9, "cb_depth": 3});
    agrees("Add a 5 mm hole with a 9 mm diameter counterbore", &one("solid.hole", wide.clone()));
    agrees("Add a hole with a 9 mm diameter counterbore", &one("solid.hole", wide));
    agrees("Add four corner holes", &holes());
    agrees("Add a 0.25 in hole", &hole_of(6.35, [50.0, 30.0]));
    agrees("Fillet the top edges with 5 mm", &one("solid.fillet", json!({"edges": [[50, 0, 6]], "radius": 5, "body": "Plate"})));
    // Only holes are checked: a boss's size is not, right or wrong.
    for radius in [10, 5] {
        let boss = json!({"radius": radius, "height": 10, "base": [50, 30, 6], "operation": "join", "targets": ["Plate"]});
        agrees("Add a 10 mm radius boss", &one("solid.cylinder", boss));
    }
    // A pattern of a hole has no size of its own.
    let (mut app, _chat, guesser, ctx) = setup(true);
    for (command, params) in [
        ("solid.box", json!({"length": 100, "width": 60, "height": 6, "corner": [-50, -30, 0], "body_name": "Plate"})),
        ("solid.hole", json!({"position": [20, 0, 6], "diameter": 6.6, "name": "BoltHole"})),
    ] {
        app.session.execute(command, &params).unwrap();
    }
    line(&mut app, &ctx, &guesser, 0.0, r#"{"step": "Pattern the 6.6 mm bolt holes", "reason": ""}"#);
    answer_commands(&mut app, &ctx, &guesser, 2.0, &one("solid.pattern.circular", json!({"features": ["BoltHole"], "axis": "Z", "count": 4})));
    assert_eq!(status(&app)["state"], "ghost", "{}", status(&app));
}

/// G68: words that give a hole another size than the ghost would draw.
#[test]
fn g68_words_that_disagree_are_withdrawn() {
    disagrees("Add a second 5 mm radius hole at the opposite corner, (90, 50).", &bored());
    disagrees("Add a second 2.5 mm hole at the opposite corner, (90, 50).", &bored());
    disagrees("Add a 4 mm radius hole", &hole_of(6.0, [50.0, 30.0]));
    disagrees("Add a hole of radius 5 mm", &hole_of(5.0, [50.0, 30.0]));
    // 6 is also where the hole starts, but the words say it of the diameter.
    disagrees("Add a 6 mm diameter hole", &hole_of(12.0, [50.0, 30.0]));
    disagrees("Add a hole 6 mm in diameter", &hole_of(12.0, [50.0, 30.0]));
    // A size after a sign and with no unit is compared as a length.
    disagrees("Add a second R5 hole", &bored());
    disagrees("Add a second ⌀2.5 hole", &bored());
    // A place or a counterbore that happens to be the hole's size does not speak for it.
    disagrees("Add a 5 mm radius hole 5 mm from each edge", &hole_of(5.0, [5.0, 5.0]));
    disagrees("Add a 2.5 mm hole 5 mm from each edge", &hole_of(5.0, [5.0, 5.0]));
    let wide = json!({"position": [50, 30, 6], "diameter": 5, "type": "counterbore", "cb_diameter": 9, "cb_depth": 3});
    disagrees("Add a 2.5 mm hole with a 9 mm diameter counterbore", &one("solid.hole", wide));
    // However the command is written.
    for size in [json!({"radius": null, "diameter": 12}), json!({"radius": "", "diameter": "12 mm"}), json!({"radius": 6, "diameter": 6})] {
        let mut p = json!({"height": 8, "base": [50, 30, -1], "operation": "subtract", "targets": ["Plate"]});
        p.as_object_mut().unwrap().extend(size.as_object().unwrap().clone());
        disagrees("Add a 6 mm diameter hole", &one("solid.cylinder", p));
    }
    // Within 5% of the hole's size, of twice it or of half it, a length is said of the hole.
    for step in ["Add a 5.2 mm hole", "Add a 10.4 mm hole", "Add a 2.6 mm hole", "Add a 0.2 in hole", "Add a .2 in hole"] {
        disagrees(step, &hole_of(5.0, [50.0, 30.0]));
    }
    // A diameter said is the hole's, however far from its size.
    disagrees("Add a 9 mm diameter hole", &hole_of(5.0, [50.0, 30.0]));
    // A place in the words does not stop a wrong size being seen.
    for step in ["Add a 3 mm hole 10 mm from each edge", "Add a 12 mm hole 10 mm from each edge", "Add a hole of radius 4 mm 10 mm from each edge"] {
        disagrees(step, &hole_of(6.0, [90.0, 50.0]));
    }
}

/// G69: in the app's language, by the app's own word for radius.
#[test]
fn g69_the_words_are_read_in_the_apps_language() {
    {
        let _zh = crate::i18n::scope("zh");
        disagrees("在对角打一个半径 5 mm 的孔", &bored());
        agrees("在对角打一个直径 5 mm 的孔", &bored());
    }
    {
        // With the ending the word has in a sentence.
        let _ru = crate::i18n::scope("ru");
        agrees("Добавить отверстие радиусом 2,5 мм", &bored());
        disagrees("Добавить отверстие радиусом 5 мм", &bored());
    }
    let _de = crate::i18n::scope("de");
    agrees("Eine Bohrung mit Radius 2,5 mm", &bored());
    disagrees("Eine Bohrung mit Radius 5 mm", &bored());
    disagrees("Eine Bohrung mit 2,5 mm Durchmesser", &bored());
}

/// G70: Tab before the answer lands nothing, then or later.
#[test]
fn g70_tab_before_words_that_disagree_lands_nothing() {
    let (mut app, chat, guesser, ctx) = plate();
    let step = json!({"step": "Add a second 5 mm radius hole at the opposite corner, (90, 50).", "reason": ""}).to_string();
    line(&mut app, &ctx, &guesser, 0.0, &step);
    let was = before(&mut app);
    frame(&mut app, &ctx, 1.8, vec![key(egui::Key::Tab)]);
    assert!(app.agent_chat.predict.shown().is_some(), "the line stays: {}", status(&app));
    assert!(ctx.memory(|m| m.focused().is_none()), "Tab is used up");
    answer_commands(&mut app, &ctx, &guesser, 2.0, &bored());
    withdrawn(&mut app, &chat, &was, "words differ");
    frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
    withdrawn(&mut app, &chat, &was, "words differ");
}

// ---- Designs not in millimetres --------------------------------------------------------------------

/// A 4 × 2.5 × 0.25 in plate with 0.2 in corners, a 0.25 in hole that a cylinder cut and one of 17/64.
fn inch_plate() -> Start {
    let mut start = setup(true);
    for (command, params) in [
        ("document.units", json!({"units": "in"})),
        ("solid.box", json!({"length": "4 in", "width": "2.5 in", "height": "0.25 in", "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"})),
        (
            "solid.fillet",
            json!({"edges": [[0, 0, 3.175], [101.6, 0, 3.175], [101.6, 63.5, 3.175], [0, 63.5, 3.175]], "radius": "0.2 in", "body": "Plate", "name": "Corners"}),
        ),
        (
            "solid.cylinder",
            json!({"radius": "0.125 in", "height": "0.5 in", "base": [10, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"}),
        ),
        ("solid.hole", json!({"position": [50, 30, 6.35], "diameter": "17/64 in", "name": "Drilled"})),
    ] {
        start.0.session.execute(command, &params).unwrap_or_else(|e| panic!("{command} {params}: {e}"));
    }
    start
}

/// The measured plate: corners of 5, a hole that a cylinder of radius 2.5 cut and one of 6.6.
fn mm_plate() -> Start {
    let mut start = plate();
    for (command, params) in [
        ("solid.fillet", json!({"edges": [[0, 0, 3], [100, 0, 3], [100, 60, 3], [0, 60, 3]], "radius": 5, "body": "Plate", "name": "Corners"})),
        ("solid.cylinder", json!({"radius": 2.5, "height": 8, "base": [10, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"})),
        ("solid.hole", json!({"position": [50, 30, 6], "diameter": 6.6, "name": "Drilled"})),
    ] {
        start.0.session.execute(command, &params).unwrap();
    }
    start
}

/// Both questions, as they are sent for the first step's words.
fn both_questions((mut app, _chat, guesser, ctx): Start) -> [String; 2] {
    line(&mut app, &ctx, &guesser, 0.0, r#"{"step": "Do the next step", "reason": ""}"#);
    guesser.prompts().try_into().unwrap()
}

/// What the questions say before the design's own geometry.
fn fixed(q: &str) -> &str {
    q.split("\n\nGeometry now (mm):").next().unwrap()
}

/// G71: in a design in mm the questions are what they were.
#[test]
fn g71_in_millimetres_the_questions_are_as_before() {
    let [step, commands] = both_questions(mm_plate());
    let history = "\
\nDesign: units mm, 4 History steps.
History (oldest first, the last 4):
1. Plate (BoxFeature): new body, height 6, length 100, width 60, corner (0, 0, 0) -> bodies: Plate
2. Corners (FilletFeature): radius 5, 4 edges
3. Hole (CylinderFeature): cut from Plate, diameter 5, height 8, base (10, 10, -1)
4. Drilled (HoleFeature): diameter 6.6, position (50, 30, 6)";
    assert_eq!(
        step,
        format!(
            "\
[Intcad next-step suggestion. Do not call any tools, do not read or change files and do not change the design. Answer with one line of JSON only.]
The user is modelling a part in Intcad and has paused after an edit. Guess the one modelling step they are most likely to do next, \
written as a short request they could send you, with sizes taken from the design's usual values where they fit.
Reply with exactly: {{\"step\": \"...\", \"reason\": \"...\"}}
- step: one imperative sentence, at most 12 words (for example \"Fillet the 4 top edges of Body1 with 2 mm\").
- reason: one short sentence on why, at most 12 words.
Name things the way the user sees them: bodies, features and sizes. A hole's size is its diameter. Never mention internal face or edge names such as F1:-y.
Write the step and the reason in English, the language of the app. If nothing sensible comes to mind, reply {{\"step\": \"\", \"reason\": \"...\"}}.
{history}
Last edits (newest last): Box; Fillet; Cylinder; Hole
Usual values in this design: fillet radius: 5; hole diameter: 5, 6.6

[Nothing is selected.]"
        )
    );
    let listed = "\
[Intcad next-step suggestion. Do not call any tools, do not read or change files and do not change the design. Answer with one line of JSON only.]
Write the Intcad commands that do exactly this step, as many as it takes, in order, with numbers from the geometry below. \
Choose sensible positions where the step leaves them open.
Step: Do the next step
Reply with exactly: {\"commands\": [{\"command\": \"...\", \"params\": {}}]}, or {\"commands\": []} if the step cannot be done with these commands.
Commands (points are [x,y,z] in the world frame; give every feature a \"name\"):
- solid.box {length, width, height, corner|center, operation: new|join|cut, targets: [bodies], body_name}
- solid.cylinder {radius|diameter, height, base: centre of one end, axis: \"X\"|\"Y\"|\"Z\", operation: new|join|cut, targets: [bodies], body_name}
- solid.extrude {sketch, distance, operation: new|join|cut, targets: [bodies]} (a finished sketch of the History)
- solid.fillet {edges: [[x,y,z], ...] a point on each edge (its mid), radius, body}; one pick takes the whole tangent chain
- solid.chamfer {edges: [[x,y,z], ...], distance, body}
- solid.shell {faces: [[x,y,z]] a point on each face to remove, thickness, body}
- solid.hole {position: [x,y,z] on a face, diameter, depth (default through all), type: simple|counterbore|countersink (cb_diameter, cb_depth | cs_diameter, cs_angle)}
- solid.pattern.rectangular {features: [names], dir1: [x,y,z], count1, spacing1, dir2, count2, spacing2}
- solid.pattern.circular {features: [names], axis: \"Z\", count}
- solid.mirror {features: [names] | bodies: [names], plane: \"XY\"|\"XZ\"|\"YZ\"|{origin, normal}}
- solid.combine {target, tools: [bodies], operation: join|cut|intersect}
";
    assert_eq!(fixed(&commands), format!("{listed}{history}"));
    let geometry = commands.split("\n\nGeometry now (mm):\nBody Plate:\n").nth(1).unwrap();
    for l in ["- edge mid [50, 0, 6] length 90", "- edge mid [0, 5, 3] length 6"] {
        assert!(geometry.lines().any(|g| g == l), "{l}: {geometry}");
    }
    assert!(geometry.lines().any(|l| l.starts_with("- face cylinder radius 2.5 axis [0, 0, 1] at [")), "{geometry}");
    assert!(geometry.lines().any(|l| l.starts_with("- face plane normal [0, 0, 1] at [") && l.ends_with(", 6]")), "{geometry}");
    assert!(geometry.lines().all(|l| l.starts_with("- face ") || l.starts_with("- edge mid [")), "{geometry}");
}

/// G71: in another unit they say which.
#[test]
fn g71_the_questions_say_the_units() {
    let [step, commands] = both_questions(inch_plate());
    assert!(step.contains("Body1 with 0.08 in\")") && step.contains("Give sizes in inches (in)."), "{step}");
    assert_eq!(commands.matches("Points are in mm").count(), 1, "{commands}");
    assert_eq!(commands.matches("every size as text with its unit, for example \"0.25 in\"").count(), 1, "{commands}");
    assert!(commands.contains("\n\nGeometry now (mm):\nBody Plate:"), "{commands}");
    for q in [&step, &commands] {
        assert!(!q.contains(" 2 mm"), "{q}");
        for place in ["corner (0, 0, 0) mm", "base (10, 10, -1) mm", "position (50, 30, 6.35) mm"] {
            assert!(q.contains(place), "{place}: {q}");
        }
        assert!(q.contains("diameter 0.25 in, height 0.5 in") && q.contains("diameter 0.265625 in") && q.contains("radius 0.2 in"), "{q}");
    }
    let (mut app, chat, guesser, ctx) = setup(true);
    for (command, params) in [
        ("document.units", json!({"units": "cm"})),
        ("solid.box", json!({"length": "10 cm", "width": "6 cm", "height": "1 cm", "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"})),
    ] {
        app.session.execute(command, &params).unwrap();
    }
    let [step, commands] = both_questions((app, chat, guesser, ctx));
    assert!(step.contains("Body1 with 0.2 cm\")") && step.contains("Give sizes in centimetres (cm)."), "{step}");
    assert!(commands.contains("for example \"0.5 cm\"") && commands.contains("corner (0, 0, 0) mm"), "{commands}");
    assert!(!step.contains("2 mm\"") && !step.contains(" in\"") && !commands.contains(" in\""), "{step}{commands}");
}

/// The sizes the questions give in History and the usual values, as the engine reads them.
fn told_sizes(app: &SolveApp, q: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for l in q.lines().filter(|l| l.contains("Feature)") || l.starts_with("Usual values")) {
        for (key, rest) in [
            "height ",
            "length ",
            "width ",
            "radius ",
            "diameter ",
            "distance ",
            "thickness ",
            "depth ",
            "spacing ",
            "radius: ",
            "diameter: ",
            "distance: ",
            "thickness: ",
        ]
        .iter()
        .flat_map(|k| l.match_indices(k).map(move |(i, k)| (k, &l[i + k.len()..])))
        {
            for value in if key.ends_with(": ") {
                rest.split(';').next().unwrap().split(", ").collect::<Vec<_>>()
            } else {
                vec![rest.split([',', ';']).next().unwrap()]
            } {
                let value = value.split(" (×").next().unwrap().split(" ->").next().unwrap().trim();
                // Text with a unit, as the engine reads it in this design.
                let unit = value.ends_with(" mm") || value.ends_with(" in");
                out.push((format!("{key}{value} {unit}"), app.session.doc.eval(value, solidcraft_engine::doc::expr::Kind::Length).unwrap()));
            }
        }
    }
    out
}

/// G72: a design made in mm and switched to inches is described with the lengths it has.
#[test]
fn g72_a_design_switched_to_inches_keeps_its_lengths() {
    let told = |q: &str| q.lines().filter(|l| l.contains("Feature)") || l.starts_with("Usual values")).collect::<Vec<_>>().join("\n");
    let (mut app, ..) = mm_plate();
    for (command, params) in [
        ("solid.cylinder", json!({"diameter": 7, "height": 8, "base": [80, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "ByDiameter"})),
        ("solid.chamfer", json!({"edges": [[50, 0, 6]], "distance": 0.5, "body": "Plate", "name": "Edge"})),
        ("solid.hole", json!({"position": [20, 40, 6], "diameter": 4, "depth": 3, "name": "Blind"})),
        ("solid.box", json!({"length": 30, "width": 20, "height": 10, "corner": [200, 0, 0], "body_name": "Box", "name": "Box"})),
        ("solid.shell", json!({"faces": [[215, 10, 10]], "thickness": 1.5, "body": "Box", "name": "Hollow"})),
    ] {
        app.session.execute(command, &params).unwrap_or_else(|e| panic!("{command}: {e}"));
    }
    let was = prompt(&mut app);
    let lengths: Vec<f64> = told_sizes(&app, &was).into_iter().map(|s| s.1).collect();
    for mm in [5.0, 6.6, 7.0, 0.5, 4.0, 3.0, 30.0, 1.5] {
        assert!(lengths.contains(&mm), "{mm}: {was}");
    }
    app.session.execute("document.units", &json!({"units": "in"})).unwrap();
    let q = prompt(&mut app);
    let now = told_sizes(&app, &q);
    assert_eq!(now.len(), lengths.len(), "{q}");
    for ((text, mm), was) in now.iter().zip(&lengths) {
        assert!(text.ends_with(" true"), "no unit: {text}: {q}");
        assert!((mm - was).abs() <= 1e-6 * was, "{text} is {mm}, was {was}");
    }
    app.session.execute("document.units", &json!({"units": "mm"})).unwrap();
    assert_eq!(told(&prompt(&mut app)), told(&was));
    assert!(q.contains("diameter 5 mm") && q.contains("diameter 7 mm") && q.contains("thickness 1.5 mm") && q.contains("distance 0.5 mm"), "{q}");
    // What is worked out of numbers without a unit is in the design's.
    let (mut app, ..) = inch_plate();
    let p = json!({"edges": [[0, 31.75, 6.35]], "radius": "1/16", "body": "Plate", "name": "Worked"});
    app.session.execute("solid.fillet", &p).unwrap();
    let q = prompt(&mut app);
    assert!(q.contains("Worked (FilletFeature): radius 0.0625 in"), "{q}");
    assert!(app.session.doc.length_inputs().iter().flatten().any(|v: &f64| (v - 1.5875).abs() < 1e-9), "as the geometry reads it");
}

/// The listed commands that take a size, on a block with a hole and a drawn outline, and their sizes
/// (here in mm).
fn sized(session: &Session) -> Vec<(&'static str, Value, &'static [&'static str])> {
    let sketch = session.doc.features.iter().find(|f| f.name == "Outline").expect("the sketch").id;
    vec![
        ("solid.box", json!({"length": 5, "width": 6, "height": 7, "corner": [200, 0, 0], "operation": "new"}), &["length", "width", "height"]),
        ("solid.cylinder", json!({"radius": 3, "height": 40, "base": [70, 30, -1], "operation": "cut", "targets": ["Plate"]}), &["radius", "height"]),
        ("solid.cylinder", json!({"diameter": 6, "height": 40, "base": [70, 30, -1], "operation": "cut", "targets": ["Plate"]}), &["diameter"]),
        ("solid.extrude", json!({"sketch": sketch, "distance": 5, "operation": "new"}), &["distance"]),
        ("solid.fillet", json!({"edges": [[0, 0, 10]], "radius": 3, "body": "Plate"}), &["radius"]),
        ("solid.chamfer", json!({"edges": [[100, 60, 10]], "distance": 2, "body": "Plate"}), &["distance"]),
        ("solid.shell", json!({"faces": [[50, 30, 0]], "thickness": 2, "body": "Plate"}), &["thickness"]),
        ("solid.hole", json!({"position": [80, 40, 20], "diameter": 6, "depth": 9}), &["diameter", "depth"]),
        (
            "solid.hole",
            json!({"position": [80, 40, 20], "diameter": 6, "type": "counterbore", "cb_diameter": 11, "cb_depth": 4}),
            &["cb_diameter", "cb_depth"],
        ),
        ("solid.hole", json!({"position": [80, 40, 20], "diameter": 6, "type": "countersink", "cs_diameter": 11, "cs_angle": 82}), &["cs_diameter"]),
        (
            "solid.pattern.rectangular",
            json!({"features": ["Hole"], "dir1": [1, 0, 0], "count1": 2, "spacing1": 60, "dir2": [0, 1, 0], "count2": 2, "spacing2": 20}),
            &["spacing1", "spacing2"],
        ),
    ]
}

/// Every size of `sized`: what the app may be given as one.
const ALL_SIZES: [&str; 13] = [
    "length",
    "width",
    "height",
    "radius",
    "diameter",
    "distance",
    "thickness",
    "depth",
    "cb_diameter",
    "cb_depth",
    "cs_diameter",
    "spacing1",
    "spacing2",
];

/// An app on that block, the same one in any unit.
fn block(units: &str) -> Start {
    let mut start = setup(true);
    for (command, params) in [
        ("document.units", json!({"units": units})),
        ("solid.box", json!({"length": "100 mm", "width": "60 mm", "height": "20 mm", "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"})),
        ("solid.hole", json!({"position": [20, 20, 20], "diameter": "5 mm", "name": "Hole"})),
        ("sketch.create", json!({"plane": "XY", "offset": "40 mm", "name": "Outline"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [10, 10]})),
        ("sketch.finish", json!({})),
    ] {
        // The unit a new design has already.
        if let Err(e) = start.0.session.execute(command, &params) {
            assert!(command == "document.units" && units == "mm", "{command}: {e}");
        }
    }
    start
}

/// `params` with each of its sizes (mm) written by `to`.
fn resized(params: &Value, to: &dyn Fn(&str, f64) -> Value) -> Value {
    let mut p = params.clone();
    for key in ALL_SIZES {
        if let Some(mm) = params.get(key).and_then(Value::as_f64) {
            p[key] = to(key, mm);
        }
    }
    p
}

fn in_mm(mm: f64) -> Value {
    json!(format!("{mm} mm"))
}

/// G73: a size without a unit is not run in a design that is not in mm, whichever it is.
#[test]
fn g73_a_size_without_a_unit_is_not_run() {
    for (units, per) in [("in", 25.4), ("cm", 10.0)] {
        for (command, params, sizes) in sized(&block(units).0.session) {
            for size in sizes {
                let bare = [json!(1.5), json!(format!("{}", 1.5 / per)), json!("3/4"), json!(" +2 "), json!("101.6 - 0.5 in"), json!("8/n")];
                for value in bare {
                    let p = resized(&params, &|key, mm| if key == *size { value.clone() } else { in_mm(mm) });
                    let mut start = block(units);
                    start.0.session.execute("parameters.add", &json!({"name": "n", "expression": "4", "unit": ""})).unwrap();
                    not_run(start, "Do the next step", &one(command, p), "no unit");
                }
            }
        }
    }
    let boss = json!({"diameter": 25.4, "height": 12.7, "base": [50, 30, 6.35], "operation": "join", "targets": ["Plate"]});
    not_run(inch_plate(), "Add a 1 in boss on Plate", &one("solid.cylinder", boss.clone()), "no unit");
    // One of several, and one that the question does not list.
    let good = resized(&json!({"length": 5, "width": 5, "height": 5, "corner": [200, 0, 0]}), &|_, mm| in_mm(mm));
    let both = json!([{"command": "solid.box", "params": good}, {"command": "solid.cylinder", "params": boss}]);
    not_run(inch_plate(), "Do the next step", &both, "no unit");
    for params in [
        json!({"position": [20, 40, 6.35], "diameter": "0.25 in", "references": [{"edge": [50.8, 0, 6.35], "distance": 1}]}),
        json!({"position": [20, 40, 6.35], "diameter": "0.25 in", "depth": "0.2 in", "thread_depth": 3}),
    ] {
        not_run(inch_plate(), "Do the next step", &one("solid.hole", params), "no unit");
    }
    // A point worked out of bare numbers is read in the design's unit, one written as them in mm.
    for key in ["corner", "center"] {
        let mut p = json!({"length": "1 in", "width": "1 in", "height": "1 in", "operation": "join", "targets": ["Plate"]});
        p[key] = json!(["101.6/2", "63.5/2", 6.35]);
        not_run(inch_plate(), "Do the next step", &one("solid.box", p), "no unit");
    }
    let radii = json!({"edges": [[50.8, 0, 6.35]], "type": "spline", "radius": "0.05 in", "radii": [1.2], "body": "Plate"});
    not_run(inch_plate(), "Do the next step", &one("solid.fillet", radii), "no unit");
}

/// G74: points, picks, directions, counts and angles need none, nor does a value that names a
/// parameter; in mm no size does.
#[test]
fn g74_what_needs_no_unit_is_drawn() {
    let hole = json!({"position": [80, 40, 20], "diameter": "6 mm", "type": "countersink", "cs_diameter": "11 mm", "cs_angle": 82});
    drawn(block("in"), "Do the next step", &one("solid.hole", hole));
    for (command, params) in [
        ("solid.mirror", json!({"features": ["Hole"], "plane": {"origin": [50, 0, 0], "normal": [1, 0, 0]}})),
        ("solid.pattern.circular", json!({"features": ["Hole"], "axis": {"origin": [50, 30, 0], "dir": [0, 0, 1]}, "count": 4, "angle": 360})),
        ("solid.pattern.rectangular", json!({"features": ["Hole"], "dir1": [1, 0, 0], "count1": "3", "spacing1": "20 mm"})),
        ("solid.mirror", json!({"bodies": ["Plate"], "plane": "YZ", "name": "7"})),
        ("solid.box", json!({"length": "1 in", "width": "1 in", "height": "1 in", "center": [200, "0", 0], "operation": "new"})),
        ("solid.extrude", json!({"sketch": "Outline", "profiles": [0], "distance": "0.2 in", "direction": "positive", "operation": "new"})),
        ("solid.hole", json!({"position": [80, 40, 20], "diameter": "6 mm", "direction": [0, 0, -1], "to": [80, 40, 0]})),
        ("solid.fillet", json!({"edges": [{"body": "Plate", "index": 0}], "radius": "0.1 in"})),
        ("solid.fillet", json!({"edges": [[0, 0, 10]], "radius": "0.1 in", "body": "Plate"})),
        ("solid.shell", json!({"faces": [[50, 30, 0]], "thickness": "0.1 in", "body": "Plate"})),
        (
            "solid.cylinder",
            json!({"radius": "0.1 in", "height": "2 in", "base": {"x": 70, "y": 30, "z": -1}, "axis": [0, 0, 1], "operation": "cut", "targets": ["Plate"], "name": "5"}),
        ),
    ] {
        drawn(block("in"), "Do the next step", &one(command, params));
    }
    let mut start = block("in");
    start.0.session.execute("parameters.add", &json!({"name": "w", "expression": "0.5 in"})).unwrap();
    let cube = json!({"length": "w/2", "width": "w", "height": "0.1 in", "corner": ["16*w", 0, 0], "operation": "new", "body_name": "Cube"});
    let app = drawn(start, "Do the next step", &one("solid.box", cube));
    let bodies = status(&app)["ghost"]["bodies"].as_array().unwrap().clone();
    let volume = bodies.iter().find(|b| b["name"] == "Cube").unwrap()["volume"].as_f64().unwrap();
    assert!((volume - 6.35 * 12.7 * 2.54).abs() < 1e-6, "{volume}");
    for (command, params, _) in sized(&block("mm").0.session) {
        drawn(block("mm"), "Do the next step", &one(command, params));
    }
}

/// G75: the same step in inches, in mm in an inch design, and in a mm design: one ghost.
#[test]
fn g75_sizes_with_a_unit_make_the_same_ghost() {
    let bodies = |app: &SolveApp| -> Vec<(String, f64, u64)> {
        let mut out: Vec<_> = status(app)["ghost"]["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| (b["name"].as_str().unwrap().to_string(), b["volume"].as_f64().unwrap(), b["faces"].as_u64().unwrap()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };
    for (command, params, _) in sized(&block("in").0.session) {
        let want = bodies(&drawn(block("mm"), "Do the next step", &one(command, params.clone())));
        let inches = resized(&params, &|_, mm| json!(format!("{} in", mm / 25.4)));
        for p in [inches, resized(&params, &|_, mm| in_mm(mm))] {
            let got = bodies(&drawn(block("in"), "Do the next step", &one(command, p.clone())));
            assert_eq!(got.len(), want.len(), "{command} {p}");
            for (g, w) in got.iter().zip(&want) {
                assert!(g.0 == w.0 && g.2 == w.2 && (g.1 - w.1).abs() <= 1e-6 * w.1, "{command} {p}: {g:?} {w:?}");
            }
        }
    }
}

/// G76: Tab in an inch design lands what was drawn, and the feature keeps its unit.
#[test]
fn g76_tab_in_an_inch_design() {
    let hole = one("solid.hole", json!({"position": [20, 40, 6.35], "diameter": "0.25 in", "name": "New"}));
    let boss = json!({"diameter": "1 in", "height": "0.5 in", "base": [50, 50, 6.35], "operation": "join", "targets": ["Plate"], "name": "New"});
    for (step, commands, kept) in [("Add a 0.25 in hole", hole, "0.25 in"), ("Add a 1 in boss on Plate", one("solid.cylinder", boss), "1 in")] {
        let (mut app, chat, _guesser, ctx, was) = worded(inch_plate(), step, &commands);
        let ghost: Vec<_> = app
            .agent_chat
            .predict
            .ghost
            .as_ref()
            .unwrap()
            .after()
            .bodies
            .iter()
            .map(|b| (b.name.clone(), b.body.volume_at(0.01).unwrap(), b.body.face_count()))
            .collect();
        frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
        assert_eq!(outcome(&app, 0), "accepted", "{step}");
        let now = volumes(&app.session);
        assert_eq!(now.len(), ghost.len());
        for (a, b) in now.iter().zip(&ghost) {
            assert!(a.0 == b.0 && a.2 == b.2 && (a.1 - b.1).abs() <= 1e-6 * b.1, "{a:?} {b:?}");
        }
        assert_eq!(app.session.undo.len(), was.undo + 1, "one undo step");
        assert_eq!(app.session.undo.last().unwrap().label, step);
        let kind = serde_json::to_string(&app.session.doc.features.last().unwrap().kind).unwrap();
        assert!(kind.contains(kept), "{kind}");
        assert!(chat.calls().is_empty());
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&*app.session.doc).unwrap(), serde_json::to_value(&*was.doc).unwrap());
    }
}

/// G77: marks and words compare lengths, whatever unit each is written in.
#[test]
fn g77_marks_and_words_compare_lengths() {
    let design = Some("ui.next_step_src_design");
    let said = Some("ui.next_step_src_said");
    for (chat, size, source) in [
        ("", "0.25 in", design),
        ("", "6.35 mm", design),
        ("make it 0.3 in", "0.3 in", said),
        ("make it 0.3", "0.3 in", said),
        ("make it 7.62 mm", "0.3 in", said),
        ("", "0.4 in", None),
        ("make it 0.4 mm", "0.4 in", None),
        ("make it 2 ft", "24 in", said),
        ("make it 2 ft", "2 in", None),
        ("make it .4", "0.4 in", said),
        ("make it .4", "4 in", None),
        ("make it 8", "8 mm", said),
    ] {
        let (mut app, ..) = inch_plate();
        if !chat.is_empty() {
            app.agent_chat.items.push(Item::User {
                text: chat.into(),
                image: false,
                pictures: Vec::new(),
                context: Default::default(),
                files: Vec::new(),
            });
        }
        let steps = vec![("solid.hole".to_string(), json!({"position": [20, 40, 6.35], "diameter": size}))];
        let marks = marks(&mut app, &steps);
        assert_eq!(marks.iter().find(|m| m.key == "diameter").unwrap().source, source, "{chat:?} {size}");
    }
    // What is selected is told in mm: its numbers are not read as inches.
    let (app, ..) = inch_plate();
    let mark = |size: &str, selection: &str, notes: &str| {
        let steps = vec![("solid.cylinder".to_string(), json!({"diameter": size}))];
        ghost::marks(&app.session, &steps, selection, "", notes)[0].source
    };
    assert_eq!(mark("3 mm", "cylindrical, radius 3 mm, index 1", ""), Some("ui.next_step_src_selection"));
    assert_eq!(mark("1 in", "cylindrical, radius 3 mm, index 1", ""), None);
    let brief = crate::agent_chat::project_brief::Brief {
        root: std::path::PathBuf::new(),
        id: String::new(),
        name: "Boards".into(),
        process: String::new(),
        material: String::new(),
        thickness_mm: Some(3.0),
        trusted: false,
        intent: String::new(),
        agents: String::new(),
        notes: Vec::new(),
    };
    let notes = notes_text(&brief);
    assert_eq!(mark("3 in", "", &notes), None, "the project's thickness is in mm");
    assert_eq!(mark("3 mm", "", &notes), Some("ui.next_step_src_notes"));
    let (mm, ..) = plate();
    let said_in = |chat: &str, size: Value| {
        let steps = vec![("solid.cylinder".to_string(), json!({"diameter": size}))];
        ghost::marks(&mm.session, &steps, "", chat, "")[0].source
    };
    assert_eq!(said_in("make it 2 in", json!("50.8 mm")), said);
    assert_eq!(said_in("make it .25", json!(0.25)), said);
    assert_eq!(said_in("make it .25", json!(25)), None);
    assert_eq!(said_in("make it 2 in", json!(2)), said, "\"2 in\" may be no length");
    let hole = |size: &str| one("solid.hole", json!({"position": [20, 40, 6.35], "diameter": size}));
    for (step, size) in [
        ("Add a 0.25 in hole", "0.25 in"),
        ("Add a 1/4 in hole", "0.25 in"),
        ("Add a 1/4\" hole", "0.25 in"),
        ("Add a 1/4-inch hole", "0.25 in"),
        ("Add a 6.35 mm hole", "0.25 in"),
        ("Add a 0.266 in hole", "17/64 in"),
        ("Add a 17/64 in hole", "17/64 in"),
        ("Add a 0.197 in hole", "5 mm"),
        ("Add a 0.133 in radius hole", "17/64 in"),
        ("Add a 0.133 in radius hole", "0.2652 in"),
        ("Add a 0.188 in hole", "3/16 in"),
        ("Add a 0.312 in hole", "0.3125 in"),
        ("Add a 0.02 ft hole", "0.24 in"),
    ] {
        drawn(inch_plate(), step, &hole(size));
    }
    for (step, size) in [
        ("Add a 0.5 in hole", "0.25 in"),
        ("Add a 0.125 in hole", "0.25 in"),
        ("Add a 0.25 in radius hole", "0.25 in"),
        ("Add a 0.27 in hole", "17/64 in"),
        ("Add a 0.2 in hole", "5 mm"),
        ("Add a 0.01 m hole", "5 mm"),
        ("Add a 0.251 in hole", "0.25 in"),
        ("Add a 17/64 in hole", "0.2656 in"),
    ] {
        not_run(inch_plate(), step, &hole(size), "words differ");
    }
    let mm = |size: Value| one("solid.hole", json!({"position": [50, 30, 6], "diameter": size}));
    agrees("Add a 6.67 mm hole", &mm(json!("20/3")));
    disagrees("Add a 3.2 mm hole", &mm(json!(3.175)));
    disagrees("Add a 6.62 mm hole", &mm(json!(6.6)));
}

/// G78: another unit is another design.
#[test]
fn g78_units_changed_under_a_suggestion() {
    let commands = one("solid.hole", json!({"position": [20, 40, 6.35], "diameter": "0.25 in"}));
    for ghost in [false, true] {
        let (mut app, chat, guesser, ctx) = inch_plate();
        line(&mut app, &ctx, &guesser, 0.0, r#"{"step": "Add a 0.25 in hole", "reason": ""}"#);
        if ghost {
            answer_commands(&mut app, &ctx, &guesser, 2.0, &commands);
            assert_eq!(status(&app)["state"], "ghost", "{}", status(&app));
        }
        app.session.execute("document.units", &json!({"units": "mm"})).unwrap();
        let was = before(&mut app);
        if !ghost {
            answer_commands(&mut app, &ctx, &guesser, 2.0, &commands);
        }
        frame(&mut app, &ctx, 3.0, vec![key(egui::Key::Tab)]);
        withdrawn(&mut app, &chat, &was, "stale");
        // As after any edit, the new design is asked about after the pause.
        guesser.ready();
        for t in [3.1, 4.4, 4.7, 4.8] {
            frame(&mut app, &ctx, t, vec![]);
        }
        assert_eq!(guesser.firsts().len(), 2, "{:?}", guesser.calls());
        assert!(guesser.firsts()[1].contains("with 2 mm") && guesser.firsts()[1].contains("units mm"), "{}", guesser.firsts()[1]);
    }
}

/// M5: how long the window's frames take while a ghost computes on a plate with 400 holes, on the
/// real clock, and what became of it. Ignored: it prints numbers
/// (`cargo test -p solidcraft-ui-egui --lib m5_frames -- --ignored --nocapture`).
#[test]
#[ignore = "measurement"]
fn m5_frames_while_a_ghost_computes_on_a_400_hole_plate() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    for (c, p) in [
        ("solid.box", json!({"length": 420, "width": 420, "height": 10, "body_name": "Plate"})),
        ("solid.cylinder", json!({"radius": 3, "height": 12, "base": [10, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"})),
        (
            "solid.pattern.rectangular",
            json!({"features": ["Hole"], "dir1": [1, 0, 0], "count1": 20, "spacing1": 20, "dir2": [0, 1, 0], "count2": 20, "spacing2": 20}),
        ),
    ] {
        app.session.execute(c, &p).unwrap();
    }
    let _ = app.session.world_state();
    app.apply_off_thread = Some(crate::dialogs::APPLY_AT_ONCE);
    line(&mut app, &ctx, &guesser, 0.0, HOLES);
    let asked = std::time::Instant::now();
    let history = history_block(&app.session.doc);
    let _ = ghost::question(&mut app.session, "Bore Plate", &history, false);
    println!("the second question took {:.1} ms to build", asked.elapsed().as_secs_f64() * 1000.0);
    let steps = json!([
        {"command": "solid.box", "params": {"length": 60, "width": 40, "height": 15, "corner": [180, 190, 10], "operation": "join", "targets": ["Plate"]}},
        {"command": "solid.cylinder", "params": {"radius": 12, "height": 40, "base": [210, 210, -5], "operation": "cut", "targets": ["Plate"]}},
    ]);
    guesser.answer(&json!({"commands": steps}).to_string());
    let start = std::time::Instant::now();
    let mut times: Vec<f64> = Vec::new();
    while start.elapsed().as_secs_f64() < 6.0 {
        let t = std::time::Instant::now();
        frame(&mut app, &ctx, 2.0 + start.elapsed().as_secs_f64(), vec![]);
        crate::preview::update(&mut app, &ctx);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    times.sort_by(|a, b| b.total_cmp(a));
    let longest: Vec<String> = times.iter().take(5).map(|t| format!("{t:.1}")).collect();
    let l = app.agent_chat.predict.log()[0].clone();
    println!(
        "{} frames, the five longest {} ms; {} after {:?} ms on the copy, {} commands ran",
        times.len(),
        longest.join(", "),
        l.outcome,
        l.engine_ms,
        l.ran
    );
}
