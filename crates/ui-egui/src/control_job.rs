//! Control-channel (MCP) requests that compute off the window thread (#1989): an agent's slow
//! command (a boolean, a fillet, a big batch, an export) used to run on the window thread, and
//! the window froze until it was done: no redraw, no orbit, no way to stop it.
//!
//! The window thread still owns the design. What leaves it is the computing:
//! - **Design changes** (`engine.execute` of a modelling command, `engine.batch`) are evaluated
//!   ahead on a copy of the session ([`solidcraft_engine::Session::ahead`]), as the history
//!   marker and undo are ([`crate::deferred`]). What that computes is kept
//!   ([`solidcraft_engine::doc::KeptResults`]); once it is in, the request runs here for real, as
//!   it always did (undo step, recovery autosave, crash journal, agent cursor, reply), and finds
//!   every feature it computed there, whatever the bounded evaluation cache dropped meanwhile.
//!   One design change computes at a time; later ones wait their turn, in order.
//! - **Exports** (`file.export`, `file.save_mesh`) change nothing in the design: they run on a
//!   copy of it as it was when they were asked for, and their reply comes from there. Nothing
//!   waits behind them.
//!
//! Meanwhile the window goes on drawing the design as it was, `ui.*` requests and inspection are
//! answered, and a chip says which agent command is computing, with a Stop button. A person's
//! own edits meanwhile go first: the agent's command is then computed again for the design as it
//! is and lands after them, never in between. If the design itself was replaced (another tab, a
//! file opened) the agent's command is refused, saying so, and nothing is applied.
//! Requests with a document precondition instead keep their original admission: an intervening
//! edit or departure refuses them before application, without recomputing for a newer base.
//!
//! Done within the app's wait ([`crate::dialogs::APPLY_AT_ONCE`]), a request just runs, as
//! before. Headless use, tests and the web build ([`SolveApp::apply_off_thread`] unset) run
//! everything at once, and so does the app unless the preference is on (Preferences > AI,
//! `agent_off_thread`, off by default).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};

use solidcraft_engine::DocumentStamp;
use solidcraft_engine::doc::KeptResults;

use crate::SolveApp;
use crate::control::ControlRequest;

/// Areas whose commands only compute the design (no files written, nothing started outside the
/// app), so evaluating them ahead on a copy has no effect but a warm cache.
const AHEAD_AREAS: &[&str] = &[
    "appearance",
    "assembly",
    "body",
    "component",
    "config",
    "construct",
    "contact",
    "explode",
    "expr",
    "face",
    "joint",
    "material",
    "mesh",
    "model",
    "occurrence",
    "parameters",
    "plastic",
    "sheet",
    "sketch",
    "solid",
    "surface",
    "timeline",
];

/// Parameters that name a file or folder: a command given one runs here, as before.
const NAMES_A_FILE: &[&str] = &["path", "paths", "file", "files", "dir", "folder", "url", "out"];

/// Commands that change nothing in the design and write a file: they run on a copy.
const ANSWERED_ON_A_COPY: &[&str] =
    &["file.export", "file.export_3mf", "file.export_obj", "file.export_step", "file.export_stl", "file.export_urdf", "file.save_mesh"];

/// Commands that only read the design, which can take long on a large one (a measure of every
/// body, an inspection that measures them, #4604; a clearance or interference check of every pair
/// of bodies, minutes on an assembly of 150): they run on a copy too, whatever the preference,
/// since an answer from the design as it was when asked has no order to keep.
const READ_ON_A_COPY: &[&str] = &[
    "inspect.measure",
    "document.inspect",
    "inspect.clearance",
    "inspect.interference",
    // Physical properties of a component or placement: every body of every placement (#8706).
    "component.properties",
    "occurrence.properties",
];

/// A read from [`READ_ON_A_COPY`] that a controlled entry (an agent) may run on a copy, because
/// the host can end it before its authorization does (docs/account/copy-read-admission.md).
pub(crate) struct StoppableRead {
    /// The command, as registered.
    pub(crate) id: &'static str,
    /// End the run now: it answers soon after, and keeps and shows nothing new. The host calls it
    /// again until the run answers, so a stop asked for before the run can see it is not lost.
    pub(crate) stop: fn(),
    /// Take away what a run that finished shows for this document (its id), given the run's
    /// parameters. Called only for a finished run whose reply the host withholds, so what an
    /// earlier run shows stays.
    pub(crate) withdraw: fn(&str, &Value),
}

/// The reads a controlled entry may run on a copy. Every other copy reading, export and snapshot
/// stays refused for a controlled entry: none of them can be stopped once it has started.
const STOPPABLE_READS: &[StoppableRead] = &[];

/// The stoppable read `id` names, if it is one.
pub(crate) fn stoppable_read(id: &str) -> Option<&'static StoppableRead> {
    #[cfg(test)]
    if let Some(row) = test_support::stoppable_row(id) {
        return Some(row);
    }
    STOPPABLE_READS.iter().find(|row| row.id == id)
}

/// Whether `req` asks for a stoppable read.
pub(crate) fn stoppable_request(req: &ControlRequest) -> bool {
    reading(req).is_some_and(|(id, _)| stoppable_read(id).is_some())
}

/// How long before its authorization ends the host stops a controlled read. It covers the time a
/// stopped read takes to answer and the commit gate after it.
pub(crate) const STOP_MARGIN: std::time::Duration = std::time::Duration::from_secs(30);

/// Why the host withholds a controlled read's reply. Each case also stops the read, and takes away
/// what a finished one shows, so nothing of it is delivered: while the commit gate is still open
/// it is settled as having no effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Withheld {
    /// Less than [`STOP_MARGIN`] of its authorization was left.
    AuthorizationEnded,
    /// Stop was pressed.
    Stopped,
    /// The chat turn it belongs to ended, or its document tab was left.
    TurnEnded,
}

