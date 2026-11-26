//! Next-step suggestions (INT-1458): off by default, nothing before an edit and the pause, one
//! guess per revision, Tab only for what is on screen, looking around keeps the suggestion, and the
//! guessing sessions. The ghost and Tab are in `predict_ghost_tests.rs`. The pause is driven by egui's input clock with scripted frames, so no test waits on the
//! wall clock.

use std::sync::{Arc, Mutex};

use serde_json::json;
use solidcraft_engine::Sel;

use super::*;
use crate::agent_chat::backend::{ChatBackend, Event, PromptPart, Usage};
use crate::agent_chat::{Item, State};

/// A backend that records what it is asked and reports what the test pushes.
#[derive(Clone, Default)]
pub(super) struct Fake {
    calls: Arc<Mutex<Vec<String>>>,
    events: Arc<Mutex<Vec<Event>>>,
    pub(super) usage: Arc<Mutex<Option<Usage>>>,
    /// Signed in with an API key.
    pub(super) key: Arc<std::sync::atomic::AtomicBool>,
}

impl Fake {
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    pub(super) fn prompts(&self) -> Vec<String> {
        self.calls().into_iter().filter_map(|c| c.strip_prefix("prompt: ").map(str::to_string)).collect()
    }
    /// The first questions: one for each suggestion.
    pub(super) fn firsts(&self) -> Vec<String> {
        self.prompts().into_iter().filter(|q| q.contains("Guess the one modelling step")).collect()
    }
    /// The agent stopped answering, as it was asked to.
    pub(super) fn cancelled(&self) {
        self.say(Event::TurnEnded { reason: "cancelled".into() });
    }
    pub(super) fn say(&self, e: Event) {
        self.events.lock().unwrap().push(e);
    }
    pub(super) fn answer(&self, text: &str) {
        self.say(Event::Text { thought: false, text: text.into() });
        self.say(Event::TurnEnded { reason: String::new() });
    }
    pub(super) fn ready(&self) {
        self.say(Event::Ready { agent: "test agent".into(), images: false });
    }
}

impl ChatBackend for Fake {
    fn poll(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
    fn prompt(&mut self, parts: &[PromptPart]) {
        let text = parts.iter().find_map(|p| if let PromptPart::Text(t) = p { Some(t.clone()) } else { None }).unwrap_or_default();
        self.calls.lock().unwrap().push(format!("prompt: {text}"));
    }
    fn cancel(&mut self) {
        self.calls.lock().unwrap().push("cancel".into());
    }
    fn new_session(&mut self) {
        self.calls.lock().unwrap().push("new_session".into());
    }
    fn answer_permission(&mut self, key: u64, option: Option<&str>) {
        self.calls.lock().unwrap().push(format!("permission: {key} {option:?}"));
    }
    fn authenticate(&mut self, _: &str) {}
    fn set_config_option(&mut self, id: &str, value: &str) {
        self.calls.lock().unwrap().push(format!("config: {id}={value}"));
    }
    fn connections(&mut self, request: crate::agent_chat::connections::Request) {
        self.calls.lock().unwrap().push(format!("connections: {:?}", request.what()));
    }
    fn take_usage(&mut self) -> Option<Usage> {
        self.usage.lock().unwrap().take()
    }
    fn key_sign_in(&self) -> bool {
        self.key.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn shutdown(&mut self, _: bool) {
        self.calls.lock().unwrap().push("shutdown".into());
    }
}

/// An app whose chat agent is connected and idle, with a guessing session about to be ready.
/// `on`: the setting is turned on (else it keeps its default).
pub(super) fn setup(on: bool) -> (SolveApp, Fake, Fake, egui::Context) {
    let mut app = SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    let chat = Fake::default();
    app.agent_chat.backend = Some(Box::new(chat.clone()));
    app.agent_chat.state = Some(State::Ready);
    app.agent_chat.argv = vec!["test-agent".into()];
    app.agent_chat.running_id = CLAUDE_CODE.into();
    app.home.open = false;
    if on {
        app.preferences.agent_next_step = true;
    }
    let guesser = Fake::default();
    app.agent_chat.predict.backend = Some(Box::new(guesser.clone()));
    app.agent_chat.predict.link = Link::Starting;
    app.agent_chat.predict.argv = app.agent_chat.argv.clone();
    guesser.ready();
    (app, chat, guesser, crate::fonts::test_context())
}

/// History's strip, where the chip is drawn in these tests.
pub(super) const ROW: egui::Rect = egui::Rect { min: egui::pos2(0.0, 600.0), max: egui::pos2(1600.0, 630.0) };

/// One frame at input time `t` (seconds) with these events, as the window runs it: the guess's
/// keys first, then the view asks for what is drawn over the design and History draws the chip.
pub(super) fn frame(app: &mut SolveApp, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
    let raw = egui::RawInput { time: Some(t), events, ..Default::default() };
    let _ = ctx.run_ui(raw, |ui| {
        tick(app, ui.ctx());
        crate::preview::update(app, ui.ctx());
        let painter = ui.painter().clone();
        let _ = paint_ghost(app, ui, &painter, 10.0, ROW);
    });
}

pub(super) fn key(k: egui::Key) -> egui::Event {
    egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

pub(super) fn release(k: egui::Key) -> egui::Event {
    egui::Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE }
}

pub(super) fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// A click in the model, away from History: how the user picks things and orbits.
pub(super) fn click_in_view() -> Vec<egui::Event> {
    let at = egui::pos2(400.0, 300.0);
    vec![egui::Event::PointerMoved(at), press(at, true), press(at, false)]
}

/// Orbit (a drag), pan and zoom: looking around the model.
pub(super) fn look_around() -> Vec<egui::Event> {
    let (a, b) = (egui::pos2(400.0, 300.0), egui::pos2(480.0, 340.0));
    vec![
        egui::Event::PointerMoved(a),
        press(a, true),
        egui::Event::PointerMoved(b),
        press(b, false),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -3.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        },
        egui::Event::Zoom(1.2),
    ]
}

pub(super) const STEP: &str = "Fillet the 4 top edges of Body1 with 2 mm";
pub(super) const GUESS: &str =
    r#"{"step": "Fillet the 4 top edges of Body1 with 2 mm", "reason": "Every block in this design has 2 mm top fillets."}"#;

/// Frames 0 and 5: the pause is over and the question goes out.
pub(super) fn asked(app: &mut SolveApp, ctx: &egui::Context, guesser: &Fake) {
    frame(app, ctx, 0.0, vec![]);
    frame(app, ctx, 5.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1, "{:?}", guesser.calls());
}

/// Asked, answered with [`GUESS`] and shown (the chip drawn).
pub(super) fn shown(app: &mut SolveApp, ctx: &egui::Context, guesser: &Fake) {
    asked(app, ctx, guesser);
    guesser.answer(GUESS);
    frame(app, ctx, 6.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_some(), "{}", status(app));
    assert!(app.agent_chat.predict.drawn, "the chip is drawn");
}

pub(super) fn edit(app: &mut SolveApp, x: f64) {
    app.session.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10, "corner": [x, 0, 0]})).unwrap();
}

pub(super) fn outcome(app: &SolveApp, i: usize) -> String {
    app.agent_chat.predict.log()[i].outcome.clone()
}

