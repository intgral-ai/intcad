//! Preview, then accept or reject (the "Cursor for CAD" spec, feature 1). With **Preview the
//! agent's changes** on (Preferences → AI, off by default), a chat turn's design changes run on a
//! copy of the open design ([`Session::scratch`]), never on the design itself. While the turn runs
//! and after it, the view draws the copy over the design the way a feature dialog's preview does
//! (`crate::preview`): material added in the preview colour, removed material in the cut colour.
//! When the turn ends, its card lists the steps in words and the bodies made, changed and
//! removed, each with its volume. **Accept** replays the steps on the open design as one undo
//! step; **Reject** drops the copy. Either way the agent's next message says which.
//!
//! Accept replays the steps rather than swapping the copy in. That choice is provisional
//! (docs/agent-chat.md, "Preview, then accept"): replay keeps the stale-edit guard and the
//! per-turn undo step exactly as a turn without preview has them. The open design's revision does
//! not move while the user reviews, and an edit of the user's in the meantime makes Accept refuse.
//!
//! What runs on the copy: `engine.execute`, `engine.batch`, `document.inspect` and
//! `document.revision`. The command list and pictures of the view (`engine.commands`, `ui.*`) go
//! to the window as before; a picture of the window shows the overlay. Files, new designs and
//! commands that need the window (`view.pick`, panel layout) are refused while the turn runs on a
//! copy: they would act on the open design, or on nothing.
//!
//! The copy computes off the window thread. A request for it moves the copy to a worker, which
//! runs the request, works out the copy's placed model for the overlay and sends the copy back;
//! the window meanwhile draws the last overlay, answers everything else, and shows which step
//! computes on the running card and on the view's agent chip, whose Stop puts the copy back as it
//! was before that request. Requests for the copy that arrive meanwhile wait their turn, in order.
//! The card's bodies and volumes are worked out there too, once the turn ends. Within the app's
//! wait ([`crate::dialogs::APPLY_AT_ONCE`]) a request is answered in the same frame; without
//! threads (tests, headless, the web: [`SolveApp::apply_off_thread`] unset) everything runs here.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use serde_json::{Value, json};
use solidcraft_engine::doc::{Document, ModelState};
use solidcraft_engine::{DocumentStamp, Session};

use super::{Item, gate, summary};
use crate::i18n::{format as trf, tr};
use crate::{ControlRequest, SolveApp};

/// Steps a card names one by one; the rest are counted.
const MAX_WORDS: usize = 12;
/// Bodies a card names in each list; the rest are counted.
pub const MAX_BODIES: usize = 8;
/// Chord tolerance of the volumes a card shows (mm). Shown only, never decided on.
const VOLUME_TOL: f64 = 0.05;
/// What the agent's next message says after Accept or Reject (for the agent, so not translated).
const ACCEPTED_NOTE: &str = "[The user accepted the changes you proposed in your previous message: they are now in the design.]";
const REJECTED_NOTE: &str = "[The user rejected the changes you proposed in your previous message: the design is as it was before that message.]";

/// A turn's copy of the design and what ran on it.
pub(crate) struct Copy {
    /// The copy: the design as it was when the turn began, plus the turn's steps.
    session: Session,
    /// The open design's identity and revision when the turn began. Accept refuses once the open
    /// design is anything else.
    start: DocumentStamp,
    /// The open design when the turn began (what Accept's undo step goes back to).
    start_doc: Arc<Document>,
    /// The commands that changed the copy, in order, as Accept replays them.
    steps: Vec<(String, Value)>,
    /// The replies of the steps not yet put in words (words are made on the window thread, in
    /// the user's language and units).
    unworded: Vec<Option<Value>>,
    /// Each step in words, for the card.
    words: Vec<String>,
    /// The copy's placed model, kept until the next step changes it.
    after: Option<Arc<ModelState>>,
    /// Counts the copy's changes, so the overlay knows when to draw it again.
    changes: u64,
    /// Time spent running the agent's requests on the copy (ms).
    busy_ms: f64,
}

/// Where a preview is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The turn runs: its changes go to the copy.
    Running,
    /// The turn ended with changes; the card's bodies and volumes are being worked out.
    Summing,
    /// The card waits for Accept or Reject.
    Review,
}

/// Where the copy is.
enum Slot {
    /// Here, on the window thread (boxed: the copy is large, the other states small).
    Here(Box<Copy>),
    /// On a worker, computing.
    Away(Away),
    /// Its worker ended without sending it back: nothing more can run on it.
    Lost,
}

/// What a worker computes on the copy.
enum Work {
    /// One of the agent's requests, answered once the copy is back.
    Request(ControlRequest),
    /// The card's lists, once the turn ended.
    Card,
}

