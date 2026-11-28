//! A stress estimate the user sets up and runs by hand: the right rail's Simulation panel
//! (PIL-294, decision 30 of `docs/design/fea-engineering-ready.md`). The Stress button, Inspect ▸
//! Estimate and the command palette open it. The user chooses a grade, adds the selected faces as
//! supports and as loads, sees the setup drawn on the model (the dry run the approval card uses,
//! [`crate::stress_overlay::approval`]), and presses Run.
//!
//! Run is the one place a free request for the estimate is made: [`ControlRequest::trusted_gui`],
//! the person's own entry (`HumanGuiFree`), queued with the app's held requests, so it is answered
//! on a copy of the design off the window thread as a reading is. An agent's estimate, the chat's
//! Allow included, keeps the entry its bridge gave it and never comes through here (decision 30:
//! an agent's call counts toward credits; the user's own use by hand is free).

use egui::{Rect, vec2};
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::fea::{self, materials::GRADES};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::SolveApp;
use crate::control::{ControlRequest, ControlResponse};
use crate::i18n::{format as trf, tr};
use crate::ui_kit;

const CMD: &str = "inspect.stress_estimate";

/// A support's hold, as the command names it, with its label.
const HOLDS: [(&str, &str); 4] = [
    ("fixed", "ui.stress_by_hand_fixed"),
    ("roller", "ui.stress_by_hand_roller"),
    ("symmetry", "ui.stress_by_hand_symmetry"),
    ("slider", "ui.stress_by_hand_slider"),
];

/// A force's direction: its unit vector, with its label. The first is the default (down).
const AXES: [([f64; 3], &str); 6] = [
    ([0.0, 0.0, -1.0], "−Z"),
    ([0.0, 0.0, 1.0], "+Z"),
    ([-1.0, 0.0, 0.0], "−X"),
    ([1.0, 0.0, 0.0], "+X"),
    ([0.0, -1.0, 0.0], "−Y"),
    ([0.0, 1.0, 0.0], "+Y"),
];

/// Faces held one way.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Support {
    pub(crate) faces: Vec<usize>,
    /// Into [`HOLDS`].
    pub(crate) hold: usize,
}

/// Faces loaded one way: a force in N along an axis, or a pressure.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Load {
    pub(crate) faces: Vec<usize>,
    pub(crate) pressure: bool,
    /// As typed: N for a force; MPa, or a value with its unit ("250 kPa"), for a pressure.
    pub(crate) value: String,
    /// Into [`AXES`] (a force only).
    pub(crate) axis: usize,
}

/// The panel's setup and the run it started.
#[derive(Default)]
pub struct State {
    /// The body every face belongs to: the first face added names it.
    pub(crate) body: Option<String>,
    pub(crate) supports: Vec<Support>,
    pub(crate) loads: Vec<Load>,
    /// Into [`GRADES`], plus one: 0 is none chosen yet, and Run waits for one.
    pub(crate) grade: usize,
    /// The last dry run: what it checked (see [`key`]) and the setup in words, or why it can't run.
    checked: Option<(u64, Result<String, String>)>,
    /// Why the last Add did nothing.
    note: Option<String>,
    /// The run started here, until it answers.
    running: Option<Receiver<ControlResponse>>,
    /// What the last run answered: its summary, or why it did not run.
    outcome: Option<Result<String, String>>,
}

