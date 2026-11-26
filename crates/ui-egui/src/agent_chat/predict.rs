//! Next-step suggestions (INT-1458, docs/design/next-step-prediction.md): after the user edits the
//! design and pauses, the connected agent guesses one next modelling step, shown as one line, and
//! then gives the commands that make it. The app runs those on a copy and draws them over the
//! design as a ghost (`ghost.rs`). Tab puts exactly the drawn ghost into the design as one undo
//! step; Esc, another key or an edit takes the suggestion away; looking around the model does not.
//! After Tab the next one is asked for at once. Off unless turned on in Preferences → Assistant
//! (each guess is a call on the user's own agent).
//!
//! Nothing an agent says here changes the design. Each suggestion is asked for in a session of its
//! own, its two questions one after the other, and the next one's session is opened as soon as the
//! last answer is in. The sessions are
//! given no Intcad tools (no MCP bridge, no in-process tools), whose permission requests are all
//! declined, and whose replies are only read as text. The commands it gives run on a copy, and
//! only those on the app's list.
//!
//! Time comes from egui's input clock, so the tests drive the pause with scripted input.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use solidcraft_engine::doc::expr::{is_bare_number, is_literal};
use solidcraft_engine::doc::{FeatureKind, Operation};

use super::backend::{ChatBackend, ConfigOption, Event, PromptPart, Usage};
use super::context::Context;
use super::ghost::{self, Ghost, Refused, Work};
use super::{Item, State, words};
use crate::SolveApp;
use crate::i18n::{format as trf, tr};
use crate::ui_kit::BackingTooltipExt as _;

/// The pause before a guess, by default (seconds).
pub const DEFAULT_PAUSE_S: f32 = 1.5;
/// The pause the setting accepts (seconds).
pub const PAUSE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=120.0;
/// History steps described to the agent, newest last.
const RECENT_STEPS: usize = 8;
/// Undo labels described to the agent, newest last.
const RECENT_EDITS: usize = 6;
/// The user's chat messages described to the agent, and the most of each.
const CHAT_MESSAGES: usize = 6;
const CHAT_CHARS: usize = 300;
/// Values listed per kind under "usual values".
const USUAL_VALUES: usize = 3;
/// The most of the agent's reply read, and of the step and reason kept.
const MAX_REPLY: usize = 8_000;
const MAX_STEP: usize = 200;
const MAX_REASON: usize = 120;
/// The chip shows at most this much of the step (characters); the rest is in its tooltip.
const CHIP_CHARS: usize = 60;
/// A guess that takes longer is given up (seconds).
const ASK_TIMEOUT_S: f64 = 120.0;
/// Guesses kept in the log.
const MAX_LOG: usize = 200;
/// Steps the user turned down (Esc, or Reject on their preview), told to the agent so it does not
/// offer them again.
const MAX_DISMISSED: usize = 5;
/// After the guessing session fails, the wait before trying again: doubling from the first, to the
/// most (seconds).
const RETRY_FIRST_S: f64 = 30.0;
const RETRY_MOST_S: f64 = 600.0;
/// `agent_next_step_model`: the guesses run on the model the agent starts a session on.
pub const SAME_MODEL: &str = "same";
/// Words in a model's name that mark one of a provider's small models.
const SMALL_MODEL_WORDS: [&str; 6] = ["mini", "haiku", "flash", "nano", "lite", "small"];
/// Reasoning efforts tried for a guess, lowest first.
const LOW_EFFORT: [&str; 4] = ["off", "none", "minimal", "low"];
/// The built-in agent's read-only mode (Mastra Code's Explore): it has no tool that writes.
const READ_ONLY_MODE: &str = "fast";

/// Suggestions an hour on a plan sign-in, by default, and what the setting accepts.
pub const DEFAULT_HOURLY: u32 = 20;
pub const HOURLY_RANGE: std::ops::RangeInclusive<u32> = 1..=500;
const HOUR_S: f64 = 3600.0;
/// A reason for no suggestions is said for this long, once (seconds).
const SAY_S: f64 = 10.0;
/// The line is shown as soon as the agent gives the step, before its commands passed. Off: with its
/// ghost. (To be turned off if more than 5 in 100 lines have to be taken back.)
const LINE_FIRST: bool = true;

/// What a guess is asked about: the design as it is, and the component active in it. One question
/// per edit; selecting things, looking around or activating a component asks nothing new.
type Key = ghost::Place;

/// The guessing session's own state.
#[derive(Clone, Debug, Default, PartialEq)]
enum Link {
    /// No guessing session.
    #[default]
    None,
    /// Started, not ready for a question yet.
    Starting,
    /// Ready, and its context is empty.
    Fresh,
    /// It was asked a suggestion's first question, and may be asked its second; the next suggestion
    /// starts a fresh session first.
    Used,
    /// It could not run (it ended, or needs a sign-in) at input time `at`; tried again after a
    /// wait that doubles with each failure in a row.
    Failed { why: String, sign_in: bool, at: f64 },
}

impl Link {
    /// A session is there or on its way: a question can wait for it.
    fn live(&self) -> bool {
        matches!(self, Link::Starting | Link::Fresh | Link::Used)
    }
}

/// What a question out to the agent asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// The step, in words.
    Step,
    /// The commands of the step shown.
    Commands,
}

/// A question out to the agent.
struct Ask {
    id: u64,
    kind: Kind,
    started: f64,
    prompt: String,
    /// Sent to the agent (else it waits for a fresh session).
    sent: bool,
    reply: String,
    /// The design was edited meanwhile: its answer is not shown.
    dropped: bool,
    /// The design and the selection it was asked about.
    key: Key,
    selection: Vec<solidcraft_engine::Sel>,
    /// The project brief it carried, if the design is in a project.
    brief: Option<super::project_brief::Brief>,
}

/// The guess shown: the line, and the ghost once its commands are drawn.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Guess {
    pub step: String,
    pub reason: String,
    /// The design's project, by name, when the question carried its brief.
    pub project: Option<String>,
    /// The project memory the agent says this step follows: each note's title and text.
    pub memory: Vec<(String, String)>,
    /// Where each number of its commands comes from, once they are drawn (`ghost::marks`).
    pub marks: Vec<ghost::Mark>,
    #[serde(skip)]
    id: u64,
}

/// What one question took, as the agent reported it (nothing is estimated).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spent {
    pub ms: Option<u64>,
    pub billed: bool,
    pub prompt_chars: usize,
    pub reply_chars: usize,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub context_tokens: Option<u64>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
}

/// What the Intcad account says of suggestions. A build without accounts needs none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Account {
    #[default]
    NotNeeded,
    Allowed,
    Denied,
    /// Not known yet, or no longer (offline): as denied.
    Unknown,
}

impl Account {
    fn allows(self) -> bool {
        matches!(self, Account::NotNeeded | Account::Allowed)
    }
}

/// One guess, for the time and cost of each (NP-6).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Logged {
    pub id: u64,
    /// When it was asked (Unix time, milliseconds).
    pub at_unix_ms: u64,
    /// From asking to the agent's last word (`None`: still out).
    pub ms: Option<u64>,
    /// `asking`, `shown` (the line), `ghost` (its ghost too), `accepted` (Tab), `dismissed`,
    /// `stale` (the design moved), `empty` (no step), `timeout`, `failed`, and why the app withdrew
    /// it ([`Refused::outcome`]).
    pub outcome: String,
    /// The question reached the agent, so its account was charged for it.
    pub billed: bool,
    pub step: String,
    pub reason: String,
    pub prompt_chars: usize,
    pub reply_chars: usize,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub context_tokens: Option<u64>,
    /// The guessing session's cost as the agent reported it, and its currency.
    pub cost: Option<f64>,
    pub currency: Option<String>,
    /// The model the question was asked on, when the agent lists its models.
    pub model: Option<String>,
    /// The question that asked for the step's commands, once it was asked.
    pub commands: Option<Spent>,
    /// How long the commands took on the copy, and how many ran there.
    pub engine_ms: Option<f64>,
    pub ran: usize,
    /// From asking to the line, and to the ghost (milliseconds).
    pub line_ms: Option<u64>,
    pub ghost_ms: Option<u64>,
}

