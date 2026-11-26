//! The ghost of a next-step suggestion (INT-1458): the commands an agent gave for its step, run on
//! a copy of the design and drawn over it before the user decides. `predict.rs` asks the questions
//! and takes the keys; this file reads the commands, decides whether they may run, runs them on the
//! copy off the window thread, and lands them on the design when the user presses Tab.
//!
//! What the agent answers is data. Only commands on a list of modelling commands run, at most
//! [`MAX_COMMANDS`], and only on the copy ([`Session::scratch`]): the open design does not move
//! before Tab. A suggestion whose commands are not all on the list, fail, change nothing, open a
//! body, repeat the last step or take longer than [`TIME_LIMIT_S`] is withdrawn.
//!
//! The copy has a stop token of its own. Every copy of a design shares the design's token on
//! purpose (`Session::stop_token`), so stopping a slow ghost with that one would stop the user's
//! own work too.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};
use solidcraft_engine::doc::expr::is_bare_number;
use solidcraft_engine::doc::{Document, Feature, ModelState};
use solidcraft_engine::kernel::Stop;
use solidcraft_engine::{DocumentStamp, Session};

use super::{gate, review, words};
use crate::SolveApp;

/// A command and its parameters.
pub type Step = (String, Value);

/// The most commands in one suggestion.
pub const MAX_COMMANDS: usize = 32;
/// The most characters of an answer read, and of the question that asks for commands.
pub const MAX_ANSWER: usize = 8_000;
pub const MAX_QUESTION: usize = 8_000;
/// A ghost that takes longer to work out is given up (seconds).
pub const TIME_LIMIT_S: f64 = 3.0;
/// Sheet-metal commands may run. Off until they are measured (the list's M8).
pub const SHEET_METAL: bool = false;
/// The faces and edges of one body listed in the question, and the bodies.
const MAX_FACES: usize = 24;
const MAX_EDGES: usize = 48;
const MAX_BODIES: usize = 4;

/// The commands a suggestion may run, with their parameters as the question names them.
const SOLID: &[(&str, &str)] = &[
    ("solid.box", "{length, width, height, corner|center, operation: new|join|cut, targets: [bodies], body_name}"),
    (
        "solid.cylinder",
        "{radius|diameter, height, base: centre of one end, axis: \"X\"|\"Y\"|\"Z\", operation: new|join|cut, targets: [bodies], body_name}",
    ),
    ("solid.extrude", "{sketch, distance, operation: new|join|cut, targets: [bodies]} (a finished sketch of the History)"),
    ("solid.fillet", "{edges: [[x,y,z], ...] a point on each edge (its mid), radius, body}; one pick takes the whole tangent chain"),
    ("solid.chamfer", "{edges: [[x,y,z], ...], distance, body}"),
    ("solid.shell", "{faces: [[x,y,z]] a point on each face to remove, thickness, body}"),
    (
        "solid.hole",
        "{position: [x,y,z] on a face, diameter, depth (default through all), type: simple|counterbore|countersink (cb_diameter, cb_depth | cs_diameter, cs_angle)}",
    ),
    ("solid.pattern.rectangular", "{features: [names], dir1: [x,y,z], count1, spacing1, dir2, count2, spacing2}"),
    ("solid.pattern.circular", "{features: [names], axis: \"Z\", count}"),
    ("solid.mirror", "{features: [names] | bodies: [names], plane: \"XY\"|\"XZ\"|\"YZ\"|{origin, normal}}"),
    ("solid.combine", "{target, tools: [bodies], operation: join|cut|intersect}"),
];
const SHEET: &[(&str, &str)] = &[
    ("sheet.flange", "{edges: [[x,y,z]] on the sheet's top or bottom edges, height, angle (default 90 deg)}"),
    ("sheet.hem", "{edges: [[x,y,z]], length}"),
    ("sheet.fold", "{points: [[x,y,z], [x,y,z]] the bend line on the sheet, angle (default 90 deg), body}"),
    ("sheet.unfold", "{body}"),
    ("sheet.refold", "{body}"),
];