impl State {
    /// The estimate's parameters, or what is still missing (in words).
    pub(crate) fn params(&self) -> Result<Value, String> {
        let mut missing = Vec::new();
        let grade = self.grade.checked_sub(1).and_then(|i| GRADES.get(i));
        if grade.is_none() {
            missing.push(tr("ui.stress_by_hand_need_grade"));
        }
        if self.supports.is_empty() {
            missing.push(tr("ui.stress_by_hand_need_support"));
        }
        if self.loads.is_empty() {
            missing.push(tr("ui.stress_by_hand_need_load"));
        }
        let (Some(body), Some(grade)) = (self.body.as_deref(), grade) else {
            return Err(trf("ui.stress_by_hand_missing", &[("what", &missing.join(", "))]));
        };
        if !missing.is_empty() {
            return Err(trf("ui.stress_by_hand_missing", &[("what", &missing.join(", "))]));
        }
        let supports: Vec<Value> =
            self.supports.iter().map(|s| json!({"face": s.faces, "type": HOLDS.get(s.hold).map_or("fixed", |h| h.0)})).collect();
        let mut loads = Vec::new();
        for (i, l) in self.loads.iter().enumerate() {
            let text = l.value.trim();
            let number = text.strip_suffix('N').unwrap_or(text).trim().parse::<f64>().ok().filter(|v| v.is_finite());
            if l.pressure {
                // A bare number is MPa; text with its unit goes as typed, for the command to read.
                let pressure = match text.parse::<f64>() {
                    Ok(v) if v.is_finite() => json!(v),
                    _ if !text.is_empty() => json!(text),
                    _ => return Err(trf("ui.stress_by_hand_bad_value", &[("n", &(i + 1).to_string())])),
                };
                loads.push(json!({"face": l.faces, "pressure": pressure}));
            } else {
                let Some(newtons) = number.filter(|v| *v != 0.0) else {
                    return Err(trf("ui.stress_by_hand_bad_value", &[("n", &(i + 1).to_string())]));
                };
                let d = AXES.get(l.axis).map_or(AXES[0].0, |a| a.0);
                loads.push(json!({"face": l.faces, "force": [d[0] * newtons, d[1] * newtons, d[2] * newtons]}));
            }
        }
        Ok(json!({"body": body, "supports": supports, "loads": loads, "material": grade.name}))
    }

    /// The selected faces, for a support or a load: all of one body, the panel's. `None`, with a
    /// note, when none is selected or one is on another body.
    fn take_faces(&mut self, selection: &[Sel]) -> Option<(String, Vec<usize>)> {
        let faces: Vec<(&str, usize)> = selection
            .iter()
            .filter_map(|s| match s {
                Sel::Face { body, index, .. } => Some((body.as_str(), *index)),
                _ => None,
            })
            .collect();
        let Some((first, _)) = faces.first() else {
            self.note = Some(tr("ui.stress_by_hand_select_faces").to_string());
            return None;
        };
        let body = self.body.clone().unwrap_or_else(|| first.to_string());
        if faces.iter().any(|(b, _)| *b != body) {
            self.note = Some(trf("ui.stress_by_hand_one_body", &[("body", &body)]));
            return None;
        }
        self.note = None;
        let mut list: Vec<usize> = faces.into_iter().map(|(_, i)| i).collect();
        list.sort_unstable();
        list.dedup();
        Some((body, list))
    }

    /// Add the selected faces as a support (held fixed).
    pub(crate) fn add_support(&mut self, selection: &[Sel]) {
        if let Some((body, faces)) = self.take_faces(selection) {
            self.body = Some(body);
            self.supports.push(Support { faces, hold: 0 });
        }
    }

    /// Add the selected faces as a load (a force down, with no value yet).
    pub(crate) fn add_load(&mut self, selection: &[Sel]) {
        if let Some((body, faces)) = self.take_faces(selection) {
            self.body = Some(body);
            self.loads.push(Load { faces, pressure: false, value: String::new(), axis: 0 });
        }
    }

    /// Forget the body once nothing uses it, so faces of another body can be added.
    fn forget_unused_body(&mut self) {
        if self.supports.is_empty() && self.loads.is_empty() {
            self.body = None;
        }
    }

    /// A run started here has not answered yet.
    pub(crate) fn running(&self) -> bool {
        self.running.is_some()
    }
}

/// What a dry run checked: the parameters and the design's revision.
fn key(app: &SolveApp, params: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    params.to_string().hash(&mut h);
    app.session.revision.hash(&mut h);
    app.session.document_id.hash(&mut h);
    h.finish()
}

/// The request Run puts on the app's queue: the person's own, free entry.
pub(crate) fn request(params: Value) -> (ControlRequest, Receiver<ControlResponse>) {
    ControlRequest::trusted_gui("engine.execute", json!({"command": CMD, "params": params}))
}

/// The Stress button, Inspect ▸ Estimate and the palette: open the Simulation panel. Without the
/// right rail (its preference off, an embed) there is nowhere to show it, and the chat opens as
/// before.
pub(crate) fn open(app: &mut SolveApp) {
    if crate::right_rail::shown(app) {
        app.preferences.right_rail_open = crate::right_rail::Item::Simulation.id().to_string();
    } else {
        crate::stress_overlay::ask_in_chat(app);
    }
}