#[test]
fn next_step_suggestions_are_off_by_default_and_ask_nothing() {
    assert!(!crate::prefs::Prefs::default().agent_next_step);
    let (mut app, chat, guesser, ctx) = setup(false);
    for t in [0.0, 5.0, 10.0, 60.0, 600.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert!(guesser.prompts().is_empty(), "{:?}", guesser.calls());
    assert!(chat.calls().is_empty(), "{:?}", chat.calls());
    // A session started before (the setting was on) is stopped, not left running.
    assert!(guesser.calls().contains(&"shutdown".to_string()));
    assert!(app.agent_chat.predict.backend.is_none());
    assert!(chip(&app).is_none(), "nothing in History");
    assert_eq!(status(&app)["state"], "off");
    assert_eq!(status(&app)["guesses"], 0);
}

#[test]
fn turning_next_step_suggestions_off_stops_every_guess_call() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    shown(&mut app, &ctx, &guesser);
    app.preferences.agent_next_step = false;
    frame(&mut app, &ctx, 7.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_none(), "the guess goes with the setting");
    assert!(guesser.calls().contains(&"shutdown".to_string()));
    for t in [20.0, 60.0, 600.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!(guesser.prompts().len(), 2, "{:?}", guesser.calls());
}

#[test]
fn no_guess_is_asked_before_the_pause_and_looking_around_does_not_restart_it() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    for t in [0.0, 0.5, 1.4] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert!(guesser.prompts().is_empty(), "nothing before 1.5 s: {:?}", guesser.calls());
    // A key press restarts the pause.
    frame(&mut app, &ctx, 1.45, vec![key(egui::Key::A)]);
    frame(&mut app, &ctx, 2.9, vec![]);
    assert!(guesser.prompts().is_empty(), "the pause counts from the last key: {:?}", guesser.calls());
    // Moving the pointer, clicking in the view, orbiting, panning and zooming do not.
    frame(&mut app, &ctx, 2.91, click_in_view());
    frame(&mut app, &ctx, 2.92, look_around());
    frame(&mut app, &ctx, 3.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1, "{:?}", guesser.calls());
    // The pause is a setting.
    let (mut app, _chat, guesser, ctx) = setup(true);
    app.preferences.agent_next_step_pause_s = 12.0;
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 11.0, vec![]);
    assert!(guesser.prompts().is_empty());
    frame(&mut app, &ctx, 12.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1);
}

#[test]
fn no_guess_while_the_agent_works_or_a_dialog_is_open() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    app.agent_chat.state = Some(State::Busy);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 30.0, vec![]);
    assert!(guesser.prompts().is_empty(), "{:?}", guesser.calls());
    app.agent_chat.state = Some(State::Ready);
    app.dialog = Some(crate::dialogs::Dialog::chat_context(&Vec::new()));
    frame(&mut app, &ctx, 60.0, vec![]);
    assert!(guesser.prompts().is_empty(), "{:?}", guesser.calls());
    app.dialog = None;
    frame(&mut app, &ctx, 61.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1);
}

/// Fix 2 and 8: nothing is asked where the chip has nowhere to go (History on the left, in the
/// Browser, hidden, or the Render workspace, which takes Tab), nor in a sketch, a tool, or a
/// skill's chat.
#[test]
fn no_guess_where_the_chip_cannot_show_or_while_a_sketch_tool_or_skill_is_on() {
    type Hold = fn(&mut SolveApp);
    let holds: [(&str, Hold, Hold); 7] = [
        ("History in the Browser", |a| a.preferences.history_in_browser = true, |a| a.preferences.history_in_browser = false),
        ("History on the left", |a| a.preferences.timeline_left = true, |a| a.preferences.timeline_left = false),
        ("History hidden", |a| a.ui.show_timeline = false, |a| a.ui.show_timeline = true),
        ("Render workspace", |a| a.ui.tab = "RENDER".into(), |a| a.ui.tab = "MODEL".into()),
        ("a sketch", |a| a.session.active_sketch = Some(1), |a| a.session.active_sketch = None),
        ("a tool", |a| a.tool = crate::tools::Tool::for_command("sketch.line"), |a| a.tool = None),
        (
            "a skill",
            |a| a.agent_chat.skill = crate::agent_chat::skill::Skill::TextToCad,
            |a| a.agent_chat.skill = crate::agent_chat::skill::Skill::Plain,
        ),
    ];
    for (what, on, off) in holds {
        let (mut app, _chat, guesser, ctx) = setup(true);
        on(&mut app);
        for t in [0.0, 5.0, 30.0, 120.0] {
            frame(&mut app, &ctx, t, vec![]);
        }
        assert!(guesser.prompts().is_empty(), "{what}: {:?}", guesser.calls());
        off(&mut app);
        frame(&mut app, &ctx, 121.0, vec![]);
        assert_eq!(guesser.prompts().len(), 1, "{what}: asked once it clears");
    }
}