/// Why a suggestion's commands are not drawn: its outcome in the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not the JSON asked for, too long, or too many commands.
    Unreadable,
    /// The agent has no commands for its step.
    Empty,
    /// A command that is not on the list. Nothing ran.
    NotAllowed,
    /// A size without a unit, in a design that is not in mm. Nothing ran.
    NoUnit,
    /// The words give a hole another size than the commands do. Nothing ran.
    WordsDiffer,
    /// A command failed on the copy.
    Failed,
    NoChange,
    NotClosed,
    /// The same as the last step of History.
    Repeat,
    TooSlow,
    /// The design moved meanwhile.
    Stale,
}

impl Refused {
    pub fn outcome(self) -> &'static str {
        match self {
            Refused::Unreadable => "unreadable",
            Refused::Empty => "no commands",
            Refused::NotAllowed => "not allowed",
            Refused::NoUnit => "no unit",
            Refused::WordsDiffer => "words differ",
            Refused::Failed => "failed",
            Refused::NoChange => "no change",
            Refused::NotClosed => "not closed",
            Refused::Repeat => "repeat",
            Refused::TooSlow => "too slow",
            Refused::Stale => "stale",
        }
    }
}

/// The commands an answer gives, as it names them.
pub fn parse(reply: &str) -> Result<Vec<Step>, Refused> {
    if reply.chars().count() > MAX_ANSWER {
        return Err(Refused::Unreadable);
    }
    let start = reply.find('{').ok_or(Refused::Unreadable)?;
    // The first object that parses, scanning its possible ends from the shortest.
    let object = reply[start..]
        .match_indices('}')
        .find_map(|(end, _)| serde_json::from_str::<Value>(&reply[start..start + end + 1]).ok())
        .ok_or(Refused::Unreadable)?;
    let commands = object.get("commands").and_then(Value::as_array).ok_or(Refused::Unreadable)?;
    if commands.is_empty() {
        return Err(Refused::Empty);
    }
    if commands.len() > MAX_COMMANDS {
        return Err(Refused::Unreadable);
    }
    commands
        .iter()
        .map(|step| {
            // The key the engine reads (`command`, then `id`), so what is checked is what would run.
            let id = gate::command_of(step).ok_or(Refused::Unreadable)?;
            let params = step.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            if params.is_object() { Ok((id.to_string(), params)) } else { Err(Refused::Unreadable) }
        })
        .collect()
}

/// The commands under the names the engine registers them by, when every one of them may run.
/// `sheet`: sheet-metal commands too.
pub fn allowed(steps: Vec<Step>, sheet: bool) -> Result<Vec<Step>, Refused> {
    steps
        .into_iter()
        .map(|(id, params)| {
            let name = gate::resolve(&id).ok_or(Refused::NotAllowed)?;
            let listed = SOLID.iter().chain(SHEET.iter().filter(|_| sheet)).any(|(c, _)| *c == name);
            if listed { Ok((name.to_string(), params)) } else { Err(Refused::NotAllowed) }
        })
        .collect()
}

/// What of a command's parameters are points, whose coordinates may be lengths worked out.
const POINTS: &[&str] = &["corner", "center", "base", "position", "origin", "point", "x", "y", "z", "start", "to", "from", "fixed", "vertex"];
/// What of a command's parameters is no size: points, picks, directions, places in a list and names.
/// Counts and angles are none either.
const NO_SIZES: &[&str] = &[
    "corner",
    "center",
    "base",
    "position",
    "origin",
    "point",
    "points",
    "x",
    "y",
    "z",
    "start",
    "to",
    "from",
    "fixed",
    "vertex",
    "edge",
    "edges",
    "face",
    "faces",
    "direction",
    "dir",
    "dir1",
    "dir2",
    "axis",
    "normal",
    "x_dir",
    "y_dir",
    "plane",
    "index",
    "profiles",
    "knots",
    "taper",
    "clearance",
    "name",
    "body_name",
    "body_names",
    "body",
    "bodies",
    "face_body",
    "feature",
    "features",
    "sketch",
    "target",
    "targets",
    "tools",
    "participants",
    "components",
    "component_name",
    "operation",
    "type",
];