/// What the worker sends back with the copy.
enum Done {
    /// The request's reply, and the copy as it was before it (for Stop).
    Reply(Value, Box<Saved>),
    /// The card's lists.
    Card(Lists),
}

type Lists = (Vec<BodyChange>, Vec<BodyChange>, Vec<BodyChange>);

/// The copy, away on a worker.
struct Away {
    done: Receiver<(Box<Copy>, Done)>,
    work: Work,
    /// When it left (ms).
    started: f64,
    /// Each step's command as the toolbar names it, and the step computing (from 1).
    labels: Vec<String>,
    now: Arc<AtomicUsize>,
    /// The person stopped it, or its turn went: the copy goes back to how it was before it.
    stopped: bool,
}

impl Away {
    /// What the chip and the running card say: the command computing, and in a batch which step
    /// it is (`3/21 · Extrude`).
    fn shown(&self) -> String {
        let total = self.labels.len();
        let now = self.now.load(Ordering::Relaxed).clamp(1, total.max(1));
        let label = self.labels.get(now - 1).cloned().unwrap_or_default();
        if total > 1 { format!("{now}/{total} · {label}") } else { label }
    }
}

/// The preview of the conversation's design: one at a time.
pub(crate) struct Preview {
    slot: Slot,
    pub(crate) phase: Phase,
    /// Ties the card in the conversation to this preview.
    pub(crate) id: u64,
    /// The app's wait for a worker (`None`: no threads, everything runs here).
    wait: Option<Duration>,
    /// Requests for the copy behind the one computing, in order.
    queued: VecDeque<ControlRequest>,
    /// What the overlay draws while the copy is away: its placed model, for its change count.
    shown: Option<(u64, Arc<ModelState>)>,
    /// The copy's steps so far, in words (for the running card, also while the copy is away).
    pub(crate) words: Vec<String>,
    /// How many steps Accept would replay.
    pub(crate) steps: usize,
    /// Time spent computing on the copy (ms).
    pub(crate) busy_ms: f64,
    /// The turn ended while the copy was away: the card is made once it is back.
    end_due: bool,
}

/// What became of a reviewed turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Waiting,
    Accepted,
    Rejected,
}

/// A body the turn made, changed or removed, with its volume before and after (mm³; `None`: not
/// there, or it could not be measured).
#[derive(Clone, Debug, PartialEq)]
pub struct BodyChange {
    pub name: String,
    pub before: Option<f64>,
    pub after: Option<f64>,
}

/// The card: what a reviewed turn would do to the design.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: u64,
    /// The steps in words, at most [`MAX_WORDS`]; `more`: how many are not named.
    pub words: Vec<String>,
    pub more: usize,
    pub made: Vec<BodyChange>,
    pub changed: Vec<BodyChange>,
    pub removed: Vec<BodyChange>,
    pub outcome: Outcome,
}

impl Copy {
    /// A copy of the open design for the turn about to start.
    pub(crate) fn of(session: &mut Session) -> Copy {
        let start = session.document_stamp();
        Copy {
            session: session.scratch(),
            start,
            start_doc: session.doc.clone(),
            steps: Vec::new(),
            unworded: Vec::new(),
            words: Vec::new(),
            after: None,
            changes: 0,
            busy_ms: 0.0,
        }
    }

    /// The copy's placed model, worked out once per change.
    fn after(&mut self) -> Option<Arc<ModelState>> {
        if self.after.is_none() {
            self.after = self.session.world_state().ok();
        }
        self.after.clone()
    }

    fn record(&mut self, id: &str, params: &Value, result: Option<&Value>) {
        self.steps.push((id.to_string(), params.clone()));
        self.unworded.push(result.cloned());
        self.after = None;
        self.changes += 1;
    }

    /// Put the steps recorded since last time in words.
    fn word(&mut self, units: summary::Units) {
        let start = self.words.len();
        for (i, result) in std::mem::take(&mut self.unworded).into_iter().enumerate() {
            let Some((id, params)) = self.steps.get(start + i) else { break };
            self.words.push(summary::step(id, params, result.as_ref(), units).unwrap_or_else(|| id.clone()));
        }
    }

    /// The copy as it is, to go back to if the request about to run is stopped. Not an engine
    /// checkpoint: those are the agent's own (`edit.checkpoint`), and taking or dropping one here
    /// would use theirs up.
    fn save(&self) -> Saved {
        let s = &self.session;
        Saved {
            at: solidcraft_engine::Snapshot {
                label: String::new(),
                doc: s.doc.clone(),
                active_sketch: s.active_sketch,
                pending_moves: s.pending_moves.clone(),
                visibility: s.visibility.clone(),
            },
            undo: s.undo.clone(),
            redo: s.redo.clone(),
            revision: s.revision,
            steps: self.steps.len(),
            changes: self.changes,
        }
    }