#[derive(Default)]
pub struct Predictor {
    backend: Option<Box<dyn ChatBackend>>,
    link: Link,
    /// Failures in a row, for the wait before the next try.
    failures: u32,
    /// The agent command the guessing session runs (the chat's own).
    argv: Vec<String>,
    /// The design and selection the pause is counted in, and when the user last did something to
    /// them (input time, seconds).
    seen: Option<Key>,
    seen_selection: Vec<solidcraft_engine::Sel>,
    last_action: f64,
    /// The design last asked about: not asked again until it is edited.
    asked: Option<Key>,
    ask: Option<Ask>,
    shown: Option<Guess>,
    /// The shown step's commands computing on a copy, then drawn.
    work: Option<Work>,
    pub(crate) ghost: Option<Ghost>,
    /// The suggestion was taken: the next question goes without a pause.
    at_once: bool,
    /// Lines shown, and those the app took back because their commands did not pass.
    lines: usize,
    withdrawn: usize,
    /// When this hour's suggestions were asked for (input time), and whether the user was told that
    /// they are used up.
    hour: std::collections::VecDeque<f64>,
    told: bool,
    /// The line shows only with its ghost ([`LINE_FIRST`] off).
    ghost_first: bool,
    /// Suggestions with any ACP agent (the tests of the built-in agent's guessing session).
    any_agent: bool,
    /// Sheet-metal commands may run ([`ghost::SHEET_METAL`]).
    sheet: bool,
    /// What the account says of suggestions.
    pub account: Account,
    /// Input time now, and when each reason for no suggestions was first said: each is said once.
    now: f64,
    said: std::collections::BTreeMap<&'static str, f64>,
    log: Vec<Logged>,
    next_id: u64,
    /// Steps the user turned down, newest last.
    dismissed: Vec<String>,
    /// The chip was drawn last frame (Tab is taken only for a guess the user can see), and was
    /// clicked.
    pub(crate) drawn: bool,
    clicked: bool,
    /// A text field had the focus last frame.
    typing: bool,
    /// Pointer presses lately (where, input time), for the demo pointer's click ripple.
    pub(crate) presses: Vec<(egui::Pos2, f64)>,
    /// Keys pressed lately (name, input time), for the demo key strip.
    pub(crate) keys: Vec<(String, f64)>,
    /// Projects the user trusts this session, by id: only theirs give the agent their notes (PRD
    /// F-6). Kept in memory for now; the PRD keeps the list in the app's preferences.
    pub(crate) trusted_projects: Vec<String>,
    /// The last question's project, for `ui.agent_chat`.
    project_seen: Option<Value>,
    /// The guessing session runs the built-in agent: it is set up with the chat's keys first, and
    /// its first session is opened once something is connected.
    builtin: bool,
    /// The guessing session runs Claude Code: its guesses default to [`CLAUDE_GUESS_MODEL`].
    claude: bool,
    /// The built-in agent's first session was asked for.
    opened: bool,
    /// The session's configuration (mode, model, reasoning effort) is being set before the first
    /// question: `ready` once the agent said the session is ready meanwhile, and the ids already
    /// set on this session, which are not set again.
    tuning: bool,
    ready: bool,
    tuned: Vec<String>,
    /// The model the session is on, as the agent last said.
    model: String,
    /// The model preference now, and the one the session was set up with: one opened ahead is not
    /// used for another.
    choice: String,
    opened_choice: String,
    /// What the session had cost when its second question went out: the agent reports the session's
    /// cost so far.
    cost_before: f64,
}

impl Drop for Predictor {
    fn drop(&mut self) {
        if let Some(mut b) = self.backend.take() {
            b.shutdown(false);
        }
    }
}

impl Predictor {
    /// The guess shown, if any.
    pub fn shown(&self) -> Option<&Guess> {
        self.shown.as_ref()
    }

    pub fn log(&self) -> &[Logged] {
        &self.log
    }

    fn entry(&mut self, id: u64) -> Option<&mut Logged> {
        self.log.iter_mut().rev().find(|l| l.id == id)
    }

    fn set_outcome(&mut self, id: u64, outcome: &str) {
        if let Some(l) = self.entry(id) {
            l.outcome = outcome.into();
        }
    }

    /// Suggestions whose questions reached the agent, and what it reported they cost.
    fn billed(&self) -> (usize, Option<f64>) {
        let billed: Vec<&Logged> = self.log.iter().filter(|l| l.billed).collect();
        let costs: Vec<f64> = billed.iter().flat_map(|l| [l.cost, l.commands.as_ref().and_then(|c| c.cost)]).flatten().collect();
        (billed.len(), (!costs.is_empty()).then(|| costs.iter().sum()))
    }

    /// The design as `key` has it was asked about.
    fn was_asked(&self, key: &Key) -> bool {
        self.asked.as_ref().is_some_and(|asked| asked.same_design(key))
    }

    /// Remember a step the user turned down, for the next questions.
    fn turned_down(&mut self, step: String) {
        self.dismissed.retain(|d| *d != step);
        self.dismissed.push(step);
        if self.dismissed.len() > MAX_DISMISSED {
            self.dismissed.remove(0);
        }
    }

    /// Stop the guessing session and forget the guess (the setting was turned off, or the chat's
    /// agent stopped). The log stays.
    fn stop(&mut self) {
        self.withdraw("dropped");
        if let Some(ask) = self.ask.take() {
            self.finish_log(ask.id, None, "dropped", 0);
        }
        if let Some(mut b) = self.backend.take() {
            b.shutdown(false);
        }
        self.link = Link::None;
        self.argv.clear();
        self.asked = None;
        self.opened = false;
        self.new_session_state();
    }

    /// A session is asked for: its configuration is set again.
    fn new_session_state(&mut self) {
        self.tuning = false;
        self.ready = false;
        self.tuned.clear();
        self.model.clear();
        self.cost_before = 0.0;
    }

    /// Open the next suggestion's session, so that its first question does not wait for one.
    fn open_next(&mut self) {
        if let Some(b) = self.backend.as_mut() {
            b.new_session();
            self.link = Link::Starting;
            self.new_session_state();
        }
    }

    /// Set the next part of the session's configuration it still needs (`options`: what it
    /// offers now). `false`: nothing left to set.
    fn tune(&mut self, options: &[ConfigOption], choice: &str) -> bool {
        self.note_model(options);
        let default = self.claude.then_some(CLAUDE_GUESS_MODEL);
        let next = next_setting(options, choice, self.builtin, default, &self.tuned);
        match (next, self.backend.as_mut()) {
            (Some((id, value)), Some(b)) => {
                log::info!("next-step suggestions: session {id} = {value}");
                b.set_config_option(&id, &value);
                self.tuned.push(id);
                self.tuning = true;
                true
            }
            _ => {
                self.tuning = false;
                false
            }
        }
    }

    fn note_model(&mut self, options: &[ConfigOption]) {
        if let Some(o) = options.iter().find(|o| o.category == "model" || o.id == "model") {
            self.model = o.current_value.clone();
        }
    }

    /// The suggestion goes, whatever stage it is at: the question out, the line, the copy that
    /// computes (it stops) and the ghost.
    fn withdraw(&mut self, outcome: &str) {
        if let Some(work) = self.work.take() {
            let ran = work.ran.load(std::sync::atomic::Ordering::Relaxed);
            if let Some(l) = self.shown.as_ref().map(|g| g.id).and_then(|id| self.entry(id)) {
                l.ran = ran;
            }
        }
        self.ghost = None;
        if let Some(g) = self.shown.take() {
            self.set_outcome(g.id, outcome);
        }
        self.cancel_ask(outcome);
    }

    /// The app takes the line back: its commands did not pass.
    fn refuse(&mut self, why: Refused) {
        self.withdrawn += 1;
        self.withdraw(why.outcome());
    }

    /// The guess whose line shows now.
    fn line(&self) -> Option<&Guess> {
        self.shown.as_ref().filter(|_| (LINE_FIRST && !self.ghost_first) || self.ghost.is_some())
    }

    /// Something of a suggestion is there: a question out, a line or a ghost.
    fn active(&self) -> bool {
        self.shown.is_some() || self.ask.as_ref().is_some_and(|a| !a.dropped)
    }

    /// The question out is given up (the design was edited, or the session stops): its answer is
    /// not shown. Its log entry stays, billed if it reached the agent.
    fn cancel_ask(&mut self, outcome: &str) {
        if let Some(ask) = self.ask.as_mut().filter(|a| !a.dropped) {
            ask.dropped = true;
            let (id, sent) = (ask.id, ask.sent);
            if sent {
                if let Some(b) = self.backend.as_mut() {
                    b.cancel();
                }
            } else {
                self.ask = None;
            }
            self.set_outcome(id, outcome);
        }
    }

    /// Note the end of the question for commands: how long it took, and what its answer was.
    fn finish_commands(&mut self, id: u64, ms: u64, reply_chars: usize) {
        if let Some(c) = self.entry(id).and_then(|l| l.commands.as_mut()) {
            c.ms = c.ms.or(Some(ms));
            c.reply_chars = reply_chars;
        }
    }

    /// Note the end of a question: how long it took and what came of it.
    fn finish_log(&mut self, id: u64, ms: Option<u64>, outcome: &str, reply_chars: usize) {
        if let Some(l) = self.entry(id) {
            l.ms = l.ms.or(ms);
            // A guess dropped while out stays dropped whatever its answer was.
            if l.outcome == "asking" {
                l.outcome = outcome.into();
            }
            l.reply_chars = reply_chars;
            log::info!(
                "next-step guess {}: {} in {} ms, billed {}, model {}, tokens in {:?} out {:?} context {:?}, cost {:?} {}",
                l.id,
                l.outcome,
                l.ms.unwrap_or(0),
                l.billed,
                l.model.as_deref().unwrap_or("?"),
                l.input_tokens,
                l.output_tokens,
                l.context_tokens,
                l.cost,
                l.currency.as_deref().unwrap_or("")
            );
        }
    }

    /// What the agent reported using, put on the question it belongs to (the latest).
    fn note_usage(&mut self, usage: Usage) {
        // A session not asked anything yet: what it reports is no question's.
        if matches!(self.link, Link::Starting | Link::Fresh) {
            return;
        }
        let out = self.ask.as_ref().map(|a| (a.id, a.kind));
        let Some(id) = out.map(|(id, _)| id).or_else(|| self.log.iter().rev().find(|l| l.billed).map(|l| l.id)) else { return };
        let before = self.cost_before;
        let Some(l) = self.entry(id) else { return };
        let second = out.map_or(l.commands.is_some(), |(_, kind)| kind == Kind::Commands);
        if let Some(c) = l.commands.as_mut().filter(|_| second) {
            c.input_tokens = usage.input_tokens.or(c.input_tokens);
            c.output_tokens = usage.output_tokens.or(c.output_tokens);
            c.total_tokens = usage.total_tokens.or(c.total_tokens);
            c.context_tokens = usage.context_tokens.or(c.context_tokens);
            if let Some((amount, currency)) = usage.cost {
                c.cost = Some((amount - before).max(0.0));
                c.currency = Some(currency);
            }
        } else {
            l.input_tokens = usage.input_tokens.or(l.input_tokens);
            l.output_tokens = usage.output_tokens.or(l.output_tokens);
            l.total_tokens = usage.total_tokens.or(l.total_tokens);
            l.context_tokens = usage.context_tokens.or(l.context_tokens);
            if let Some((amount, currency)) = usage.cost {
                l.cost = Some(amount);
                l.currency = Some(currency);
            }
        }
    }
}