impl Withheld {
    fn reply(self, req: &ControlRequest, label: &str) -> Value {
        let guarded = req.guarded.is_some();
        match self {
            Withheld::AuthorizationEnded => crate::control::admission_refusal_json(&solidcraft_engine::entitlement::Refusal::new(
                solidcraft_engine::entitlement::RefusalCode::UnsafeToStart,
                format!("stopped: `{label}` would have outlasted its authorization; nothing was shown or charged"),
            )),
            Withheld::Stopped => crate::control::stopped_command_response(&req.method, &req.params, guarded, label),
            Withheld::TurnEnded => crate::control::cancelled_request_response(&req.method, &req.params, guarded),
        }
    }
}

/// A controlled read running on a copy: the admission it runs under, carried to its commit.
struct Controlled {
    row: &'static StoppableRead,
    admission: solidcraft_engine::entitlement::Admission,
    /// The document it reads, for [`StoppableRead::withdraw`].
    document: String,
    withheld: Option<Withheld>,
    /// Stops it at its authorization's end when the window draws no frames.
    watchdog: Option<Watchdog>,
}

/// A thread that calls a controlled read's `stop` once its stop time passes, and again until the
/// read answers. The window draws no frames while it is minimised, so it cannot rely on
/// [`SolveApp::poll_control_jobs`] alone; each frame moves the stop time as the lease is renewed.
/// Its clock does not count a suspend; the window's own check, on the host's clock, covers that.
/// The web build has no threads and no controlled entries: it never starts one.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Watchdog {
    stop_at: Arc<std::sync::Mutex<std::time::Instant>>,
    done: Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::Thread,
}

impl Watchdog {
    #[cfg(not(target_arch = "wasm32"))]
    fn start(stop: fn(), left: std::time::Duration) -> Option<Watchdog> {
        let stop_at = Arc::new(std::sync::Mutex::new(stop_time(left)));
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (at, ended) = (stop_at.clone(), done.clone());
        let handle = std::thread::Builder::new()
            .name("agent-read-stop".into())
            .spawn(move || {
                while !ended.load(Ordering::Relaxed) {
                    let Ok(due) = at.lock().map(|at| *at) else { return };
                    let now = std::time::Instant::now();
                    if now >= due {
                        stop();
                        std::thread::park_timeout(std::time::Duration::from_millis(250));
                    } else {
                        std::thread::park_timeout(due - now);
                    }
                }
            })
            .ok()?;
        Some(Watchdog { stop_at, done, thread: handle.thread().clone() })
    }

    #[cfg(target_arch = "wasm32")]
    fn start(_stop: fn(), _left: std::time::Duration) -> Option<Watchdog> {
        None
    }

    /// The authorization now has `left`: stop the read that long, less the margin, from now.
    #[cfg(not(target_arch = "wasm32"))]
    fn moved(&self, left: std::time::Duration) {
        if let Ok(mut at) = self.stop_at.lock() {
            *at = stop_time(left);
        }
        self.thread.unpark();
    }

    #[cfg(target_arch = "wasm32")]
    fn moved(&self, _left: std::time::Duration) {}
}

/// When to stop a read whose authorization has `left` now.
#[cfg(not(target_arch = "wasm32"))]
fn stop_time(left: std::time::Duration) -> std::time::Instant {
    let now = std::time::Instant::now();
    now.checked_add(left.saturating_sub(STOP_MARGIN)).unwrap_or(now)
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        self.thread.unpark();
    }
}

/// Exports and readings computing at once; more run here.
const MAX_ANSWERING: usize = 4;

/// How often a design change is computed again because the design changed under it before it
/// runs here anyway.
const MAX_TRIES: u8 = 3;

/// The command's own id (as registered), if it has one.
fn spec_id(command: &str) -> Option<&'static str> {
    solidcraft_engine::find_command(command).map(|c| c.id)
}

fn names_a_file(params: &Value) -> bool {
    params.as_object().is_some_and(|o| o.keys().any(|k| NAMES_A_FILE.contains(&k.as_str())))
}

/// Can `command` be evaluated ahead on a copy of the session with no effect outside it?
pub(crate) fn computes_ahead(command: &str, params: &Value) -> bool {
    let Some(id) = spec_id(command) else { return false };
    if matches!(id, "edit.undo" | "edit.redo") {
        return true;
    }
    let area = id.split('.').next().unwrap_or_default();
    AHEAD_AREAS.contains(&area) && !["export", "import", "save", "library"].iter().any(|w| id.contains(w)) && !names_a_file(params)
}

/// The command an `engine.execute` request (its `params`) runs and its parameters.
fn executed(params: &Value) -> Option<(&str, Value)> {
    let command = params.get("command").or_else(|| params.get("id")).and_then(Value::as_str)?;
    Some((command, params.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}))))
}

/// A request that only reads the design and can take long ([`READ_ON_A_COPY`]): an
/// `engine.execute` of such a command, or the `document.inspect` method. The command and its
/// parameters.
fn reading(req: &ControlRequest) -> Option<(&'static str, Value)> {
    match req.method.as_str() {
        "document.inspect" => Some(("document.inspect", req.params.clone())),
        "engine.execute" | "command" => {
            let (command, params) = executed(&req.params)?;
            spec_id(command).filter(|id| READ_ON_A_COPY.contains(id)).map(|id| (id, params))
        }
        _ => None,
    }
}

/// Requests that change the design: they wait while one computes.
pub(crate) fn changes_the_design(method: &str) -> bool {
    matches!(method, "engine.execute" | "command" | "engine.script" | "engine.batch" | "document.import" | "document.saved")
}