    /// Back to `saved`: as if the request after it never ran (its revision too, so the agent's
    /// stamp from before it still holds).
    fn restore(&mut self, saved: Saved) {
        let s = &mut self.session;
        let moved = !Arc::ptr_eq(&s.doc, &saved.at.doc) || s.active_sketch != saved.at.active_sketch || s.pending_moves != saved.at.pending_moves;
        s.doc = saved.at.doc;
        s.active_sketch = saved.at.active_sketch;
        s.pending_moves = saved.at.pending_moves;
        s.visibility = saved.at.visibility;
        s.undo = saved.undo;
        s.redo = saved.redo;
        s.selection.clear();
        if moved {
            s.refresh();
        }
        s.revision = saved.revision;
        self.steps.truncate(saved.steps);
        self.unworded.truncate(saved.steps.saturating_sub(self.words.len()));
        self.words.truncate(saved.steps);
        self.after = None;
        self.changes = saved.changes;
    }
}

/// The copy before a request ([`Copy::save`]).
struct Saved {
    at: solidcraft_engine::Snapshot,
    undo: Vec<solidcraft_engine::Snapshot>,
    redo: Vec<solidcraft_engine::Snapshot>,
    revision: u64,
    steps: usize,
    changes: u64,
}

impl Preview {
    fn here(&self) -> Option<&Copy> {
        match &self.slot {
            Slot::Here(copy) => Some(copy.as_ref()),
            _ => None,
        }
    }

    /// Copy what the window shows of the copy (words, counts, the overlay's model).
    fn mirror(&mut self, units: summary::Units) {
        let Slot::Here(copy) = &mut self.slot else { return };
        copy.word(units);
        self.words = copy.words.clone();
        self.steps = copy.steps.len();
        self.busy_ms = copy.busy_ms;
        // Worked out with the copy, on its worker. Not there (the copy went back to before a
        // stopped request): what was drawn before that request still shows the copy.
        if copy.steps.is_empty() {
            self.shown = None;
        } else if let Some(after) = copy.after.clone() {
            self.shown = Some((copy.changes, after));
        }
    }
}

/// Whether a request of the agent's runs on the turn's copy (`Some(true)`), goes to the window as
/// before (`Some(false)`), or is refused while the turn runs on a copy (`None`).
pub(crate) fn route(method: &str, params: &Value) -> Option<bool> {
    match method {
        "engine.execute" | "command" => {
            let id = crate::control::execute_command(params).unwrap_or_default();
            let id = solidcraft_engine::find_command(id).map_or(id, |spec| spec.id);
            if id == "engine.commands" {
                Some(false)
            } else if id == "view.pick" || solidcraft_engine::panels::commands::is_command(id) {
                None
            } else {
                Some(true)
            }
        }
        "engine.batch" => {
            let steps = params.get("commands").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
            let window = steps.iter().filter_map(crate::control::batch_step_command).any(|id| {
                let id = solidcraft_engine::find_command(id).map_or(id, |spec| spec.id);
                id == "view.pick" || solidcraft_engine::panels::commands::is_command(id)
            });
            (!window).then_some(true)
        }
        "document.inspect" | "document.revision" => Some(true),
        _ => Some(false),
    }
}

/// The refusal for a request that can't run while the turn's changes go to a copy.
pub(crate) fn refusal() -> Value {
    json!({"ok": false, "error": "not available while your changes are previewed on a copy of the design: the user accepts or rejects them when you finish; send the design changes only"})
}

/// A change command (one that the gate counts as a change), as Accept replays it.
fn changes_the_design(id: &str, params: &Value) -> bool {
    matches!(gate::command(id, params, usize::MAX), gate::Verdict::Run { change: true })
}

/// Run one of the agent's requests (`method`, `params`) on the copy and answer it as the window
/// would have. `now`: the step computing, counted from 1.
fn answer(copy: &mut Copy, method: &str, params: &Value, now: &AtomicUsize) -> Value {
    let started = crate::now_ms();
    let reply = answer_inner(copy, method, params, now);
    copy.busy_ms += crate::now_ms() - started;
    reply
}