/// Fix 9: one question per revision. Selecting something else restarts the pause but asks nothing
/// new; an edit does.
#[test]
fn one_guess_per_revision_and_a_selection_change_asks_nothing_new() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    edit(&mut app, 0.0);
    asked(&mut app, &ctx, &guesser);
    // An agent that lists several candidates still gives one step: the first.
    guesser.answer(&format!("Here are some ideas:\n{GUESS}\n{}", r#"{"step": "Shell Body1 to 1.5 mm", "reason": "Second idea."}"#));
    frame(&mut app, &ctx, 6.0, vec![]);
    let g = app.agent_chat.predict.shown().cloned().unwrap();
    assert_eq!(g.step, STEP);
    assert_eq!(g.reason, "Every block in this design has 2 mm top fillets.");
    guesser.answer(r#"{"commands": [{"command": "solid.fillet", "params": {"edges": [[0, 5, 10]], "radius": 2, "body": "Body1"}}]}"#);
    // While it is shown, no second guess is asked for however long the user waits.
    for t in [7.0, 20.0, 60.0, 600.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert!(app.agent_chat.predict.ghost.is_some(), "{}", status(&app));
    assert_eq!(guesser.firsts().len(), 1, "{:?}", guesser.calls());
    // Dismissed, then another selection: the same revision is not asked about again.
    frame(&mut app, &ctx, 601.0, vec![key(egui::Key::Escape)]);
    let body = app.session.world_state().unwrap().bodies[0].name.clone();
    app.session.selection = vec![Sel::Body { name: body }];
    for t in [602.0, 610.0, 700.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!(guesser.firsts().len(), 1, "a selection is not an edit: {:?}", guesser.calls());
    // An edit is: after the pause, one new question in a fresh session.
    edit(&mut app, 20.0);
    frame(&mut app, &ctx, 701.0, vec![]);
    frame(&mut app, &ctx, 706.0, vec![]);
    assert!(guesser.calls().contains(&"new_session".to_string()), "{:?}", guesser.calls());
    assert_eq!(guesser.firsts().len(), 1, "the question waits for the fresh session");
    guesser.ready();
    frame(&mut app, &ctx, 706.5, vec![]);
    assert_eq!(guesser.firsts().len(), 2, "{:?}", guesser.calls());
    assert!(guesser.firsts()[1].contains("BoxFeature"), "{}", guesser.firsts()[1]);
}

/// Fix 5: the chip is short, with an ellipsis; the whole step, the reason, the keys and the
/// session's count and cost are in its tooltip.
#[test]
fn the_chip_is_short_and_its_tooltip_has_the_reason_keys_count_and_cost() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    asked(&mut app, &ctx, &guesser);
    let long = "Add four 5 mm through holes in the corners of the base plate, 8 mm in from each edge, then countersink them";
    *guesser.usage.lock().unwrap() = Some(Usage { cost: Some((0.12, "USD".into())), ..Default::default() });
    guesser.answer(&json!({"step": long, "reason": "The plate has no way to be fixed yet."}).to_string());
    frame(&mut app, &ctx, 6.0, vec![]);
    let (text, tip, guess) = chip(&app).unwrap();
    assert!(guess);
    assert!(text.starts_with("Next (Tab): Add four 5 mm") && text.ends_with('…'), "{text}");
    assert!(text.chars().count() <= "Next (Tab): ".len() + CHIP_CHARS + 1, "{text}");
    let tip = tip.unwrap();
    assert!(tip.contains(long) && tip.contains("The plate has no way to be fixed yet."), "{tip}");
    assert!(tip.contains("Tab puts it into the design") && tip.contains("Esc"), "{tip}");
    assert!(tip.contains("Suggestions this session: 1, $0.12"), "{tip}");
    // A short step is shown whole, and a long reason is cut on reading.
    assert_eq!(short(STEP), STEP);
    let (_, reason) = parse_reply(&json!({"step": "x", "reason": "why ".repeat(100)}).to_string()).unwrap();
    assert!(reason.chars().count() <= MAX_REASON, "{reason}");
}

#[test]
fn esc_dismisses_the_guess_and_it_is_not_offered_again() {
    let (mut app, chat, guesser, ctx) = setup(true);
    shown(&mut app, &ctx, &guesser);
    frame(&mut app, &ctx, 7.0, vec![key(egui::Key::Escape)]);
    assert!(app.agent_chat.predict.shown().is_none());
    assert!(chip(&app).is_none_or(|(_, _, guess)| !guess));
    assert!(app.esc_handled, "Esc is used up by the guess, not passed on");
    // The next question tells the agent not to offer it again.
    let q = prompt(&mut app);
    assert!(q.contains("turned down (dismissed, or rejected after a preview); do not suggest them again:\n- Fillet the 4 top edges"), "{q}");
    assert!(chat.prompts().is_empty(), "nothing is sent: {:?}", chat.calls());
    assert_eq!(outcome(&app, 0), "dismissed");
    // Tab afterwards sends nothing.
    frame(&mut app, &ctx, 8.0, vec![key(egui::Key::Tab)]);
    assert!(chat.prompts().is_empty(), "{:?}", chat.calls());
}

/// Fix 4: a key press or an edit dismisses the chip; looking around does not, and a question out
/// is only cancelled by an edit.
#[test]
fn keys_and_edits_dismiss_the_guess_and_looking_around_does_not() {
    // Looking around keeps the chip.
    let (mut app, _chat, guesser, ctx) = setup(true);
    shown(&mut app, &ctx, &guesser);
    frame(&mut app, &ctx, 7.0, click_in_view());
    frame(&mut app, &ctx, 7.1, look_around());
    frame(&mut app, &ctx, 7.2, vec![]);
    assert!(app.agent_chat.predict.shown().is_some(), "orbit, pan, zoom and clicks keep it");
    // A key that is not Tab or Esc dismisses it.
    frame(&mut app, &ctx, 8.0, vec![key(egui::Key::A)]);
    assert!(app.agent_chat.predict.shown().is_none());
    assert_eq!(outcome(&app, 0), "dismissed");
    // An edit dismisses a guess shown, too.
    let (mut app, _chat, guesser, ctx) = setup(true);
    shown(&mut app, &ctx, &guesser);
    edit(&mut app, 0.0);
    frame(&mut app, &ctx, 7.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_none());
}

/// Fix 4: a late answer is shown if the design and the selection are still the ones asked about,
/// whatever else the user did; else it is `stale`. An edit cancels the question out, and the call
/// stays logged as billed.
#[test]
fn a_late_answer_is_shown_if_the_design_and_selection_held() {
    // Looking around and pressing keys while the question is out: the answer still shows.
    let (mut app, _chat, guesser, ctx) = setup(true);
    asked(&mut app, &ctx, &guesser);
    frame(&mut app, &ctx, 5.5, look_around());
    frame(&mut app, &ctx, 5.6, vec![key(egui::Key::A)]);
    assert!(!guesser.calls().contains(&"cancel".to_string()), "{:?}", guesser.calls());
    guesser.answer(GUESS);
    frame(&mut app, &ctx, 9.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_some(), "{}", status(&app));
    // Another selection meanwhile: not shown, logged `stale`.
    let (mut app, _chat, guesser, ctx) = setup(true);
    edit(&mut app, 0.0);
    asked(&mut app, &ctx, &guesser);
    let body = app.session.world_state().unwrap().bodies[0].name.clone();
    app.session.selection = vec![Sel::Body { name: body }];
    frame(&mut app, &ctx, 5.5, vec![]);
    guesser.answer(GUESS);
    frame(&mut app, &ctx, 6.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_none());
    assert_eq!(outcome(&app, 0), "stale");
    // An edit meanwhile: the question is cancelled, its answer not shown; still billed.
    let (mut app, _chat, guesser, ctx) = setup(true);
    asked(&mut app, &ctx, &guesser);
    edit(&mut app, 0.0);
    frame(&mut app, &ctx, 5.5, vec![]);
    assert!(guesser.calls().contains(&"cancel".to_string()), "{:?}", guesser.calls());
    guesser.answer(GUESS);
    frame(&mut app, &ctx, 6.0, vec![]);
    assert!(app.agent_chat.predict.shown().is_none(), "{}", status(&app));
    let l = &app.agent_chat.predict.log()[0];
    assert_eq!((l.outcome.as_str(), l.ms, l.billed), ("stale", Some(1000), true));
    assert_eq!(status(&app)["billedCalls"], 1);
}

/// Ask Here (Ctrl+K) and the guess stay out of each other's way: the Ctrl+K press dismisses a
/// guess shown, no guess is asked while the box is open, and the box's Escape is its own.
#[test]
fn ask_here_and_the_guess_do_not_clash() {
    let (mut app, chat, guesser, ctx) = setup(true);
    shown(&mut app, &ctx, &guesser);
    let ctrl_k = egui::Event::Key { key: egui::Key::K, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    frame(&mut app, &ctx, 7.0, vec![ctrl_k]);
    assert!(app.agent_chat.predict.shown().is_none(), "Ctrl+K is a key press: the guess goes");
    assert!(chat.prompts().is_empty());
    guesser.cancelled();
    app.agent_chat.ask_box.open = true;
    edit(&mut app, 0.0);
    for t in [8.0, 20.0, 60.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!(guesser.firsts().len(), 1, "no guess while the box is open: {:?}", guesser.calls());
    app.esc_handled = false;
    frame(&mut app, &ctx, 61.0, vec![key(egui::Key::Escape)]);
    assert!(!app.esc_handled, "Escape is left to the box");
    app.agent_chat.ask_box.open = false;
    frame(&mut app, &ctx, 62.0, vec![]);
    frame(&mut app, &ctx, 67.0, vec![]);
    guesser.ready();
    frame(&mut app, &ctx, 67.1, vec![]);
    assert_eq!(guesser.firsts().len(), 2, "with the box closed, the pause and the guess resume");
}

/// The demo found it: egui reads Tab to move keyboard focus before the guess takes the key, so the
/// first Tab sent the guess and also put focus on a widget, and the second Tab was then taken for
/// typing and dismissed the next guess.
#[test]
fn tab_keeps_working_with_widgets_on_screen_and_leaves_a_text_field_alone() {
    let (mut app, chat, guesser, ctx) = setup(true);
    let mut text = String::new();
    let mut run = |app: &mut SolveApp, t: f64, events: Vec<egui::Event>, focus_field: bool| {
        let raw = egui::RawInput { time: Some(t), events, ..Default::default() };
        let _ = ctx.run_ui(raw, |ui| {
            tick(app, ui.ctx());
            let _ = ui.button("a button");
            let field = ui.text_edit_singleline(&mut text);
            if focus_field {
                field.request_focus();
            }
            let painter = ui.painter().clone();
            let _ = paint_ghost(app, ui, &painter, 10.0, ROW);
        });
    };
    run(&mut app, 0.0, vec![], false);
    run(&mut app, 5.0, vec![], false);
    guesser.answer(GUESS);
    run(&mut app, 6.0, vec![], false);
    run(&mut app, 7.0, vec![key(egui::Key::Tab)], false);
    run(&mut app, 7.1, vec![], false);
    assert!(ctx.memory(|m| m.focused().is_none()), "Tab on the suggestion moved no focus");
    assert!(app.agent_chat.predict.shown().is_some(), "its ghost is not drawn yet: {}", status(&app));
    assert!(chat.prompts().is_empty(), "{:?}", chat.calls());
    // In a text field, Tab is the field's; typing there drops the suggestion.
    run(&mut app, 7.3, vec![], true);
    run(&mut app, 7.4, vec![key(egui::Key::Tab)], true);
    assert!(chat.prompts().is_empty(), "{:?}", chat.calls());
    assert!(app.agent_chat.predict.shown().is_some());
    run(&mut app, 7.5, vec![egui::Event::Text("5".into())], true);
    assert!(app.agent_chat.predict.shown().is_none());
}

#[test]
fn tab_with_no_guess_is_left_alone() {
    let (mut app, chat, guesser, ctx) = setup(true);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 1.0, vec![key(egui::Key::Tab)]);
    assert!(chat.prompts().is_empty());
    assert!(guesser.prompts().is_empty(), "Tab is a key press: the pause starts again");
}

#[test]
fn each_guess_logs_its_time_and_cost() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    asked(&mut app, &ctx, &guesser);
    assert!(app.agent_chat.predict.log()[0].billed, "billed once sent");
    *guesser.usage.lock().unwrap() = Some(Usage {
        input_tokens: Some(1830),
        output_tokens: Some(42),
        total_tokens: Some(1872),
        context_tokens: None,
        cost: Some((0.0061, "USD".into())),
    });
    guesser.answer(GUESS);
    frame(&mut app, &ctx, 7.25, vec![]);
    let l = app.agent_chat.predict.log()[0].clone();
    assert_eq!(l.outcome, "shown");
    assert_eq!(l.ms, Some(2250));
    assert_eq!((l.input_tokens, l.output_tokens, l.total_tokens), (Some(1830), Some(42), Some(1872)));
    assert_eq!((l.cost, l.currency.as_deref()), (Some(0.0061), Some("USD")));
    assert!(l.prompt_chars > 0 && l.reply_chars > 0);
    let s = status(&app);
    assert_eq!(s["state"], "shown");
    assert_eq!(s["meanMs"], 2250);
    assert_eq!(s["totalCost"], 0.0061);
    assert_eq!(s["billedCalls"], 1);
    assert_eq!(s["log"][0]["step"], STEP);
}

/// Fix 3: where the chip goes, the state is said: thinking, connect an agent, paused.
#[test]
fn the_chips_place_says_what_is_happening() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    frame(&mut app, &ctx, 0.0, vec![]);
    assert!(chip(&app).is_none(), "nothing during the pause");
    frame(&mut app, &ctx, 5.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1);
    assert_eq!(chip(&app).unwrap().0, "Next step: thinking…");
    // A chat with no agent connected.
    app.agent_chat.state = None;
    assert_eq!(chip(&app).unwrap().0, "Next step: connect an agent");
    assert_eq!(status(&app)["state"], "no agent");
    // The agent wants a sign-in.
    let (mut app, _chat, guesser, ctx) = setup(true);
    asked(&mut app, &ctx, &guesser);
    guesser.say(Event::AuthRequired { methods: Vec::new() });
    frame(&mut app, &ctx, 6.0, vec![]);
    assert_eq!(chip(&app).unwrap().0, "Next step paused: sign in to the agent");
    assert_eq!(status(&app)["state"], "paused: sign in");
}

/// Fix 3: a guessing session that ends is tried again after 30 s, then after a wait that doubles.
#[test]
fn a_failed_guessing_session_is_tried_again_with_backoff() {
    let (mut app, _chat, guesser, ctx) = setup(true);
    asked(&mut app, &ctx, &guesser);
    guesser.say(Event::Exited { tail: vec!["boom".into()] });
    frame(&mut app, &ctx, 6.0, vec![]);
    assert_eq!(status(&app)["state"], "paused");
    assert_eq!(chip(&app).unwrap().0, "Next step paused: the agent stopped; trying again soon");
    assert_eq!(outcome(&app, 0), "failed");
    assert_eq!(app.agent_chat.predict.failures, 1);
    assert_eq!(wake_in(&app, 6.0), Some(30.0), "the window wakes for the retry");
    frame(&mut app, &ctx, 35.0, vec![]);
    assert!(matches!(app.agent_chat.predict.link, Link::Failed { at, .. } if at == 6.0), "not before 30 s");
    // At 36 s it starts the agent again ("test-agent" does not exist here, so it fails again)...
    frame(&mut app, &ctx, 36.0, vec![]);
    assert!(matches!(app.agent_chat.predict.link, Link::Failed { at, .. } if at == 36.0), "{:?}", app.agent_chat.predict.link);
    assert_eq!(app.agent_chat.predict.failures, 2);
    // ...and waits twice as long before the next try.
    frame(&mut app, &ctx, 95.0, vec![]);
    assert_eq!(app.agent_chat.predict.failures, 2);
    frame(&mut app, &ctx, 96.0, vec![]);
    assert_eq!(app.agent_chat.predict.failures, 3);
    assert_eq!(retry_wait(1), 30.0);
    assert_eq!(retry_wait(5), 480.0);
    assert_eq!(retry_wait(50), RETRY_MOST_S);
    assert_eq!(guesser.prompts().len(), 1);
}

#[test]
fn the_reply_gives_one_step_and_its_reason() {
    assert_eq!(parse_reply(GUESS), Some((STEP.into(), "Every block in this design has 2 mm top fillets.".into())));
    // Fenced, with words around it.
    let fenced = format!("Sure.\n```json\n{GUESS}\n```\nHope that helps.");
    assert_eq!(parse_reply(&fenced).unwrap().0, STEP);
    // No JSON: the first line is the step.
    assert_eq!(parse_reply("\n  \"Extrude Sketch2 by 5 mm\"\nbecause…"), Some(("Extrude Sketch2 by 5 mm".into(), String::new())));
    // Nothing to suggest.
    assert_eq!(parse_reply(r#"{"step": "", "reason": "The part looks finished."}"#).unwrap().0, "");
    assert_eq!(parse_reply("   "), None);
    // One line, bounded: newlines and control characters go, a long step is cut.
    let long = format!(r#"{{"step": "Fillet\nthe\tedges {}", "reason": "x"}}"#, "a".repeat(500));
    let (step, _) = parse_reply(&long).unwrap();
    assert!(step.starts_with("Fillet the edges ") && !step.contains('\n') && step.chars().count() <= MAX_STEP, "{step}");
}

#[test]
fn the_question_carries_history_usual_values_selection_and_the_chat() {
    let (mut app, _chat, _guesser, _ctx) = setup(true);
    let mut doc = (*app.session.doc).clone();
    let feature = |id: u64, name: &str, kind: serde_json::Value| {
        let mut v = kind;
        v["id"] = json!(id);
        v["name"] = json!(name);
        serde_json::from_value::<solidcraft_engine::doc::Feature>(v).unwrap()
    };
    doc.features.push(feature(1, "Fillet1", json!({"type": "fillet", "edges": [], "radius": "2"})));
    doc.features.push(feature(2, "Fillet2", json!({"type": "fillet", "edges": [], "radius": "2"})));
    doc.features.push(feature(3, "Fillet3", json!({"type": "fillet", "edges": [], "radius": "4"})));
    doc.features.push(feature(4, "Shell1", json!({"type": "shell", "faces": [], "thickness": "1.5"})));
    app.session.doc = Arc::new(doc);
    app.agent_chat.items.push(Item::User {
        text: "Make a 40 x 30 x 20 block".into(),
        image: false,
        pictures: Vec::new(),
        context: Default::default(),
        files: Vec::new(),
    });
    let q = prompt(&mut app);
    assert!(q.contains("\"step\"") && q.contains("\"reason\""), "{q}");
    assert!(q.contains("Do not call any tools"), "{q}");
    assert!(q.contains("at most 12 words"), "{q}");
    assert!(q.contains("Never mention internal face or edge names such as F1:-y"), "{q}");
    assert!(q.contains("in English, the language of the app"), "{q}");
    assert!(q.contains("4 History steps"), "{q}");
    assert!(q.contains("1. Fillet1 (FilletFeature): radius 2"), "{q}");
    assert!(q.contains("4. Shell1 (ShellFeature): thickness 1.5"), "{q}");
    assert!(q.contains("fillet radius: 2 (×2), 4"), "{q}");
    assert!(q.contains("shell (wall) thickness: 1.5"), "{q}");
    assert!(q.contains("[Nothing is selected.]"), "{q}");
    assert!(q.contains("- Make a 40 x 30 x 20 block"), "{q}");
    // Fix 8: the reply comes in the app's language.
    let _zh = crate::i18n::scope("zh");
    assert!(prompt(&mut app).contains("in Simplified Chinese, the language of the app"));
}

/// Fix 9: the setting is found by the words people search for.
#[test]
fn the_setting_is_found_as_predict_autocomplete_or_suggest() {
    for word in ["predict", "autocomplete", "suggest"] {
        assert!(crate::prefs_search::keywords("Suggest the next step after a pause").iter().any(|k| k.contains(word)), "{word}");
    }
}

// The built-in agent and a small model (design doc, "Built-in agent and a small model", B1–B9;
// B10 is in connections.rs).

pub(super) const SONNET: &str = "anthropic/claude-sonnet-x";
pub(super) const HAIKU: &str = "anthropic/claude-haiku-y";
pub(super) const MINI: &str = "openai/gpt-5.4-mini";

fn option(id: &str, category: &str, current: &str, values: &[&str]) -> ConfigOption {
    ConfigOption {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        category: category.into(),
        current_value: current.into(),
        options: values
            .iter()
            .map(|v| crate::agent_chat::backend::ConfigChoice { value: v.to_string(), name: v.to_string(), description: String::new(), group: None })
            .collect(),
    }
}

/// What the built-in agent (Mastra Code) offers a session: its model, mode and reasoning effort.
pub(super) fn mastra_options(mode: &str, model: &str, effort: &str) -> Vec<ConfigOption> {
    vec![
        option("model", "model", model, &[SONNET, HAIKU, MINI]),
        option("mode", "mode", mode, &["build", "plan", "fast"]),
        option("thought_level", "thought_level", effort, &["off", "low", "medium", "high"]),
    ]
}

/// The same options with one of them set, as the agent answers a setting.
fn with(mut options: Vec<ConfigOption>, id: &str, value: &str) -> Vec<ConfigOption> {
    for o in options.iter_mut().filter(|o| o.id == id) {
        o.current_value = value.into();
    }
    options
}

fn snapshot(connected: bool) -> crate::agent_chat::connections::Snapshot {
    crate::agent_chat::connections::Snapshot::parse(&json!({
        "plans": [{"id": "anthropic", "name": "Claude Pro/Max", "state": if connected { "in" } else { "out" }, "prefix": "anthropic"}],
        "keys": [],
    }))
    .unwrap()
}

/// [`setup`] with the built-in agent as the guessing agent: its process is up and has answered
/// with a snapshot, and no session is open yet.
fn setup_builtin(connected: bool) -> (SolveApp, Fake, egui::Context) {
    let (mut app, _chat, guesser, ctx) = setup(true);
    guesser.events.lock().unwrap().clear();
    app.agent_chat.predict.builtin = true;
    app.agent_chat.predict.any_agent = true;
    guesser.say(Event::Connections(snapshot(connected)));
    (app, guesser, ctx)
}

pub(super) fn configs(guesser: &Fake) -> Vec<String> {
    guesser.calls().into_iter().filter(|c| c.starts_with("config: ")).collect()
}

/// The last setting sent, as `(id, value)`.
fn last_config(guesser: &Fake) -> (String, String) {
    let call = configs(guesser).pop().unwrap();
    let (id, value) = call.trim_start_matches("config: ").split_once('=').unwrap();
    (id.into(), value.into())
}

/// Answer each setting the guessing session sends (`answered` were answered before), each with the
/// options as they then are, one frame apart from input time `t`; the time of the last frame.
pub(super) fn answer_settings(
    app: &mut SolveApp,
    ctx: &egui::Context,
    guesser: &Fake,
    mut options: Vec<ConfigOption>,
    mut answered: usize,
    mut t: f64,
) -> f64 {
    while configs(guesser).len() > answered && answered < 20 {
        let (id, value) = last_config(guesser);
        options = with(options, &id, &value);
        guesser.say(Event::ConfigOptionFinished { id, result: Ok(options.clone()) });
        answered += 1;
        t += 1.0;
        frame(app, ctx, t, vec![]);
    }
    t
}

/// A built-in guessing session from its snapshot to its first question, with the preference
/// `choice` and every setting answered.
fn tuned_session(choice: &str) -> (SolveApp, Fake) {
    let (mut app, guesser, ctx) = setup_builtin(true);
    app.preferences.agent_next_step_model = choice.into();
    let options = mastra_options("build", SONNET, "medium");
    frame(&mut app, &ctx, 0.0, vec![]);
    guesser.say(Event::ConfigOptions(options.clone()));
    guesser.ready();
    frame(&mut app, &ctx, 1.0, vec![]);
    let t = answer_settings(&mut app, &ctx, &guesser, options, 0, 1.0);
    frame(&mut app, &ctx, t.max(5.0) + 1.0, vec![]);
    (app, guesser)
}

/// B1: the session is put in the read-only mode, on the small model of its provider and the lowest
/// effort, one setting after the other, and the question waits for the last answer.
#[test]
fn a_built_in_guess_runs_read_only_on_a_small_model_with_the_lowest_effort() {
    let (mut app, guesser, ctx) = setup_builtin(true);
    frame(&mut app, &ctx, 0.0, vec![]);
    assert_eq!(guesser.calls(), ["new_session"], "a session once something is connected");
    let options = mastra_options("build", SONNET, "medium");
    guesser.say(Event::ConfigOptions(options.clone()));
    guesser.ready();
    frame(&mut app, &ctx, 1.0, vec![]);
    assert_eq!(configs(&guesser), ["config: mode=fast"]);
    let options = with(options, "mode", "fast");
    guesser.say(Event::ConfigOptionFinished { id: "mode".into(), result: Ok(options.clone()) });
    frame(&mut app, &ctx, 2.0, vec![]);
    assert_eq!(configs(&guesser), ["config: mode=fast".to_string(), format!("config: model={HAIKU}")]);
    let options = with(options, "model", HAIKU);
    guesser.say(Event::ConfigOptionFinished { id: "model".into(), result: Ok(options.clone()) });
    frame(&mut app, &ctx, 3.0, vec![]);
    assert_eq!(configs(&guesser).last().map(String::as_str), Some("config: thought_level=off"));
    // The pause is over, and the question still waits for the last setting.
    frame(&mut app, &ctx, 6.0, vec![]);
    assert!(guesser.prompts().is_empty(), "{:?}", guesser.calls());
    guesser.say(Event::ConfigOptionFinished { id: "thought_level".into(), result: Ok(with(options, "thought_level", "off")) });
    frame(&mut app, &ctx, 7.0, vec![]);
    assert_eq!(guesser.prompts().len(), 1, "{:?}", guesser.calls());
    assert!(guesser.calls().last().is_some_and(|c| c.starts_with("prompt: ")), "the question comes after every setting");
    assert_eq!(app.agent_chat.predict.log()[0].model.as_deref(), Some(HAIKU));
}

/// B2: `same` sets nothing: the guess runs as the agent opens the session, as before.
#[test]
fn same_leaves_the_guessing_session_as_the_agent_opens_it() {
    let (app, guesser) = tuned_session(SAME_MODEL);
    assert!(configs(&guesser).is_empty(), "{:?}", guesser.calls());
    assert_eq!(guesser.prompts().len(), 1, "{:?}", guesser.calls());
    assert_eq!(app.agent_chat.predict.log()[0].model.as_deref(), Some(SONNET));
}

/// B3: a model named in the preference is used when the agent lists it; an unlisted one is not
/// sent, and the mode and effort are still set.
#[test]
fn a_named_model_is_used_only_when_the_agent_lists_it() {
    let (app, guesser) = tuned_session(MINI);
    assert_eq!(configs(&guesser), ["config: mode=fast".to_string(), format!("config: model={MINI}"), "config: thought_level=off".into()]);
    assert_eq!(app.agent_chat.predict.log()[0].model.as_deref(), Some(MINI));
    let (app, guesser) = tuned_session("someone/unlisted-model");
    assert_eq!(configs(&guesser), ["config: mode=fast", "config: thought_level=off"]);
    assert_eq!(guesser.prompts().len(), 1);
    assert_eq!(app.agent_chat.predict.log()[0].model.as_deref(), Some(SONNET));
}

/// B4: with nothing connected no session is asked for, and the chip's place asks for a sign-in.
/// (Frames stop before the first retry, 30 s after the failure.)
#[test]
fn with_nothing_connected_the_built_in_guess_asks_for_a_sign_in() {
    let (mut app, guesser, ctx) = setup_builtin(false);
    for t in [0.0, 0.1] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!(chip(&app).map(|(text, ..)| text), Some(tr("ui.next_step_paused_sign_in").to_string()));
    for t in [5.0, 10.0, 20.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert!(!guesser.calls().contains(&"new_session".to_string()), "{:?}", guesser.calls());
    assert!(guesser.prompts().is_empty());
    assert_eq!(status(&app)["state"], "paused: sign in");
}

/// B5: a setting the agent refuses is not sent again, and the guess runs on what the session has.
#[test]
fn a_refused_setting_leaves_the_session_as_it_is_and_the_guess_still_goes() {
    let (mut app, guesser, ctx) = setup_builtin(true);
    frame(&mut app, &ctx, 0.0, vec![]);
    let options = mastra_options("build", SONNET, "medium");
    guesser.say(Event::ConfigOptions(options.clone()));
    guesser.ready();
    frame(&mut app, &ctx, 1.0, vec![]);
    guesser.say(Event::ConfigOptionFinished { id: "mode".into(), result: Ok(with(options, "mode", "fast")) });
    frame(&mut app, &ctx, 2.0, vec![]);
    guesser.say(Event::ConfigOptionFinished { id: "model".into(), result: Err("Unknown session configuration selection".into()) });
    for t in [3.0, 6.0, 7.0] {
        frame(&mut app, &ctx, t, vec![]);
    }
    assert_eq!(configs(&guesser), ["config: mode=fast".to_string(), format!("config: model={HAIKU}")], "not sent again");
    assert_eq!(guesser.prompts().len(), 1, "{:?}", guesser.calls());
}

/// B6: which model counts as a provider's small one.
#[test]
fn the_small_model_is_the_first_small_one_of_the_sessions_provider() {
    let pick = |current: &str, values: &[&str]| small_model(&option("model", "model", current, values));
    assert_eq!(pick(SONNET, &[SONNET, MINI, HAIKU]).as_deref(), Some(HAIKU), "the session's provider first");
    assert_eq!(pick(SONNET, &[SONNET, MINI]).as_deref(), Some(MINI), "else another provider's");
    assert_eq!(pick(SONNET, &[SONNET, "google/gemini-2.5-pro"]), None, "gemini is not a mini");
    assert_eq!(pick("google/gemini-2.5-flash", &["google/gemini-2.5-flash", MINI]), None, "already on a small one");
    assert_eq!(pick(SONNET, &[SONNET, "openai/gpt-6.2-sol"]), None, "none listed");
}

/// B7: the agent's own news of a setting while it is out sends nothing more; the next setting
/// follows the answer.
#[test]
fn news_of_a_setting_while_it_is_out_sends_no_second_setting() {
    let (mut app, guesser, ctx) = setup_builtin(true);
    frame(&mut app, &ctx, 0.0, vec![]);
    let options = mastra_options("build", SONNET, "medium");
    guesser.say(Event::ConfigOptions(options.clone()));
    guesser.ready();
    frame(&mut app, &ctx, 1.0, vec![]);
    // `config_option_update` after the mode changed, before the reply to the setting.
    guesser.say(Event::ConfigOptions(with(options.clone(), "mode", "fast")));
    frame(&mut app, &ctx, 1.5, vec![]);
    assert_eq!(configs(&guesser), ["config: mode=fast"]);
    guesser.say(Event::ConfigOptionFinished { id: "mode".into(), result: Ok(with(options, "mode", "fast")) });
    frame(&mut app, &ctx, 2.0, vec![]);
    assert_eq!(configs(&guesser), ["config: mode=fast".to_string(), format!("config: model={HAIKU}")]);
}

/// G79: a package may carry the built-in agent beside the agents on the computer. The guessing
/// session is started as the built-in agent is (its data folder, its connections setup, its session
/// once it says it is connected) only when the chat runs that agent.
#[test]
fn g79_the_built_in_agent_beside_another_is_not_taken_for_it() {
    let folder = std::env::temp_dir().join(format!("intcad-g79-{}", std::process::id()));
    // No program is there, so nothing starts: what is checked is how it would have been started.
    let missing = "/nonexistent/intcad-g79-agent";
    for (running, carried, as_builtin) in
        [(CLAUDE_CODE, true, false), (crate::agent_chat::connections::AGENT_ID, true, true), (CLAUDE_CODE, false, false)]
    {
        let (mut app, _chat, _guesser, ctx) = setup(true);
        app.agent_chat.argv = vec![missing.into()];
        app.agent_chat.running_id = running.into();
        app.builtin_agent = carried.then(|| crate::agent_chat::agents::Builtin {
            packaged: true,
            program: missing.into(),
            args: Vec::new(),
            data: Some(folder.clone()),
        });
        let p = &mut app.agent_chat.predict;
        p.backend = None;
        p.link = Link::None;
        p.any_agent = running == crate::agent_chat::connections::AGENT_ID;
        start_session(&mut app, &ctx, 0.0);
        let p = &app.agent_chat.predict;
        assert_eq!(p.builtin, as_builtin, "{running}, the package's agent carried: {carried}");
        assert!(matches!(p.link, Link::Failed { .. }) && p.backend.is_none(), "{running}: nothing started");
    }
    let _ = std::fs::remove_dir_all(&folder);
}

/// B8: the fresh session of the next question is set up again, the same way.
#[test]
fn each_fresh_guessing_session_is_tuned_again() {
    let (mut app, guesser) = tuned_session("");
    let ctx = crate::fonts::test_context();
    let first = configs(&guesser);
    assert_eq!(first.len(), 3, "{first:?}");
    guesser.answer(GUESS);
    frame(&mut app, &ctx, 20.0, vec![]);
    edit(&mut app, 20.0);
    frame(&mut app, &ctx, 21.0, vec![]);
    guesser.cancelled();
    frame(&mut app, &ctx, 27.0, vec![]);
    assert_eq!(guesser.calls().iter().filter(|c| *c == "new_session").count(), 2, "{:?}", guesser.calls());
    let options = mastra_options("build", SONNET, "medium");
    guesser.say(Event::ConfigOptions(options.clone()));
    guesser.ready();
    frame(&mut app, &ctx, 28.0, vec![]);
    let t = answer_settings(&mut app, &ctx, &guesser, options, 3, 28.0);
    frame(&mut app, &ctx, t + 1.0, vec![]);
    assert_eq!(configs(&guesser)[3..], first[..], "the same settings again");
    assert_eq!(guesser.firsts().len(), 2, "{:?}", guesser.calls());
}

/// B9: the built-in agent's guessing process is started as the chat's: with the chat's data folder,
/// and handed its setup once `initialize` is answered, before any session. Without the folder it is
/// not started.
#[test]
fn the_built_in_guessing_process_gets_the_chats_folder_and_setup() {
    use std::time::{Duration, Instant};
    let dir = std::env::temp_dir().join(format!("intcad-predict-builtin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (seen, data) = (dir.join("seen"), dir.join("mastra"));
    // A stand-in agent: writes its data folder and every line it is sent, and answers initialize.
    let script = r#"printf '%s\n' "$MASTRA_APP_DATA_DIR" > "$0"; IFS= read -r line; printf '%s\n' "$line" >> "$0"; printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"agentInfo":{"name":"fake"}}}'; while IFS= read -r line; do printf '%s\n' "$line" >> "$0"; done"#;
    let argv = ["sh".to_string(), "-c".to_string(), script.to_string(), seen.to_string_lossy().into_owned()];
    let ctx = crate::fonts::test_context();
    let setup = crate::agent_chat::connections::Setup { keys: vec![("anthropic".into(), "k-predict".into())], ..Default::default() };
    let mut b = start_backend(&argv, &ctx, Some((Some(data.clone()), setup.clone()))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let text = loop {
        let _ = b.poll();
        let text = std::fs::read_to_string(&seen).unwrap_or_default();
        if text.contains("connections/setup") {
            break text;
        }
        assert!(Instant::now() < deadline, "no setup: {text}");
        std::thread::sleep(Duration::from_millis(20));
    };
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], data.to_string_lossy(), "the chat's data folder");
    assert!(lines[1].contains("\"initialize\""), "{lines:?}");
    assert!(lines[2].contains("_intcad/connections/setup") && lines[2].contains("k-predict"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("session/new")), "no session before something is connected: {lines:?}");
    b.shutdown(true);
    // Without the folder: the chat's own refusal, and no process.
    std::fs::remove_file(&seen).unwrap();
    let refused = start_backend(&argv, &ctx, Some((None, setup))).err().unwrap();
    assert_eq!(refused, tr("ui.agent_chat_no_data_folder"));
    std::thread::sleep(Duration::from_millis(200));
    assert!(!seen.exists(), "nothing started");
    let _ = std::fs::remove_dir_all(&dir);
}

// The lean guessing session on Claude Code (design doc, "Test cases for the lean session", L1–L4).

/// What Claude Code's adapter says of itself in its `initialize` reply: its package name and the
/// offer of `systemPrompt` with `override`.
#[cfg(unix)]
pub(super) const CLAUDE_INIT: &str = r#"{"agentCapabilities":{"sessionCapabilities":{"systemPrompt":{"mode":{"override":{}}}}},"agentInfo":{"name":"@agentclientprotocol/claude-agent-acp","title":"Claude Agent","version":"0.107.0"}}"#;

/// A stand-in agent (a shell script) in a folder of its own: it writes every line it is sent to
/// `seen`, answers `initialize` with `init`, each `session/new` with sessions s1, s2… (or, when
/// the request has `refuse` in it, with an error), and each prompt with the end of the turn. It is
/// signed in with an API key.
#[cfg(unix)]
fn stand_in(name: &str, init: &str, refuse: &str) -> (std::path::PathBuf, Vec<String>) {
    signed_in(name, init, refuse, &["api_key"])
}

/// [`stand_in`], saying it is signed in with the first of `kinds` before its `initialize` reply and
/// with the next one after each `session/new` (empty: it says nothing).
#[cfg(unix)]
pub(super) fn signed_in(name: &str, init: &str, refuse: &str, kinds: &[&str]) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("intcad-predict-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = r#"seen="$0"; init="$1"; refuse="$2"; kinds="$3"; n=0; k=0
say() {
  k=$((k+1)); kind=$(printf '%s' "$kinds" | cut -d'|' -f$k)
  if [ -n "$kind" ]; then printf '{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"%s","label":"test"}}}\n' "$kind"; fi
}
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$seen"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) say; printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$init" ;;
    *'"method":"session/new"'*)
      if [ -n "$refuse" ] && printf '%s' "$line" | grep -q "$refuse"; then
        printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"Invalid params"}}\n' "$id"
      else
        n=$((n+1)); printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"s%s"}}\n' "$id" "$n"; say
      fi ;;
    *'"method":"session/prompt"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id" ;;
  esac
done"#;
    let seen = dir.join("seen").to_string_lossy().into_owned();
    let kinds = format!("{}|", kinds.join("|"));
    let argv = ["sh", "-c", script, &seen, init, refuse, &kinds].map(str::to_string).to_vec();
    (dir, argv)
}

/// The requests the stand-in in `dir` was sent with `method`, in order.
#[cfg(unix)]
pub(super) fn sent(dir: &std::path::Path, method: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("seen"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["method"] == method)
        .collect()
}

/// Poll `b` until `done` holds for the stand-in's folder (30 s at most); the events it gave.
#[cfg(unix)]
pub(super) fn poll_until(b: &mut dyn ChatBackend, dir: &std::path::Path, done: &dyn Fn(&std::path::Path) -> bool) -> Vec<Event> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut events = Vec::new();
    while !done(dir) {
        events.extend(b.poll());
        assert!(Instant::now() < deadline, "timed out: {:?}", std::fs::read_to_string(dir.join("seen")));
        std::thread::sleep(Duration::from_millis(20));
    }
    events.extend(b.poll());
    events
}

/// L1: Claude Code offering an override, signed in with an API key, gets a lean guessing session: the note is its whole system
/// prompt, no tools and none of the user's setup, and no note appended to Claude Code's own.
#[cfg(unix)]
#[test]
fn claude_codes_guessing_session_is_lean_when_it_offers_an_override() {
    let (dir, argv) = stand_in("lean", CLAUDE_INIT, "");
    let ctx = crate::fonts::test_context();
    let mut b = start_backend(&argv, &ctx, None).unwrap();
    poll_until(b.as_mut(), &dir, &|d| !sent(d, "session/new").is_empty());
    let params = &sent(&dir, "session/new")[0]["params"];
    assert_eq!(params["systemPrompt"], json!({"mode": "override", "content": [{"type": "text", "text": SESSION_NOTE}]}));
    let options = &params["_meta"]["claudeCode"]["options"];
    assert_eq!(options["tools"], json!([]), "{params}");
    assert_eq!(options["settingSources"], json!([]), "{params}");
    assert!(params["_meta"].get("systemPrompt").is_none(), "no appended note: {params}");
    assert_eq!(params["mcpServers"], json!([]));
    b.shutdown(true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// L2: the chat's own session with the same agent is what it was before the lean session.
#[cfg(unix)]
#[test]
fn the_chats_session_with_claude_code_is_untouched() {
    let (dir, argv) = stand_in("chat", CLAUDE_INIT, "");
    let start = crate::agent_chat::backend::StartContext { cwd: dir.clone(), bridge: None, tools: None, wake: Box::new(|| {}) };
    let mut b = crate::agent_chat::acp_client::AcpBackend::start(&argv, start).unwrap();
    poll_until(&mut b, &dir, &|d| !sent(d, "session/new").is_empty());
    let note = crate::agent_chat::acp::SYSTEM_NOTE;
    let before = json!({
        "cwd": dir.to_string_lossy(),
        "mcpServers": [],
        "_meta": {
            "systemPrompt": {"append": note},
            "claudeCode": {"options": {"allowDangerouslySkipPermissions": false, "disallowedTools": crate::agent_chat::acp::CLAUDE_DISALLOWED_TOOLS}},
        },
        "systemPrompt": {"mode": "append", "content": [{"type": "text", "text": note}]},
    });
    assert_eq!(sent(&dir, "session/new")[0]["params"].to_string(), before.to_string());
    b.shutdown(true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// L3: an agent that offers no `systemPrompt` gets the note as before: appended, with no override,
/// no `tools` and no `settingSources`.
#[cfg(unix)]
#[test]
fn without_the_offer_the_guessing_session_gets_the_note_as_before() {
    let (dir, argv) = stand_in("no-offer", r#"{"agentInfo":{"name":"@agentclientprotocol/claude-agent-acp","version":"0.107.0"}}"#, "");
    let ctx = crate::fonts::test_context();
    let mut b = start_backend(&argv, &ctx, None).unwrap();
    poll_until(b.as_mut(), &dir, &|d| !sent(d, "session/new").is_empty());
    let params = &sent(&dir, "session/new")[0]["params"];
    assert_eq!(params["_meta"]["systemPrompt"]["append"], crate::agent_chat::acp::system_note(SESSION_NOTE));
    assert!(params.get("systemPrompt").is_none(), "{params}");
    let options = &params["_meta"]["claudeCode"]["options"];
    assert!(options.get("tools").is_none() && options.get("settingSources").is_none(), "{params}");
    b.shutdown(true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Warnings logged while the tests run.
#[cfg(unix)]
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(unix)]
struct Warnings;

#[cfg(unix)]
impl log::Log for Warnings {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            WARNINGS.lock().unwrap().push(record.args().to_string());
        }
    }
    fn flush(&self) {}
}

/// L4: a refused lean session is logged once and asked for again as before; the guess is asked on
/// that session, and the process asks for no lean session again.
#[cfg(unix)]
#[test]
fn a_refused_lean_session_falls_back_once_and_the_guess_still_goes() {
    static LOGGER: Warnings = Warnings;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);
    let (dir, argv) = stand_in("refused", CLAUDE_INIT, "settingSources");
    let ctx = crate::fonts::test_context();
    let mut b = start_backend(&argv, &ctx, None).unwrap();
    let mut events = poll_until(b.as_mut(), &dir, &|d| sent(d, "session/new").len() >= 2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !events.iter().any(|e| matches!(e, Event::Ready { .. })) {
        assert!(std::time::Instant::now() < deadline, "not ready: {events:?}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        events.extend(b.poll());
    }
    assert!(!events.iter().any(|e| matches!(e, Event::Error(_))), "the refusal is no error to show: {events:?}");
    b.prompt(&[PromptPart::Text("guess".into())]);
    poll_until(b.as_mut(), &dir, &|d| !sent(d, "session/prompt").is_empty());
    assert_eq!(sent(&dir, "session/prompt")[0]["params"]["sessionId"], "s1", "the guess is asked on the second session");
    // The next question's fresh session.
    b.new_session();
    poll_until(b.as_mut(), &dir, &|d| sent(d, "session/new").len() >= 3);
    let sessions = sent(&dir, "session/new");
    assert_eq!(sessions.len(), 3, "{sessions:?}");
    assert!(sessions[0].to_string().contains("settingSources"), "the first was lean");
    for s in &sessions[1..] {
        assert!(!s.to_string().contains("settingSources") && !s.to_string().contains("\"tools\""), "{s}");
        assert_eq!(s["params"]["systemPrompt"]["mode"], "append", "no override: {s}");
        assert_eq!(s["params"]["_meta"]["systemPrompt"]["append"], crate::agent_chat::acp::system_note(SESSION_NOTE));
    }
    let warned = WARNINGS.lock().unwrap().iter().filter(|w| w.contains("lean session")).count();
    assert_eq!(warned, 1, "{:?}", WARNINGS.lock().unwrap());
    b.shutdown(true);
    let _ = std::fs::remove_dir_all(&dir);
}