/// A size of `steps` has no unit, in a design that is not in mm. The engine would read it in the
/// design's unit, and the geometry the agent was given is in mm. A point's plain numbers are mm.
pub fn without_unit(doc: &Document, steps: &[Step]) -> bool {
    fn bare(doc: &Document, key: &str, v: &Value) -> bool {
        let plain = NO_SIZES.contains(&key) || key.starts_with("count") || key.contains("angle");
        match v {
            Value::Array(a) => a.iter().any(|v| bare(doc, key, v)),
            Value::Object(o) => o.iter().any(|(key, v)| bare(doc, key, v)),
            Value::Number(n) => !plain && n.as_f64() != Some(0.0),
            Value::String(s) => words::lacks_unit(doc, s) && !(plain && (is_bare_number(s) || !POINTS.contains(&key))),
            _ => false,
        }
    }
    words::other_unit(doc).is_some() && steps.iter().any(|(_, params)| bare(doc, "", params))
}

/// The design, and where in it commands put what they make: the active component and placement
/// are no part of the design's revision.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    stamp: DocumentStamp,
    component: u64,
    occurrence: Option<Vec<u64>>,
}

impl Place {
    pub fn of(session: &mut Session) -> Place {
        Place { stamp: session.document_stamp(), component: session.active_component, occurrence: session.active_occurrence_path.clone() }
    }

    /// The same design, wherever in it.
    pub fn same_design(&self, other: &Place) -> bool {
        self.stamp == other.stamp
    }
}

/// What the copy's commands came to.
pub struct Drawn {
    /// The design they were run on, and where in it: they land only there.
    pub start: Place,
    pub steps: Vec<Step>,
    before: Arc<ModelState>,
    after: Arc<ModelState>,
    /// How long they took on the copy (milliseconds).
    pub ms: f64,
}

/// Commands computing on a copy. Dropped, it stops them.
pub struct Work {
    stop: Stop,
    done: Receiver<Result<Drawn, Refused>>,
    /// Input time it started (seconds).
    pub started: f64,
    /// Commands run so far on the copy.
    pub ran: Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for Work {
    fn drop(&mut self) {
        self.stop.stop();
    }
}

impl Work {
    /// What it came to, once it is done (`None`: still computing at input time `now`).
    pub fn poll(&self, now: f64) -> Option<Result<Drawn, Refused>> {
        match self.done.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Disconnected) => Some(Err(Refused::Failed)),
            Err(TryRecvError::Empty) if now - self.started >= TIME_LIMIT_S => Some(Err(Refused::TooSlow)),
            Err(TryRecvError::Empty) => None,
        }
    }

    #[cfg(test)]
    pub fn stop_token(&self) -> Stop {
        self.stop.clone()
    }

    /// A worker that says nothing until the test does.
    #[cfg(test)]
    pub fn held(now: f64) -> (Work, std::sync::mpsc::Sender<Result<Drawn, Refused>>) {
        let (tx, done) = std::sync::mpsc::channel();
        (Work { stop: Stop::new(), done, started: now, ran: Arc::default() }, tx)
    }
}

/// Run `steps` on a copy of the open design: on a worker when the app has threads, here otherwise.
/// `wake` is called once it is done.
pub fn start(app: &mut SolveApp, steps: Vec<Step>, now: f64, wake: impl FnOnce() + Send + 'static) -> Work {
    let start = Place::of(&mut app.session);
    let copy = app.session.scratch();
    let stop = Stop::new();
    let ran: Arc<std::sync::atomic::AtomicUsize> = Arc::default();
    let (tx, done) = std::sync::mpsc::channel();
    let (token, counter) = (stop.clone(), ran.clone());
    let threads = app.apply_off_thread.is_some();
    let job = move || {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(copy, start, steps, &token, &counter))).unwrap_or(Err(Refused::Failed));
        let _ = tx.send(result);
        wake();
    };
    // Refused a thread, the suggestion is dropped (`Work::poll`: the sender is gone), not computed
    // on the window's.
    if threads {
        let _ = solidcraft_engine::kernel::kernel_thread("next-step-ghost").spawn(job);
    } else {
        job();
    }
    Work { stop, done, started: now, ran }
}