/// Check the setup again when it changed (a dry run on a copy, which draws its supports and loads
/// on the model), and drop the drawing when the setup is no longer whole.
pub(crate) fn check(app: &mut SolveApp) {
    match app.stress_by_hand.params() {
        Ok(params) => {
            let k = key(app, &params);
            if app.stress_by_hand.checked.as_ref().is_some_and(|(at, _)| *at == k) {
                return;
            }
            let outcome = crate::stress_overlay::approval(app, "engine.execute", &json!({"command": CMD, "params": params}))
                .unwrap_or_else(|| Err(tr("ui.stress_by_hand_not_checked").to_string()));
            app.stress_by_hand.checked = Some((k, outcome));
        }
        Err(_) => {
            if app.stress_by_hand.checked.take().is_some_and(|(_, r)| r.is_ok()) {
                fea::clear_preview(&app.session.document_id);
            }
        }
    }
}

/// Is the setup whole and checked, and nothing running, so Run can start it?
pub(crate) fn ready(app: &SolveApp) -> bool {
    let s = &app.stress_by_hand;
    let checked = s.params().ok().is_some_and(|p| s.checked.as_ref().is_some_and(|(at, r)| *at == key(app, &p) && r.is_ok()));
    checked && !s.running() && fea::progress().is_none()
}

/// Run: the setup as checked, as the person's own request, on the app's queue.
pub(crate) fn run(app: &mut SolveApp) -> bool {
    if !ready(app) {
        return false;
    }
    let Ok(params) = app.stress_by_hand.params() else { return false };
    let (req, reply) = request(params);
    app.control_held.push_back(req);
    app.stress_by_hand.running = Some(reply);
    app.stress_by_hand.outcome = None;
    true
}

/// Each frame: the run's answer, once it comes, and the setup's drawing, gone when the panel is.
pub(crate) fn poll(app: &mut SolveApp, ctx: &egui::Context) {
    if let Some(rx) = app.stress_by_hand.running.as_ref() {
        let answer = match rx.try_recv() {
            Ok(reply) => Some(outcome_of(&reply)),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
                None
            }
            Err(TryRecvError::Disconnected) => Some(Err(tr("ui.stress_by_hand_no_answer").to_string())),
        };
        if let Some(answer) = answer {
            app.stress_by_hand.running = None;
            app.stress_by_hand.outcome = Some(answer);
        }
    }
    let shown = crate::right_rail::shown(app) && crate::right_rail::open(app) == Some(crate::right_rail::Item::Simulation);
    if !shown && !app.stress_by_hand.running() && app.stress_by_hand.checked.take().is_some_and(|(_, r)| r.is_ok()) {
        fea::clear_preview(&app.session.document_id);
    }
}

/// A run's reply in words: its summary, or why it did not run.
fn outcome_of(reply: &Value) -> Result<String, String> {
    if reply["ok"] == true {
        Ok(reply["result"]["summary"].as_str().unwrap_or_default().to_string())
    } else {
        Err(reply["error"].as_str().map_or_else(|| reply.to_string(), str::to_string))
    }
}

/// The panel: material, supports, loads, the setup in words, and Run.
pub(crate) fn panel(app: &mut SolveApp, ui: &mut egui::Ui, area: Rect) {
    check(app);
    let t = crate::theme::Tokens::get();
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(area.shrink2(vec2(t.space_12, t.space_8))).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui = &mut child;
    egui::ScrollArea::vertical().id_salt("sc_stress_by_hand").auto_shrink([false, true]).show(ui, |ui| form(app, ui));
}

/// A row's choice with its Remove button after it, both inside the row: laid out from the right,
/// so the dropdown takes only the room Remove leaves (it takes all the room it is given, and a
/// Remove pushed past the panel's edge would also widen everything under it). True when Remove
/// was pressed.
fn removed_with_choice(ui: &mut egui::Ui, id: &str, selected: &mut usize, options: &[&str]) -> bool {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let removed = ui_kit::button(ui, tr("ui.stress_by_hand_remove"), ui_kit::Button::Small).clicked();
        ui_kit::dropdown(ui, id, selected, options);
        removed
    })
    .inner
}