/// The step as the chip shows it: at most [`CHIP_CHARS`] characters, cut at a word.
fn short(step: &str) -> String {
    if step.chars().count() <= CHIP_CHARS {
        return step.to_string();
    }
    let head: String = step.chars().take(CHIP_CHARS).collect();
    let cut = head.rfind(' ').filter(|&at| at > CHIP_CHARS / 2).map_or(head.as_str(), |at| &head[..at]);
    format!("{}…", cut.trim_end_matches([',', ';', ':', ' ']))
}

/// An amount in US dollars, with cents, or with the first figures of a fraction of a cent.
fn dollars(amount: f64) -> String {
    if amount >= 0.1 || amount == 0.0 {
        format!("${amount:.2}")
    } else if amount >= 0.01 {
        format!("${amount:.3}")
    } else {
        format!("${amount:.4}")
    }
}

/// The History strip, where the chip goes, is on screen: at the bottom (not on the left, not in
/// the Browser) and not given up to the Render workspace (which takes Tab for itself).
fn strip_shown(app: &SolveApp) -> bool {
    app.ui.show_timeline && !app.preferences.history_in_browser && !app.preferences.timeline_left && !crate::render_workspace::active(app)
}

/// The line is shown over the Assistant's message box, at the bottom centre of the window.
fn in_page(app: &SolveApp) -> bool {
    app.agent_chat.open && super::centred(app) && !crate::render_workspace::active(app)
}

/// What the page shows over its message box (`web.rs`): the step in full with what was guessed in
/// it and its reason, or why there is none; null for nothing.
pub fn page(app: &SolveApp) -> Value {
    if !app.preferences.agent_next_step || app.home.open || crate::render_workspace::active(app) {
        return Value::Null;
    }
    let p = &app.agent_chat.predict;
    if let Some(g) = p.line() {
        let mut text = g.step.clone();
        let guessed = guessed_words(&g.marks);
        if !guessed.is_empty() {
            text.push_str(&trf("ui.next_step_guessed", &[("values", &guessed.join(", "))]));
        }
        if !g.reason.is_empty() {
            text.push_str(" · ");
            text.push_str(&g.reason);
        }
        // The key is named once it does something.
        let ready = p.ghost.is_some();
        return json!({"label": tr(if ready { "ui.next_step_label_tab" } else { "ui.next_step_label" }), "text": text, "guess": true, "ready": ready});
    }
    status_words(app).map_or(Value::Null, |text| json!({"label": null, "text": text, "guess": false, "ready": false}))
}

/// The page's Tab and Esc in its empty message box, as the view's own.
pub fn from_page(app: &mut SolveApp, accept: bool) {
    if !app.agent_chat.predict.active() || app.dialog.is_some() || app.agent_chat.ask_box.open || crate::render_workspace::active(app) {
        return;
    }
    if !accept {
        turn_down(app);
    } else if app.agent_chat.predict.ghost.as_ref().is_some_and(|g| g.on_screen) {
        land(app);
    }
}

/// Why there is no suggestion, or that one is on its way. Each reason is said for a while once.
fn status_words(app: &SolveApp) -> Option<String> {
    let p = &app.agent_chat.predict;
    if let Some(why) = blocked(app) {
        let fresh = p.said.get(why).is_none_or(|at| p.now - at < SAY_S);
        return fresh.then(|| tr(why).to_string());
    }
    p.ask.as_ref().filter(|a| a.sent && !a.dropped).map(|_| tr("ui.next_step_thinking").to_string())
}

/// Why no suggestion can come now (a string's key).
fn blocked(app: &SolveApp) -> Option<&'static str> {
    let p = &app.agent_chat.predict;
    if !app.agent_chat.state().running() || app.agent_chat.argv.is_empty() {
        return Some("ui.next_step_connect");
    }
    if !supported(app) {
        return Some("ui.next_step_unsupported");
    }
    if !p.account.allows() {
        return Some("ui.next_step_account");
    }
    match &p.link {
        Link::Failed { sign_in: true, .. } => Some("ui.next_step_paused_sign_in"),
        Link::Failed { .. } => Some("ui.next_step_paused"),
        _ => None,
    }
}

/// What goes at the end of History: the guess (its words, its tooltip) or a short status, or
/// nothing. `true` with a guess.
pub fn chip(app: &SolveApp) -> Option<(String, Option<String>, bool)> {
    // The page says it where it is shown.
    if !app.preferences.agent_next_step || !strip_shown(app) || in_page(app) || app.home.open {
        return None;
    }
    let p = &app.agent_chat.predict;
    if let Some(g) = p.line() {
        let (n, cost) = p.billed();
        let cost = cost.map_or_else(|| tr("ui.next_step_cost_unreported").to_string(), dollars);
        let tip = trf("ui.next_step_tip", &[("step", &g.step), ("reason", &g.reason), ("n", &n.to_string()), ("cost", &cost)]);
        let guessed = guessed_words(&g.marks);
        let mut text = trf("ui.next_step_chip", &[("step", &short(&g.step))]);
        if !guessed.is_empty() {
            text.push_str(&trf("ui.next_step_guessed", &[("values", &guessed.join(", "))]));
        }
        let known: Vec<String> = g.marks.iter().filter_map(|m| m.source.map(|s| format!("{}: {}", m.text, tr(s)))).collect();
        let tip = if known.is_empty() { tip } else { format!("{tip}\n{}", known.join("\n")) };
        return Some((text, Some(tip), true));
    }
    status_words(app).map(|text| (text, None, false))
}

/// Paint the chip after History's last item, from `x`; the x it ends at (`x` when none).
pub(crate) fn paint_ghost(app: &mut SolveApp, ui: &egui::Ui, painter: &egui::Painter, x: f32, row: egui::Rect) -> f32 {
    let Some((text, tip, guess)) = chip(app) else { return x };
    let t = crate::theme::Tokens::get();
    let visuals = ui.visuals();
    let ink = if guess { visuals.text_color() } else { visuals.weak_text_color() };
    // The ghost of the next History item: the step's command as a not-yet-made chip, before its words.
    let left = card::ghost(app).map_or(x, |ghost| card::paint_ghost(painter, ui.ctx(), x + t.space_4, row, ghost));
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(t.font_label), ink);
    let pad = egui::vec2(t.space_4, t.space_2);
    let rect = egui::Rect::from_min_size(egui::pos2(left + t.space_4, row.center().y - galley.size().y / 2.0 - pad.y), galley.size() + pad * 2.0);
    if guess {
        painter.rect_filled(rect, t.radius_sm, visuals.widgets.hovered.weak_bg_fill);
    }
    painter.galley(rect.min + pad, galley, ink);
    crate::scenario::publish_rect("history:next_step", rect);
    crate::scenario::publish_handle("history:next_step", rect.center());
    let sense = if guess { egui::Sense::click() } else { egui::Sense::hover() };
    let mut response = ui.interact(rect, ui.id().with("sc_next_step_ghost"), sense);
    if let Some(tip) = tip {
        response = response.backing_on_hover_text(tip);
    }
    if guess {
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let p = &mut app.agent_chat.predict;
        p.drawn = true;
        p.clicked |= response.clicked();
    }
    rect.right()
}

/// Did the user do something with the keyboard this frame (a key, typing, a paste)? Pointer
/// moves, clicks, drags and the wheel are how the model is looked at: they are not. Nor are Tab and
/// Esc where they are another place's own (`ours` is false).
fn user_acted(input: &egui::InputState, ours: bool) -> bool {
    input.events.iter().any(|e| match e {
        egui::Event::Key { key: egui::Key::Tab | egui::Key::Escape, pressed, .. } => *pressed && ours,
        egui::Event::Key { pressed, .. } => *pressed,
        egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Copy | egui::Event::Cut => true,
        _ => false,
    })
}

/// Once a frame, before anything else reads the keys: Tab and Esc (or a click) on the suggestion,
/// then the pause, the questions and their answers.
pub fn tick(app: &mut SolveApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    card::note_keys(app, ctx, now);
    let visible = std::mem::take(&mut app.agent_chat.predict.drawn);
    let clicked = std::mem::take(&mut app.agent_chat.predict.clicked);
    // Typing in a text field: Tab and Esc are its own (egui takes the focus away before this frame
    // when Esc is pressed in one). The Ask Here box (Ctrl+K), a dialog and the Render workspace,
    // where no ghost is drawn, have their own too.
    let typing = ctx.text_edit_focused();
    let typed = std::mem::replace(&mut app.agent_chat.predict.typing, typing) && ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let free = !typing && !typed && app.dialog.is_none() && !app.agent_chat.ask_box.open && !crate::render_workspace::active(app);
    let mut handled = false;
    if free && app.agent_chat.predict.active() {
        let ready = app.agent_chat.predict.ghost.as_ref().is_some_and(|g| g.on_screen);
        let shown = app.agent_chat.predict.line().is_some() && (visible || ready || in_page(app));
        if shown && (clicked || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))) {
            // egui read the Tab already to move keyboard focus to the next widget: it stays put.
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            // Before the ghost is drawn Tab is used up and does nothing: nothing unseen lands.
            if ready {
                land(app);
            }
            handled = true;
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            app.esc_handled = true;
            turn_down(app);
            handled = true;
        }
    }
    let acted = !handled && ctx.input(|i| user_acted(i, free));
    app.agent_chat.predict.now = now;
    if app.preferences.agent_next_step
        && let Some(why) = blocked(app)
    {
        app.agent_chat.predict.said.entry(why).or_insert(now);
    }
    step(app, ctx, now, acted);
    // The pause ends, or a failed session's wait runs out, with no input to wake the window.
    if let Some(wait) = wake_in(app, now) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.05)));
    }
}