/// Run `steps` on `copy` and check what came of them.
fn compute(mut copy: Session, start: Place, steps: Vec<Step>, stop: &Stop, ran: &std::sync::atomic::AtomicUsize) -> Result<Drawn, Refused> {
    let began = crate::now_ms();
    stop.within(|| {
        let before = copy.world_state().map_err(|_| Refused::Failed)?;
        let start_doc = copy.doc.clone();
        for (id, params) in &steps {
            if stop.is_stopped() {
                return Err(Refused::TooSlow);
            }
            ran.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            copy.execute(id, params).map_err(|e| {
                log::info!("next-step ghost: {id} failed on the copy: {e}");
                Refused::Failed
            })?;
        }
        let after = copy.world_state().map_err(|_| Refused::Failed)?;
        // A value written as `name = 40` defines a parameter, which is no step of the list.
        if own_parameters(&start_doc) != own_parameters(&copy.doc) {
            return Err(Refused::NotAllowed);
        }
        if repeats(&start_doc, &copy.doc) {
            return Err(Refused::Repeat);
        }
        let closed = |state: &ModelState, name: &str| state.body(name).map(|b| b.is_closed());
        let (made, changed, removed) = review::body_changes(&before, &after);
        let opened = made.iter().chain(&changed).any(|b| now_open(closed(&before, &b.name), closed(&after, &b.name)));
        verdict(!(made.is_empty() && changed.is_empty() && removed.is_empty()), opened)?;
        Ok(Drawn { start, steps, before, after, ms: crate::now_ms() - began })
    })
}

/// A body that was closed, or is new, and is open now (`None`: not there).
fn now_open(before: Option<bool>, after: Option<bool>) -> bool {
    after == Some(false) && before != Some(false)
}

/// Whether what the commands did may be drawn: they `changed` a body, and left none open.
fn verdict(changed: bool, opened: bool) -> Result<(), Refused> {
    match (changed, opened) {
        (false, _) => Err(Refused::NoChange),
        (true, true) => Err(Refused::NotClosed),
        (true, false) => Ok(()),
    }
}

/// The parameters that are not a feature's own.
fn own_parameters(doc: &Document) -> Vec<&str> {
    doc.params.iter().filter(|p| !p.model).map(|p| p.name.as_str()).collect()
}

/// The features the commands added are the same as the ones before them in History (the marker may
/// stand before its end), name aside.
fn repeats(before: &Document, after: &Document) -> bool {
    let ids: std::collections::HashSet<u64> = before.features.iter().map(|f| f.id).collect();
    let new: Vec<&Feature> = after.features.iter().filter(|f| !ids.contains(&f.id)).collect();
    let end = before.marker.unwrap_or(before.features.len()).min(before.features.len());
    let Some(from) = end.checked_sub(new.len()).filter(|_| !new.is_empty()) else { return false };
    let kind = |f: &Feature| serde_json::to_value(&f.kind).ok();
    before.features[from..end].iter().zip(new).all(|(old, new)| !old.suppressed && kind(old).is_some() && kind(old) == kind(new))
}

/// A ghost over the design, waiting for Tab or Esc.
pub struct Ghost {
    pub drawn: Drawn,
    /// Tells this ghost's drawing from another's.
    id: u64,
    /// Its drawing was asked for (the view shows it): Tab is taken only for a ghost on screen.
    pub on_screen: bool,
    /// The bodies it would leave, measured the first time they are asked for.
    bodies: std::cell::OnceCell<Vec<Value>>,
}

impl Ghost {
    pub fn new(id: u64, drawn: Drawn) -> Ghost {
        Ghost { drawn, id, on_screen: false, bodies: std::cell::OnceCell::new() }
    }

    /// For `ui.agent_chat`: its commands, and the bodies it would leave.
    pub fn status(&self) -> Value {
        let measure = |b: &solidcraft_engine::doc::ModelBody| {
            let m = b.measure().ok();
            json!({"name": b.name, "volume": m.as_ref().filter(|_| b.is_closed()).map(|m| m.volume), "faces": m.map(|m| m.faces)})
        };
        let bodies = self.bodies.get_or_init(|| self.drawn.after.bodies.iter().map(measure).collect());
        let commands: Vec<Value> = self.drawn.steps.iter().map(|(id, params)| json!({"command": id, "params": params})).collect();
        json!({"commands": commands, "ms": self.drawn.ms, "onScreen": self.on_screen, "bodies": bodies})
    }

    /// The bodies of the copy, for the tests.
    #[cfg(test)]
    pub fn after(&self) -> &ModelState {
        &self.drawn.after
    }
}