/// Whether `doc` is the design `session` shows, as it is or edited since (an undo step away).
fn shows_design(session: &solidcraft_engine::Session, doc: &Arc<solidcraft_engine::doc::Document>) -> bool {
    Arc::ptr_eq(&session.doc, doc) || session.undo.iter().chain(&session.redo).any(|snap| Arc::ptr_eq(&snap.doc, doc))
}

/// The reply to a request whose design left the window while it computed.
fn design_replaced(label: &str) -> Value {
    json!({
        "ok": false,
        "error": format!("the design was replaced (another tab, or a file opened) while `{label}` computed; nothing was applied"),
    })
}

/// What a request computes off the window thread.
enum Work {
    /// Evaluated ahead: once in, the request runs here. `doc` and `revision` are the design it
    /// was computed for.
    Ahead { commands: Vec<(String, Value)>, doc: Arc<solidcraft_engine::doc::Document>, revision: u64, tries: u8, kept: Arc<KeptResults> },
    /// Run on a copy; the reply comes from there.
    Answer { source: Option<DocumentStamp> },
}

/// A control request computing off the window thread.
pub struct ControlJob {
    req: ControlRequest,
    work: Work,
    /// The worker's result: `Some` reply for [`Work::Answer`].
    done: Receiver<Option<Value>>,
    /// When it was asked for (ms).
    pub(crate) started: f64,
    /// The command, as the toolbar names it.
    pub(crate) label: String,
    /// How far it has got (evaluated ahead).
    steps: Option<Arc<Steps>>,
    /// A controlled read's admission and stop ([`Work::Answer`] only).
    controlled: Option<Controlled>,
}

impl ControlJob {
    fn poll(&self) -> Option<Option<Value>> {
        match self.done.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Disconnected) => Some(None),
            Err(TryRecvError::Empty) => None,
        }
    }
}

fn label(app: &SolveApp, command: &str) -> String {
    let language = crate::i18n::Locale::for_language(&app.preferences.language);
    language.label(solidcraft_engine::find_command(command).map_or(command, |c| c.label)).to_string()
}