/// Esc: the suggestion goes, and its step is not offered again.
fn turn_down(app: &mut SolveApp) {
    let p = &mut app.agent_chat.predict;
    if let Some(step) = p.shown.as_ref().map(|g| g.step.clone()) {
        p.turned_down(step);
    }
    p.withdraw("dismissed");
}

/// Tab or a click on a drawn ghost: its steps go into the design as one undo step. Nothing is sent
/// to the agent, and there is no card: the ghost was the preview.
fn land(app: &mut SolveApp) {
    let p = &mut app.agent_chat.predict;
    let (Some(g), Some(ghost)) = (p.shown.take(), p.ghost.take()) else { return };
    // No longer allowed since it was drawn: nothing new goes into the design by it.
    if !p.account.allows() {
        p.set_outcome(g.id, "not authorised");
        return;
    }
    match ghost::land(app, &ghost.drawn, &g.step) {
        Ok(()) => {
            let p = &mut app.agent_chat.predict;
            p.set_outcome(g.id, "accepted");
            p.at_once = true;
            app.set_status(trf("ui.next_step_status_added", &[("step", &g.step)]), false);
        }
        Err((why, error)) => {
            app.agent_chat.predict.set_outcome(g.id, why.outcome());
            if !error.is_empty() {
                app.set_status(error, true);
            }
        }
    }
}

/// The chat's agent is connected, idle and an ACP agent, the chip has somewhere to go, and nothing
/// else holds the user: no turn, preview, dialog, tool, sketch or Ask Here box. Only in plain chat:
/// a skill's turn does not run as a preview, and Tab's must.
fn may_ask(app: &SolveApp) -> bool {
    let chat = &app.agent_chat;
    chat.state() == State::Ready
        && chat.skill == super::skill::Skill::Plain
        && chat.turn.is_none()
        && chat.preview.is_none()
        && !chat.ask_box.open
        && chat.pending.is_empty()
        && chat.queued.is_none()
        && app.dialog.is_none()
        && app.tool.is_none()
        && app.session.active_sketch.is_none()
        && !app.home.open
        && chat.predict.account.allows()
        && (strip_shown(app) || in_page(app))
}

/// The chat runs an agent suggestions are made with: Claude Code or Codex, over ACP.
fn supported(app: &SolveApp) -> bool {
    let chat = &app.agent_chat;
    let acp = !chat.argv.is_empty() && !chat.mastra && chat.running_endpoint.is_empty();
    acp && (chat.predict.any_agent || [CLAUDE_CODE, CODEX].contains(&chat.running_id.as_str()))
}

fn pause(app: &SolveApp) -> f64 {
    let s = app.preferences.agent_next_step_pause_s;
    f64::from(if PAUSE_RANGE.contains(&s) { s } else { DEFAULT_PAUSE_S })
}

/// The wait before trying a failed guessing session again, after `failures` in a row.
fn retry_wait(failures: u32) -> f64 {
    (RETRY_FIRST_S * 2f64.powi(i32::try_from(failures.saturating_sub(1).min(16)).unwrap_or(16))).min(RETRY_MOST_S)
}

/// Seconds until something is due with no input to wake the window: the end of the pause, or the
/// next try of a failed session.
fn wake_in(app: &SolveApp, now: f64) -> Option<f64> {
    let p = &app.agent_chat.predict;
    if !app.preferences.agent_next_step {
        return None;
    }
    if let Link::Failed { at, .. } = p.link {
        let left = at + retry_wait(p.failures) - now;
        return (left > 0.0).then_some(left);
    }
    if let Some(work) = &p.work {
        let left = work.started + ghost::TIME_LIMIT_S - now;
        return Some(left.max(0.05));
    }
    let waiting = p.ask.is_none() && p.shown.is_none() && p.seen.as_ref().is_some_and(|k| !p.was_asked(k)) && p.link.live() && may_ask(app);
    // Once the pause is over, the guessing session's own answer wakes the window.
    let left = p.last_action + pause(app) - now;
    (waiting && left > 0.0).then_some(left)
}

/// The pause, the question and the answer, at input time `now` (`acted`: the user pressed a key or
/// typed).
pub(crate) fn step(app: &mut SolveApp, ctx: &egui::Context, now: f64, acted: bool) {
    if !app.preferences.agent_next_step || !app.agent_chat.state().running() || app.home.open || !supported(app) {
        if app.agent_chat.predict.link != Link::None || app.agent_chat.predict.shown.is_some() || app.agent_chat.predict.ask.is_some() {
            app.agent_chat.predict.stop();
        }
        app.agent_chat.predict.seen = None;
        return;
    }
    let key = Key::of(&mut app.session);
    let selection = app.session.selection.clone();
    {
        // The agent works, or its preview is on the copy: the view is the chat's.
        let chats = app.agent_chat.turn.is_some() || app.agent_chat.preview.is_some() || app.agent_chat.queued.is_some();
        let pause = pause(app);
        let p = &mut app.agent_chat.predict;
        let edited = p.seen.as_ref() != Some(&key);
        if edited || acted || p.seen_selection != selection {
            p.last_action = now;
        }
        if edited {
            // An edit: what was guessed is about a design that is gone.
            p.seen = Some(key.clone());
            p.withdraw("stale");
            // The edit was the suggestion itself, taken with Tab.
            if std::mem::take(&mut p.at_once) {
                p.last_action = now - pause;
            }
        } else if p.shown.is_some() && (acted || chats) {
            p.withdraw("dismissed");
        }
        p.seen_selection = selection.clone();
    }
    start_session(app, ctx, now);
    poll(app, ctx, now, &key, &selection);
    compute(app, now);
    let p = &app.agent_chat.predict;
    let due = p.ask.is_none() && p.shown.is_none() && !p.was_asked(&key) && p.link.live() && now - p.last_action >= pause(app);
    if due && may_ask(app) && within_hour(app, now) {
        ask(app, now, key);
    }
}

/// Whether this hour's suggestions are not used up at `now`: on a plan sign-in there is a limit, on
/// an API key none. The user is told once when they are.
fn within_hour(app: &mut SolveApp, now: f64) -> bool {
    let limit = hourly(app);
    let p = &mut app.agent_chat.predict;
    while p.hour.front().is_some_and(|at| now - at >= HOUR_S) {
        p.hour.pop_front();
        p.told = false;
    }
    if p.backend.as_ref().is_some_and(|b| b.key_sign_in()) || p.hour.len() < limit {
        return true;
    }
    if !std::mem::replace(&mut p.told, true) {
        app.set_status(trf("ui.next_step_hour_used", &[("n", &limit.to_string())]), false);
    }
    false
}

fn hourly(app: &SolveApp) -> usize {
    let n = app.preferences.agent_next_step_per_hour;
    usize::try_from(if HOURLY_RANGE.contains(&n) { n } else { DEFAULT_HOURLY }).unwrap_or(usize::MAX)
}

/// Start the guessing session for the chat's agent, again for another agent, or again after a
/// failure once its wait is over and the chat's agent is ready.
fn start_session(app: &mut SolveApp, ctx: &egui::Context, now: f64) {
    if !supported(app) {
        return;
    }
    let argv = app.agent_chat.argv.clone();
    let ready = app.agent_chat.state() == State::Ready;
    let p = &mut app.agent_chat.predict;
    if p.argv != argv {
        p.stop();
        p.failures = 0;
        p.argv = argv.clone();
    }
    if let Link::Failed { at, .. } = p.link {
        if !ready || now - at < retry_wait(p.failures) {
            return;
        }
        if let Some(mut b) = p.backend.take() {
            b.shutdown(false);
        }
        p.link = Link::None;
    }
    if p.backend.is_some() {
        return;
    }
    // The built-in agent keeps its sign-ins in the app's own folder and takes its keys from the
    // app: its guessing process gets the same folder and the chat's last setup.
    let builtin = runs_builtin(app).then(|| {
        let data = app.builtin_agent.as_ref().and_then(|found| found.data.clone()).filter(|folder| std::fs::create_dir_all(folder).is_ok());
        (data, app.agent_chat.conn.last_setup.clone().unwrap_or_default())
    });
    let claude = app.agent_chat.running_id == CLAUDE_CODE;
    let p = &mut app.agent_chat.predict;
    p.builtin = builtin.is_some();
    p.claude = claude;
    p.opened = false;
    p.new_session_state();
    match start_backend(&argv, ctx, builtin) {
        Ok(b) => {
            p.backend = Some(b);
            p.link = Link::Starting;
        }
        Err(e) => {
            log::warn!("next-step suggestions could not start the agent: {e}");
            fail(p, now, e, false);
        }
    }
}

/// Whether the chat runs the agent the package carries. A package may carry it beside the agents
/// found on the computer, and the chat may run one of those.
fn runs_builtin(app: &SolveApp) -> bool {
    super::agents::is_builtin(app) && app.agent_chat.running_id == super::connections::AGENT_ID
}

/// The built-in agent's data folder (`None`: there is none) and the setup to hand it.
type BuiltinStart = (Option<std::path::PathBuf>, super::connections::Setup);