fn answer_inner(copy: &mut Copy, method: &str, params: &Value, now: &AtomicUsize) -> Value {
    // The stale-edit guard holds on the copy as it does on the design: a write that names the
    // revision it read runs only on that revision of the copy.
    let expected = match crate::execution_guard::parse_expected_document(params) {
        Ok(expected) => expected,
        Err(error) => return crate::execution_guard::refusal_json(&error),
    };
    if let Some(expected) = &expected {
        let current = copy.session.document_stamp();
        // The copy numbers its own revisions; until it changes, it is the design the agent read
        // before the turn, so that design's stamp still holds.
        let unchanged = copy.changes == 0 && Arc::ptr_eq(&copy.session.doc, &copy.start_doc);
        if current != *expected && !(unchanged && *expected == copy.start) {
            let error = crate::execution_guard::GuardRefusal::Stale { expected: expected.clone(), current: Some(current) };
            return crate::execution_guard::refusal_json(&error);
        }
    }
    let level = match crate::control::request_param_check(method, params) {
        Ok(level) => level,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let mut p = params.clone();
    if let Some(o) = p.as_object_mut() {
        for key in ["allow_paths", "design_folders", "param_check", "expectedDocument"] {
            o.remove(key);
        }
    }
    let own = copy.session.param_check;
    if let Some(level) = level {
        copy.session.param_check = level;
    }
    let mut reply = run(copy, method, &p, now);
    copy.session.param_check = own;
    if let (Some(level), Some(o)) = (level, reply.as_object_mut()) {
        o.insert("param_check".into(), json!(level.name()));
    }
    if expected.is_some()
        && reply.get("ok") == Some(&Value::Bool(true))
        && let Some(o) = reply.as_object_mut()
    {
        o.insert("documentStamp".into(), crate::execution_guard::stamp_json(&copy.session.document_stamp()));
    }
    reply
}

fn ok(v: Value) -> Value {
    json!({"ok": true, "result": v})
}

fn err(e: impl std::fmt::Display) -> Value {
    json!({"ok": false, "error": e.to_string()})
}

/// A batch's steps run on the copy, counting each as it starts.
struct Counting<'a> {
    session: &'a mut Session,
    now: &'a AtomicUsize,
}

fn execute_counting(c: &mut Counting<'_>, id: &str, params: Value) -> Result<Value, String> {
    c.now.fetch_add(1, Ordering::Relaxed);
    c.session.execute(id, &params).map_err(|e| e.to_string())
}

fn execute_history(c: &mut Counting<'_>, id: &str, params: Value) -> Result<Value, String> {
    c.session.execute(id, &params).map_err(|e| e.to_string())
}

fn run(copy: &mut Copy, method: &str, p: &Value, now: &AtomicUsize) -> Value {
    match method {
        "engine.execute" | "command" => {
            let Some(id) = crate::control::execute_command(p) else { return err("missing `command`") };
            let params = p.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            now.store(1, Ordering::Relaxed);
            match copy.session.execute(id, &params) {
                Ok(v) => {
                    if changes_the_design(id, &params) {
                        copy.record(id, &params, Some(&v));
                    }
                    ok(v)
                }
                Err(e) => err(e),
            }
        }
        "engine.batch" => {
            let Some(steps) = p.get("commands").and_then(Value::as_array).filter(|s| s.len() <= 10_000) else {
                return err("engine.batch: commands must be an array of at most 10000 steps");
            };
            if let Some(why) = crate::control::batch_refusal(steps) {
                return err(why);
            }
            if let Some(refusal) = solidcraft_engine::batch_preflight::check(steps, copy.session.param_check, &[]) {
                return ok(refusal.reply(steps.len()));
            }
            let reply = solidcraft_engine::batch::run_document_steps(
                &mut Counting { session: &mut copy.session, now },
                steps,
                p,
                execute_counting,
                execute_history,
                solidcraft_engine::batch::CheckpointPolicy::Required,
            );
            // What stayed on the copy: every step, none (rolled back), or the steps before the
            // one that failed (`rollback: false`).
            let kept = match reply.get("failed") {
                None => steps.len(),
                Some(_) if reply.get("rolled_back") == Some(&Value::Bool(true)) => 0,
                Some(_) => reply.get("completed").and_then(Value::as_u64).map_or(0, |n| usize::try_from(n).unwrap_or(0)),
            };
            let results = reply.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
            for (i, step) in steps.iter().take(kept).enumerate() {
                let Some(id) = crate::control::batch_step_command(step) else { continue };
                let params = step.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
                if changes_the_design(id, &params) {
                    copy.record(id, &params, results.get(i));
                }
            }
            ok(reply)
        }
        "document.inspect" | "document.revision" => match copy.session.execute(method, p) {
            Ok(v) => ok(v),
            Err(e) => err(e),
        },
        _ => refusal(),
    }
}

/// What a worker is asked to do with the copy.
enum Ask {
    Request { method: String, params: Value },
    Card { before: Option<Arc<ModelState>> },
}

