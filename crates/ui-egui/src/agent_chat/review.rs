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

use std::sync::Arc;

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
    /// Each step in words, for the card.
    words: Vec<String>,
    /// The copy's placed model, kept until the next step changes it.
    after: Option<Arc<ModelState>>,
    /// Counts the copy's changes, so the overlay knows when to draw it again.
    changes: u64,
    /// Time spent running the agent's requests on the copy (ms).
    pub(crate) busy_ms: f64,
}

/// Where a preview is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The turn runs: its changes go to the copy.
    Running,
    /// The turn ended with changes: the card waits for Accept or Reject.
    Review,
}

/// The preview of the conversation's design: one at a time.
pub(crate) struct Preview {
    pub(crate) copy: Copy,
    pub(crate) phase: Phase,
    /// Ties the card in the conversation to this preview.
    pub(crate) id: u64,
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
            words: Vec::new(),
            after: None,
            changes: 0,
            busy_ms: 0.0,
        }
    }

    /// The commands Accept replays.
    pub(crate) fn steps(&self) -> &[(String, Value)] {
        &self.steps
    }

    /// The copy's placed model, worked out once per change.
    fn after(&mut self) -> Option<Arc<ModelState>> {
        if self.after.is_none() {
            self.after = self.session.world_state().ok();
        }
        self.after.clone()
    }

    fn record(&mut self, id: &str, params: &Value, result: Option<&Value>, units: summary::Units) {
        self.steps.push((id.to_string(), params.clone()));
        self.words.push(summary::step(id, params, result, units).unwrap_or_else(|| id.to_string()));
        self.after = None;
        self.changes += 1;
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

fn execute(session: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    session.execute(id, &params).map_err(|e| e.to_string())
}

/// A change command (one that the gate counts as a change), as Accept replays it.
fn changes_the_design(id: &str, params: &Value) -> bool {
    matches!(gate::command(id, params, usize::MAX), gate::Verdict::Run { change: true })
}

/// Run one of the agent's requests on the copy and answer it as the window would have.
pub(crate) fn answer(copy: &mut Copy, req: &ControlRequest, units: summary::Units) -> Value {
    let started = crate::now_ms();
    let reply = answer_inner(copy, req, units);
    copy.busy_ms += crate::now_ms() - started;
    reply
}

fn answer_inner(copy: &mut Copy, req: &ControlRequest, units: summary::Units) -> Value {
    // The stale-edit guard holds on the copy as it does on the design: a write that names the
    // revision it read runs only on that revision of the copy.
    let expected = match crate::execution_guard::parse_expected_document(&req.params) {
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
    let level = match crate::control::request_param_check(&req.method, &req.params) {
        Ok(level) => level,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let mut p = req.params.clone();
    if let Some(o) = p.as_object_mut() {
        for key in ["allow_paths", "design_folders", "param_check", "expectedDocument"] {
            o.remove(key);
        }
    }
    let own = copy.session.param_check;
    if let Some(level) = level {
        copy.session.param_check = level;
    }
    let mut reply = run(copy, &req.method, &p, units);
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

fn run(copy: &mut Copy, method: &str, p: &Value, units: summary::Units) -> Value {
    match method {
        "engine.execute" | "command" => {
            let Some(id) = crate::control::execute_command(p) else { return err("missing `command`") };
            let params = p.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            match copy.session.execute(id, &params) {
                Ok(v) => {
                    if changes_the_design(id, &params) {
                        copy.record(id, &params, Some(&v), units);
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
                &mut copy.session,
                steps,
                p,
                execute,
                execute,
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
                    copy.record(id, &params, results.get(i), units);
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

/// The bodies a turn made, changed and removed: placed bodies by name, compared by their shared
/// shape (a body the turn left alone is the very same body), with their volumes for the card.
pub fn body_changes(before: &ModelState, after: &ModelState) -> (Vec<BodyChange>, Vec<BodyChange>, Vec<BodyChange>) {
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
    chat.preview = Some(Preview { copy: Copy::of(&mut app.session), phase: Phase::Running, id });
}

/// The turn ended: a copy it changed waits for the user on a card; an unchanged one goes.
pub(crate) fn end(chat: &mut super::AgentChat, session: &mut Session) {
    let Some(preview) = chat.preview.as_mut().filter(|p| p.phase == Phase::Running) else { return };
    if preview.copy.steps.is_empty() {
        chat.preview = None;
        return;
    }
    preview.phase = Phase::Review;
    let id = preview.id;
    let before = session.world_state().ok();
    let after = preview.copy.after();
    let (made, changed, removed) = match (before, after) {
        (Some(b), Some(a)) => body_changes(&b, &a),
        _ => Default::default(),
    };
    let words: Vec<String> = preview.copy.words.iter().take(MAX_WORDS).cloned().collect();
    let more = preview.copy.words.len().saturating_sub(words.len());
    chat.push(Item::Review(Card { id, words, more, made, changed, removed, outcome: Outcome::Waiting }));
}

/// A preview waits for Accept or Reject.
pub(crate) fn waiting(chat: &super::AgentChat) -> bool {
    chat.preview.as_ref().is_some_and(|p| p.phase == Phase::Review)
}

/// The running turn's changes go to a copy.
pub(crate) fn running(chat: &super::AgentChat) -> bool {
    chat.preview.as_ref().is_some_and(|p| p.phase == Phase::Running)
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
    let Some(preview) = app.agent_chat.preview.as_ref().filter(|p| p.id == id && p.phase == Phase::Review) else {
        return Err(tr("ui.agent_chat_review_gone").to_string());
    };
    if app.session.document_stamp() != preview.copy.start {
        let why = tr("ui.agent_chat_review_stale").to_string();
        app.agent_chat.notice(why.clone(), true);
        return Err(why);
    }
    let steps = preview.copy.steps.clone();
    let start_doc = preview.copy.start_doc.clone();
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

/// Drop a preview without a word to the agent (New chat: its session starts over).
pub(crate) fn forget(chat: &mut super::AgentChat) {
    if let Some(id) = chat.preview.take().map(|p| p.id) {
        set_outcome(chat, id, Outcome::Rejected);
    }
    chat.review_note = None;
}

/// What the view draws over the design while a turn's changes are previewed: a key that changes
/// whenever the drawing would, the open design's placed model, and the copy's.
pub(crate) fn overlay(app: &mut SolveApp) -> Option<(u64, Arc<ModelState>, Arc<ModelState>)> {
    let revision = app.session.revision;
    let uid = app.session.uid;
    let preview = app.agent_chat.preview.as_mut()?;
    if preview.copy.steps.is_empty() {
        return None;
    }
    let after = preview.copy.after()?;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ("agent_chat_preview", preview.id, preview.copy.changes, revision, uid).hash(&mut h);
        h.finish()
    };
    let before = app.session.world_state().ok()?;
    Some((key, before, after))
}

/// The volume a card shows, in the design's units.
pub fn volume_text(mm3: Option<f64>, units: summary::Units) -> String {
    mm3.and_then(|v| summary::volume_text(v, units)).unwrap_or_else(|| "?".into())
}