/// A session of the same agent with no way to reach the design: no MCP bridge, no tools.
/// `builtin`: the agent is the built-in one, started as the chat starts it.
#[cfg(not(target_arch = "wasm32"))]
fn start_backend(argv: &[String], ctx: &egui::Context, builtin: Option<BuiltinStart>) -> Result<Box<dyn ChatBackend>, String> {
    use super::acp_client::AcpBackend;
    let wake = ctx.clone();
    let start = super::backend::StartContext { cwd: super::working_dir(), bridge: None, tools: None, wake: Box::new(move || wake.request_repaint()) };
    let mut b = match builtin {
        None => AcpBackend::start(argv, start)?,
        Some((data, setup)) => {
            // Without its folder the agent would keep its sign-ins in, and read the agent files
            // of, the user's own home: it is not started (as the chat's is not).
            let folder = data.ok_or_else(|| tr("ui.agent_chat_no_data_folder").to_string())?;
            let mut b = AcpBackend::start_builtin(argv, start, &folder)?;
            b.connections(super::connections::Request::Setup(setup));
            b
        }
    };
    b.set_instructions(SESSION_NOTE);
    // Claude Code's guessing sessions are lean (decision 16): the note is their whole prompt.
    b.lean_sessions(SESSION_NOTE);
    Ok(Box::new(b))
}

#[cfg(target_arch = "wasm32")]
fn start_backend(_: &[String], _: &egui::Context, _: Option<BuiltinStart>) -> Result<Box<dyn ChatBackend>, String> {
    Err("desktop only".into())
}

/// The next setting the guessing session needs, from what it offers now (`choice`: the
/// `agent_next_step_model` preference): the built-in agent's read-only mode, then the guess model
/// `default` of this agent when it lists it (Claude Code's Sonnet) and else a small model, then
/// the lowest reasoning effort the
/// model takes. Ids in `tried` were set on this session already and are not set again. `same`
/// asks for nothing: the session stays as the agent opens it.
fn next_setting(options: &[ConfigOption], choice: &str, builtin: bool, default: Option<&str>, tried: &[String]) -> Option<(String, String)> {
    let choice = choice.trim();
    if choice == SAME_MODEL {
        return None;
    }
    let offers = |o: &ConfigOption, value: &str| o.options.iter().any(|c| c.value == value);
    let open = |o: &&ConfigOption| !tried.contains(&o.id);
    if builtin
        && let Some(o) = options.iter().filter(open).find(|o| o.id == "mode")
        && o.current_value != READ_ONLY_MODE
        && offers(o, READ_ONLY_MODE)
    {
        return Some((o.id.clone(), READ_ONLY_MODE.into()));
    }
    if let Some(o) = options.iter().filter(open).find(|o| o.category == "model" || o.id == "model") {
        let target = match choice {
            "" => default.filter(|model| offers(o, model)).map(str::to_string).or_else(|| small_model(o)),
            _ => offers(o, choice).then(|| choice.to_string()),
        };
        if let Some(target) = target.filter(|t| *t != o.current_value) {
            return Some((o.id.clone(), target));
        }
    }
    let o = options.iter().filter(open).find(|o| o.category == "thought_level")?;
    let low = LOW_EFFORT.iter().find(|value| offers(o, value))?;
    (o.current_value != *low).then(|| (o.id.clone(), low.to_string()))
}

/// The first small model (by name: mini, haiku, flash…) the model option lists of the provider the
/// session is on, else of any provider; `None` when it lists none or the session is on one.
fn small_model(o: &ConfigOption) -> Option<String> {
    let small = |id: &str| {
        let name = id.rsplit('/').next().unwrap_or(id).to_ascii_lowercase();
        name.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| SMALL_MODEL_WORDS.contains(&word))
    };
    if small(&o.current_value) {
        return None;
    }
    let provider = |id: &str| id.rsplit_once('/').map_or(String::new(), |(provider, _)| provider.to_string());
    let here = provider(&o.current_value);
    let smalls: Vec<&str> = o.options.iter().map(|c| c.value.as_str()).filter(|v| small(v)).collect();
    smalls.iter().find(|v| provider(v) == here).or(smalls.first()).map(|v| v.to_string())
}

/// The agent id of Claude Code in the agent list.
const CLAUDE_CODE: &str = "claude";
const CODEX: &str = "codex";

/// Claude Code's guesses run on Sonnet, not its first small model, Haiku, whose guesses read worse
/// (decision 15).
const CLAUDE_GUESS_MODEL: &str = "sonnet";

/// Added to the guessing session's system note; a lean session's whole system prompt.
const SESSION_NOTE: &str = "This session only suggests the user's next modelling step in words. It has no Intcad tools: never call a tool, never read or write files, never run commands. Answer with the JSON the message asks for and nothing else.";

/// Ask for a guess about `key`, with what is selected now.
fn ask(app: &mut SolveApp, now: f64, key: Key) {
    let context = super::context::describe_selection(&mut app.session);
    let brief = project_brief(app);
    let prompt = build_prompt(app, &context, brief.as_ref());
    let selection = app.session.selection.clone();
    let p = &mut app.agent_chat.predict;
    p.project_seen = brief.as_ref().map(|b| {
        json!({"name": b.name, "root": b.root.display().to_string(), "trusted": b.trusted,
               "notes": b.notes.iter().map(|n| n.title.clone()).collect::<Vec<_>>()})
    });
    p.next_id += 1;
    let id = p.next_id;
    let at_unix_ms = unix_ms();
    p.log.push(Logged { id, at_unix_ms, outcome: "asking".into(), prompt_chars: prompt.chars().count(), ..Default::default() });
    if p.log.len() > MAX_LOG {
        p.log.remove(0);
    }
    p.asked = Some(key.clone());
    p.ask = Some(Ask { id, kind: Kind::Step, started: now, prompt, sent: false, reply: String::new(), dropped: false, key, selection, brief });
    send_question(p);
}

/// Ask for the commands of the step just shown, in the session that gave it.
fn ask_commands(app: &mut SolveApp, now: f64, id: u64, key: Key) {
    let Some(step) = app.agent_chat.predict.shown.as_ref().map(|g| g.step.clone()) else { return };
    let history = history_block(&app.session.doc);
    let prompt = ghost::question(&mut app.session, &step, &history, ghost::SHEET_METAL || app.agent_chat.predict.sheet);
    let p = &mut app.agent_chat.predict;
    if let Some(l) = p.entry(id) {
        l.commands = Some(Spent { prompt_chars: prompt.chars().count(), ..Default::default() });
        p.cost_before = l.cost.unwrap_or(0.0);
    }
    // Its commands name what they work on, so the selection may change meanwhile.
    let selection = Vec::new();
    p.ask =
        Some(Ask { id, kind: Kind::Commands, started: now, prompt, sent: false, reply: String::new(), dropped: false, key, selection, brief: None });
    send_question(p);
}

/// Where each number of a suggestion's commands comes from: the app's own reading, whatever the
/// agent says of them.
fn marks(app: &mut SolveApp, steps: &[ghost::Step]) -> Vec<ghost::Mark> {
    let selection = super::context::describe_selection(&mut app.session).prompt_block();
    let chat: Vec<String> = app
        .agent_chat
        .items
        .iter()
        .rev()
        .filter_map(|i| if let Item::User { text, .. } = i { Some(text.clone()) } else { None })
        .take(CHAT_MESSAGES)
        .collect();
    let notes = project_brief(app).map_or_else(String::new, |b| notes_text(&b));
    ghost::marks(&app.session, steps, &selection, &chat.join("\n"), &notes)
}

/// What the project's brief says, for the numbers of a suggestion to be found in.
pub(crate) fn notes_text(b: &super::project_brief::Brief) -> String {
    let mut out = format!("{}\n{}\n{}", b.thickness_mm.map_or_else(String::new, |t| format!("{t} mm")), b.intent, b.agents);
    for n in &b.notes {
        out.push_str(&format!("\n{}\n{}", n.title, n.text));
    }
    out
}

/// The values marked as guessed, each said once.
fn guessed_words(marks: &[ghost::Mark]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in marks.iter().filter(|m| m.source.is_none()) {
        let said = if m.place { tr("ui.next_step_guessed_place").to_string() } else { m.text.clone() };
        if !out.contains(&said) {
            out.push(said);
        }
    }
    out
}

/// The copy that computes is done, or took too long.
fn compute(app: &mut SolveApp, now: f64) {
    let p = &mut app.agent_chat.predict;
    let Some(result) = p.work.as_ref().and_then(|w| w.poll(now)) else { return };
    let id = p.shown.as_ref().map(|g| g.id);
    match result {
        Ok(drawn) => {
            let ran = p.work.take().map_or(0, |w| w.ran.load(std::sync::atomic::Ordering::Relaxed));
            let Some(id) = id else { return };
            let at = unix_ms();
            if let Some(l) = p.entry(id) {
                l.outcome = "ghost".into();
                l.engine_ms = Some(drawn.ms);
                l.ran = ran;
                l.ghost_ms = Some(at.saturating_sub(l.at_unix_ms));
            }
            let marks = marks(app, &drawn.steps);
            let p = &mut app.agent_chat.predict;
            if let Some(g) = p.shown.as_mut() {
                g.marks = marks;
            }
            p.ghost = Some(Ghost::new(id, drawn));
        }
        Err(why) => p.refuse(why),
    }
}