/// Do `ask` with the copy (on a worker, or here without threads). A panic in the engine is an
/// error reply: the copy always comes back.
fn compute(copy: &mut Copy, ask: Ask, now: &AtomicUsize) -> Done {
    match ask {
        Ask::Request { method, params } => {
            let saved = Box::new(copy.save());
            let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| answer(copy, &method, &params, now)))
                .unwrap_or_else(|_| err(format!("`{method}` failed on the copy of the design")));
            // The overlay's model, worked out here rather than on the window thread.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| copy.after()));
            Done::Reply(reply, saved)
        }
        Ask::Card { before } => {
            let after = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| copy.after())).ok().flatten();
            let lists = match (before, after) {
                (Some(b), Some(a)) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body_changes(&b, &a))).unwrap_or_default(),
                _ => Default::default(),
            };
            Done::Card(lists)
        }
    }
}

/// The toolbar's name for each step of a request for the copy.
fn labels(method: &str, params: &Value) -> Vec<String> {
    let name = |id: &str| solidcraft_engine::find_command(id).map_or_else(|| id.to_string(), |c| crate::i18n::tr_label(c.label).to_string());
    match method {
        "engine.batch" => {
            params.get("commands").and_then(Value::as_array).map_or(&[][..], Vec::as_slice).iter().filter_map(crate::control::batch_step_command).map(name).collect()
        }
        "engine.execute" | "command" => crate::control::execute_command(params).map(name).into_iter().collect(),
        _ => vec![method.to_string()],
    }
}

/// What became of a copy sent away.
enum Sent {
    /// Done already (within the app's wait, or here without threads).
    Back(Box<(Box<Copy>, Done, Work)>),
    /// Computing on a worker.
    Away,
    /// The copy was not here, or its worker ended without it.
    Lost,
}