/// What the view draws for the ghost: a key that changes whenever the drawing would, and the two
/// models ([`review::overlay`]'s shape). `None`: there is no ghost, or the design moved under it.
pub(crate) fn overlay(app: &mut SolveApp) -> Option<(u64, Arc<ModelState>, Arc<ModelState>)> {
    use std::hash::{Hash, Hasher};
    let place = Place::of(&mut app.session);
    let uid = app.session.uid;
    if app.session.active_sketch.is_some() {
        return None;
    }
    let ghost = app.agent_chat.predict.ghost.as_mut().filter(|g| g.drawn.start == place)?;
    ghost.on_screen = true;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("next_step_ghost", ghost.id, place.stamp.revision, uid).hash(&mut h);
    Some((h.finish(), ghost.drawn.before.clone(), ghost.drawn.after.clone()))
}

/// Put a ghost's steps into the design as one undo step named `label`. `Err`: the design is as it
/// was.
pub fn land(app: &mut SolveApp, drawn: &Drawn, label: &str) -> Result<(), (Refused, String)> {
    if Place::of(&mut app.session) != drawn.start || app.session.active_sketch.is_some() {
        return Err((Refused::Stale, String::new()));
    }
    let start_doc = app.session.doc.clone();
    let start_undo = app.session.undo.len();
    let checkpoint = app.session.checkpoint();
    for (command, params) in &drawn.steps {
        if let Err(e) = app.run(command, params.clone()) {
            let _ = app.session.restore_checkpoint(checkpoint);
            return Err((Refused::Failed, e));
        }
    }
    app.session.drop_checkpoint(checkpoint);
    super::merge_undo(&mut app.session.undo, &start_doc, start_undo, label);
    Ok(())
}

/// The question that asks for `step`'s commands, at most [`MAX_QUESTION`] characters: the step,
/// the commands that may be used, `history` (the design's last steps), and the faces and edges of
/// the bodies it is about.
pub fn question(session: &mut Session, step: &str, history: &str, sheet: bool) -> String {
    let units = words::other_unit(&session.doc).map_or_else(String::new, |u| {
        format!("Points are in mm. Write every size as text with its unit, for example \"{}\": one without is not used.\n", words::spoken(u).2)
    });
    let mut out = format!(
        "[Intcad next-step suggestion. Do not call any tools, do not read or change files and do not change the design. Answer with one line of JSON only.]\n\
         Write the Intcad commands that do exactly this step, as many as it takes, in order, with numbers from the geometry below. \
         Choose sensible positions where the step leaves them open.\n\
         {units}Step: {step}\n\
         Reply with exactly: {{\"commands\": [{{\"command\": \"...\", \"params\": {{}}}}]}}, or {{\"commands\": []}} if the step cannot be done with these commands.\n\
         Commands (points are [x,y,z] in the world frame; give every feature a \"name\"):\n"
    );
    for (command, params) in SOLID.iter().chain(SHEET.iter().filter(|_| sheet)) {
        out.push_str(&format!("- {command} {params}\n"));
    }
    out.push_str(history);
    out.push_str("\n\nGeometry now (mm):");
    let mut left = 0;
    for body in bodies_of(session, step) {
        for line in body_lines(session, &body, &mut left) {
            // Room is kept for the line that says how many were left out.
            if out.chars().count() + line.chars().count() + 80 > MAX_QUESTION {
                left += 1;
            } else {
                out.push_str(&line);
            }
        }
    }
    if left > 0 {
        out.push_str(&format!("\n({left} more faces and edges are not listed.)"));
    }
    out
}

/// The bodies a step is about: those it names; else those of the last History step; else all.
fn bodies_of(session: &Session, step: &str) -> Vec<String> {
    let Ok(state) = session.world_state() else { return Vec::new() };
    let all: Vec<String> = state.bodies.iter().map(|b| b.name.clone()).collect();
    let lower = step.to_lowercase();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let named = |name: &str| {
        let name = name.to_lowercase();
        !name.is_empty()
            && lower
                .match_indices(&name)
                .any(|(at, _)| !lower[..at].chars().next_back().is_some_and(word) && !lower[at + name.len()..].chars().next().is_some_and(word))
    };
    let mut about: Vec<String> = all.iter().filter(|n| named(n)).cloned().collect();
    if about.is_empty() {
        let doc = &session.doc;
        let end = doc.marker.unwrap_or(doc.features.len()).min(doc.features.len());
        if let Some(last) = end.checked_sub(1).and_then(|i| doc.features.get(i)) {
            let kind = serde_json::to_value(&last.kind).map(|v| v.to_string()).unwrap_or_default();
            about = all
                .iter()
                .filter(|n| last.body_names.contains(n) || last.participants.contains(n) || kind.contains(&format!("\"{n}\"")))
                .cloned()
                .collect();
        }
    }
    if about.is_empty() {
        about = all;
    }
    about.truncate(MAX_BODIES);
    about
}