fn form(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get();
    ui_kit::text(ui, tr("ui.stress_by_hand_intro"), ui_kit::Text::Note(tr("ui.stress_by_hand_intro")));
    ui.add_space(t.space_8);

    ui_kit::section_heading(ui, tr("ui.stress_by_hand_material"));
    let mut grades: Vec<&str> = vec![tr("ui.stress_by_hand_choose_grade")];
    grades.extend(GRADES.iter().map(|g| g.name));
    let mut grade = app.stress_by_hand.grade.min(grades.len() - 1);
    ui_kit::choice_row(ui, tr("ui.stress_by_hand_grade"), "sc_stress_by_hand_grade", &mut grade, &grades);
    app.stress_by_hand.grade = grade;
    ui.add_space(t.space_8);

    ui_kit::section_heading(ui, tr("ui.stress_by_hand_supports"));
    let holds: Vec<&str> = HOLDS.iter().map(|h| tr(h.1)).collect();
    let mut remove = None;
    for (i, s) in app.stress_by_hand.supports.iter_mut().enumerate() {
        let label = trf("ui.stress_by_hand_support_n", &[("n", &(i + 1).to_string()), ("count", &s.faces.len().to_string())]);
        ui_kit::row(ui, &label, |ui| {
            if removed_with_choice(ui, &format!("sc_stress_by_hand_hold_{i}"), &mut s.hold, &holds) {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        app.stress_by_hand.supports.remove(i);
        app.stress_by_hand.forget_unused_body();
    }
    let add = ui_kit::button(ui, tr("ui.stress_by_hand_add_support"), ui_kit::Button::Normal);
    crate::scenario::publish_target("stress:add_support", add.rect);
    if add.clicked() {
        let selection = app.session.selection.clone();
        app.stress_by_hand.add_support(&selection);
    }
    ui.add_space(t.space_8);

    ui_kit::section_heading(ui, tr("ui.stress_by_hand_loads"));
    let kinds = [tr("ui.stress_by_hand_force"), tr("ui.stress_by_hand_pressure")];
    let axes: Vec<&str> = AXES.iter().map(|a| a.1).collect();
    let mut remove = None;
    for (i, l) in app.stress_by_hand.loads.iter_mut().enumerate() {
        let label = trf("ui.stress_by_hand_load_n", &[("n", &(i + 1).to_string()), ("count", &l.faces.len().to_string())]);
        let mut kind = usize::from(l.pressure);
        ui_kit::row(ui, &label, |ui| {
            if removed_with_choice(ui, &format!("sc_stress_by_hand_kind_{i}"), &mut kind, &kinds) {
                remove = Some(i);
            }
        });
        l.pressure = kind == 1;
        let (value, hint) =
            if l.pressure { (tr("ui.stress_by_hand_pressure_value"), "0.2 MPa") } else { (tr("ui.stress_by_hand_force_value"), "500 N") };
        ui_kit::text_row(ui, value, &mut l.value, hint);
        if !l.pressure {
            ui_kit::choice_row(ui, tr("ui.stress_by_hand_direction"), &format!("sc_stress_by_hand_axis_{i}"), &mut l.axis, &axes);
        }
    }
    if let Some(i) = remove {
        app.stress_by_hand.loads.remove(i);
        app.stress_by_hand.forget_unused_body();
    }
    let add = ui_kit::button(ui, tr("ui.stress_by_hand_add_load"), ui_kit::Button::Normal);
    crate::scenario::publish_target("stress:add_load", add.rect);
    if add.clicked() {
        let selection = app.session.selection.clone();
        app.stress_by_hand.add_load(&selection);
    }
    if let Some(note) = app.stress_by_hand.note.clone() {
        ui_kit::status_line(ui, &note, ui_kit::StatusTone::Warning);
    }
    ui.add_space(t.space_8);

    ui_kit::section_heading(ui, tr("ui.stress_by_hand_setup"));
    match (app.stress_by_hand.params(), app.stress_by_hand.checked.as_ref()) {
        (Err(missing), _) => {
            ui_kit::status_line(ui, &missing, ui_kit::StatusTone::Dim);
        }
        (Ok(_), Some((_, Ok(setup)))) => {
            for line in setup.lines() {
                ui_kit::status_line(ui, line, ui_kit::StatusTone::Dim);
            }
        }
        (Ok(_), Some((_, Err(why)))) => {
            ui_kit::status_line(ui, why, ui_kit::StatusTone::Error);
        }
        (Ok(_), None) => {}
    }
    ui.add_space(t.space_8);

    // Disabled, it takes the kit's plain disabled look: a dimmed primary button can't be read.
    let can = ready(app);
    let run_button = ui_kit::button(ui, tr("ui.stress_by_hand_run"), if can { ui_kit::Button::Primary } else { ui_kit::Button::Enabled(false) });
    crate::scenario::publish_target("stress:run", run_button.rect);
    if run_button.clicked() {
        run(app);
    }
    if app.stress_by_hand.running() {
        ui_kit::status_line(ui, tr("ui.stress_by_hand_running"), ui_kit::StatusTone::Dim);
    }
    match &app.stress_by_hand.outcome {
        Some(Ok(summary)) if !summary.is_empty() => {
            let summary = summary.clone();
            ui_kit::status_line(ui, &summary, ui_kit::StatusTone::Dim);
        }
        Some(Err(why)) => {
            let why = why.clone();
            ui_kit::status_line(ui, &why, ui_kit::StatusTone::Error);
        }
        _ => {}
    }
    // The chat is the agent route: offered only while the release switch lets agents ask.
    if !crate::stress_switch::ask_assistant(app) {
        return;
    }
    ui.add_space(t.space_8);
    let ask = ui_kit::button(ui, tr("ui.stress_by_hand_ask_assistant"), ui_kit::Button::Ghost);
    crate::scenario::publish_target("stress:ask_assistant", ask.rect);
    if ask.clicked() {
        crate::stress_overlay::ask_in_chat(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_engine::entitlement::EntryCategory;

    fn app_with_a_block() -> SolveApp {
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        app.run("solid.box", json!({"length": 10, "width": 20, "height": 30})).unwrap();
        app
    }

    /// The face of `Body1` with a triangle wholly at height `z`, as the view picks it.
    fn face_at_height(app: &SolveApp, z: f64) -> Sel {
        let st = crate::open_meshing::session_world(&app.session);
        let m = st.body("Body1").unwrap().mesh();
        let (tri, f) =
            m.triangles.iter().zip(&m.tri_face).find(|(t, _)| m.tri(t).is_some_and(|ps| ps.iter().all(|p| (p.z - z).abs() < 1e-9))).unwrap();
        let ps = m.tri(tri).unwrap();
        Sel::Face { body: "Body1".into(), index: *f as usize, point: (ps[0] + ps[1] + ps[2]) * (1.0 / 3.0) }
    }

    fn index(s: &Sel) -> usize {
        match s {
            Sel::Face { index, .. } => *index,
            _ => usize::MAX,
        }
    }

    /// The block held at its bottom and pushed down on its top with 500 N, in 6061-T6.
    fn set_up(app: &mut SolveApp) {
        app.session.selection = vec![face_at_height(app, 0.0)];
        let selection = app.session.selection.clone();
        app.stress_by_hand.add_support(&selection);
        app.session.selection = vec![face_at_height(app, 30.0)];
        let selection = app.session.selection.clone();
        app.stress_by_hand.add_load(&selection);
        app.stress_by_hand.loads[0].value = "500".into();
        app.stress_by_hand.grade = 1;
    }

    /// T1: the rows and the grade become the command's parameters; what is missing is said.
    #[test]
    fn the_panels_rows_become_the_estimates_parameters() {
        let mut app = app_with_a_block();
        assert!(app.stress_by_hand.params().is_err(), "nothing set up yet");
        set_up(&mut app);
        let (bottom, top) = (index(&face_at_height(&app, 0.0)), index(&face_at_height(&app, 30.0)));
        let p = app.stress_by_hand.params().unwrap();
        assert_eq!(
            p,
            json!({
                "body": "Body1",
                "supports": [{"face": [bottom], "type": "fixed"}],
                "loads": [{"face": [top], "force": [0.0, 0.0, -500.0]}],
                "material": GRADES[0].name,
            })
        );
        app.stress_by_hand.loads[0].pressure = true;
        app.stress_by_hand.loads[0].value = "250 kPa".into();
        app.stress_by_hand.supports[0].hold = 2;
        let p = app.stress_by_hand.params().unwrap();
        assert_eq!(p["loads"][0], json!({"face": [top], "pressure": "250 kPa"}));
        assert_eq!(p["supports"][0]["type"], "symmetry");
        app.stress_by_hand.grade = 0;
        assert!(app.stress_by_hand.params().is_err(), "Run waits for a grade");
        // A face of another body is not added.
        let other = Sel::Face { body: "Body2".into(), index: 0, point: solidcore_record::Vec3::new(0.0, 0.0, 0.0) };
        app.stress_by_hand.add_load(&[other]);
        assert_eq!(app.stress_by_hand.loads.len(), 1);
        assert!(app.stress_by_hand.note.is_some());
    }

    /// Drive the app's queue until `reply` answers (the copy's worker, then its settlement).
    fn answer(app: &mut SolveApp, reply: &Receiver<ControlResponse>, seconds: u64) -> Value {
        let ctx = crate::fonts::test_context();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        loop {
            app.drain_control(&ctx);
            app.poll_control_jobs(&ctx);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
            assert!(std::time::Instant::now() < until, "no answer in {seconds} s");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// T2: Run makes the person's own, free request, and the desktop's copy path (the off-thread
    /// wait on, as in the desktop app) runs it: here its dry run, which starts no program.
    #[test]
    fn a_run_by_hand_is_the_free_gui_entry_and_is_answered() {
        let mut app = app_with_a_block();
        app.apply_off_thread = Some(std::time::Duration::from_secs(1));
        set_up(&mut app);
        check(&mut app);
        // The dry run behind the panel checked the setup (and drew it: the preview is one slot for
        // the whole process, which other tests fill too, so it is not looked at here).
        assert!(ready(&app), "{:?}", app.stress_by_hand.checked);
        assert!(run(&mut app));
        assert_eq!(app.control_held.len(), 1);
        let req = &app.control_held[0];
        assert_eq!(req.entry(), EntryCategory::HumanGuiFree);
        assert!(crate::control::free_entry(req.entry()));
        assert_eq!(req.params["command"], CMD);
        // The same request as a dry run, through the same queue: answered, not refused.
        app.control_held.clear();
        app.stress_by_hand.running = None;
        let mut params = app.stress_by_hand.params().unwrap();
        params["dry_run"] = json!(true);
        let (req, reply) = request(params);
        app.control_held.push_back(req);
        let v = answer(&mut app, &reply, 60);
        assert_eq!(v["ok"], true, "{v}");
        assert!(v["result"]["setup"].as_array().is_some_and(|s| !s.is_empty()), "{v}");
    }

    /// T2b: a real solve started by hand runs and shows its result. It needs gmsh and CalculiX
    /// on this computer, as the real-solve suite does.
    #[test]
    #[ignore = "needs gmsh and CalculiX"]
    fn a_solve_started_by_hand_runs_and_shows_its_result() {
        let mut app = app_with_a_block();
        app.apply_off_thread = Some(std::time::Duration::from_secs(1));
        set_up(&mut app);
        check(&mut app);
        assert!(run(&mut app));
        let reply = app.stress_by_hand.running.take().unwrap();
        let v = answer(&mut app, &reply, 900);
        assert_eq!(v["ok"], true, "{v}");
        assert!(v["result"]["fraction_of_yield"].as_f64().is_some_and(|f| f > 0.0), "{v}");
        assert!(fea::shown(&app.session.document_id).is_some(), "the result is shown on the model");
    }

    /// T3: an agent's estimate, as its bridge makes it (and as the chat's Allow releases it), is
    /// not the free entry and is not answered free on the copy path. Until the charged admission
    /// of spec section 8 (2026-11-27) lands, that path refuses it before it starts.
    #[test]
    fn an_agents_estimate_is_not_the_free_entry() {
        let mut app = app_with_a_block();
        app.apply_off_thread = Some(std::time::Duration::from_secs(1));
        set_up(&mut app);
        let mut params = app.stress_by_hand.params().unwrap();
        params["dry_run"] = json!(true);
        let (req, reply) = ControlRequest::trusted_bridge("engine.execute", json!({"command": CMD, "params": params}));
        assert_eq!(req.entry(), EntryCategory::GuiBridgeMcp);
        assert!(!crate::control::free_entry(req.entry()));
        app.control_held.push_back(req);
        let v = answer(&mut app, &reply, 60);
        assert_ne!(v["ok"], true, "an agent's estimate is not answered as a free run: {v}");
    }

    /// T5: in the window, the Stress button opens the Simulation panel, not the chat, and the
    /// panel draws Run once the setup is whole. Without the right rail it opens the chat as before.
    #[test]
    fn the_stress_button_opens_the_simulation_panel() {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        // The harness starts on the window without the rail, which the desktop shows by default.
        h.call("ui.prefs", json!({"right_rail": true}));
        h.call("ui.resize", json!({"width": 1400, "height": 900}));
        h.app.run("solid.box", json!({"length": 10, "width": 20, "height": 30})).unwrap();
        h.frames(2);
        h.app.start_at(CMD, None);
        assert_eq!(crate::right_rail::open(&h.app), Some(crate::right_rail::Item::Simulation));
        assert!(!h.app.agent_chat.open, "the chat stays closed");
        set_up(&mut h.app);
        h.frames(2);
        assert!(ready(&h.app), "{:?}", h.app.stress_by_hand.checked);
        assert!(crate::scenario::handle_at("stress:run").is_some(), "Run is drawn");
        assert!(h.app.control_held.is_empty(), "nothing runs before Run");
        h.app.preferences.right_rail_open.clear();
        h.frames(1);
        h.app.preferences.right_rail = false;
        h.app.start_at(CMD, None);
        assert!(h.app.agent_chat.open, "no rail: the chat opens");
    }

    /// The release switch (spec decision 32) with the run by hand: at `by_hand` the Stress button
    /// opens the Simulation panel, its Run makes the person's own free request and is answered,
    /// and the panel offers no Assistant; at `agents` it does; at `off` the button opens nothing
    /// and Simulation is empty. Without the rail, the button needs `agents`, since it opens the chat.
    #[test]
    fn the_release_switch_decides_what_the_button_and_the_panel_offer() {
        for level in ["off", "by_hand", "agents"] {
            let mut h = crate::scenario::Harness::new();
            h.app.home.open = false;
            h.app.preferences.stress_estimate = level.into();
            h.call("ui.prefs", json!({"right_rail": true}));
            h.call("ui.resize", json!({"width": 1400, "height": 900}));
            h.app.run("solid.box", json!({"length": 10, "width": 20, "height": 30})).unwrap();
            h.frames(2);
            h.app.start_at(CMD, None);
            let opened = crate::right_rail::open(&h.app) == Some(crate::right_rail::Item::Simulation);
            assert_eq!(opened, level != "off", "{level}: the button opens the panel");
            assert!(!h.app.agent_chat.open, "{level}: the chat stays closed");
            // Shown by hand at `off` too (as a tester might, with the rail item): it stays empty.
            h.app.preferences.right_rail_open = crate::right_rail::Item::Simulation.id().to_string();
            set_up(&mut h.app);
            h.frames(2);
            assert_eq!(crate::scenario::handle_at("stress:run").is_some(), level != "off", "{level}: Run is drawn");
            assert_eq!(crate::scenario::handle_at("stress:ask_assistant").is_some(), level == "agents", "{level}: Ask the Assistant");
            if level != "off" {
                assert!(ready(&h.app), "{level}: {:?}", h.app.stress_by_hand.checked);
                assert!(run(&mut h.app), "{level}");
                assert_eq!(h.app.control_held.len(), 1);
                assert_eq!(h.app.control_held[0].entry(), EntryCategory::HumanGuiFree, "{level}: the person's own request");
                // Its dry run, through the same queue on the copy path, is answered: never refused.
                h.app.control_held.clear();
                h.app.stress_by_hand.running = None;
                h.app.apply_off_thread = Some(std::time::Duration::from_secs(1));
                let mut params = h.app.stress_by_hand.params().unwrap();
                params["dry_run"] = json!(true);
                let (req, reply) = request(params);
                h.app.control_held.push_back(req);
                let v = answer(&mut h.app, &reply, 60);
                assert_eq!(v["ok"], true, "{level}: {v}");
            }
            // Without the rail the button would open the chat: only at `agents`.
            h.app.preferences.right_rail_open.clear();
            h.app.preferences.right_rail = false;
            h.frames(1);
            h.app.start_at(CMD, None);
            assert_eq!(h.app.agent_chat.open, level == "agents", "{level}: no rail");
        }
    }

    /// Nothing the panel says is a verdict, as the legend's words aren't.
    #[test]
    fn the_panels_words_never_judge_the_part() {
        let en = crate::i18n::Locale::for_language("en");
        for id in [
            "ui.stress_by_hand_intro",
            "ui.stress_by_hand_run",
            "ui.stress_by_hand_running",
            "ui.stress_by_hand_setup",
            "ui.stress_by_hand_ask_assistant",
            "ui.stress_by_hand_missing",
            "ui.stress_by_hand_no_answer",
        ] {
            let w = en.format(id, &[]).to_lowercase();
            assert!(!w.starts_with("ui."), "{id}: no English text");
            for verdict in ["pass", "fail", "safe", "approved", "acceptable"] {
                assert!(!w.contains(verdict), "`{verdict}` in `{w}`");
            }
        }
    }
}