/// Send the copy away to do `ask`: on a worker when the app has threads, here otherwise.
fn send_away(preview: &mut Preview, work: Work, ask: Ask, labels: Vec<String>) -> Sent {
    let Slot::Here(mut copy) = std::mem::replace(&mut preview.slot, Slot::Lost) else {
        refuse_lost(work);
        return Sent::Lost;
    };
    let now = Arc::new(AtomicUsize::new(0));
    let Some(wait) = preview.wait else {
        let done = compute(&mut copy, ask, &now);
        return Sent::Back(Box::new((copy, done, work)));
    };
    // The worker starts before the copy goes to it, so a refused thread leaves the copy here.
    let (give, take) = std::sync::mpsc::channel::<(Box<Copy>, Ask)>();
    let (tx, done) = std::sync::mpsc::channel();
    let counting = now.clone();
    let worker = std::thread::Builder::new().name("agent-preview".into()).spawn(move || {
        if let Ok((mut copy, ask)) = take.recv() {
            let done = compute(&mut copy, ask, &counting);
            let _ = tx.send((copy, done));
        }
    });
    if worker.is_err() {
        let done = compute(&mut copy, ask, &now);
        return Sent::Back(Box::new((copy, done, work)));
    }
    if let Err(std::sync::mpsc::SendError((mut copy, ask))) = give.send((copy, ask)) {
        let done = compute(&mut copy, ask, &now);
        return Sent::Back(Box::new((copy, done, work)));
    }
    let away = Away { done, work, started: crate::now_ms(), labels, now, stopped: false };
    if !wait.is_zero() {
        match away.done.recv_timeout(wait) {
            Ok((copy, done)) => return Sent::Back(Box::new((copy, done, away.work))),
            Err(RecvTimeoutError::Disconnected) => {
                refuse_lost(away.work);
                return Sent::Lost;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    preview.slot = Slot::Away(away);
    Sent::Away
}

/// The copy is gone (its worker ended without it): the request it computed is answered with an
/// error; the slot stays [`Slot::Lost`], so the turn's later requests are refused and its preview
/// is dropped when it ends.
fn refuse_lost(work: Work) {
    if let Work::Request(req) = work {
        let _ = req.reply.send(err("the copy of the design was lost while it computed; nothing was applied"));
    }
}

/// After the copy was lost: the requests waiting for it are refused, and once the turn has ended
/// the preview goes, saying so, rather than waiting for a card that cannot come.
fn lost(app: &mut SolveApp) {
    let Some(preview) = app.agent_chat.preview.as_mut() else { return };
    preview.slot = Slot::Lost;
    for req in preview.queued.drain(..) {
        let _ = req.reply.send(err("the copy of the design was lost; nothing more can be previewed in this message"));
    }
    if preview.phase != Phase::Running {
        app.agent_chat.preview = None;
        app.agent_chat.notice(tr("ui.agent_chat_review_lost").to_string(), true);
    }
}

/// Run one of the agent's requests for the copy, now or after the one computing.
pub(crate) fn hand_over(app: &mut SolveApp, req: ControlRequest) {
    let Some(preview) = app.agent_chat.preview.as_mut() else { return };
    match &preview.slot {
        Slot::Lost => {
            let _ = req.reply.send(err("the copy of the design was lost; nothing more can be previewed in this message"));
        }
        Slot::Away(_) => preview.queued.push_back(req),
        Slot::Here(_) => start_request(app, req),
    }
}

fn start_request(app: &mut SolveApp, req: ControlRequest) {
    if req.refuse_cancelled_chat() {
        return;
    }
    let Some(preview) = app.agent_chat.preview.as_mut() else { return };
    req.started.store(true, Ordering::Relaxed);
    let labels = labels(&req.method, &req.params);
    let ask = Ask::Request { method: req.method.clone(), params: req.params.clone() };
    match send_away(preview, Work::Request(req), ask, labels) {
        Sent::Back(back) => {
            let (copy, done, work) = *back;
            come_back(app, copy, done, work, false);
        }
        Sent::Away => {}
        Sent::Lost => lost(app),
    }
}

/// The copy is back with what it did: put it here, answer the request (or make the card), and go
/// on with what waited for it.
fn come_back(app: &mut SolveApp, copy: Box<Copy>, done: Done, work: Work, stopped: bool) {
    let units = summary::Units { units: &app.session.doc.units, decimals: app.preferences.length_decimals };
    let Some(preview) = app.agent_chat.preview.as_mut() else { return };
    preview.slot = Slot::Here(copy);
    match (work, done) {
        (Work::Request(req), Done::Reply(reply, saved)) => {
            // Stopped, or its turn went: the copy goes back to how it was, the reply was given.
            let gone = stopped || crate::control::chat_turn_cancelled(req.chat_turn.as_deref());
            if gone {
                if let Slot::Here(copy) = &mut preview.slot {
                    copy.restore(*saved);
                }
                if !stopped {
                    let _ = req.refuse_cancelled_chat();
                }
            } else {
                let _ = req.reply.send(reply);
            }
            preview.mirror(units);
        }
        (Work::Card, Done::Card((made, changed, removed))) => {
            preview.mirror(units);
            preview.phase = Phase::Review;
            let id = preview.id;
            let words: Vec<String> = preview.words.iter().take(MAX_WORDS).cloned().collect();
            let more = preview.words.len().saturating_sub(words.len());
            app.agent_chat.push(Item::Review(Card { id, words, more, made, changed, removed, outcome: Outcome::Waiting }));
            return;
        }
        _ => {}
    }
    go_on(app);
}

/// The copy is here: the next waiting request starts, or the card once the turn has ended.
fn go_on(app: &mut SolveApp) {
    let Some(preview) = app.agent_chat.preview.as_mut() else { return };
    if !matches!(preview.slot, Slot::Here(_)) {
        return;
    }
    if let Some(req) = preview.queued.pop_front() {
        start_request(app, req);
    } else if preview.end_due {
        preview.end_due = false;
        let before = app.session.world_state().ok();
        sum_up(app, before);
    }
}

/// Each frame: take the copy back from its worker once it is done.
pub(crate) fn poll(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(preview) = app.agent_chat.preview.as_mut() else { return };
    let Slot::Away(away) = &preview.slot else { return };
    match away.done.try_recv() {
        Err(TryRecvError::Empty) => {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        Ok((copy, done)) => {
            let Slot::Away(away) = std::mem::replace(&mut preview.slot, Slot::Lost) else { return };
            come_back(app, copy, done, away.work, away.stopped);
            ctx.request_repaint();
        }
        Err(TryRecvError::Disconnected) => {
            let Slot::Away(away) = std::mem::replace(&mut preview.slot, Slot::Lost) else { return };
            // A stopped request was answered already.
            if !away.stopped {
                refuse_lost(away.work);
            }
            lost(app);
        }
    }
}

/// Since when the copy has been computing (ms), and what, for the view's agent chip.
pub(crate) fn computing(chat: &super::AgentChat) -> Option<(f64, String)> {
    match &chat.preview.as_ref()?.slot {
        Slot::Away(away) if !away.stopped && matches!(away.work, Work::Request(_)) => Some((away.started, away.shown())),
        _ => None,
    }
}

/// Stop the agent's request computing on the copy: it is answered as stopped now, and the copy
/// goes back to how it was before it once its worker is done. `false`: nothing computes.
pub(crate) fn stop(chat: &mut super::AgentChat) -> bool {
    let Some(Slot::Away(away)) = chat.preview.as_mut().map(|p| &mut p.slot) else { return false };
    if away.stopped {
        return false;
    }
    let Work::Request(req) = &away.work else { return false };
    away.stopped = true;
    let label = away.shown();
    let _ = req.reply.send(crate::control::stopped_command_response(&req.method, &req.params, req.guarded.is_some(), &label));
    true
}

/// The bodies a turn made, changed and removed: placed bodies by name, compared by their shared
/// shape (a body the turn left alone is the very same body), with their volumes for the card.
pub fn body_changes(before: &ModelState, after: &ModelState) -> Lists {
    let volume = |b: &solidcraft_engine::doc::ModelBody| b.body.volume_at(VOLUME_TOL);
    let (mut made, mut changed, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    for a in &after.bodies {
        match before.body(&a.name) {
            None => made.push(BodyChange { name: a.name.clone(), before: None, after: volume(a) }),
            Some(b) if !b.body.same(&a.body) => {
                let (was, now) = (volume(b), volume(a));
                // A body evaluated again into an equal shape is not a change the user has to judge.
                let equal = match (was, now) {
                    (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * x.abs().max(1.0) && b.body.outline_bounds() == a.body.outline_bounds(),
                    _ => false,
                };
                if !equal {
                    changed.push(BodyChange { name: a.name.clone(), before: was, after: now });
                }
            }
            Some(_) => {}
        }
    }
    for b in &before.bodies {
        if after.body(&b.name).is_none() {
            removed.push(BodyChange { name: b.name.clone(), before: volume(b), after: None });
        }
    }
    (made, changed, removed)
}

/// Start the turn on a copy, when the preference asks for it and the turn may change the design.
pub(crate) fn begin(app: &mut SolveApp) {
    let chat = &mut app.agent_chat;
    if !app.preferences.agent_chat_preview || chat.preview.is_some() {
        return;
    }
    chat.next_preview += 1;
    let id = chat.next_preview;
    chat.preview = Some(Preview {
        slot: Slot::Here(Box::new(Copy::of(&mut app.session))),
        phase: Phase::Running,
        id,
        wait: app.apply_off_thread,
        queued: VecDeque::new(),
        shown: None,
        words: Vec::new(),
        steps: 0,
        busy_ms: 0.0,
        end_due: false,
    });
}

/// The turn ended: a copy it changed waits for the user on a card (made by [`settle`], once the
/// copy is here); an unchanged one goes.
pub(crate) fn end(chat: &mut super::AgentChat) {
    let Some(preview) = chat.preview.as_mut().filter(|p| p.phase == Phase::Running) else { return };
    match &preview.slot {
        Slot::Lost => {
            chat.preview = None;
            chat.notice(tr("ui.agent_chat_review_lost").to_string(), true);
            return;
        }
        Slot::Away(_) => {
            preview.phase = Phase::Summing;
            preview.end_due = true;
            return;
        }
        Slot::Here(copy) if copy.steps.is_empty() && preview.queued.is_empty() => {
            chat.preview = None;
            return;
        }
        Slot::Here(_) => {}
    }
    preview.phase = Phase::Summing;
    preview.end_due = true;
}

/// The turn's card: its lists are worked out with the copy (on a worker), then it waits.
fn sum_up(app: &mut SolveApp, before: Option<Arc<ModelState>>) {
    let Some(preview) = app.agent_chat.preview.as_mut() else { return };
    if preview.here().is_some_and(|c| c.steps.is_empty()) {
        app.agent_chat.preview = None;
        return;
    }
    let labels = vec![tr("ui.agent_chat_review_title").to_string()];
    match send_away(preview, Work::Card, Ask::Card { before }, labels) {
        Sent::Back(back) => {
            let (copy, done, work) = *back;
            come_back(app, copy, done, work, false);
        }
        Sent::Away => {}
        Sent::Lost => lost(app),
    }
}

/// After a turn ended: the card is made once the copy is here. Called each frame and right after
/// the turn's end.
pub(crate) fn settle(app: &mut SolveApp) {
    if app.agent_chat.preview.as_ref().is_some_and(|p| p.phase == Phase::Summing && p.end_due && matches!(p.slot, Slot::Here(_))) {
        go_on(app);
    }
}

/// A preview waits for Accept or Reject (or its card is being made).
pub(crate) fn waiting(chat: &super::AgentChat) -> bool {
    chat.preview.as_ref().is_some_and(|p| p.phase != Phase::Running)
}

/// The running turn's changes go to a copy.
pub(crate) fn running(chat: &super::AgentChat) -> bool {
    chat.preview.as_ref().is_some_and(|p| p.phase == Phase::Running)
}

/// The running card: the steps so far and what computes now. `None`: no turn runs on a copy, or
/// it has not changed it yet.
pub(crate) fn progress(chat: &super::AgentChat) -> Option<(Vec<String>, Option<(f64, String)>)> {
    let preview = chat.preview.as_ref().filter(|p| p.phase != Phase::Review)?;
    let computing = computing(chat);
    (preview.steps > 0 || computing.is_some()).then(|| (preview.words.clone(), computing))
}

fn set_outcome(chat: &mut super::AgentChat, id: u64, outcome: Outcome) {
    for item in &mut chat.items {
        if let Item::Review(card) = item
            && card.id == id
        {
            card.outcome = outcome.clone();
        }
    }
}

/// Accept: replay the turn's steps on the open design as one undo step, unless the design changed
/// since the preview began. `Err`: why nothing changed.
pub fn accept(app: &mut SolveApp, id: u64) -> Result<(), String> {
    let Some(copy) = app.agent_chat.preview.as_ref().filter(|p| p.id == id && p.phase == Phase::Review).and_then(Preview::here) else {
        return Err(tr("ui.agent_chat_review_gone").to_string());
    };
    if app.session.document_stamp() != copy.start {
        let why = tr("ui.agent_chat_review_stale").to_string();
        app.agent_chat.notice(why.clone(), true);
        return Err(why);
    }
    let steps = copy.steps.clone();
    let start_doc = copy.start_doc.clone();
    let start_undo = app.session.undo.len();
    let label = app.agent_chat.items.iter().rev().find_map(|i| match i {
        Item::User { text, .. } => Some(text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(48).collect::<String>()),
        _ => None,
    });
    let checkpoint = app.session.checkpoint();
    for (i, (command, params)) in steps.iter().enumerate() {
        if let Err(e) = app.run(command, params.clone()) {
            let _ = app.session.restore_checkpoint(checkpoint);
            let why = trf("ui.agent_chat_review_failed", &[("n", &(i + 1).to_string()), ("error", &e)]);
            app.agent_chat.notice(why.clone(), true);
            return Err(why);
        }
    }
    app.session.drop_checkpoint(checkpoint);
    let label = trf("ui.agent_chat_undo_label", &[("message", label.as_deref().unwrap_or(""))]);
    let merged = super::merge_undo(&mut app.session.undo, &start_doc, start_undo, &label);
    let chat = &mut app.agent_chat;
    chat.preview = None;
    set_outcome(chat, id, Outcome::Accepted);
    chat.seen = Some(app.session.doc.clone());
    chat.review_note = Some(ACCEPTED_NOTE.to_string());
    if merged > 0 {
        chat.push(Item::Changes { steps: merged, doc: app.session.doc.clone(), undone: false });
    }
    Ok(())
}

/// Reject: drop the copy; the open design stays as it is.
pub fn reject(app: &mut SolveApp, id: u64) {
    let chat = &mut app.agent_chat;
    if chat.preview.as_ref().is_none_or(|p| p.id != id || p.phase != Phase::Review) {
        return;
    }
    chat.preview = None;
    set_outcome(chat, id, Outcome::Rejected);
    chat.seen = Some(app.session.doc.clone());
    chat.review_note = Some(REJECTED_NOTE.to_string());
}

/// Drop a preview without a word to the agent (New chat: its session starts over). A copy away on
/// a worker is left to finish unseen; requests waiting for it are refused.
pub(crate) fn forget(chat: &mut super::AgentChat) {
    if let Some(preview) = chat.preview.take() {
        for req in preview.queued {
            let _ = req.reply.send(refusal());
        }
        if let Slot::Away(Away { work: Work::Request(req), stopped: false, .. }) = preview.slot {
            let _ = req.reply.send(refusal());
        }
        set_outcome(chat, preview.id, Outcome::Rejected);
    }
    chat.review_note = None;
}

/// What the view draws over the design while a turn's changes are previewed: a key that changes
/// whenever the drawing would, the open design's placed model, and the copy's (as last worked
/// out: while the copy computes, the overlay stays as it was).
pub(crate) fn overlay(app: &mut SolveApp) -> Option<(u64, Arc<ModelState>, Arc<ModelState>)> {
    let revision = app.session.revision;
    let uid = app.session.uid;
    let preview = app.agent_chat.preview.as_ref()?;
    let (changes, after) = preview.shown.clone()?;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ("agent_chat_preview", preview.id, changes, revision, uid).hash(&mut h);
        h.finish()
    };
    let before = app.session.world_state().ok()?;
    Some((key, before, after))
}

/// The volume a card shows, in the design's units.
pub fn volume_text(mm3: Option<f64>, units: summary::Units) -> String {
    mm3.and_then(|v| summary::volume_text(v, units)).unwrap_or_else(|| "?".into())
}