/// A body's faces and edges as the question lists them; `left` counts those past the limits.
fn body_lines(session: &mut Session, body: &str, left: &mut usize) -> Vec<String> {
    let point = |v: &Value| {
        let n: Vec<String> =
            v.as_array().into_iter().flatten().filter_map(Value::as_f64).map(|x| format!("{}", (x * 100.0).round() / 100.0 + 0.0)).collect();
        (n.len() == 3).then(|| format!("[{}]", n.join(", ")))
    };
    let mut out = vec![format!("\nBody {body}:")];
    if let Ok(v) = session.execute("model.faces", &json!({"body": body})) {
        let faces = v.get("faces").and_then(Value::as_array).cloned().unwrap_or_default();
        *left += faces.len().saturating_sub(MAX_FACES);
        for f in faces.iter().take(MAX_FACES) {
            let at = f.get("point").and_then(point).or_else(|| f.get("centroid").and_then(point)).unwrap_or_default();
            let what = if let Some(c) = f.get("cylinder").filter(|c| !c.is_null()) {
                let radius = c.get("radius").and_then(Value::as_f64).unwrap_or(0.0);
                format!("cylinder radius {} axis {}", (radius * 1000.0).round() / 1000.0, c.get("axis").and_then(point).unwrap_or_default())
            } else if let Some(n) = f.get("plane_normal").and_then(point) {
                format!("plane normal {n}")
            } else {
                "curved".to_string()
            };
            out.push(format!("\n- face {what} at {at}"));
        }
    }
    if let Ok(v) = session.execute("model.edges", &json!({"body": body})) {
        let edges = v.get("edges").and_then(Value::as_array).cloned().unwrap_or_default();
        *left += edges.len().saturating_sub(MAX_EDGES);
        for e in edges.iter().take(MAX_EDGES) {
            let length = e.get("length").and_then(Value::as_f64).unwrap_or(0.0);
            out.push(format!("\n- edge mid {} length {}", e.get("mid").and_then(point).unwrap_or_default(), (length * 100.0).round() / 100.0));
        }
    }
    out
}

/// A number of a suggestion's commands, and where it comes from.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Mark {
    /// The parameter it is for (`radius`, `position.x`).
    pub key: String,
    pub text: String,
    /// The string naming its source; `None`: the agent guessed it.
    pub source: Option<&'static str>,
    /// It says where something goes.
    pub place: bool,
}

/// Sizes a command takes; places are points. Counts, names and picks of what is there are neither.
pub const SIZES: &[&str] = &[
    "radius",
    "diameter",
    "length",
    "width",
    "height",
    "distance",
    "thickness",
    "depth",
    "spacing1",
    "spacing2",
    "angle",
    "cb_diameter",
    "cb_depth",
    "cs_diameter",
    "cs_angle",
    "distance2",
    "radius2",
    "spacing",
];
pub const PLACES: &[&str] = &["position", "base", "corner", "center", "origin"];

/// What a size is of: a hole's and a cylinder's radius and diameter are one thing (a diameter), and
/// a fillet's radius is another, so a hole as wide as a fillet is round is not a size the design has.
fn size_of(kind: &str, key: &str, v: f64) -> (String, f64) {
    match (kind, key) {
        ("cylinder" | "hole", "radius") => ("bore".into(), 2.0 * v),
        ("cylinder" | "hole", "diameter") => ("bore".into(), v),
        _ => (format!("{kind} {key}"), v),
    }
}