/// Send the question waiting: the first in a fresh session, the second in the first's. A session
/// that is not (or no longer) the right one for it is replaced by a fresh one first.
fn send_question(p: &mut Predictor) {
    let Some(ask) = p.ask.as_mut().filter(|a| !a.sent) else { return };
    let Some(b) = p.backend.as_mut() else { return };
    let right = b.session_fits_sign_in()
        && match ask.kind {
            Kind::Step => p.link == Link::Fresh && p.opened_choice == p.choice,
            Kind::Commands => true,
        };
    match p.link {
        Link::Fresh | Link::Used if right => {
            b.prompt(&[PromptPart::Text(ask.prompt.clone())]);
            ask.sent = true;
            let (id, kind) = (ask.id, ask.kind);
            // A suggestion counts once in its hour, when it is first sent.
            if kind == Kind::Step {
                p.hour.push_back(ask.started);
            }
            p.link = Link::Used;
            let model = (!p.model.is_empty()).then(|| p.model.clone());
            if let Some(l) = p.entry(id) {
                match (kind, l.commands.as_mut()) {
                    (Kind::Commands, Some(c)) => c.billed = true,
                    _ => {
                        l.billed = true;
                        l.model = model;
                    }
                }
            }
        }
        Link::Fresh | Link::Used => p.open_next(),
        _ => {}
    }
}

/// The session is ready for a question once its configuration is set.
fn session_ready(p: &mut Predictor) {
    if p.link == Link::Starting {
        p.link = Link::Fresh;
        p.opened_choice = p.choice.clone();
    }
    send_question(p);
}

/// What the guessing session said. `key` and `selection`: the design and selection now, which a
/// late answer must still match to be shown.
fn poll(app: &mut SolveApp, ctx: &egui::Context, now: f64, key: &Key, selection: &[solidcraft_engine::Sel]) {
    let choice = app.preferences.agent_next_step_model.clone();
    let mut ended: Option<String> = None;
    let p = &mut app.agent_chat.predict;
    p.choice.clone_from(&choice);
    let Some(b) = p.backend.as_mut() else { return };
    let events = b.poll();
    let usage = b.take_usage();
    if let Some(u) = usage {
        p.note_usage(u);
    }
    for e in events {
        match e {
            // The built-in agent opens no session by itself, and refuses one while nothing is
            // connected (`connections` on the snapshot): the first is asked for once it can be.
            Event::Connections(snapshot) if p.builtin && !p.opened => {
                if snapshot.connected() {
                    p.opened = true;
                    if let Some(b) = p.backend.as_mut() {
                        b.new_session();
                    }
                } else {
                    fail(p, now, tr("ui.next_step_sign_in").to_string(), true);
                }
            }
            // A fresh session's configuration: set what it needs before its question. While a
            // setting is out, the agent's own news of it (`config_option_update`) is only read:
            // the backend takes one setting at a time.
            Event::ConfigOptions(options) => {
                if p.link == Link::Starting && !p.tuning {
                    p.tune(&options, &choice);
                } else {
                    p.note_model(&options);
                }
            }
            Event::ConfigOptionFinished { id, result } => {
                let more = match result {
                    Ok(options) => p.tune(&options, &choice),
                    Err(e) => {
                        // The guess still runs, on what the session has.
                        log::warn!("next-step suggestions: the agent did not take {id}: {e}");
                        false
                    }
                };
                if !more {
                    p.tuning = false;
                    if std::mem::take(&mut p.ready) {
                        session_ready(p);
                    }
                }
            }
            Event::Ready { .. } => {
                if p.tuning {
                    p.ready = true;
                } else {
                    session_ready(p);
                }
            }
            Event::Text { thought: false, text } => {
                if let Some(ask) = p.ask.as_mut()
                    && ask.reply.len() < MAX_REPLY
                {
                    ask.reply.push_str(&text);
                }
            }
            // Nothing the guessing session asks to do is allowed.
            Event::Permission { key, .. } => {
                if let Some(b) = p.backend.as_mut() {
                    b.answer_permission(key, None);
                }
            }
            Event::TurnEnded { reason } => {
                // What it used for this answer, before the answer is filed.
                if let Some(u) = p.backend.as_mut().and_then(|b| b.take_usage()) {
                    p.note_usage(u);
                }
                ended = Some(reason);
            }
            Event::AuthRequired { .. } => fail(p, now, tr("ui.next_step_sign_in").to_string(), true),
            Event::Exited { tail } => {
                p.backend = None;
                fail(p, now, tail.last().cloned().unwrap_or_else(|| "the agent ended".into()), false);
            }
            // A session it could not open (a sign-in it needs is `AuthRequired`).
            Event::Error(e) if p.link == Link::Starting => fail(p, now, e, false),
            Event::Error(e) => log::warn!("next-step suggestions: {e}"),
            _ => {}
        }
    }
    if let Some(u) = p.backend.as_mut().and_then(|b| b.take_usage()) {
        p.note_usage(u);
    }
    // An answer that never comes is given up, and a session for a shown step that never opens.
    if let Some(ask) = p.ask.as_ref()
        && (ask.sent || ask.kind == Kind::Commands)
        && !ask.dropped
        && now - ask.started > ASK_TIMEOUT_S
    {
        let id = ask.id;
        p.withdraw("timeout");
        p.set_outcome(id, "timeout");
    }
    if let Some(reason) = ended {
        end_question(app, ctx, now, &reason, key, selection);
        // Nothing more to ask in this session.
        let p = &mut app.agent_chat.predict;
        if p.link == Link::Used && p.ask.is_none() {
            p.open_next();
        }
    }
}

fn fail(p: &mut Predictor, now: f64, why: String, sign_in: bool) {
    log::warn!("next-step suggestions paused: {why}");
    if let Some(ask) = p.ask.take() {
        let ms = elapsed_ms(ask.started, now);
        // Nothing came of the question: once the session is back, the design is asked about again.
        // Not for a suggestion the user or the app gave up, nor for the second question, which
        // would ask (and pay for) the first once more each time.
        if ask.kind == Kind::Step && !ask.dropped {
            p.asked = None;
        }
        match ask.kind {
            Kind::Step => p.finish_log(ask.id, Some(ms), "failed", ask.reply.chars().count()),
            Kind::Commands => {
                p.finish_commands(ask.id, ms, ask.reply.chars().count());
                p.refuse(Refused::Failed);
            }
        }
    }
    p.failures = p.failures.saturating_add(1);
    p.link = Link::Failed { why, sign_in, at: now };
}

/// Now as Unix time in milliseconds, for the log (0 where the clock is not available).
#[cfg(not(target_arch = "wasm32"))]
fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn unix_ms() -> u64 {
    0
}

fn elapsed_ms(start: f64, now: f64) -> u64 {
    ((now - start).max(0.0) * 1000.0).round() as u64
}

/// The agent finished answering: the line is shown and its commands are asked for, or its commands
/// run on a copy, if the design and selection are still the ones it was asked about, even when the
/// user did other things meanwhile.
fn end_question(app: &mut SolveApp, ctx: &egui::Context, now: f64, reason: &str, key: &Key, selection: &[solidcraft_engine::Sel]) {
    let p = &mut app.agent_chat.predict;
    let Some(ask) = p.ask.take() else { return };
    let ms = elapsed_ms(ask.started, now);
    let reply_chars = ask.reply.chars().count();
    if ask.kind == Kind::Commands {
        p.finish_commands(ask.id, ms, reply_chars);
        if !ask.dropped {
            end_commands(app, ctx, now, reason, &ask, key);
        }
        return;
    }
    if ask.dropped {
        p.finish_log(ask.id, Some(ms), "dropped", reply_chars);
        return;
    }
    let guess = if reason.is_empty() { parse_reply(&ask.reply) } else { None };
    if reason.is_empty() {
        p.failures = 0;
    }
    let Some((step, why)) = guess.filter(|(step, _)| !step.is_empty()) else {
        p.finish_log(ask.id, Some(ms), if reason.is_empty() { "empty" } else { "failed" }, reply_chars);
        return;
    };
    if let Some(l) = p.entry(ask.id) {
        l.step = step.clone();
        l.reason = why.clone();
        l.line_ms = Some(ms);
    }
    if ask.key != *key || ask.selection != selection {
        p.finish_log(ask.id, Some(ms), "stale", reply_chars);
        return;
    }
    if p.dismissed.iter().any(|d| same_step(d, &step)) {
        p.finish_log(ask.id, Some(ms), Refused::Repeat.outcome(), reply_chars);
        return;
    }
    p.finish_log(ask.id, Some(ms), "shown", reply_chars);
    // The notes the agent says the step follows, as numbered in the question.
    let memory = ask.brief.as_ref().map_or_else(Vec::new, |b| {
        parse_memory(&ask.reply).into_iter().filter_map(|n| b.notes.get(n.wrapping_sub(1))).map(|n| (n.title.clone(), n.text.clone())).collect()
    });
    let project = ask.brief.as_ref().map(|b| b.name.clone());
    p.lines += 1;
    p.shown = Some(Guess { step, reason: why, project, memory, marks: Vec::new(), id: ask.id });
    ask_commands(app, now, ask.id, ask.key);
}

/// The answer to the question for commands: they run on a copy if they may.
fn end_commands(app: &mut SolveApp, ctx: &egui::Context, now: f64, reason: &str, ask: &Ask, key: &Key) {
    let p = &mut app.agent_chat.predict;
    let Some(step) = p.shown.as_ref().filter(|g| g.id == ask.id).map(|g| g.step.clone()) else { return };
    if ask.key != *key {
        p.refuse(Refused::Stale);
        return;
    }
    let sheet = ghost::SHEET_METAL || p.sheet;
    let steps = if reason.is_empty() { ghost::parse(&ask.reply).and_then(|steps| ghost::allowed(steps, sheet)) } else { Err(Refused::Failed) };
    let steps = steps.and_then(|steps| if ghost::without_unit(&app.session.doc, &steps) { Err(Refused::NoUnit) } else { Ok(steps) });
    let steps = steps.and_then(|steps| if words::differ(&app.session.doc, &step, &steps) { Err(Refused::WordsDiffer) } else { Ok(steps) });
    match steps {
        Ok(steps) => {
            let wake = ctx.clone();
            let work = ghost::start(app, steps, now, move || wake.request_repaint());
            app.agent_chat.predict.work = Some(work);
        }
        Err(why) => p.refuse(why),
    }
}