/// Run `f` on a worker thread (with the request's folder scope), answering on the returned
/// channel. `None` without threads (the web).
fn spawn(
    scope: Option<solidcraft_engine::io::vfs::PathScope>,
    f: impl FnOnce() -> Option<Value> + Send + 'static,
) -> Option<Receiver<Option<Value>>> {
    #[cfg(test)]
    let f = match crate::history_off_thread_tests::guarded_workers::spawn(scope.clone(), f) {
        Ok(rx) => return Some(rx),
        Err(f) => f,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("agent".into())
        .spawn(move || {
            let r = solidcraft_engine::io::vfs::scoped(scope, f);
            let _ = tx.send(r);
        })
        .ok()?;
    Some(rx)
}

/// How far a request evaluating ahead has got: the step running (from 1) of how many, and
/// each step's command as the toolbar names it.
pub(crate) struct Steps {
    now: AtomicUsize,
    labels: Vec<String>,
    total: usize,
}

impl Steps {
    fn new(app: &SolveApp, request: &Value, commands: &[(String, Value)]) -> Arc<Steps> {
        // A batch counts all its steps, also those after the ones computed ahead.
        let total = request.get("commands").and_then(Value::as_array).map_or(commands.len(), Vec::len).max(commands.len());
        Arc::new(Steps { now: AtomicUsize::new(0), labels: commands.iter().map(|(c, _)| label(app, c)).collect(), total })
    }

    /// What the chip says: the command running, and in a batch which step it is
    /// (`3/21 · Extrude`).
    pub(crate) fn shown(&self) -> String {
        let now = self.now.load(Ordering::Relaxed).max(1);
        let label = self.labels.get(now - 1).or(self.labels.last()).cloned().unwrap_or_default();
        if self.total > 1 { format!("{now}/{} · {label}", self.total) } else { label }
    }
}

/// Evaluate `commands` ahead on a copy of the session, on a worker (with the folder scope
/// `request` names). What it computes is kept for the run here.
fn evaluate_ahead(
    app: &SolveApp,
    request: &Value,
    commands: Vec<(String, Value)>,
) -> Option<(Receiver<Option<Value>>, Arc<KeptResults>, Arc<Steps>)> {
    let scope = crate::control::request_scope(app, request).ok()?;
    let ahead = app.session.ahead();
    let kept = ahead.kept();
    let steps = Steps::new(app, request, &commands);
    let counting = steps.clone();
    let rx = spawn(scope, move || {
        let _ = ahead.evaluate_counting(&commands, &counting.now);
        None
    })?;
    Some((rx, kept, steps))
}

/// Start computing a design change ahead, for the design as it is now.
fn begin_ahead(
    app: &SolveApp,
    req: &ControlRequest,
    commands: Vec<(String, Value)>,
    tries: u8,
) -> Option<(Work, Receiver<Option<Value>>, Arc<Steps>)> {
    let (rx, kept, steps) = evaluate_ahead(app, &req.params, commands.clone())?;
    Some((Work::Ahead { commands, doc: app.session.doc.clone(), revision: app.session.revision, tries, kept }, rx, steps))
}

/// Admit a request answered on a copy (`id`), before any worker starts. Free GUI and browser work
/// answers on the copy as before (`Ok(None)`). A controlled entry may only run a stoppable read
/// (`Ok(Some)`): it passes everything a dispatch here passes, and carries its admission to
/// [`SolveApp::answered`]'s commit gate. Any other copy answer for a controlled entry is refused
/// before a grant or a worker, as before. `Err`: refused, and answered.
fn admit_copy_path(app: &mut SolveApp, req: &mut ControlRequest, id: &str) -> Result<Option<Controlled>, ()> {
    let controlled = !crate::control::free_entry(req.entry);
    let row = if controlled { stoppable_read(id) } else { None };
    if controlled && row.is_none() {
        crate::control::release_unstarted(app, req);
        let _ = req.reply.send(crate::control::async_controlled_refusal(req).unwrap_or_else(|| json!({"ok":false,"error":"refused"})));
        return Err(());
    }
    if let Err(refusal) = crate::control::admit_once(app, req) {
        let _ = req.reply.send(crate::control::admission_refusal_json(&refusal));
        return Err(());
    }
    let Some(row) = row else { return Ok(None) };
    match crate::control::start_dispatch(app, req) {
        Ok(admission) => {
            // A bridge read may outlast one lease: the host renews it while it runs.
            crate::control::register_control_renewal(app, req, &admission);
            let watchdog = crate::control::authorization_left(app, req, &admission).and_then(|left| Watchdog::start(row.stop, left));
            Ok(Some(Controlled { row, admission, document: app.session.document_id.clone(), withheld: None, watchdog }))
        }
        Err(refused) => {
            let _ = req.reply.send(outcome_value(refused));
            Err(())
        }
    }
}

/// A reading's successful reply carries the stamp of the design it read.
fn stamped(work: &Work, mut reply: Value) -> Value {
    if reply["ok"] == true
        && let Work::Answer { source: Some(source) } = work
    {
        reply["documentStamp"] = crate::execution_guard::stamp_json(source);
    }
    reply
}

/// The reply an outcome sends on the copy path, where nothing renders a screenshot.
fn outcome_value(out: crate::control::Outcome) -> Value {
    match out {
        crate::control::Outcome::Done(value) => value,
        crate::control::Outcome::Screenshot { .. } => json!({"ok": false, "error": "a read on a copy answers no screenshot"}),
    }
}

impl SolveApp {
    /// Both immediate and deferred completion are only cache warming: the original admission
    /// must still match on the owner before installing kept results or touching history.
    pub(crate) fn finish_control_ahead(&mut self, ctx: &egui::Context, req: ControlRequest, kept: Arc<KeptResults>) {
        let mut req = req;
        if req.refuse_cancelled_chat() {
            crate::control::release_unstarted(self, &mut req);
            return;
        }
        if let Err(refusal) = crate::control::admit_once(self, &mut req) {
            let _ = req.reply.send(crate::control::admission_refusal_json(&refusal));
            return;
        }
        if let Err(error) = crate::execution_guard::check_application(self, &req) {
            crate::control::release_unstarted(self, &mut req);
            let _ = req.reply.send(crate::execution_guard::refusal_json(&error));
            return;
        }
        self.run_with_kept(Some(kept), |app| app.run_control(ctx, req));
    }

    /// Run a control request here with what was computed ahead for it at hand ([`KeptResults`]):
    /// its features are found there rather than computed again on the window thread.
    pub(crate) fn run_with_kept<R>(&mut self, kept: Option<Arc<KeptResults>>, run: impl FnOnce(&mut SolveApp) -> R) -> R {
        let keeps = kept.is_some();
        if keeps {
            self.session.model.keep(kept);
        }
        let r = run(self);
        if keeps {
            self.session.model.keep(None);
        }
        r
    }
}

/// A command the agent cursor acts out, evaluating ahead while the pointer glides to it.
pub struct Ahead {
    done: Receiver<Option<Value>>,
    /// What it computes, for the run once the pointer is there.
    pub(crate) kept: Arc<KeptResults>,
    /// When it started (ms).
    pub(crate) started: f64,
    /// The command, as the toolbar names it.
    pub(crate) label: String,
    /// How far it has got.
    pub(crate) steps: Arc<Steps>,
}

impl Ahead {
    /// Start evaluating the design changes of the `request` (`method`: an `engine.execute` or an
    /// `engine.batch`) the pointer acts out; `None` when it just runs once the pointer is there
    /// (not computed off the window thread, or nothing to compute).
    pub(crate) fn begin(app: &SolveApp, method: &str, request: &Value) -> Option<Ahead> {
        app.apply_off_thread?;
        if !app.preferences.agent_off_thread || app.file_load.busy() || app.file_picker.is_some() {
            return None;
        }
        let commands = ahead_commands(method, request);
        let label = label(app, &commands.first()?.0);
        let (done, kept, steps) = evaluate_ahead(app, request, commands)?;
        Some(Ahead { done, kept, started: crate::now_ms(), label, steps })
    }

    /// The evaluation is in (or its worker ended).
    pub(crate) fn finished(&self) -> bool {
        !matches!(self.done.try_recv(), Err(TryRecvError::Empty))
    }
}

/// The design changes a request (`method`, `params`) would make that can be computed ahead: an
/// `engine.execute` of such a command, or the steps of a batch up to the first that can't.
/// Empty: it just runs.
fn ahead_commands(method: &str, p: &Value) -> Vec<(String, Value)> {
    match method {
        "engine.execute" | "command" => match executed(p) {
            Some((command, params)) if computes_ahead(command, &params) => vec![(command.to_string(), params)],
            _ => Vec::new(),
        },
        "engine.batch" => {
            // Timings measure the run here: it must do the work.
            if ["timings", "profile"].iter().any(|k| p.get(*k).and_then(Value::as_bool) == Some(true)) {
                return Vec::new();
            }
            let Some(steps) = p.get("commands").and_then(Value::as_array) else { return Vec::new() };
            // As the batch will run them: with `coalesce_parameters`, a run of parameter edits
            // as one edit (#5822).
            let coalesce = p.get("coalesce_parameters").and_then(Value::as_bool).unwrap_or(false);
            solidcraft_engine::batch_params::plan(steps, coalesce)
                .into_iter()
                .map(|run| match run {
                    solidcraft_engine::batch_params::BatchRun::Step(i) => {
                        let step = steps.get(i)?;
                        let command = step.get("command").or_else(|| step.get("id")).and_then(Value::as_str)?;
                        Some((command.to_string(), step.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}))))
                    }
                    solidcraft_engine::batch_params::BatchRun::Params { changes, .. } => {
                        Some(("parameters.change".to_string(), solidcraft_engine::batch_params::combined_params(&changes)))
                    }
                })
                .map_while(|c| c.filter(|(command, params)| computes_ahead(command, params)))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Wait the app's wait for a worker: `Some(result)` when it finished within it.
fn within_wait(app: &SolveApp, rx: &Receiver<Option<Value>>) -> Option<Option<Value>> {
    let wait = app.apply_off_thread?;
    if wait.is_zero() {
        return None;
    }
    match rx.recv_timeout(wait) {
        Ok(r) => Some(r),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Some(None),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
    }
}

impl SolveApp {
    /// Since when an agent's design change has been computing off the window thread (ms), and
    /// its command.
    pub fn agent_computing(&self) -> Option<(f64, String)> {
        self.control_job
            .as_ref()
            .map(|j| (j.started, j.steps.as_ref().map_or_else(|| j.label.clone(), |s| s.shown())))
            .or_else(|| self.agent.computing())
            .or_else(|| crate::agent_chat::review::computing(&self.agent_chat))
    }

    /// Stop waiting for the agent's design change that is computing: it is answered as stopped
    /// and nothing of it is applied (its worker finishes on its own, unseen). Requests waiting
    /// behind it go on.
    pub fn stop_agent_command(&mut self) {
        self.stop_controlled_reads();
        let stopped = match self.control_job.take() {
            Some(mut job) => {
                // The job never reached the dispatch boundary; its admission is unused.
                crate::control::release_unstarted(self, &mut job.req);
                let response = crate::control::stopped_command_response(&job.req.method, &job.req.params, job.req.guarded.is_some(), &job.label);
                Some((job.req.reply, response))
            }
            None => self.agent.stop().map(|(reply, response, mut reissue)| {
                crate::control::release_reissue_unstarted(self, &mut reissue, solidcraft_engine::entitlement::RefusalCode::UnsafeToStart);
                (reply, response)
            }),
        };
        match stopped {
            Some((reply, response)) => {
                let _ = reply.send(response);
            }
            // The agent chat's request computing on its copy of the design (preview, then accept).
            None => {
                crate::agent_chat::review::stop(&mut self.agent_chat);
            }
        }
    }

    /// Take a control request off the window thread when it computes long (see the module
    /// notes). Returns it when it should simply run here now.
    pub(crate) fn offload_control(&mut self, ctx: &egui::Context, mut req: ControlRequest) -> Option<ControlRequest> {
        if req.refuse_cancelled_chat() {
            return None;
        }
        // Normally admitted by the queue; direct callers must pin the same live source too.
        if let Err(error) = crate::execution_guard::admit(self, &mut req) {
            let _ = req.reply.send(crate::execution_guard::refusal_json(&error));
            return None;
        }
        if self.apply_off_thread.is_none() || self.file_load.busy() || self.file_picker.is_some() {
            return Some(req);
        }
        // Reading the design (a measure, an inspection that measures): on a copy, answered from
        // there, preference or not. On a large assembly it took a minute on the window thread,
        // and the window, answering nothing meanwhile, was closed as not responding (#4604).
        if self.control_answering.len() < MAX_ANSWERING
            && let Some((id, params)) = reading(&req)
        {
            let Ok(controlled) = admit_copy_path(self, &mut req, id) else { return None };
            return self.answer_on_a_copy(req, id, params, controlled);
        }
        // A snapshot freezes and writes a large artifact: answer on a copy even when the general
        // agent-off-thread preference is off, so a large container never blocks a frame.
        if matches!(req.method.as_str(), "engine.execute" | "command")
            && self.control_answering.len() < MAX_ANSWERING
            && let Some((command, params)) = executed(&req.params)
            && spec_id(command) == Some("document.snapshot.capture")
        {
            let Ok(controlled) = admit_copy_path(self, &mut req, "document.snapshot.capture") else { return None };
            return self.answer_on_a_copy(req, "document.snapshot.capture", params, controlled);
        }
        if !self.preferences.agent_off_thread {
            return Some(req);
        }
        // An export: on a copy, answered from there.
        if matches!(req.method.as_str(), "engine.execute" | "command")
            && self.control_answering.len() < MAX_ANSWERING
            && let Some((command, params)) = executed(&req.params)
            && let Some(id) = spec_id(command).filter(|id| ANSWERED_ON_A_COPY.contains(id))
        {
            let Ok(controlled) = admit_copy_path(self, &mut req, id) else { return None };
            return self.answer_on_a_copy(req, id, params, controlled);
        }
        let commands = ahead_commands(&req.method, &req.params);
        let Some(first) = commands.first().map(|(c, _)| c.clone()) else { return Some(req) };
        // Computing ahead is only cache warming for the real dispatch, which resolves this same
        // typed admission; refuse a controlled entry without authority before any worker starts.
        if let Err(refusal) = crate::control::admit_once(self, &mut req) {
            let _ = req.reply.send(crate::control::admission_refusal_json(&refusal));
            return None;
        }
        let Some((work, rx, steps)) = begin_ahead(self, &req, commands, 0) else { return Some(req) };
        let job = ControlJob { label: label(self, &first), req, work, done: rx, started: crate::now_ms(), steps: Some(steps), controlled: None };
        if within_wait(self, &job.done).is_some() {
            // Computed within the wait: it runs now, as before, finding what was computed.
            let Work::Ahead { kept, .. } = job.work else { return Some(job.req) };
            self.finish_control_ahead(ctx, job.req, kept);
            return None;
        }
        self.control_job = Some(job);
        None
    }

    /// Run command `id` on a copy of the session, off the window thread, and answer `req` from
    /// there: at once when it is done within the app's wait, otherwise once it is in. `None`: it
    /// is taken care of; `Some`: it runs here (no threads).
    fn answer_on_a_copy(&mut self, req: ControlRequest, id: &'static str, params: Value, controlled: Option<Controlled>) -> Option<ControlRequest> {
        let scope = match crate::control::request_scope(self, &req.params) {
            Ok(scope) => scope,
            Err(error) => return self.unstarted_read(req, controlled, error),
        };
        // The request's own level (#9184) checks it on the copy, and the reply says so, as
        // `control::handle` does; a word that is no level is refused there.
        let level = match crate::control::request_param_check(&req.method, &req.params) {
            Ok(level) => level,
            Err(error) => return self.unstarted_read(req, controlled, error),
        };
        // Capture on the owner before scratch resets its followed revision. The worker only
        // reads this pinned copy; answered rechecks the live source, never the scratch stamp.
        let source = req.guarded.as_ref().map(|_| self.session.document_stamp());
        let mut copy = self.session.scratch();
        if id == "document.snapshot.capture" {
            // `scratch` drops the file path; a snapshot must resolve relative design links from
            // the caller's design folder, never from the process working directory.
            copy.path = self.session.path.clone();
        }
        if let Some(level) = level {
            copy.param_check = level;
        }
        // The copy has no history and no file (#9624): an inspection reports the design's own.
        let s = &self.session;
        let own = (id == "document.inspect").then(|| (s.undo.len(), s.redo.len(), s.path.clone()));
        // The dispatch boundary of a controlled read: from here on it settles in `answered`.
        if controlled.is_some() {
            req.work_started.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let rx = spawn(scope, move || {
            let mut reply = match copy.execute(id, &params) {
                Ok(mut v) => {
                    if let (Some((undo, redo, path)), Some(o)) = (own, v.as_object_mut()) {
                        o.insert("undo".into(), json!(undo));
                        o.insert("redo".into(), json!(redo));
                        o.insert("path".into(), json!(path));
                    }
                    json!({"ok": true, "result": v})
                }
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            };
            if let Some(level) = level {
                reply["param_check"] = json!(level.name());
            }
            Some(reply)
        });
        let Some(rx) = rx else {
            if controlled.is_none() {
                return Some(req);
            }
            return self.unstarted_read(req, controlled, "this build cannot read the design on a copy".into());
        };
        let job =
            ControlJob { label: label(self, id), req, work: Work::Answer { source }, done: rx, started: crate::now_ms(), steps: None, controlled };
        match within_wait(self, &job.done) {
            Some(reply) => self.answered(job, reply),
            None => self.control_answering.push(job),
        }
        None
    }

    /// A read that could not start on a copy. A free one runs here, as before (`Some`); a
    /// controlled one was admitted, so it is answered and settled now, as having had no effect.
    fn unstarted_read(&mut self, req: ControlRequest, controlled: Option<Controlled>, error: String) -> Option<ControlRequest> {
        let Some(controlled) = controlled else { return Some(req) };
        req.local_no_effect.store(true, std::sync::atomic::Ordering::Relaxed);
        let out =
            crate::control::finish_dispatch(self, &req, &controlled.admission, crate::control::Outcome::Done(json!({"ok": false, "error": error})));
        let _ = req.reply.send(outcome_value(out));
        None
    }

    /// An export's or a reading's reply, from its copy, with the echo an export leaves when it
    /// runs here. A controlled read's goes through [`SolveApp::answered_controlled`].
    fn answered(&mut self, mut job: ControlJob, reply: Option<Value>) {
        if let Some(controlled) = job.controlled.take() {
            return self.answered_controlled(job, controlled, reply);
        }
        if job.req.refuse_cancelled_chat() {
            return;
        }
        if !crate::control::free_entry(job.req.entry) && job.req.admission.is_none() {
            let _ = job.req.reply.send(crate::control::async_controlled_refusal(&job.req).unwrap_or_else(|| json!({"ok":false,"error":"refused"})));
            return;
        }
        if let Err(error) = crate::execution_guard::check_application(self, &job.req) {
            let _ = job.req.reply.send(crate::execution_guard::refusal_json(&error));
            return;
        }
        let mut reply = reply.unwrap_or_else(|| json!({"ok": false, "error": format!("`{}` ended without a reply", job.label)}));
        if reply["ok"] == true
            && let Work::Answer { source: Some(source) } = &job.work
        {
            reply["documentStamp"] = crate::execution_guard::stamp_json(source);
        }
        let export = executed(&job.req.params).and_then(|(command, _)| spec_id(command)).is_some_and(|id| ANSWERED_ON_A_COPY.contains(&id));
        if reply["ok"] == true && export {
            let language = crate::i18n::Locale::for_language(&self.preferences.language);
            self.session.echo(language.format("ui.command_done", &[("command", job.label.as_str())]));
        }
        let _ = job.req.reply.send(reply);
    }

    /// A controlled read's reply, through the same commit gate and single settlement as a
    /// dispatch here ([`crate::control::finish_dispatch`]).
    ///
    /// While the gate is open, a reply the host withholds (see [`Withheld`], or a document that
    /// changed while it read) is settled as having had no effect: the read ran on a copy that is
    /// dropped, a stopped run shows nothing new, and what a finished one shows is taken away. Once
    /// the gate is closed nothing is decided here: the result takes the existing late path, which
    /// discards it and defers its settlement under the original operation, never releasing it.
    fn answered_controlled(&mut self, job: ControlJob, controlled: Controlled, reply: Option<Value>) {
        let req = &job.req;
        let reply = reply.unwrap_or_else(|| json!({"ok": false, "error": format!("`{}` ended without a reply", job.label)}));
        let out = if crate::control::commit_open(self, req, &controlled.admission) {
            let mut withheld = controlled.withheld;
            if withheld.is_none() && crate::control::chat_turn_cancelled(req.chat_turn.as_deref()) {
                withheld = Some(Withheld::TurnEnded);
            }
            let finished = reply["ok"] == true;
            let withhold = |reply: Value| {
                if finished {
                    let params = reading(req).map(|(_, params)| params).unwrap_or_default();
                    (controlled.row.withdraw)(&controlled.document, &params);
                }
                req.local_no_effect.store(true, std::sync::atomic::Ordering::Relaxed);
                reply
            };
            match (withheld, crate::execution_guard::check_application(self, req)) {
                (Some(reason), _) => withhold(reason.reply(req, &job.label)),
                // The document changed while it read: the reply is about a design no longer shown.
                (None, Err(error)) => withhold(crate::execution_guard::refusal_json(&error)),
                (None, Ok(_)) => stamped(&job.work, reply),
            }
        } else {
            reply
        };
        let out = crate::control::finish_dispatch(self, req, &controlled.admission, crate::control::Outcome::Done(out));
        let _ = req.reply.send(outcome_value(out));
    }

    /// Each frame, for a controlled read on a copy: stop it when its authorization is about to
    /// end, or its chat turn ended, and keep stopping it until it answers. Otherwise move its
    /// watchdog's stop time to the live (possibly renewed) cutoff.
    fn watch_controlled_reads(&mut self) {
        for i in 0..self.control_answering.len() {
            let Some(job) = self.control_answering.get(i) else { continue };
            let Some(controlled) = job.controlled.as_ref() else { continue };
            let left = crate::control::authorization_left(self, &job.req, &controlled.admission);
            let reason = if controlled.withheld.is_some() {
                controlled.withheld
            } else if crate::control::chat_turn_cancelled(job.req.chat_turn.as_deref()) {
                Some(Withheld::TurnEnded)
            } else if left.is_none_or(|left| left < STOP_MARGIN) {
                Some(Withheld::AuthorizationEnded)
            } else {
                None
            };
            let Some(controlled) = self.control_answering.get_mut(i).and_then(|job| job.controlled.as_mut()) else { continue };
            match (reason, left, controlled.watchdog.as_ref()) {
                (Some(reason), ..) => {
                    controlled.withheld = Some(reason);
                    (controlled.row.stop)();
                }
                (None, Some(left), Some(watchdog)) => watchdog.moved(left),
                (None, ..) => {}
            }
        }
    }

    /// Stop the controlled reads computing on copies (Stop was pressed): each answers soon after,
    /// as stopped, and is settled as having had no effect.
    fn stop_controlled_reads(&mut self) {
        for controlled in self.control_answering.iter_mut().filter_map(|job| job.controlled.as_mut()) {
            controlled.withheld.get_or_insert(Withheld::Stopped);
            (controlled.row.stop)();
        }
    }

    /// Each frame: exports that are done reply; a design change whose evaluation is in runs.
    pub(crate) fn poll_control_jobs(&mut self, ctx: &egui::Context) {
        if !self.control_answering.is_empty() {
            self.watch_controlled_reads();
            let mut i = 0;
            while i < self.control_answering.len() {
                // A controlled read whose turn ended is stopped above and settles when it answers.
                if self.control_answering.get(i).is_some_and(|job| job.controlled.is_none() && job.req.refuse_cancelled_chat()) {
                    let mut job = self.control_answering.remove(i);
                    crate::control::release_unstarted(self, &mut job.req);
                    continue;
                }
                match self.control_answering.get(i).and_then(ControlJob::poll) {
                    Some(reply) => {
                        let job = self.control_answering.remove(i);
                        self.answered(job, reply);
                    }
                    None => i += 1,
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let Some(mut job) = self.control_job.take() else { return };
        if job.req.refuse_cancelled_chat() {
            crate::control::release_unstarted(self, &mut job.req);
            return;
        }
        // A guarded request never waits for a new base or uses legacy Arc ancestry to decide
        // its target. In particular, leaving and returning cannot revive its original token.
        if let Err(error) = crate::execution_guard::check_application(self, &job.req) {
            crate::control::release_unstarted(self, &mut job.req);
            let _ = job.req.reply.send(crate::execution_guard::refusal_json(&error));
            return;
        }
        // The design it computes for has left the window (closed, another tab, a file opened):
        // refused now, however long its computation has left, so the requests behind it, which
        // are for the design shown, don't wait for it.
        let left = matches!(&job.work, Work::Ahead { doc, .. } if !shows_design(&self.session, doc));
        if left {
            crate::control::release_unstarted(self, &mut job.req);
            let _ = job.req.reply.send(design_replaced(&job.label));
            return;
        }
        if job.poll().is_none() {
            self.control_job = Some(job);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        // A file opening meanwhile: the request waits for it as any design change does.
        if self.file_load.busy() || self.file_picker.is_some() {
            self.control_held.push_front(job.req);
            return;
        }
        let Work::Ahead { commands, doc, revision, tries, kept } = job.work else {
            crate::control::release_unstarted(self, &mut job.req);
            return;
        };
        let s = &self.session;
        let same = Arc::ptr_eq(&s.doc, &doc);
        if !shows_design(s, &doc) {
            crate::control::release_unstarted(self, &mut job.req);
            let _ = job.req.reply.send(design_replaced(&job.label));
            return;
        }
        if job.req.guarded.is_none() && (!same || s.revision != revision) && tries < MAX_TRIES {
            // A person's edit went first: computed again for the design as it is now.
            if let Some((work, rx, steps)) = begin_ahead(self, &job.req, commands, tries + 1) {
                self.control_job = Some(ControlJob { work, done: rx, steps: Some(steps), ..job });
                return;
            }
        }
        self.big_operation |= crate::now_ms() - job.started >= crate::recovery_ui::BIG_OPERATION_MS;
        self.finish_control_ahead(ctx, job.req, kept);
        ctx.request_repaint();
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// The candidate store is process-global; tests that replace it take this lock so parallel
    /// libtest threads cannot swap it under one another. A poisoned lock still runs the test.
    pub(crate) static CANDIDATE_STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A real copy reading, already computed, held just before the owner's answer path.
    pub(crate) fn completed_read(app: &mut SolveApp, req: ControlRequest, id: &'static str, params: Value) -> ControlJob {
        let source = req.guarded.as_ref().map(|_| app.session.document_stamp());
        let mut copy = app.session.scratch();
        let reply = match copy.execute(id, &params) {
            Ok(v) => json!({"ok":true,"result":v}),
            Err(e) => json!({"ok":false,"error":e.to_string()}),
        };
        let (tx, done) = std::sync::mpsc::channel();
        tx.send(Some(reply)).unwrap();
        ControlJob { req, work: Work::Answer { source }, done, started: crate::now_ms(), label: id.into(), steps: None, controlled: None }
    }

    thread_local! {
        static STOPPABLE: std::cell::RefCell<Vec<&'static StoppableRead>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Make `id` a stoppable read on this test's thread (the window thread in a test), with its
    /// own `stop` and `withdraw`. Other tests' threads keep `develop`'s empty table.
    pub(crate) fn stoppable_on_this_thread(id: &'static str, stop: fn(), withdraw: fn(&str, &Value)) {
        let row: &'static StoppableRead = Box::leak(Box::new(StoppableRead { id, stop, withdraw }));
        STOPPABLE.with(|rows| rows.borrow_mut().push(row));
    }

    pub(super) fn stoppable_row(id: &str) -> Option<&'static StoppableRead> {
        STOPPABLE.with(|rows| rows.borrow().iter().copied().find(|row| row.id == id))
    }

    pub(crate) fn answer_completed(app: &mut SolveApp, job: ControlJob) {
        let reply = within_wait(app, &job.done).expect("a pre-completed receiver takes the immediate branch");
        app.answered(job, reply);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::scenario::Harness;

    /// #11: a snapshot capture is answered on a copy off the window thread even when the general
    /// agent-off-thread preference is off, and the copy keeps the caller's design path so a
    /// relative design link resolves from the design's folder.
    #[test]
    fn snapshot_capture_answers_on_a_copy_with_the_callers_path() {
        let _store = crate::control_job::test_support::CANDIDATE_STORE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = Harness::new();
        h.app.home.open = false;
        h.app.apply_off_thread = Some(std::time::Duration::ZERO);
        h.app.preferences.agent_off_thread = false;
        let dir = h.dir.clone();
        let part_path = dir.join("Part.sc");
        let mut part = solidcraft_engine::Session::default();
        part.execute("solid.box", &json!({"length": 20, "width": 10, "height": 5})).unwrap();
        part.execute("file.save", &json!({"path": part_path})).unwrap();
        let assembly_path = dir.join("Assembly.sc");
        let mut assembly = solidcraft_engine::Session::default();
        let inserted = assembly.execute("component.insert_design", &json!({"path": part_path})).unwrap();
        assembly.execute("file.save", &json!({"path": assembly_path})).unwrap();
        let component = inserted["component"].as_u64().unwrap();
        assembly.path = Some(assembly_path.to_string_lossy().into_owned());
        assembly.doc_mut().links.get_mut(&component).unwrap().path = "Part.sc".into();
        h.app.session = assembly;

        let store = dir.join("candidates");
        solidcraft_engine::cmd::set_candidate_store(Some(store.clone()));
        let stamp = h.app.session.document_stamp();
        let (req, rx) = crate::control::ControlRequest::trusted_gui(
            "engine.execute",
            json!({"command":"document.snapshot.capture","params":{},
                "expectedDocument":{"documentId":stamp.document_id,"revision":stamp.revision.to_string()}}),
        );
        let gate = crate::history_off_thread_tests::guarded_workers::gate();
        assert!(h.app.offload_control(&h.ctx, req).is_none(), "the capture runs off the window thread");
        gate.computed();
        gate.release();
        h.app.poll_control_jobs(&h.ctx);
        let reply = rx.try_recv().expect("the off-frame capture answered");
        assert_eq!(reply["ok"], true, "{reply}");
        let candidate = &reply["result"]["candidate"];
        let id = candidate["candidateId"].as_str().unwrap();
        let verified = solidcraft_engine::io::candidate::lookup(&store, id).unwrap();
        let design = verified.files.iter().find(|file| file.path == "design.sc").unwrap();
        let captured = solidcraft_engine::io::read_design(&design.bytes).unwrap();
        assert_eq!(
            captured.links.get(&component).map(|link| link.path.as_str()),
            Some(format!("inputs/design-link-{component}.sc").as_str()),
            "the captured link names the frozen input, read with the caller's design path"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