/// Where each number of `steps` comes from: the design's own sizes (of the same thing) and places,
/// its parameters, what is selected, what the user said in the chat, or the project's notes.
/// Anything else was guessed.
pub fn marks(session: &Session, steps: &[Step], selection: &str, chat: &str, notes: &str) -> Vec<Mark> {
    let doc = &session.doc;
    let (mut sizes, mut places) = (Vec::new(), [Vec::new(), Vec::new(), Vec::new()]);
    for f in &doc.features {
        if let Ok(kind) = serde_json::to_value(&f.kind) {
            let name = kind.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
            collect(&kind, &name, doc, &mut sizes, &mut places);
        }
    }
    let same = |a: f64, b: f64| (a - b).abs() <= 1e-6 * a.abs().max(1.0);
    // What is selected is told in mm; a person means mm or the design's unit.
    let (mm, either) = ([1.0], [1.0, words::design_unit(doc)]);
    let texts: [(&str, &[f64], &str); 3] =
        [(selection, &mm, "ui.next_step_src_selection"), (chat, &either, "ui.next_step_src_said"), (notes, &either, "ui.next_step_src_notes")];
    let texts = texts.map(|(t, bare, s)| (words::known(t, bare), s));
    let elsewhere = |v: f64| texts.iter().find(|(known, _)| known.iter().any(|k| same(*k, v))).map(|(_, s)| *s);
    let source = |v: &Value, in_design: &dyn Fn(f64) -> bool| {
        if names_parameter(doc, v) {
            return Some("ui.next_step_src_parameter");
        }
        number(doc, v).and_then(|v| if in_design(v) { Some("ui.next_step_src_design") } else { elsewhere(v) })
    };
    let mut out = Vec::new();
    for (command, params) in steps {
        let Some(params) = params.as_object() else { continue };
        let kind = command.rsplit('.').next().unwrap_or_default();
        for (key, value) in params {
            if SIZES.contains(&key.as_str()) {
                let Some(text) = shown(value) else { continue };
                let used = |v: f64| {
                    let (what, v) = size_of(kind, key, v);
                    sizes.iter().any(|(w, k)| *w == what && same(*k, v))
                };
                out.push(Mark { key: key.clone(), text, source: source(value, &used), place: false });
            } else if let Some(point) = value.as_array().filter(|_| PLACES.contains(&key.as_str())) {
                for ((axis, known), coord) in ["x", "y", "z"].iter().zip(&places).zip(point) {
                    let Some(text) = shown(coord) else { continue };
                    let used = |v: f64| known.iter().any(|k| same(*k, v));
                    out.push(Mark { key: format!("{key}.{axis}"), text, source: source(coord, &used), place: true });
                }
            }
        }
    }
    out
}

/// The sizes (with what each is of) and the places' coordinates in the record of a feature of `kind`.
fn collect(v: &Value, kind: &str, doc: &Document, sizes: &mut Vec<(String, f64)>, places: &mut [Vec<f64>; 3]) {
    match v {
        Value::Object(o) => {
            for (key, v) in o {
                if SIZES.contains(&key.as_str()) && !v.is_object() && !v.is_array() {
                    sizes.extend(number(doc, v).map(|v| size_of(kind, key, v)));
                } else if let Some(point) = v.as_array().filter(|a| a.len() == 3 && PLACES.contains(&key.as_str())) {
                    for (known, coord) in places.iter_mut().zip(point) {
                        known.extend(number(doc, coord));
                    }
                } else {
                    collect(v, kind, doc, sizes, places);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|v| collect(v, kind, doc, sizes, places)),
        _ => {}
    }
}

/// A value as a person reads it.
fn shown(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => n.as_f64().map(|n| n.to_string()),
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().chars().take(40).collect()),
        _ => None,
    }
}

/// A value as a number, in millimetres (or degrees): an expression is worked out with the design's
/// parameters.
fn number(doc: &Document, v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok().or_else(|| words::millimetres(doc, s)),
        _ => None,
    }
}

/// An expression that names one of the design's parameters.
fn names_parameter(doc: &Document, v: &Value) -> bool {
    let Some(s) = v.as_str() else { return false };
    let words: Vec<&str> = s.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    doc.params.iter().any(|p| words.contains(&p.name.as_str()))
}

#[cfg(test)]
pub(crate) fn solid_commands() -> Vec<&'static str> {
    SOLID.iter().map(|(c, _)| *c).collect()
}

#[cfg(test)]
pub(crate) fn sheet_commands() -> Vec<&'static str> {
    SHEET.iter().map(|(c, _)| *c).collect()
}

#[cfg(test)]
pub(crate) fn verdict_of(changed: bool, before: Option<bool>, after: Option<bool>) -> Result<(), Refused> {
    verdict(changed, now_open(before, after))
}