/// The same step, however it is written: letters and digits only, in lower case.
fn same_step(a: &str, b: &str) -> bool {
    let bare = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    bare(a) == bare(b)
}

/// One line of text: no control characters, spaces collapsed, at most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()).collect();
    let line = words.join(" ");
    if line.chars().count() > max { format!("{}…", line.chars().take(max.saturating_sub(1)).collect::<String>()) } else { line }
}

/// The step and its reason from the agent's reply: the JSON object asked for, found in whatever
/// else it wrote; failing that, its first line as the step. Only one step is ever taken.
pub(crate) fn parse_reply(reply: &str) -> Option<(String, String)> {
    let text = reply.trim();
    if text.is_empty() {
        return None;
    }
    let object = text.find('{').and_then(|start| {
        // The first object that parses, scanning its possible ends from the shortest.
        text[start..].match_indices('}').find_map(|(end, _)| serde_json::from_str::<Value>(&text[start..start + end + 1]).ok())
    });
    if let Some(v) = object.filter(Value::is_object) {
        let field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        return Some((one_line(&field("step"), MAX_STEP), one_line(&field("reason"), MAX_REASON)));
    }
    let first = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some((one_line(first.trim_matches(|c| c == '"' || c == '`'), MAX_STEP), String::new()))
}

/// The numbers of the project notes the reply says its step follows (`"memory": [2, 3]`), at most
/// three, each once.
pub(crate) fn parse_memory(reply: &str) -> Vec<usize> {
    let Some(start) = reply.find('{') else { return Vec::new() };
    let object = reply[start..].match_indices('}').find_map(|(end, _)| serde_json::from_str::<Value>(&reply[start..start + end + 1]).ok());
    let mut out: Vec<usize> = Vec::new();
    for n in object.as_ref().and_then(|v| v.get("memory")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64) {
        let n = usize::try_from(n).unwrap_or(0);
        if n > 0 && !out.contains(&n) && out.len() < 3 {
            out.push(n);
        }
    }
    out
}

/// The brief of the saved design's project, if it is in one (trusted or not).
fn project_brief(app: &SolveApp) -> Option<super::project_brief::Brief> {
    let root = super::project_brief::find(app.session.path.as_deref())?;
    let trusted = super::project_brief::project_id(&root).is_some_and(|id| app.agent_chat.predict.trusted_projects.contains(&id));
    super::project_brief::read(&root, trusted).map_err(|e| log::warn!("next-step: the project brief could not be read: {e}")).ok()
}

/// Trust the saved design's project for this session: its notes go to the agent. `Err`: no project.
pub fn trust_project(app: &mut SolveApp) -> Result<String, String> {
    let root = super::project_brief::find(app.session.path.as_deref()).ok_or("the open design is not saved in a project")?;
    let id = super::project_brief::project_id(&root).ok_or("the project has no id in its project.toml")?;
    let p = &mut app.agent_chat.predict;
    if !p.trusted_projects.contains(&id) {
        p.trusted_projects.push(id.clone());
    }
    Ok(id)
}

/// A hole's diameter, as the questions give it: a Hole's, and that of a cylinder that cuts, which the
/// design keeps as a radius.
fn bore(doc: &solidcraft_engine::doc::Document, kind: &FeatureKind) -> Option<String> {
    let (size, times) = match kind {
        FeatureKind::Hole { diameter, .. } => (diameter, 1.0),
        FeatureKind::Cylinder { radius, operation: Operation::Cut, .. } => (radius, 2.0),
        _ => return None,
    };
    let mm = times * words::millimetres(doc, size)?;
    if words::other_unit(doc).is_none() {
        return Some(doc.design_length(mm).1.trim_end_matches(" mm").to_string());
    }
    // As it was made: in mm where the design was in mm then.
    let in_mm = is_literal(size) && words::lacks_unit(doc, size) || size.contains("mm");
    Some(if in_mm { format!("{} mm", (mm * 1e6).round() / 1e6) } else { doc.design_length(mm).1 })
}

/// A size as the questions give it, with a unit where the design is not in mm: a number stored without
/// one is in mm, and what is worked out of such numbers in the design's unit.
fn sized(doc: &solidcraft_engine::doc::Document, size: &str) -> String {
    if words::other_unit(doc).is_none() || !words::lacks_unit(doc, size) {
        return size.to_string();
    }
    if is_literal(size) {
        format!("{} mm", size.trim())
    } else {
        words::millimetres(doc, size).map_or_else(|| size.to_string(), |mm| doc.design_length(mm).1)
    }
}

/// Short, readable values of a feature's own parameters: what it does to the bodies (an operation
/// left out reads as a body still to be cut), where it is, its lengths, angles and counts.
fn feature_values(doc: &solidcraft_engine::doc::Document, f: &solidcraft_engine::doc::Feature) -> String {
    const KEYS: &[&str] =
        &["distance", "radius", "thickness", "diameter", "depth", "length", "width", "height", "angle", "count", "spacing", "offset"];
    const PLACES: &[&str] = &["corner", "base", "center", "position", "origin"];
    const PICKS: &[&str] = &["edges", "faces"];
    fn walk(doc: &solidcraft_engine::doc::Document, v: &Value, depth: usize, out: &mut Vec<String>) {
        let Some(o) = v.as_object() else { return };
        for (k, v) in o {
            if out.len() >= 5 {
                return;
            }
            let of = |s: &str| if matches!(k.as_str(), "angle" | "count") { s.to_string() } else { sized(doc, s) };
            match v {
                Value::String(s) if KEYS.contains(&k.as_str()) && s.len() <= 40 => out.push(format!("{k} {}", of(s))),
                Value::Number(n) if KEYS.contains(&k.as_str()) => out.push(format!("{k} {}", of(&n.to_string()))),
                Value::Object(_) if depth < 2 => walk(doc, v, depth + 1, out),
                _ => {}
            }
        }
    }
    /// The first value of `keys` in `v`, at most two levels down.
    fn find<'a>(v: &'a Value, keys: &[&str], depth: usize, fits: fn(&Value) -> bool) -> Option<(String, &'a Value)> {
        let o = v.as_object()?;
        keys.iter()
            .find_map(|k| o.get(*k).filter(|v| fits(v)).map(|v| ((*k).to_string(), v)))
            .or_else(|| if depth < 2 { o.values().find_map(|v| find(v, keys, depth + 1, fits)) } else { None })
    }
    let Ok(mut kind) = serde_json::to_value(&f.kind) else { return String::new() };
    if let (Some(diameter), Some(o)) = (bore(doc, &f.kind), kind.as_object_mut()) {
        o.remove("radius");
        o.insert("diameter".into(), diameter.into());
    }
    let mut out = Vec::new();
    walk(doc, &kind, 0, &mut out);
    // A point's numbers are mm, which is said where the design is in another unit: of all three, or
    // of each where some are worked out.
    let point = |v: &Value| match v.as_array() {
        Some(a) if a.len() == 3 => {
            let coords: Vec<String> =
                a.iter().map(|x| x.as_f64().map(|n| n.to_string()).or_else(|| x.as_str().map(str::to_string)).unwrap_or_default()).collect();
            let bare = coords.iter().all(|c| is_bare_number(c));
            let coords: Vec<String> = coords.into_iter().map(|c| if bare { c } else { sized(doc, &c) }).collect();
            let unit = if bare && words::other_unit(doc).is_some() { " mm" } else { "" };
            Some(format!("({}){unit}", coords.join(", ")))
        }
        _ => None,
    };
    if let Some((key, v)) = find(&kind, PLACES, 0, |v| v.as_array().is_some_and(|a| a.len() == 3))
        && let Some(at) = point(v)
    {
        out.push(format!("{key} {at}"));
    }
    if let Some((key, v)) = find(&kind, PICKS, 0, Value::is_array) {
        out.push(format!("{} {key}", v.as_array().map_or(0, Vec::len)));
    }
    if let Some((_, v)) = find(&kind, &["operation"], 0, Value::is_string) {
        let into = if f.participants.is_empty() { String::new() } else { format!(" {}", f.participants.join(", ")) };
        out.insert(
            0,
            match v.as_str() {
                Some("cut") => format!("cut from{}", if into.is_empty() { " the bodies there" } else { &into }),
                Some("join") => format!("joined to{}", if into.is_empty() { " the bodies there" } else { &into }),
                Some("intersect") => format!("intersected with{}", if into.is_empty() { " the bodies there" } else { &into }),
                _ => "new body".to_string(),
            },
        );
    }
    out.join(", ")
}

/// History's last steps as the questions list them.
fn history_block(doc: &solidcraft_engine::doc::Document) -> String {
    let end = doc.marker.unwrap_or(doc.features.len()).min(doc.features.len());
    let mut out = format!("\nDesign: units {}, {} History steps", doc.units, end);
    if end < doc.features.len() {
        out.push_str(&format!(" ({} more rolled back after the History marker)", doc.features.len() - end));
    }
    out.push('.');
    if end > 0 {
        out.push_str(&format!("\nHistory (oldest first, the last {}):", end.min(RECENT_STEPS)));
        for (i, f) in doc.features[..end].iter().enumerate().skip(end.saturating_sub(RECENT_STEPS)) {
            let name = if f.name.is_empty() { format!("#{}", f.id) } else { f.name.chars().take(60).collect() };
            let values = feature_values(doc, f);
            let suppressed = if f.suppressed { ", suppressed" } else { "" };
            let values = if values.is_empty() { String::new() } else { format!(": {values}") };
            let bodies: Vec<String> = f.body_names.iter().take(4).map(|b| b.chars().take(40).collect()).collect();
            let bodies = if bodies.is_empty() { String::new() } else { format!(" -> bodies: {}", bodies.join(", ")) };
            out.push_str(&format!("\n{}. {name} ({}{suppressed}){values}{bodies}", i + 1, f.kind.type_name()));
        }
    }
    out
}

/// The values this design uses again and again: fillet radii, chamfer sizes, wall thicknesses and
/// hole diameters, most used first.
fn usual_values(doc: &solidcraft_engine::doc::Document) -> Vec<String> {
    let mut kinds: Vec<(&str, Vec<(String, usize)>)> =
        vec![("fillet radius", Vec::new()), ("chamfer distance", Vec::new()), ("shell (wall) thickness", Vec::new()), ("hole diameter", Vec::new())];
    for f in doc.features.iter().filter(|f| !f.suppressed) {
        let (slot, value) = match &f.kind {
            FeatureKind::Fillet { radius, .. } => (0, sized(doc, radius)),
            FeatureKind::Chamfer { distance, .. } => (1, sized(doc, distance)),
            FeatureKind::Shell { thickness, .. } => (2, sized(doc, thickness)),
            kind => match (bore(doc, kind), kind) {
                (Some(diameter), _) => (3, diameter),
                (None, FeatureKind::Hole { diameter, .. }) => (3, sized(doc, diameter)),
                _ => continue,
            },
        };
        let values = &mut kinds[slot].1;
        match values.iter_mut().find(|(v, _)| *v == value) {
            Some((_, n)) => *n += 1,
            None => values.push((value.chars().take(40).collect(), 1)),
        }
    }
    kinds
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(name, mut values)| {
            values.sort_by_key(|v| std::cmp::Reverse(v.1));
            let list: Vec<String> = values.iter().take(USUAL_VALUES).map(|(v, n)| if *n > 1 { format!("{v} (×{n})") } else { v.clone() }).collect();
            format!("{name}: {}", list.join(", "))
        })
        .collect()
}

/// The language the app's interface is in, named for the agent.
fn language_name(code: &str) -> String {
    let name = match code {
        "en" => "English",
        "en-GB" => "British English",
        "zh" => "Simplified Chinese",
        "zh-TW" => "Traditional Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt-BR" => "Brazilian Portuguese",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "ar" => "Arabic",
        "he" => "Hebrew",
        "vi" => "Vietnamese",
        "tr" => "Turkish",
        "pl" => "Polish",
        "nl" => "Dutch",
        "cs" => "Czech",
        "bs" => "Bosnian",
        "da" => "Danish",
        "fi" => "Finnish",
        "gl" => "Galician",
        "hr" => "Croatian",
        "hu" => "Hungarian",
        "id" => "Indonesian",
        "ka" => "Georgian",
        "ms" => "Malay",
        "nb" => "Norwegian Bokmål",
        "ro" => "Romanian",
        "sr" => "Serbian",
        "sv" => "Swedish",
        _ => return format!("the language with the code {code}"),
    };
    name.to_string()
}

/// The question about what is selected now (the tests read it).
#[cfg(test)]
pub(crate) fn prompt(app: &mut SolveApp) -> String {
    let context = super::context::describe_selection(&mut app.session);
    let brief = project_brief(app);
    build_prompt(app, &context, brief.as_ref())
}

/// The question: what the agent needs to guess about the design and `context` (the selection when
/// asked), and the answer's shape; with the design's project `brief` when it is in one.
fn build_prompt(app: &mut SolveApp, context: &Context, brief: Option<&super::project_brief::Brief>) -> String {
    let selection = context.prompt_block();
    let doc = Arc::clone(&app.session.doc);
    let language = language_name(crate::i18n::language());
    let (example, units) = words::other_unit(&doc).map_or(("2 mm", String::new()), |u| {
        let (name, example, _) = words::spoken(u);
        (example, format!("Give sizes in {name} ({u}). "))
    });
    let mut out = format!(
        "[Intcad next-step suggestion. Do not call any tools, do not read or change files and do not change the design. Answer with one line of JSON only.]\n\
         The user is modelling a part in Intcad and has paused after an edit. Guess the one modelling step they are most likely to do next, \
         written as a short request they could send you, with sizes taken from the design's usual values where they fit.\n\
         Reply with exactly: {{\"step\": \"...\", \"reason\": \"...\"}}\n\
         - step: one imperative sentence, at most 12 words (for example \"Fillet the 4 top edges of Body1 with {example}\").\n\
         - reason: one short sentence on why, at most 12 words.\n\
         Name things the way the user sees them: bodies, features and sizes. A hole's size is its diameter. {units}\
         Never mention internal face or edge names such as F1:-y.\n",
    );
    out.push_str(&format!(
        "Write the step and the reason in {language}, the language of the app. If nothing sensible comes to mind, reply {{\"step\": \"\", \"reason\": \"...\"}}.\n"
    ));
    out.push_str(&history_block(&doc));
    let edits: Vec<String> = app.session.undo.iter().rev().take(RECENT_EDITS).map(|s| one_line(&s.label, 80)).filter(|l| !l.is_empty()).collect();
    if !edits.is_empty() {
        out.push_str(&format!("\nLast edits (newest last): {}", edits.into_iter().rev().collect::<Vec<_>>().join("; ")));
    }
    let usual = usual_values(&doc);
    out.push_str("\nUsual values in this design: ");
    out.push_str(&if usual.is_empty() { "none yet".to_string() } else { usual.join("; ") });
    out.push_str("\n\n");
    out.push_str(if selection.is_empty() { "[Nothing is selected.]" } else { &selection });
    let dismissed = &app.agent_chat.predict.dismissed;
    if !dismissed.is_empty() {
        out.push_str("\n\nSuggestions the user turned down (dismissed, or rejected after a preview); do not suggest them again:");
        for d in dismissed {
            out.push_str(&format!("\n- {d}"));
        }
    }
    let said: Vec<String> = app
        .agent_chat
        .items
        .iter()
        .filter_map(|i| match i {
            Item::User { text, .. } if !text.trim().is_empty() => Some(one_line(text, CHAT_CHARS)),
            _ => None,
        })
        .collect();
    if !said.is_empty() {
        out.push_str("\n\nThe user's messages in this chat (oldest first):");
        for s in said.iter().skip(said.len().saturating_sub(CHAT_MESSAGES)) {
            out.push_str(&format!("\n- {s}"));
        }
    }
    if let Some(b) = brief {
        out.push_str(&b.prompt_block());
        if !b.notes.is_empty() {
            out.push_str(
                "\n\nWhere the project memory applies, take the step's sizes and choices from it rather than from general habit. \
                 Add \"memory\": the numbers of the notes your step follows, for example {\"step\": \"...\", \"reason\": \"...\", \"memory\": [2]}, or [] if none.",
            );
        }
    }
    out
}

/// For `ui.agent_chat`: the setting, what is happening, the guess shown and each guess's time and
/// cost.
pub fn status(app: &SolveApp) -> Value {
    let p = &app.agent_chat.predict;
    let state = if !app.preferences.agent_next_step {
        "off"
    } else if p.ghost.is_some() {
        "ghost"
    } else if p.shown.is_some() {
        "shown"
    } else if !app.agent_chat.state().running() || !supported(app) {
        "no agent"
    } else if p.ask.as_ref().is_some_and(|a| !a.dropped) {
        "asking"
    } else {
        match p.link {
            Link::Failed { sign_in: true, .. } => "paused: sign in",
            Link::Failed { .. } => "paused",
            Link::None | Link::Starting => "starting",
            Link::Fresh | Link::Used => "waiting",
        }
    };
    let failed = match &p.link {
        Link::Failed { why, .. } => Some(why.clone()),
        _ => None,
    };
    let done: Vec<&Logged> = p.log.iter().filter(|l| l.ms.is_some()).collect();
    let mean_ms = (!done.is_empty()).then(|| done.iter().filter_map(|l| l.ms).sum::<u64>() / done.len() as u64);
    let (billed, cost) = p.billed();
    json!({
        "enabled": app.preferences.agent_next_step,
        "pauseS": pause(app),
        "state": state,
        "chip": chip(app).map(|(text, _, guess)| json!({"text": text, "guess": guess})),
        "failed": failed,
        "failures": p.failures,
        "shown": p.shown,
        "ghost": p.ghost.as_ref().map(Ghost::status),
        "hourUsed": p.hour.len(),
        "perHour": hourly(app),
        "keySignIn": p.backend.as_ref().is_some_and(|b| b.key_sign_in()),
        "lines": p.lines,
        "withdrawn": p.withdrawn,
        "guesses": p.log.len(),
        "billedCalls": billed,
        "project": p.project_seen,
        "trustedProjects": p.trusted_projects,
        "meanMs": mean_ms,
        "totalCost": cost,
        "log": p.log.iter().rev().take(50).rev().collect::<Vec<_>>(),
    })
}

#[path = "predict_card.rs"]
pub(crate) mod card;

#[path = "predict_panel.rs"]
pub(crate) mod panel;

#[cfg(test)]
#[path = "predict_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "predict_ghost_tests.rs"]
mod ghost_tests;
