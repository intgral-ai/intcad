//! The ACP backend (desktop only): starts the agent the user picked as a child process and holds
//! the protocol's side of the conversation (initialize, the session, prompts, permission questions,
//! sign-in). The panel sees only [`Event`]s.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use serde::{Serialize, Serializer};
use serde_json::{Value, json};
use solidcraft_engine::{DocumentStamp, entitlement::BridgeScopeCheck};

use super::acp::{self, Incoming};
use super::backend::{ChatBackend, Event, PromptPart, StartContext, TestError};
use super::connections;
use super::process::{AgentProcess, FromAgent, ProcessIoError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// `session/close` of a guessing session.
    CloseSession,
    Initialize,
    SessionNew,
    RetiredSessionNew,
    Prompt,
    Authenticate,
    SetMode,
    SetModel,
    SetConfig,
    /// One of the built-in agent's `_intcad/connections/*` requests.
    Connections(connections::What),
}

/// Give a cancelled prompt time to finish before disconnecting an unresponsive agent.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// One budget to get a retired session's ID (if still opening) and finish its cleanup.
const PREDICTION_TEARDOWN_GRACE: Duration = Duration::from_secs(5);

/// The ACP prompt frame borrows the text and base64 from the composer. Preflight can therefore
/// reject an oversized prompt without copying all of its attachments into JSON values.
#[derive(Serialize)]
struct PromptRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: PromptParams<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptParams<'a> {
    session_id: &'a str,
    prompt: PromptBlocks<'a>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    meta: Option<Value>,
}

struct PromptBlocks<'a>(&'a [PromptPart]);

impl Serialize for PromptBlocks<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Resource<'a> {
            uri: &'a str,
            #[serde(rename = "mimeType")]
            mime: &'a str,
            blob: &'a str,
        }
        #[derive(Serialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum Block<'a> {
            Text {
                text: &'a str,
            },
            Image {
                #[serde(rename = "mimeType")]
                mime: &'a str,
                data: &'a str,
            },
            Resource {
                resource: Resource<'a>,
            },
        }
        serializer.collect_seq(self.0.iter().map(|part| match part {
            PromptPart::Text(text) => Block::Text { text },
            PromptPart::Image { mime, base64 } => Block::Image { mime, data: base64 },
            PromptPart::Document { uri, mime, base64, .. } => Block::Resource { resource: Resource { uri, mime, blob: base64 } },
        }))
    }
}

/// Variables the agent doesn't inherit: a control token set for the app's own `--control` port
/// would otherwise reach the agent and outrank the chat endpoint's own token file.
const ENV_REMOVE: &[&str] = &["SOLIDCRAFT_CONTROL_TOKEN"];
const PACKAGED_ENV_REMOVE: &[&str] = &["SOLIDCRAFT_CONTROL_TOKEN", "SOLIDCRAFT_MASTRA_SDK", "NODE_OPTIONS", "NODE_PATH"];

/// The variable that tells the built-in agent where to keep its sign-ins and its own data.
pub const DATA_DIR_ENV: &str = "MASTRA_APP_DATA_DIR";

/// Mastra's own switch for its usage reporting. The built-in agent is always started with it
/// set, whatever program it is; the agent's entry sets it for itself as well.
pub const TELEMETRY_OFF_ENV: &str = "MASTRA_TELEMETRY_DISABLED";

/// Lean guessing sessions on a plan sign-in. Off until the agent's provider says it may be.
const LEAN_ON_PLANS: bool = false;
/// The kind of sign-in that is an API key, as the Claude adapter names it.
const KEY_SIGN_IN: &str = "api_key";

struct CadTurn {
    prompt: u64,
    session: String,
    document_id: String,
    active: Arc<AtomicBool>,
    replies_seen: HashSet<String>,
    check: Option<Arc<std::sync::Mutex<BridgeScopeCheck>>>,
    /// A dispatched call may have changed the design or left its outcome unknown.
    effect: bool,
}

impl CadTurn {
    /// The callback names the session, prompt and document this desktop turn actually owns, and
    /// that turn is still live. A callback from an older prompt never borrows a later one.
    fn owns(&self, stamp: &DocumentStamp, params: &Value) -> bool {
        self.active.load(Ordering::Acquire)
            && stamp.document_id == self.document_id
            && params["sessionId"].as_str() == Some(self.session.as_str())
            && params["turn"].as_str() == Some(self.prompt.to_string().as_str())
    }
}

struct CadReply {
    turn: u64,
    rpc: Value,
    result: Receiver<(Result<Value, String>, bool)>,
}

struct PackagedCad {
    tools: Sender<crate::ControlRequest>,
    wake: Arc<dyn Fn() + Send + Sync>,
    document: Option<DocumentStamp>,
    next_active: Option<Arc<AtomicBool>>,
    next_check: Option<BridgeScopeCheck>,
    turn: Option<CadTurn>,
    reply: Option<CadReply>,
    /// Calls whose turn was retired before the worker answered: answered at once so the Agent
    /// can use its one in-flight slot in the next turn.
    stale: Vec<Value>,
    /// The last turn had (or may have had) a design effect. Read before the next prompt clears it.
    last_effect: bool,
}

impl PackagedCad {
    fn retire(&mut self) {
        if let Some(turn) = self.turn.take() {
            turn.active.store(false, Ordering::Release);
            self.last_effect |= turn.effect;
        }
        if let Some(reply) = self.reply.take() {
            // The worker may already have applied the change; never replay this turn by itself.
            self.last_effect = true;
            if self.stale.len() < 64 {
                self.stale.push(reply.rpc);
            }
        }
        self.next_active = None;
        self.next_check = None;
    }
}

impl Drop for PackagedCad {
    fn drop(&mut self) {
        self.retire();
    }
}

pub struct AcpBackend {
    proc: AgentProcess,
    cwd: std::path::PathBuf,
    bridge: Option<acp::McpBridge>,
    /// The agent's `initialize` reply: which routes the system note can take into a session.
    init: Value,
    /// The system note the next session gets ([`acp::system_note`]).
    note: String,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    session: Option<String>,
    agent: String,
    images: bool,
    documents: bool,
    auth: Vec<acp::AuthMethod>,
    /// Open permission questions: our key → the agent's request id.
    permissions: HashMap<u64, Value>,
    next_key: u64,
    cancelling: Option<(u64, Instant)>,
    prediction_teardown_started: Option<Instant>,
    prediction_open_queued: bool,
    prompt_refused: Option<ProcessIoError>,
    exited: bool,
    config_pending: Option<(u64, String)>,
    modern_config: bool,
    /// The agent is the built-in one: after `initialize` it is handed its keys and custom
    /// addresses (`setup`), and a session is opened only when the panel asks for one, since the
    /// agent refuses a session while nothing is connected.
    built_in: bool,
    /// `initialize` is answered.
    initialized: bool,
    /// The `setup` given before `initialize` was answered: it goes right after.
    setup: Option<connections::Request>,
    /// The project folder the next session may work in ([`ChatBackend::set_project`]). Only the
    /// built-in agent is handed it.
    project: Option<std::path::PathBuf>,
    /// What the open session used, as the agent reported it, not yet taken.
    usage: super::backend::Usage,
    /// The note of lean sessions ([`AcpBackend::lean_sessions`]); `None`: sessions as the chat's.
    lean: Option<String>,
    /// The `session/new` out was a lean one: its id.
    lean_request: Option<u64>,
    /// The last session asked for is a lean one.
    lean_session: bool,
    /// The agent refused a lean session: this process asks for none again.
    lean_refused: bool,
    /// Lean sessions on a plan sign-in too ([`LEAN_ON_PLANS`]).
    lean_on_plans: bool,
    /// The kind of sign-in the agent last reported (`_auth/status_update`).
    sign_in: Option<String>,
    cad: Option<PackagedCad>,
    /// The variables taken out of the agent's environment when it was started.
    #[cfg(test)]
    env_removed: &'static [&'static str],
}

impl AcpBackend {
    /// Start the built-in agent and say hello. Nothing follows by itself: the panel hands over
    /// the keys ([`ChatBackend::connections`]) and opens the session ([`ChatBackend::new_session`]).
    /// `data`: the folder of the app's own where the agent keeps its sign-ins and its other
    /// data (`MASTRA_APP_DATA_DIR`), so it shares none with a Mastra Code the user installed.
    /// It is not optional: without it the entry leaves the agent in the user's own home.
    /// Mastra's usage reporting is switched off for it ([`TELEMETRY_OFF_ENV`]).
    pub fn start_builtin(argv: &[String], ctx: StartContext, data: &std::path::Path) -> Result<AcpBackend, String> {
        let env = [(DATA_DIR_ENV, data.as_os_str()), (TELEMETRY_OFF_ENV, std::ffi::OsStr::new("1"))];
        Self::spawn(argv, ctx, &env, ENV_REMOVE, true)
    }

    pub(super) fn start_packaged(argv: &[String], mut ctx: StartContext, data: &std::path::Path) -> Result<AcpBackend, String> {
        let argv = Self::packaged_argv(argv)?;
        let tools = ctx.tools.take().ok_or_else(|| "The packaged Agent has no desktop tool queue".to_string())?;
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::from(ctx.wake);
        let process_wake = Arc::clone(&wake);
        ctx.wake = Box::new(move || process_wake());
        let mut backend = Self::spawn_packaged(&argv, ctx, data)?;
        backend.cad = Some(PackagedCad {
            tools,
            wake,
            document: None,
            next_active: None,
            next_check: None,
            turn: None,
            reply: None,
            stale: Vec::new(),
            last_effect: false,
        });
        Ok(backend)
    }

    /// The package's built-in agent for next-step suggestions: started as the chat's
    /// ([`AcpBackend::start_packaged`]: the package's own Node and entry, the same variables taken out
    /// of its environment), but with no desktop tool queue and no MCP bridge, so its sessions are
    /// never handed the design's tools.
    pub(super) fn start_packaged_guesser(argv: &[String], mut ctx: StartContext, data: &std::path::Path) -> Result<AcpBackend, String> {
        let argv = Self::packaged_argv(argv)?;
        ctx.tools = None;
        Self::spawn_packaged(&argv, ctx, data)
    }

    /// The package's Node and entry ([`AcpBackend::packaged_agent`]) as the argument list to start.
    fn packaged_argv(argv: &[String]) -> Result<[String; 2], String> {
        let (program, entry) = Self::packaged_agent(argv)?;
        Ok([
            program.to_str().ok_or_else(|| "The packaged Agent's Node path is not valid text".to_string())?.to_string(),
            entry.to_str().ok_or_else(|| "The packaged Agent's entry path is not valid text".to_string())?.to_string(),
        ])
    }

    /// Start the package's agent with its data folder, its usage reporting off and the variables
    /// that could make it load other code taken out ([`PACKAGED_ENV_REMOVE`]). No MCP bridge.
    fn spawn_packaged(argv: &[String; 2], mut ctx: StartContext, data: &std::path::Path) -> Result<AcpBackend, String> {
        ctx.bridge = None;
        let env = [(DATA_DIR_ENV, data.as_os_str()), (TELEMETRY_OFF_ENV, std::ffi::OsStr::new("1"))];
        Self::spawn(argv, ctx, &env, PACKAGED_ENV_REMOVE, true)
    }

    /// The variables taken out of the agent's environment when it was started.
    #[cfg(test)]
    pub(crate) fn env_removed(&self) -> &'static [&'static str] {
        self.env_removed
    }

    /// The one program pair the paid lane may start: this package's own Node, entry and SDK.
    /// Discovery found the folder; this checks the bytes belong together before any authority
    /// is offered. Anything else is refused, not downgraded to another folder.
    pub(super) fn packaged_agent(argv: &[String]) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
        let [program, entry] = argv else { return Err("The packaged Agent must be one program and one entry".into()) };
        let (program, entry) = (std::path::Path::new(program), std::path::Path::new(entry));
        let node = if cfg!(windows) { "node.exe" } else { "node" };
        let refused = || "The built-in Agent is not the package's own program".to_string();
        if !program.is_absolute() || program.file_name().and_then(std::ffi::OsStr::to_str) != Some(node) {
            return Err(refused());
        }
        if !entry.is_absolute() || entry.file_name().and_then(std::ffi::OsStr::to_str) != Some("main.mjs") {
            return Err(refused());
        }
        let root = std::fs::canonicalize(program.parent().ok_or_else(refused)?).map_err(|_| refused())?;
        if !root.join(node).is_file() {
            return Err(refused());
        }
        let entry = std::fs::canonicalize(entry).map_err(|_| refused())?;
        if entry.parent() != Some(root.join("agent").as_path()) {
            return Err(refused());
        }
        for needed in ["agent.json", "node_modules/@mastra/code-sdk/package.json", "node_modules/@agentclientprotocol/sdk/package.json"] {
            if !root.join(needed).is_file() {
                return Err(format!("The built-in Agent package is incomplete: {needed} is missing"));
            }
        }
        Ok((root.join(node), entry))
    }

    /// Start `argv` and say hello (`initialize`); the session follows when the agent answers.
    pub fn start(argv: &[String], ctx: StartContext) -> Result<AcpBackend, String> {
        Self::spawn(argv, ctx, &[], ENV_REMOVE, false)
    }

    fn spawn(
        argv: &[String],
        ctx: StartContext,
        env: &[(&str, &std::ffi::OsStr)],
        remove: &'static [&'static str],
        built_in: bool,
    ) -> Result<AcpBackend, String> {
        let wake = ctx.wake;
        let proc = AgentProcess::spawn_with_env(argv, &ctx.cwd, remove, env, built_in, wake)?;
        // An agent inside WSL is told the same folder and bridge as WSL shows them (#5546).
        let (cwd, bridge) =
            if acp::runs_in_wsl(argv) { (acp::wsl_path(&ctx.cwd), ctx.bridge.as_ref().map(acp::McpBridge::in_wsl)) } else { (ctx.cwd, ctx.bridge) };
        let mut b = AcpBackend {
            proc,
            cwd,
            bridge,
            init: Value::Null,
            note: acp::SYSTEM_NOTE.to_string(),
            next_id: 0,
            pending: HashMap::new(),
            session: None,
            agent: String::new(),
            images: false,
            documents: false,
            auth: Vec::new(),
            permissions: HashMap::new(),
            next_key: 0,
            cancelling: None,
            prediction_teardown_started: None,
            prediction_open_queued: false,
            prompt_refused: None,
            exited: false,
            config_pending: None,
            modern_config: false,
            built_in,
            initialized: false,
            setup: None,
            project: None,
            usage: super::backend::Usage::default(),
            lean: None,
            lean_request: None,
            lean_session: false,
            lean_refused: false,
            lean_on_plans: LEAN_ON_PLANS,
            sign_in: None,
            cad: None,
            #[cfg(test)]
            env_removed: remove,
        };
        b.request(Pending::Initialize, "initialize", acp::initialize_params(env!("CARGO_PKG_VERSION")));
        Ok(b)
    }

    fn request(&mut self, what: Pending, method: &str, params: Value) {
        self.next_id += 1;
        self.pending.insert(self.next_id, what);
        self.proc.send(&acp::request(self.next_id, method, params));
    }

    /// Updates belong to the open session. An empty session id is tolerated for older peers, but
    /// a named session other than the active one is retired and must not reach the panel.
    fn accepts_session(&self, session: &str) -> bool {
        match self.session.as_deref() {
            Some(active) => session.is_empty() || session == active,
            None => session.is_empty(),
        }
    }

    /// As if [`LEAN_ON_PLANS`] were on.
    #[cfg(test)]
    pub(crate) fn lean_on_plans(&mut self) {
        self.lean_on_plans = true;
    }

    /// Open every session of this process lean where the agent takes it
    /// ([`acp::lean_session_params`]), with `note` as its whole system prompt: the guessing
    /// sessions of next-step suggestions. Once the agent refuses one, sessions open as the chat's.
    pub fn lean_sessions(&mut self, note: &str) {
        self.lean = Some(note.to_string());
    }

    fn is_builtin_guesser(&self) -> bool {
        self.built_in && self.lean.is_some()
    }

    fn close_prediction(&mut self, session: String) -> bool {
        if !acp::closes_sessions(&self.init) {
            return false;
        }
        if !self.pending.values().any(|what| *what == Pending::CloseSession) {
            self.prediction_teardown_started.get_or_insert_with(Instant::now);
            self.request(Pending::CloseSession, "session/close", json!({"sessionId": session}));
        }
        true
    }

    fn open_queued_prediction(&mut self) {
        if self.prediction_open_queued
            && self.prediction_teardown_started.is_none()
            && !self.pending.values().any(|what| matches!(what, Pending::SessionNew | Pending::RetiredSessionNew))
        {
            self.prediction_open_queued = false;
            self.new_session();
        }
    }

    fn stop_guesser_after_teardown_failure(&mut self, why: &str) -> Event {
        log::warn!("next-step suggestions: {why}; stopping the guessing process");
        self.proc.shutdown(false);
        self.finish_exit(vec![crate::i18n::tr("ui.agent_chat_exited").to_string()])
    }

    fn new_session(&mut self) {
        self.config_pending = None;
        self.pending.retain(|_, request| *request != Pending::SetConfig);
        // INT-1473: the built-in agent's file tools reach a project it is handed, and its note
        // says what they are for. Any other agent's session is as it was.
        // A guessing session is lean on an API key only: on a plan, and until the agent says what it
        // is signed in with, it runs as the agent opens it, less the user's MCP servers and every MCP tool.
        let guesses = self.lean.is_some();
        // The built-in agent's guessing session: the guessing note alone, marked for the agent to
        // open it read-only, with no MCP and no design tool (decision 27). Never a project.
        if let Some(note) = self.lean.as_deref().filter(|_| self.built_in) {
            let params = acp::next_step_session_params(&self.cwd, note);
            self.request(Pending::SessionNew, "session/new", params);
            self.lean_request = None;
            self.lean_session = false;
            return;
        }
        let may_be_lean = !self.lean_refused && (self.lean_on_plans || self.sign_in.as_deref() == Some(KEY_SIGN_IN));
        let lean = self.lean.as_deref().filter(|_| may_be_lean).and_then(|note| acp::lean_session_params(&self.cwd, &self.init, note));
        let sent_lean = lean.is_some();
        let mut params = lean.unwrap_or_else(|| {
            let project = self.project.as_deref().filter(|_| self.built_in);
            let note = if project.is_some() { acp::project_note(&self.note) } else { self.note.clone() };
            let params = acp::session_new_params(&self.cwd, self.bridge.as_ref(), &self.init, &note, project);
            if guesses { acp::without_mcp(params) } else { params }
        });
        // A guessing session is never handed the design's tools.
        if self.cad.is_some() && !guesses {
            params["_meta"]["intcad"]["cadBridge"] = json!({"version":1,"tools":solidcraft_mcp::tool_definitions()});
        }
        self.request(Pending::SessionNew, "session/new", params);
        self.lean_request = sent_lean.then_some(self.next_id);
        self.lean_session = sent_lean;
    }

    fn revoke_cad(&mut self) {
        if let Some(cad) = &mut self.cad {
            cad.retire();
        }
    }

    fn end_cad_prompt(&mut self, id: u64) {
        if self.cad.as_ref().and_then(|cad| cad.turn.as_ref()).is_some_and(|turn| turn.prompt == id) {
            self.revoke_cad();
        }
    }

    fn dispatch_cad(&mut self, rpc: Value, params: Value) {
        let Some(cad) = self.cad.as_mut() else {
            self.proc.send(&acp::reply_error(rpc, acp::METHOD_NOT_FOUND, "No private CAD dispatcher is attached to this Agent"));
            return;
        };
        let refuse = |rpc| self.proc.send(&acp::reply_error(rpc, -32602, "The desktop CAD callback is not bound to an available turn"));
        let (Some(turn), Some(stamp), Some(name), Some(arguments)) =
            (cad.turn.as_mut(), cad.document.as_ref(), params["name"].as_str(), params.get("arguments").filter(|a| a.is_object()))
        else {
            refuse(rpc);
            return;
        };
        if !turn.owns(stamp, &params)
            || !(rpc.is_u64() || rpc.as_str().is_some_and(|id| (1..=128).contains(&id.len())))
            || name.len() > 64
            || cad.reply.is_some()
            || turn.replies_seen.len() >= 1024
            || !turn.replies_seen.insert(rpc.to_string())
        {
            refuse(rpc);
            return;
        }
        let prompt = turn.prompt;
        let tools = cad.tools.clone();
        let active = Arc::clone(&turn.active);
        let document = stamp.clone();
        let working_dir = self.cwd.clone();
        let name = name.to_string();
        let arguments = arguments.clone();
        let wake = Arc::clone(&cad.wake);
        let (tx, result) = std::sync::mpsc::sync_channel(1);
        // This check remains reusable within one active turn and is run only off the window.
        let check = turn.check.clone().map(|source| Box::new(move || source.lock().ok().and_then(|check| check())) as BridgeScopeCheck);
        if std::thread::Builder::new()
            .name("acp-cad".into())
            .spawn(move || {
                let reply = super::host_bridge::serve_acp_cad(tools, check, active, document, working_dir, name, arguments);
                let _ = tx.send(reply);
                wake();
            })
            .is_err()
        {
            refuse(rpc);
            return;
        }
        cad.reply = Some(CadReply { turn: prompt, rpc, result });
    }

    fn poll_cad(&mut self) {
        let Some(cad) = self.cad.as_mut() else { return };
        for rpc in std::mem::take(&mut cad.stale) {
            self.proc.send(&acp::reply_error(rpc, -32603, "The originating CAD turn ended"));
        }
        let Some(reply) = cad.reply.take() else { return };
        let outcome = match reply.result.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty) => {
                cad.reply = Some(reply);
                return;
            }
            Err(TryRecvError::Disconnected) => (Err("The desktop CAD worker stopped".into()), true),
        };
        let (result, effect) = outcome;
        let current = cad.turn.as_ref().is_some_and(|turn| turn.prompt == reply.turn && turn.active.load(Ordering::Acquire));
        if effect {
            cad.last_effect = true;
            if let Some(turn) = cad.turn.as_mut().filter(|turn| turn.prompt == reply.turn) {
                turn.effect = true;
            }
        }
        if !current {
            self.proc.send(&acp::reply_error(reply.rpc, -32603, "The originating CAD turn ended"));
        } else {
            let message = match result {
                Ok(value) => json!({"jsonrpc":"2.0","id":reply.rpc,"result":value}),
                Err(error) => acp::reply_error(reply.rpc, -32603, &error),
            };
            self.proc.send(&message);
        }
    }

    fn handle(&mut self, msg: Incoming, out: &mut Vec<Event>) {
        match msg {
            Incoming::Reply { id, result } => {
                let Some(what) = self.pending.remove(&id) else { return };
                if what == Pending::Prompt {
                    self.end_cad_prompt(id);
                }
                if self.cancelling.is_some_and(|(request, _)| request == id) {
                    self.cancelling = None;
                }
                if self.lean_request == Some(id) {
                    self.lean_request = None;
                    // Any refusal, a sign-in one too: the user's own setup may be what signs in.
                    if let Err((_, message)) = &result {
                        log::warn!(
                            "next-step suggestions: the agent refused a lean session ({message}); its sessions open as the chat's from now on"
                        );
                        self.lean_refused = true;
                        self.new_session();
                        return;
                    }
                }
                match (what, result) {
                    (Pending::Initialize, Ok(v)) => {
                        self.agent = acp::agent_info(&v);
                        self.images = acp::takes_images(&v);
                        self.documents = acp::takes_documents(&v);
                        self.auth = acp::auth_methods(&v);
                        self.init = v;
                        self.initialized = true;
                        if !self.built_in {
                            self.new_session();
                        } else if let Some(setup) = self.setup.take() {
                            self.send_connections(&setup);
                        }
                    }
                    (Pending::SessionNew, Ok(v)) => match v.get("sessionId").and_then(Value::as_str) {
                        Some(s) => {
                            // New chat has opened successfully: retire the old turn before Ready.
                            // Its late reply and cancellation deadline must not end the new turn.
                            if let Some((request, _)) = self.cancelling.take() {
                                self.pending.remove(&request);
                                out.push(Event::TurnEnded { reason: "cancelled".into() });
                            }
                            self.session = Some(s.to_string());
                            // The panel retires the previous session's state with this id before
                            // anything else the new session sends.
                            out.push(Event::Session(s.to_string()));
                            match acp::config_options(&v) {
                                acp::ConfigParse::Snapshot(options) => {
                                    self.modern_config = true;
                                    out.push(Event::ConfigOptions(options));
                                }
                                _ => {
                                    self.modern_config = false;
                                    if let Some((current, available)) = acp::modes(&v) {
                                        out.push(Event::Modes { current, available });
                                    }
                                    if let Some((current, available)) = acp::models(&v) {
                                        out.push(Event::Models { current, available });
                                    }
                                }
                            }
                            out.push(Event::Documents(self.documents));
                            out.push(Event::Ready { agent: self.agent.clone(), images: self.images });
                        }
                        None => out.push(Event::Error("session/new: no sessionId".into())),
                    },
                    (Pending::RetiredSessionNew, result) => {
                        let closing = result
                            .ok()
                            .and_then(|v| v.get("sessionId")?.as_str().map(str::to_owned))
                            .is_some_and(|session| self.close_prediction(session));
                        if !closing {
                            self.prediction_teardown_started = None;
                        }
                        self.open_queued_prediction();
                    }
                    (Pending::Prompt, Ok(v)) => {
                        if let Some(usage) = acp::turn_usage(&v) {
                            self.usage.merge(usage);
                        }
                        let reason = v.get("stopReason").and_then(Value::as_str).unwrap_or("");
                        out.push(Event::TurnEnded { reason: if reason == "end_turn" { String::new() } else { reason.to_string() } });
                    }
                    // Signed in: try the session again.
                    (Pending::Authenticate, Ok(_)) => self.new_session(),
                    (Pending::CloseSession, result) if self.prediction_teardown_started.take().is_some() => {
                        if result.is_ok() {
                            self.open_queued_prediction();
                        } else {
                            out.push(self.stop_guesser_after_teardown_failure("prediction session close failed"));
                        }
                    }
                    (Pending::SetMode | Pending::SetModel | Pending::CloseSession, Ok(_)) => {}
                    // Other agents that offer close keep their existing best-effort behavior.
                    (Pending::CloseSession, Err((_, message))) => log::debug!("the agent did not close a session: {message}"),
                    (Pending::SetConfig, Ok(v)) => {
                        if let Some((request_id, config_id)) = self.config_pending.take()
                            && request_id == id
                        {
                            let result = match acp::config_options(&v) {
                                acp::ConfigParse::Snapshot(o) => Ok(o),
                                _ => Err("The agent returned no valid configuration options.".into()),
                            };
                            out.push(Event::ConfigOptionFinished { id: config_id, result });
                        }
                    }
                    (Pending::Connections(what), Ok(v)) => out.push(match connections::Snapshot::parse(&v) {
                        Some(snapshot) if what == connections::What::Setup && self.lean.is_some() => Event::PredictionSetup(snapshot),
                        Some(snapshot) if what == connections::What::Logout => Event::ConnectionsLogout(snapshot),
                        Some(snapshot) => Event::Connections(snapshot),
                        None => Event::ConnectionsFailed { what, code: 0 },
                    }),
                    // Before the general arm: `busy` has the code of ACP's "authentication
                    // required", and a refused request for a sign-in is no reason to drop the session.
                    (Pending::Connections(what), Err((code, _))) => out.push(Event::ConnectionsFailed { what, code }),
                    // `session/set_model` is still unstable in ACP: an agent can list `models` and
                    // still not serve it (#9970). It keeps its own model; that is no error to show.
                    (Pending::SetModel, Err((acp::METHOD_NOT_FOUND, message))) => {
                        log::debug!("agent does not serve session/set_model: {message}");
                    }
                    (what, Err((code, message))) => {
                        let config_error = what == Pending::SetConfig;
                        if what == Pending::SetConfig
                            && let Some((request_id, config_id)) = self.config_pending.take()
                            && request_id == id
                        {
                            out.push(Event::ConfigOptionFinished { id: config_id, result: Err(message.clone()) });
                        }
                        let auth = code == acp::AUTH_REQUIRED || message.to_ascii_lowercase().contains("auth");
                        if auth {
                            self.session = None;
                            self.modern_config = false;
                            self.config_pending = None;
                            self.pending.retain(|_, request| *request != Pending::SetConfig);
                            if what == Pending::Prompt {
                                out.push(Event::TurnEnded { reason: "auth".into() });
                            }
                            out.push(Event::AuthRequired { methods: self.auth.clone() });
                        } else if what == Pending::SetModel {
                            out.push(Event::ModelRejected(message));
                        } else if !config_error {
                            if what == Pending::Prompt {
                                out.push(Event::TurnEnded { reason: "error".into() });
                            }
                            out.push(Event::Error(message));
                        }
                    }
                }
            }
            Incoming::Text { session, thought, text } if self.accepts_session(&session) => out.push(Event::Text { thought, text }),
            Incoming::Tool { session, id, title, kind, status, input, output, picture } if self.accepts_session(&session) => {
                out.push(Event::Tool { id: id.clone(), title, kind, status, input, output });
                if let Some(bytes) = picture {
                    out.push(Event::ToolPicture { id, bytes });
                }
            }
            Incoming::Plan { session, steps } if self.accepts_session(&session) => out.push(Event::Plan(steps)),
            Incoming::Permission { session, rpc, title, kind, options } => {
                if !self.accepts_session(&session) {
                    // A retired or unknown session cannot be approved: answer the protocol's
                    // cancellation outcome instead of leaving the agent waiting.
                    self.proc.send(&acp::permission_reply(rpc, None));
                    return;
                }
                self.next_key += 1;
                self.permissions.insert(self.next_key, rpc);
                out.push(Event::Permission { key: self.next_key, title, kind, options });
            }
            Incoming::Usage { session, usage } if self.accepts_session(&session) => self.usage.merge(usage),
            Incoming::Text { .. } | Incoming::Tool { .. } | Incoming::Plan { .. } | Incoming::Usage { .. } => {}
            // The built-in agent says why a turn failed. Its kind decides what the panel shows,
            // so the general arm's guess from the error's code and words is not made, and the
            // session stays: the user signs in again and goes on in it.
            Incoming::Refused { id, error, problem } if self.pending.get(&id) == Some(&Pending::Prompt) => {
                self.pending.remove(&id);
                self.end_cad_prompt(id);
                if self.cancelling.is_some_and(|(request, _)| request == id) {
                    self.cancelling = None;
                }
                if problem.kind == connections::TurnKind::Other {
                    out.push(Event::TurnEnded { reason: "error".into() });
                    out.push(Event::Error(error.1));
                } else {
                    out.push(Event::TurnProblem(problem));
                    out.push(Event::TurnEnded { reason: "error".into() });
                }
            }
            Incoming::Refused { id, error, .. } => self.handle(Incoming::Reply { id, result: Err(error) }, out),
            Incoming::Connections(snapshot) => out.push(Event::Connections(snapshot)),
            Incoming::AuthStatus { label, kind } => {
                self.sign_in = kind.or(self.sign_in.take());
                if let Some(label) = label {
                    out.push(Event::AuthStatus(label));
                }
            }
            Incoming::ModeChanged(m) if !self.modern_config => out.push(Event::Modes { current: m, available: Vec::new() }),
            Incoming::ModelChanged(m) if !self.modern_config => out.push(Event::Models { current: m, available: Vec::new() }),
            Incoming::ConfigOptions { session, options } if self.session.as_deref() == Some(session.as_str()) => {
                out.push(Event::ConfigOptions(options))
            }
            Incoming::Commands { session, commands } if self.session.as_deref() == Some(session.as_str()) => {
                out.push(Event::Commands { session, commands })
            }
            Incoming::ModeChanged(_) | Incoming::ModelChanged(_) | Incoming::ConfigOptions { .. } | Incoming::Commands { .. } => {}
            Incoming::CadTool { rpc, params } => self.dispatch_cad(rpc, params),
            Incoming::Unsupported { rpc, method } => {
                self.proc.send(&acp::reply_error(rpc, acp::METHOD_NOT_FOUND, &format!("Intcad does not serve `{method}`")));
            }
            Incoming::Ignored => {}
        }
    }

    fn send_connections(&mut self, request: &connections::Request) {
        let what = request.what();
        self.request(Pending::Connections(what), what.method(), request.params());
    }

    fn finish_exit(&mut self, tail: Vec<String>) -> Event {
        self.revoke_cad();
        self.exited = true;
        self.session = None;
        self.pending.clear();
        self.cancelling = None;
        self.prediction_teardown_started = None;
        self.prediction_open_queued = false;
        self.config_pending = None;
        Event::Exited { tail }
    }

    fn poll_at(&mut self, now: Instant) -> Vec<Event> {
        let mut out = Vec::new();
        if self.exited {
            return out;
        }
        // A late reply must not reopen the old process after its cleanup deadline.
        if self.prediction_teardown_started.is_some_and(|started| now.saturating_duration_since(started) >= PREDICTION_TEARDOWN_GRACE) {
            return vec![self.stop_guesser_after_teardown_failure("prediction session teardown timed out")];
        }
        if let Some(error) = self.prompt_refused.take() {
            let key = match error {
                ProcessIoError::OversizedFrame => "ui.agent_chat_prompt_too_large",
                ProcessIoError::ResourceExhausted => "ui.agent_chat_prompt_send_full",
                _ => "ui.agent_chat_prompt_send_failed",
            };
            out.push(Event::TurnEnded { reason: "error".into() });
            out.push(Event::Error(crate::i18n::tr(key).to_string()));
        }
        for _ in 0..512 {
            match self.proc.from_agent.try_recv() {
                Ok(FromAgent::Line(l)) => {
                    let read = if self.built_in { acp::parse_builtin(&l) } else { acp::parse(&l) };
                    if let Some(m) = read {
                        self.handle(m, &mut out);
                    }
                    if self.exited {
                        break;
                    }
                }
                Ok(FromAgent::Exited(tail)) => {
                    out.push(self.finish_exit(tail));
                    break;
                }
                Err(_) => break,
            }
        }
        self.poll_cad();
        // Drain replies first: an acknowledgement already waiting must keep the connection.
        if self.cancelling.is_some_and(|(_, started)| now.saturating_duration_since(started) >= CANCEL_GRACE) {
            self.proc.shutdown(false);
            out.push(self.finish_exit(vec![crate::i18n::tr("ui.agent_chat_cancel_timeout").to_string()]));
        }
        out
    }
}

impl ChatBackend for AcpBackend {
    fn poll(&mut self) -> Vec<Event> {
        self.poll_at(Instant::now())
    }

    fn poll_wake_after(&self) -> Option<Duration> {
        self.prediction_teardown_started.map(|at| PREDICTION_TEARDOWN_GRACE.saturating_sub(at.elapsed()))
    }

    fn prompt(&mut self, parts: &[PromptPart]) {
        if let Some(s) = self.session.clone() {
            let prompt = self.next_id + 1;
            let request = PromptRequest {
                jsonrpc: "2.0",
                id: prompt,
                method: "session/prompt",
                params: PromptParams {
                    session_id: &s,
                    prompt: PromptBlocks(parts),
                    meta: self.cad.is_some().then(|| json!({"intcad": {"cadTurn": prompt.to_string()}})),
                },
            };
            if let Err(error) = self.proc.send_prompt(&request) {
                self.revoke_cad();
                if let Some(cad) = &mut self.cad {
                    cad.last_effect = false;
                }
                self.prompt_refused = Some(error);
                return;
            }
            self.next_id = prompt;
            self.pending.insert(prompt, Pending::Prompt);
            if let Some(cad) = &mut self.cad {
                let active = cad.next_active.take();
                let check = cad.next_check.take();
                cad.retire();
                // Any effect of the turn that just ended was read by the panel already.
                cad.last_effect = false;
                if let (Some(active), Some(document)) = (active, &cad.document) {
                    cad.turn = Some(CadTurn {
                        prompt,
                        session: s,
                        document_id: document.document_id.clone(),
                        active,
                        replies_seen: HashSet::new(),
                        check: check.map(|check| Arc::new(std::sync::Mutex::new(check))),
                        effect: false,
                    });
                }
            }
        }
    }

    fn take_usage(&mut self) -> Option<super::backend::Usage> {
        let usage = std::mem::take(&mut self.usage);
        (!usage.is_empty()).then_some(usage)
    }

    fn cancel(&mut self) {
        self.revoke_cad();
        // Open permission questions are answered "cancelled" first, as the protocol asks.
        for (_, rpc) in self.permissions.drain() {
            self.proc.send(&acp::permission_reply(rpc, None));
        }
        if let Some(s) = &self.session {
            self.proc.send(&acp::notification("session/cancel", json!({"sessionId": s})));
            // Repeated Stop presses must not postpone the fallback. An idle session needs none.
            if self.cancelling.is_none() {
                self.cancelling =
                    self.pending.iter().find_map(|(&id, request)| (*request == Pending::Prompt).then_some(id)).map(|id| (id, Instant::now()));
            }
        }
    }

    fn new_session(&mut self) {
        if self.is_builtin_guesser() {
            if self.exited || self.pending.values().any(|what| *what == Pending::SessionNew) {
                return;
            }
            if self.session.is_some() {
                self.retire_prediction_session();
            }
            self.prediction_open_queued = true;
            self.open_queued_prediction();
            return;
        }
        if self.pending.values().any(|p| *p == Pending::Prompt) {
            self.cancel();
        }
        self.revoke_cad();
        // A guessing session is asked once and not gone back to: left open, each would keep the
        // agent's processes for it (Claude Code: two, about 320 MB). The chat's stay as they were.
        if let Some(s) = self.session.take().filter(|_| self.lean.is_some() && acp::closes_sessions(&self.init)) {
            self.request(Pending::CloseSession, "session/close", json!({"sessionId": s}));
        }
        self.modern_config = false;
        self.config_pending = None;
        self.pending.retain(|_, request| *request != Pending::SetConfig);
        self.new_session();
    }

    fn set_instructions(&mut self, text: &str) {
        self.note = acp::system_note(text);
    }

    fn key_sign_in(&self) -> bool {
        self.sign_in.as_deref() == Some(KEY_SIGN_IN)
    }

    fn session_fits_sign_in(&self) -> bool {
        !self.lean_session || self.lean_on_plans || self.key_sign_in()
    }

    fn set_project(&mut self, root: Option<&std::path::Path>) {
        if self.project.as_deref() != root {
            self.revoke_cad();
            self.project = root.map(std::path::Path::to_path_buf);
        }
    }

    fn set_document(&mut self, document_id: &str, revision: u64) {
        if let Some(cad) = &mut self.cad {
            if cad.document.as_ref().is_some_and(|previous| previous.document_id != document_id) {
                cad.retire();
            }
            cad.document = Some(DocumentStamp { document_id: document_id.to_owned(), revision });
        }
    }

    fn set_turn_control(&mut self, active: Arc<AtomicBool>) {
        if let Some(cad) = &mut self.cad {
            cad.next_active = Some(active);
        }
    }

    fn set_bridge_scope_check(&mut self, check: Option<BridgeScopeCheck>) {
        if let Some(cad) = &mut self.cad {
            cad.next_check = check;
        }
    }

    fn turn_had_bridge_effect(&self) -> bool {
        self.cad.as_ref().is_some_and(|cad| cad.last_effect)
    }

    fn retire_bridge_authority(&mut self) {
        self.revoke_cad();
    }

    fn answer_permission(&mut self, key: u64, option: Option<&str>) {
        if let Some(rpc) = self.permissions.remove(&key) {
            self.proc.send(&acp::permission_reply(rpc, option));
        }
    }

    fn authenticate(&mut self, method: &str) {
        self.request(Pending::Authenticate, "authenticate", json!({"methodId": method}));
    }

    fn set_mode(&mut self, mode: &str) {
        if let Some(s) = self.session.clone() {
            self.request(Pending::SetMode, "session/set_mode", json!({"sessionId": s, "modeId": mode}));
        }
    }

    fn set_model(&mut self, model: &str) {
        if let Some(s) = self.session.clone() {
            self.request(Pending::SetModel, "session/set_model", acp::set_model_params(&s, model));
        }
    }

    fn set_config_option(&mut self, id: &str, value: &str) {
        if self.config_pending.is_some() {
            return;
        }
        if let Some(s) = self.session.clone() {
            self.request(Pending::SetConfig, "session/set_config_option", acp::set_config_option_params(&s, id, value));
            self.config_pending = Some((self.next_id, id.to_string()));
        }
    }

    fn connections(&mut self, request: connections::Request) {
        if !self.built_in || self.exited {
            return;
        }
        if self.initialized {
            self.send_connections(&request);
        } else if request.what() == connections::What::Setup {
            // The newest replaces one still waiting: `setup` hands over everything each time.
            self.setup = Some(request);
        }
    }

    fn forget_session(&mut self) {
        self.revoke_cad();
        self.session = None;
        self.modern_config = false;
        self.config_pending = None;
        self.pending.retain(|_, request| *request != Pending::SetConfig);
    }

    fn retire_prediction_session(&mut self) {
        if !self.is_builtin_guesser() {
            return;
        }
        self.prediction_open_queued = false;
        if !self.permissions.is_empty() || self.pending.values().any(|what| *what == Pending::Prompt) {
            self.cancel();
        }
        if let Some(session) = self.session.take() {
            self.close_prediction(session);
        }
        self.cancelling = None;
        self.pending.retain(|_, request| !matches!(request, Pending::Prompt | Pending::SetConfig));
        for request in self.pending.values_mut() {
            if *request == Pending::SessionNew {
                *request = Pending::RetiredSessionNew;
                self.prediction_teardown_started.get_or_insert_with(Instant::now);
            }
        }
        self.config_pending = None;
        self.modern_config = false;
    }
    fn pid(&self) -> Option<u32> {
        self.proc.pid()
    }

    fn shutdown(&mut self, wait: bool) {
        self.revoke_cad();
        self.proc.shutdown(wait);
    }
}

/// Test connection: start the agent, say hello, open a session without tools, end it. What it
/// answered (its name and version), or what went wrong (it isn't signed in, it didn't answer).
pub fn test_agent(argv: &[String], cwd: &std::path::Path) -> Result<String, TestError> {
    let mut p = AgentProcess::spawn(argv, cwd, ENV_REMOVE, || {}).map_err(TestError::Other)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let wait_for = |p: &AgentProcess, id: u64| -> Result<Value, TestError> {
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match p.from_agent.recv_timeout(left) {
                Ok(FromAgent::Line(l)) => {
                    if let Some(Incoming::Reply { id: got, result }) = acp::parse(&l)
                        && got == id
                    {
                        return result.map_err(|(code, m)| {
                            if code == acp::AUTH_REQUIRED || m.to_ascii_lowercase().contains("auth") {
                                TestError::AuthRequired
                            } else {
                                TestError::Other(m)
                            }
                        });
                    }
                }
                Ok(FromAgent::Exited(tail)) => {
                    return Err(TestError::Other(tail.last().cloned().unwrap_or_else(|| crate::i18n::tr("ui.agent_chat_exited").to_string())));
                }
                Err(_) => return Err(TestError::Other(crate::i18n::tr("ui.ai_no_answer").to_string())),
            }
        }
    };
    p.send(&acp::request(1, "initialize", acp::initialize_params(env!("CARGO_PKG_VERSION"))));
    let init = wait_for(&p, 1);
    let session_cwd = if acp::runs_in_wsl(argv) { acp::wsl_path(cwd) } else { cwd.to_path_buf() };
    let result = init.and_then(|v| {
        p.send(&acp::request(2, "session/new", acp::session_new_params(&session_cwd, None, &v, acp::SYSTEM_NOTE, None)));
        wait_for(&p, 2).map(|_| {
            let who = acp::agent_info(&v);
            crate::i18n::format("ui.ai_test_acp_ok", &[("agent", if who.is_empty() { "?" } else { &who })])
        })
    });
    p.shutdown(true);
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::agent_chat::backend::StartContext;

    fn ready_backend() -> Result<AcpBackend, String> {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "while IFS= read -r line; do :; done".into()], ctx)?;
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        Ok(backend)
    }

    #[test]
    fn builtin_acp_delivers_a_full_pdf_and_recovers_after_multiple_oversized_pictures() {
        // Check the real child's wire (counts bytes and answers the exact request id). Keep stdout
        // small: only the desktop-to-agent direction gets the built-in prompt allowance.
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let peer = r#"/"method":"session\/prompt"/ {
            match($0, /"id":[0-9]+/); id = substr($0, RSTART + 5, RLENGTH - 5);
            printf "{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":{\"stopReason\":\"end_turn\",\"bytes\":%d}}\n", id, length($0) + 1; fflush();
        }"#;
        let mut backend = AcpBackend::spawn(&["awk".into(), peer.into()], ctx, &[("LC_ALL", std::ffi::OsStr::new("C"))], ENV_REMOVE, true).unwrap();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        AcpBackend::new_session(&mut backend);
        backend.handle(Incoming::Reply { id: 2, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        let receive = |backend: &mut AcpBackend| {
            let Ok(FromAgent::Line(line)) = backend.proc.from_agent.recv_timeout(Duration::from_secs(10)) else {
                panic!("peer did not see a prompt")
            };
            let value: Value = serde_json::from_str(&line).unwrap();
            let mut events = Vec::new();
            backend.handle(acp::parse_builtin(&line).unwrap(), &mut events);
            assert_eq!(events, [Event::TurnEnded { reason: String::new() }]);
            value["result"]["bytes"].as_u64().unwrap()
        };

        backend.prompt(&[
            PromptPart::Text("Read this PDF, please.".into()),
            PromptPart::Document {
                name: "file.pdf".into(),
                uri: "file:///file.pdf".into(),
                mime: "application/pdf".into(),
                base64: "AAAA".repeat(4 << 20), // 12 MiB decoded
            },
        ]);
        assert!(receive(&mut backend) > 16 << 20);
        let pictures = (0..4).map(|_| PromptPart::Image { mime: "image/jpeg".into(), base64: "A".repeat(5_000_000) }).collect::<Vec<_>>();
        backend.prompt(&pictures);
        assert!(matches!(
            backend.poll().as_slice(),
            [Event::TurnEnded { reason }, Event::Error(notice)]
                if reason == "error" && notice == crate::i18n::tr("ui.agent_chat_prompt_too_large")
        ));
        assert!(!backend.pending.values().any(|p| *p == Pending::Prompt));
        backend.prompt(&[PromptPart::Text("Words only".into())]);
        assert!(receive(&mut backend) < 512);
        backend.shutdown(true);
    }

    #[test]
    fn packaged_acp_cad_callback_is_not_an_ordinary_agent_request() {
        let line = r#"{"jsonrpc":"2.0","id":7,"method":"_intcad/cad/tool","params":{"sessionId":"s1","turn":"1","name":"set_parameter","arguments":{"name":"int1481_acceptance_marker","expression":"13 mm"}}}"#;
        assert!(matches!(acp::parse(line), Some(Incoming::Unsupported { .. })));
        let Some(Incoming::CadTool { rpc, params }) = acp::parse_builtin(line) else {
            panic!("the packaged-agent parser must deliver the CAD callback to its private dispatcher");
        };
        assert_eq!(rpc, 7);
        assert_eq!(params["sessionId"], "s1");
        assert_eq!(params["turn"], "1");
        assert_eq!(params["name"], "set_parameter");
        assert_eq!(params["arguments"]["expression"], "13 mm");
    }

    /// The paid lane starts only this package's Node and entry, with the pinned SDK present.
    /// A folder found beside the app is not enough: a different program or a stripped SDK is
    /// refused before any authority is offered.
    #[test]
    fn the_paid_lane_starts_only_the_packages_own_agent() {
        let dir = std::env::temp_dir().join(format!("intcad-packaged-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("usr/lib/intcad/mastra-agent");
        for folder in ["agent", "node_modules/@mastra/code-sdk", "node_modules/@agentclientprotocol/sdk"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(root.join("node"), b"").unwrap();
        std::fs::write(root.join("agent/main.mjs"), b"").unwrap();
        std::fs::write(root.join("agent.json"), b"{}").unwrap();
        std::fs::write(root.join("node_modules/@mastra/code-sdk/package.json"), b"{}").unwrap();
        std::fs::write(root.join("node_modules/@agentclientprotocol/sdk/package.json"), b"{}").unwrap();
        let argv = || vec![root.join("node").to_string_lossy().into_owned(), root.join("agent/main.mjs").to_string_lossy().into_owned()];
        assert_eq!(AcpBackend::packaged_agent(&argv()).unwrap().1, std::fs::canonicalize(root.join("agent/main.mjs")).unwrap());

        let elsewhere = dir.join("elsewhere/main.mjs");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        std::fs::write(&elsewhere, b"").unwrap();
        let moved = [argv()[0].clone(), elsewhere.to_string_lossy().into_owned()];
        assert!(AcpBackend::packaged_agent(&moved).is_err(), "the entry must be the package's agent/main.mjs");

        std::fs::remove_file(root.join("node_modules/@mastra/code-sdk/package.json")).unwrap();
        assert!(AcpBackend::packaged_agent(&argv()).is_err(), "a package without its SDK is not started");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A retired turn answers the callback it still owed the Agent, so the Agent's one in-flight
    /// slot is free for the next turn instead of waiting on the old worker.
    #[test]
    fn a_retired_cad_callback_releases_the_agents_reply_slot() {
        let (tools, _requests) = std::sync::mpsc::channel();
        let (_tx, result) = std::sync::mpsc::sync_channel(1);
        let mut cad = PackagedCad {
            tools,
            wake: Arc::new(|| {}),
            document: None,
            next_active: None,
            next_check: None,
            turn: Some(CadTurn {
                prompt: 3,
                session: "s1".into(),
                document_id: "d1".into(),
                active: Arc::new(AtomicBool::new(true)),
                replies_seen: HashSet::new(),
                check: None,
                effect: false,
            }),
            reply: Some(CadReply { turn: 3, rpc: json!(71), result }),
            stale: Vec::new(),
            last_effect: false,
        };
        cad.retire();
        assert_eq!(cad.stale, [json!(71)], "the owed callback gets an answer");
        assert!(cad.last_effect, "a dispatched call may have changed the design");
        assert!(cad.turn.is_none());
        cad.retire();
        assert_eq!(cad.stale, [json!(71)], "nothing is answered twice");
    }

    /// A callback belongs only to the live session, prompt and document it names. An older
    /// prompt, another session, another document or a stopped turn never borrows the new one.
    #[test]
    fn a_cad_callback_belongs_only_to_its_own_live_turn() {
        let stamp = DocumentStamp { document_id: "d1".into(), revision: 7 };
        let turn = CadTurn {
            prompt: 9,
            session: "s1".into(),
            document_id: "d1".into(),
            active: Arc::new(AtomicBool::new(true)),
            replies_seen: HashSet::new(),
            check: None,
            effect: false,
        };
        let params = |turn_id: &str, session: &str| json!({"sessionId":session,"turn":turn_id,"name":"set_parameter","arguments":{}});
        assert!(turn.owns(&stamp, &params("9", "s1")), "its own turn");
        assert!(!turn.owns(&stamp, &params("8", "s1")), "an older prompt");
        assert!(!turn.owns(&stamp, &params("9", "s2")), "another session");
        assert!(!turn.owns(&DocumentStamp { document_id: "d2".into(), revision: 7 }, &params("9", "s1")), "another document");
        turn.active.store(false, Ordering::Release);
        assert!(!turn.owns(&stamp, &params("9", "s1")), "a stopped or replaced turn");
    }

    #[test]
    fn an_unresponsive_cancel_disconnects_the_agent() {
        let mut backend = ready_backend().unwrap();
        backend.prompt(&[PromptPart::Text("stalled search".into())]);
        backend.cancel();
        let started = backend.cancelling.unwrap().1;
        assert!(backend.poll_at(started + CANCEL_GRACE - Duration::from_millis(1)).is_empty());
        backend.cancel();
        let events = backend.poll_at(started + CANCEL_GRACE);
        assert!(matches!(events.as_slice(), [Event::Exited { .. }]), "Stop must not wait indefinitely: {events:?}");
        assert!(backend.pid().is_none());
        assert!(backend.session.is_none() && backend.pending.is_empty());
        assert!(backend.poll().is_empty(), "the exit is reported only once");
    }

    #[test]
    fn a_cancelled_prompt_reply_keeps_the_connection() {
        for reply in [Ok(json!({"stopReason":"cancelled"})), Err((-32603, "operation aborted".into()))] {
            let mut backend = ready_backend().unwrap();
            backend.prompt(&[PromptPart::Text("search".into())]);
            let request = backend.next_id;
            backend.cancel();
            let mut events = Vec::new();
            backend.handle(Incoming::Reply { id: request, result: reply }, &mut events);
            assert!(events.iter().any(|e| matches!(e, Event::TurnEnded { .. })));
            assert!(backend.poll_at(Instant::now() + CANCEL_GRACE).is_empty());
            assert!(backend.pid().is_some() && !backend.exited);
            backend.shutdown(true);
        }
    }

    #[test]
    fn cancelling_an_idle_session_does_not_disconnect_it() {
        let mut backend = ready_backend().unwrap();
        backend.cancel();
        assert!(backend.poll_at(Instant::now() + CANCEL_GRACE).is_empty());
        assert!(backend.pid().is_some() && !backend.exited);
        backend.shutdown(true);
    }

    #[test]
    fn a_new_session_retires_the_old_cancelled_prompt() {
        let mut backend = ready_backend().unwrap();
        backend.prompt(&[PromptPart::Text("old search".into())]);
        let old_prompt = backend.next_id;
        backend.cancel();
        ChatBackend::new_session(&mut backend);
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"s2"})) }, &mut events);
        assert!(matches!(events.first(), Some(Event::TurnEnded { reason }) if reason == "cancelled"));
        assert!(matches!(events.last(), Some(Event::Ready { .. })));
        backend.prompt(&[PromptPart::Text("new search".into())]);
        assert!(backend.poll_at(Instant::now() + CANCEL_GRACE).is_empty(), "a new session has no stale cancellation deadline");
        backend.cancel();
        events.clear();
        backend.handle(Incoming::Reply { id: old_prompt, result: Ok(json!({"stopReason":"cancelled"})) }, &mut events);
        assert!(events.is_empty(), "an old reply must not end the new turn or clear its cancellation deadline");
        assert!(matches!(backend.poll_at(Instant::now() + CANCEL_GRACE).as_slice(), [Event::Exited { .. }]));
    }

    /// The built-in agent, started on a program that swallows what it is sent; `initialize` is
    /// answered when `ready`.
    fn builtin_backend(ready: bool) -> Result<AcpBackend, String> {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend =
            AcpBackend::start_builtin(&["sh".into(), "-c".into(), "while IFS= read -r line; do :; done".into()], ctx, &std::env::temp_dir())?;
        if ready {
            backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut Vec::new());
        }
        Ok(backend)
    }

    fn connections_snapshot(plan: &str) -> Value {
        json!({
            "plans": [{"id": "openai-codex", "name": "ChatGPT Plus/Pro (Codex Subscription)", "state": plan, "account": null, "prefix": "openai"}],
            "keys": [{"id": "openai", "name": "OpenAI", "saved": false, "shadowed": false, "prefix": "openai"}],
            "custom": [], "login": null, "problem": null, "dates": {},
        })
    }

    fn setup(key: &str) -> connections::Request {
        connections::Request::Setup(connections::Setup { keys: vec![("openai".into(), key.into())], ..Default::default() })
    }

    fn sent(backend: &AcpBackend) -> Vec<Pending> {
        let mut ids: Vec<(&u64, &Pending)> = backend.pending.iter().collect();
        ids.sort_by_key(|(id, _)| **id);
        ids.into_iter().map(|(_, what)| *what).collect()
    }

    fn prediction_backend() -> Result<AcpBackend, String> {
        let mut backend = builtin_backend(false)?;
        backend.lean_sessions("guess");
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentCapabilities":{"sessionCapabilities":{"close":{}}}})) }, &mut Vec::new());
        Ok(backend)
    }

    fn opened_prediction() -> Result<AcpBackend, String> {
        let mut backend = prediction_backend()?;
        ChatBackend::new_session(&mut backend);
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"old"})) }, &mut Vec::new());
        Ok(backend)
    }

    #[test]
    fn g86_a_guess_waits_for_the_close_ack_before_opening_one_replacement() {
        let mut backend = opened_prediction().unwrap();
        let before = backend.next_id;
        ChatBackend::new_session(&mut backend);
        ChatBackend::new_session(&mut backend);
        assert_eq!(sent(&backend), [Pending::CloseSession], "no replacement or duplicate close before the ACK");
        assert_eq!(backend.next_id, before + 1);
        assert!(backend.poll_at(Instant::now()).is_empty());
        backend.handle(Incoming::Reply { id: before + 1, result: Ok(json!({})) }, &mut Vec::new());
        assert_eq!(sent(&backend), [Pending::SessionNew]);
        assert_eq!(backend.next_id, before + 2, "duplicate requests coalesced into one replacement");
    }

    #[test]
    fn g86_a_close_error_retires_the_guesser_without_opening_a_replacement() {
        let mut backend = opened_prediction().unwrap();
        let close_id = backend.next_id + 1;
        ChatBackend::new_session(&mut backend);
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: close_id, result: Err((-32603, "cleanup failed".into())) }, &mut events);
        events.extend(backend.poll_at(Instant::now()));
        assert!(matches!(events.as_slice(), [Event::Exited { .. }]), "{events:?}");
        assert!(backend.pid().is_none());
        ChatBackend::new_session(&mut backend);
        assert_eq!(backend.next_id, close_id, "no request after or during failed cleanup");
        assert!(backend.pending.is_empty());
    }

    #[test]
    fn g86_an_expired_close_retires_the_guesser_and_ignores_the_late_ack() {
        let mut backend = opened_prediction().unwrap();
        let close_id = backend.next_id + 1;
        ChatBackend::new_session(&mut backend);
        let started = backend.prediction_teardown_started.unwrap();
        assert!(backend.poll_wake_after().is_some(), "silent cleanup still schedules a poll");
        assert!(backend.poll_at(started + Duration::from_millis(4_999)).is_empty());
        ChatBackend::new_session(&mut backend);
        let mut events = backend.poll_at(started + Duration::from_secs(5));
        assert!(matches!(events.as_slice(), [Event::Exited { .. }]), "{events:?}");
        assert!(backend.pid().is_none());
        backend.handle(Incoming::Reply { id: close_id, result: Ok(json!({})) }, &mut events);
        assert_eq!(events.len(), 1, "no event from the old process");
        ChatBackend::new_session(&mut backend);
        assert_eq!(backend.next_id, close_id, "no request after or during expired cleanup");
        assert!(backend.pending.is_empty());
        assert!(backend.poll_wake_after().is_none());
    }

    #[test]
    fn g86_a_missing_retired_open_reply_disconnects_without_sending_another_request() {
        let mut backend = prediction_backend().unwrap();
        ChatBackend::new_session(&mut backend);
        let stale = backend.next_id;
        backend.retire_prediction_session();
        ChatBackend::new_session(&mut backend);
        let started = backend.prediction_teardown_started.unwrap();
        let wake = backend.poll_wake_after();
        assert_eq!(sent(&backend), [Pending::RetiredSessionNew]);
        assert!(backend.poll_at(started + Duration::from_secs(4)).is_empty());
        backend.retire_prediction_session();
        ChatBackend::new_session(&mut backend);
        let mut events = backend.poll_at(started + Duration::from_secs(5));
        assert!(matches!(events.as_slice(), [Event::Exited { .. }]), "the retired open must expire even without a reply: {events:?}");
        assert!(wake.is_some(), "the frame must wake even without child traffic");
        assert!(backend.pid().is_none());
        backend.handle(Incoming::Reply { id: stale, result: Ok(json!({"sessionId":"too late"})) }, &mut events);
        backend.connections(connections::Request::Setup(Default::default()));
        ChatBackend::new_session(&mut backend);
        assert_eq!(events.len(), 1);
        assert_eq!(backend.next_id, stale, "no close, setup or replacement on the old process");
        assert!(backend.pending.is_empty());
        assert!(backend.poll_wake_after().is_none());
    }

    #[test]
    fn g86_a_late_open_reply_does_not_extend_the_teardown_deadline() {
        let mut backend = prediction_backend().unwrap();
        ChatBackend::new_session(&mut backend);
        let stale = backend.next_id;
        backend.retire_prediction_session();
        ChatBackend::new_session(&mut backend);
        let started = backend.prediction_teardown_started.unwrap();
        assert!(backend.poll_at(started + Duration::from_millis(4_999)).is_empty());
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: stale, result: Ok(json!({"sessionId":"stale"})) }, &mut events);
        assert!(events.is_empty(), "a timely old open reply starts cleanup, not a new session");
        assert_eq!(sent(&backend), [Pending::CloseSession]);
        assert_eq!(backend.prediction_teardown_started, Some(started), "waiting for close shares the retired-open deadline");
        let events = backend.poll_at(started + Duration::from_secs(5));
        assert!(matches!(events.as_slice(), [Event::Exited { .. }]));
        ChatBackend::new_session(&mut backend);
        assert_eq!(backend.next_id, stale + 1, "only the close was sent");
    }

    #[test]
    fn retiring_a_guess_closes_even_a_late_session_open_without_publishing_ready() {
        let mut backend = prediction_backend().unwrap();
        let mut events = Vec::new();
        backend.connections(setup("fake"));
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(connections_snapshot("in")) }, &mut events);
        assert!(matches!(events.pop(), Some(Event::PredictionSetup(_))));
        ChatBackend::new_session(&mut backend);
        let stale = backend.next_id;
        backend.retire_prediction_session();
        ChatBackend::new_session(&mut backend);
        assert_eq!(sent(&backend), [Pending::RetiredSessionNew], "an unresolved session must be accounted for before replacing it");
        backend.handle(Incoming::Reply { id: stale, result: Ok(json!({"sessionId":"stale"})) }, &mut events);
        assert!(events.is_empty(), "stale session must not become ready");
        assert!(backend.session.is_none());
        assert_eq!(sent(&backend), [Pending::CloseSession]);
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({})) }, &mut events);
        assert_eq!(sent(&backend), [Pending::SessionNew]);
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"current"})) }, &mut events);
        assert!(matches!(events.last(), Some(Event::Ready { .. })));
        assert_eq!(backend.session.as_deref(), Some("current"));
    }

    #[test]
    fn g86_retirement_withdraws_a_replacement_queued_during_close() {
        let mut backend = opened_prediction().unwrap();
        ChatBackend::new_session(&mut backend);
        let close_id = backend.next_id;
        backend.retire_prediction_session();
        backend.handle(Incoming::Reply { id: close_id, result: Ok(json!({})) }, &mut Vec::new());
        assert!(backend.pending.is_empty(), "credentials must be set up again before opening a session");
        assert_eq!(backend.next_id, close_id);
    }

    /// B14, on the wire: after `initialize` the built-in agent is handed its keys, and no session
    /// is asked for until the panel opens one (the agent refuses a session while nothing is
    /// connected). A `setup` given before `initialize` is answered waits for that answer.
    #[test]
    fn the_built_in_agent_is_set_up_after_initialize_and_gets_no_session_by_itself() {
        let mut backend = builtin_backend(false).unwrap();
        backend.connections(setup("k1"));
        backend.connections(connections::Request::List);
        assert_eq!(sent(&backend), [Pending::Initialize], "nothing goes before the agent has answered initialize");
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(sent(&backend), [Pending::Connections(connections::What::Setup)], "setup follows initialize; no session/new");
        backend.handle(Incoming::Reply { id: 2, result: Ok(connections_snapshot("out")) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::Connections(snapshot)] if !snapshot.connected()), "{events:?}");
        assert!(backend.pending.is_empty() && backend.session.is_none(), "still no session asked for");
        // The panel opens it once something is connected.
        ChatBackend::new_session(&mut backend);
        assert_eq!(sent(&backend), [Pending::SessionNew]);
        events.clear();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        assert!(matches!(events.last(), Some(Event::Ready { .. })));
        // Another agent ignores the requests altogether.
        let mut other = ready_backend().unwrap();
        other.connections(setup("k1"));
        assert!(other.pending.is_empty());
        other.shutdown(true);
        backend.shutdown(true);
    }

    /// The requests of contract section 2 as they go out, and the snapshot as it comes back:
    /// in a reply and in the agent's own `changed` notification.
    #[test]
    fn connection_requests_carry_their_parameters_and_answers_are_snapshots() {
        use connections::{Request, What};
        let cases = [
            (Request::List, What::List, "_intcad/connections/list", json!({})),
            (Request::Login("openai-codex".into()), What::Login, "_intcad/connections/login", json!({"id": "openai-codex"})),
            (
                Request::Code { id: "anthropic".into(), code: "abc#def".into() },
                What::Code,
                "_intcad/connections/code",
                json!({"id": "anthropic", "code": "abc#def"}),
            ),
            (Request::Reopen, What::Reopen, "_intcad/connections/reopen", json!({})),
            (Request::Cancel, What::Cancel, "_intcad/connections/cancel", json!({})),
            (Request::Logout("xai".into()), What::Logout, "_intcad/connections/logout", json!({"id": "xai"})),
        ];
        let mut backend = builtin_backend(true).unwrap();
        for (request, what, method, params) in cases {
            assert_eq!((request.what(), request.what().method(), request.params()), (what, method, params));
            backend.connections(request);
            assert_eq!(backend.pending.get(&backend.next_id), Some(&Pending::Connections(what)));
        }
        let custom = connections::SetupCustom {
            name: "my-llm".into(),
            url: "http://localhost:11434/v1".into(),
            api_key: None,
            models: vec!["llama3.2".into()],
        };
        let keyed = connections::SetupCustom { api_key: Some("k2".into()), ..custom.clone() };
        let mut all = connections::Setup { keys: vec![("openai".into(), "k1".into())], custom: vec![custom, keyed], ..Default::default() };
        let handed = json!({"keys": {"openai": "k1"}, "custom": [
            {"name": "my-llm", "url": "http://localhost:11434/v1", "models": ["llama3.2"]},
            {"name": "my-llm", "url": "http://localhost:11434/v1", "models": ["llama3.2"], "apiKey": "k2"},
        ]});
        assert_eq!(Request::Setup(all.clone()).params(), handed, "without words of the app's, the agent keeps its own");
        // The sentence the browser shows after the sign-in page has called back goes with it.
        all.callback = "Go back to Intcad.".into();
        let mut worded = handed.clone();
        worded["words"] = json!({"callback": "Go back to Intcad."});
        assert_eq!(Request::Setup(all).params(), worded);
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(connections_snapshot("in")) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::ConnectionsLogout(snapshot)] if snapshot.connected()), "{events:?}");
        events.clear();
        let changed = json!({"jsonrpc": "2.0", "method": "_intcad/connections/changed", "params": connections_snapshot("expired")}).to_string();
        backend.handle(acp::parse_builtin(&changed).unwrap(), &mut events);
        assert!(
            matches!(events.as_slice(), [Event::Connections(snapshot)] if snapshot.plans[0].state == connections::PlanState::Expired),
            "{events:?}"
        );
        // An agent that is not the built-in one is not read this way: the notification means nothing from it.
        assert_eq!(acp::parse(&changed), Some(Incoming::Ignored));
        backend.shutdown(true);
    }

    /// A refused connection request is reported as that and nothing else. `busy` (a second
    /// sign-in while one is under way) has the code of ACP's "authentication required": it must
    /// not drop the session or ask for a sign-in, and the agent's words (which could repeat what
    /// it was sent) go nowhere.
    #[test]
    fn a_refused_connection_request_keeps_the_session_and_says_nothing_of_its_words() {
        let mut backend = builtin_backend(true).unwrap();
        ChatBackend::new_session(&mut backend);
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        events.clear();
        backend.connections(connections::Request::Login("openai-codex".into()));
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((acp::AUTH_REQUIRED, "busy".into())) }, &mut events);
        assert_eq!(events, [Event::ConnectionsFailed { what: connections::What::Login, code: acp::AUTH_REQUIRED }]);
        assert_eq!(backend.session.as_deref(), Some("s1"), "the session stays");
        events.clear();
        backend.connections(setup("sk-never-shown"));
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((-32602, "bad key sk-never-shown: authentication".into())) }, &mut events);
        assert_eq!(events, [Event::ConnectionsFailed { what: connections::What::Setup, code: -32602 }]);
        assert!(!format!("{events:?}").contains("sk-never-shown"));
        // An answer that is no snapshot is a failure too, not a silent nothing.
        events.clear();
        backend.connections(connections::Request::List);
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"plans": "none"})) }, &mut events);
        assert_eq!(events, [Event::ConnectionsFailed { what: connections::What::List, code: 0 }]);
        backend.shutdown(true);
    }

    /// B15/B16, on the wire: a prompt the built-in agent refuses with `data.intcad` is reported
    /// by its kind, the session stays (the user signs in again and goes on), and the general
    /// guess from the error's code and words is not made. `other` is shown in the agent's words.
    #[test]
    fn a_failed_turn_of_the_built_in_agent_is_reported_by_its_kind() {
        use connections::{TurnKind, TurnProblem};
        let refusal = |id: u64, code: i64, kind: &str| {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": "authentication failed upstream", "data": {"intcad": {"kind": kind, "prefix": "openai"}}}}).to_string()
        };
        let kinds = [
            ("auth", TurnKind::Auth),
            ("quota", TurnKind::Quota),
            ("rate", TurnKind::Rate),
            ("network", TurnKind::Network),
            ("model", TurnKind::Model),
            ("attachments", TurnKind::Attachments),
        ];
        for (word, kind) in kinds {
            let mut backend = builtin_backend(true).unwrap();
            ChatBackend::new_session(&mut backend);
            let mut events = Vec::new();
            backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
            events.clear();
            backend.prompt(&[PromptPart::Text("make a box".into())]);
            // The code and the words would read as "sign in" to the general arm.
            backend.handle(acp::parse_builtin(&refusal(backend.next_id, acp::AUTH_REQUIRED, word)).unwrap(), &mut events);
            assert_eq!(
                events,
                [Event::TurnProblem(TurnProblem { kind, prefix: "openai".into() }), Event::TurnEnded { reason: "error".into() }],
                "{word}"
            );
            assert_eq!(backend.session.as_deref(), Some("s1"), "{word}: the session stays");
            assert!(backend.pending.is_empty());
            backend.shutdown(true);
        }
        let mut backend = builtin_backend(true).unwrap();
        ChatBackend::new_session(&mut backend);
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        events.clear();
        backend.prompt(&[PromptPart::Text("make a box".into())]);
        backend.handle(acp::parse_builtin(&refusal(backend.next_id, -32603, "other")).unwrap(), &mut events);
        assert_eq!(events, [Event::TurnEnded { reason: "error".into() }, Event::Error("authentication failed upstream".into())]);
        assert_eq!(backend.session.as_deref(), Some("s1"));
        // The same error from any other agent is read as before: by its code and words.
        let mut other = ready_backend().unwrap();
        other.prompt(&[PromptPart::Text("make a box".into())]);
        events.clear();
        other.handle(acp::parse(&refusal(other.next_id, acp::AUTH_REQUIRED, "quota")).unwrap(), &mut events);
        assert!(matches!(events.as_slice(), [Event::TurnEnded { reason }, Event::AuthRequired { .. }] if reason == "auth"), "{events:?}");
        other.shutdown(true);
        backend.shutdown(true);
    }

    /// INT-1513, on the wire: the built-in agent's `initialize` answer as it is now (it takes
    /// pictures and documents) lets the panel send them with a message; its earlier answer did not.
    #[test]
    fn the_built_in_agent_says_it_takes_pictures_and_documents() {
        let capabilities = json!({"loadSession": false, "mcpCapabilities": {"http": true, "sse": false}});
        let mut now = capabilities.clone();
        now["promptCapabilities"] = json!({"image": true, "embeddedContext": true});
        for (answer, takes) in [(now, true), (capabilities, false)] {
            let mut backend = builtin_backend(false).unwrap();
            let mut events = Vec::new();
            backend.handle(
                Incoming::Reply { id: 1, result: Ok(json!({"agentInfo": {"name": "mastracode"}, "agentCapabilities": answer})) },
                &mut events,
            );
            ChatBackend::new_session(&mut backend);
            backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId": "s1"})) }, &mut events);
            assert!(events.contains(&Event::Documents(takes)), "{events:?}");
            assert!(events.iter().any(|event| matches!(event, Event::Ready { images, .. } if *images == takes)), "{events:?}");
            backend.shutdown(true);
        }
    }

    /// Forgetting the session stops what the agent still sends for it from being taken in; the
    /// next one opens as usual.
    #[test]
    fn a_forgotten_session_takes_in_nothing_more() {
        let mut backend = builtin_backend(true).unwrap();
        ChatBackend::new_session(&mut backend);
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(snapshot("s1", "fast")) }, &mut events);
        backend.forget_session();
        events.clear();
        let update = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"late"}}}}"#;
        backend.handle(acp::parse_builtin(update).unwrap(), &mut events);
        backend.prompt(&[PromptPart::Text("nobody to ask".into())]);
        assert!(events.is_empty() && backend.pending.is_empty(), "{events:?}");
        ChatBackend::new_session(&mut backend);
        assert_eq!(sent(&backend), [Pending::SessionNew]);
        backend.shutdown(true);
    }

    /// The built-in agent's process is told where to keep its sign-ins and its own data
    /// (`MASTRA_APP_DATA_DIR`) and is started with Mastra's usage reporting off, and what the
    /// entry reads for itself is not taken from it.
    #[test]
    fn the_built_in_agent_is_started_with_its_data_folder() {
        let dir = std::env::temp_dir().join(format!("intcad-builtin-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (seen, data) = (dir.join("seen"), dir.join("settings").join("mastra"));
        let script = r#"printf '%s|%s\n' "$MASTRA_APP_DATA_DIR" "$MASTRA_TELEMETRY_DISABLED" > "$0"; while IFS= read -r line; do :; done"#;
        let argv = ["sh".to_string(), "-c".to_string(), script.to_string(), seen.to_string_lossy().into_owned()];
        let ctx = || StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let read = |seen: &std::path::Path| {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Ok(text) = std::fs::read_to_string(seen)
                    && text.ends_with('\n')
                {
                    return text;
                }
                assert!(Instant::now() < deadline, "the agent did not start");
                std::thread::yield_now();
            }
        };
        let mut backend = AcpBackend::start_builtin(&argv, ctx(), &data).unwrap();
        assert_eq!(read(&seen).trim_end(), format!("{}|1", data.to_string_lossy()));
        backend.shutdown(true);
        // Any other agent gets no such variable from the app: only what this process has itself.
        std::fs::remove_file(&seen).unwrap();
        let mut other = AcpBackend::start(&argv, ctx()).unwrap();
        assert_eq!(
            read(&seen).trim_end(),
            format!("{}|{}", std::env::var(DATA_DIR_ENV).unwrap_or_default(), std::env::var(TELEMETRY_OFF_ENV).unwrap_or_default())
        );
        other.shutdown(true);
        assert!(!ENV_REMOVE.contains(&"SOLIDCRAFT_MASTRA_SDK") && !ENV_REMOVE.contains(&DATA_DIR_ENV), "{ENV_REMOVE:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// INT-1473, on the wire: the built-in agent handed a project names it in `session/new`
    /// (`_meta.intcad.project`, the session's own folder staying `cwd`) and its note says what file
    /// tools are for. Once the project is taken back, and for any other agent handed one, the
    /// session is opened as before: no project and the note that keeps file tools off.
    #[test]
    fn only_the_built_in_agent_is_handed_its_project_and_only_while_it_has_one() {
        let dir = std::env::temp_dir().join(format!("intcad-builtin-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("bracket");
        // A fake agent that writes down every line it is sent, in the file named by `$0`.
        let agent = |log: &std::path::Path| {
            let script = r#"while IFS= read -r line; do printf '%s\n' "$line" >> "$0"; done"#;
            ["sh".to_string(), "-c".to_string(), script.to_string(), log.to_string_lossy().into_owned()]
        };
        let ctx = || StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        // The parameters of the `session/new` with this id, as the agent read them.
        let session_new = |log: &std::path::Path, id: u64| -> Value {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let text = std::fs::read_to_string(log).unwrap_or_default();
                let sent = text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["method"] == "session/new" && v["id"] == id);
                if let Some(request) = sent {
                    return request["params"].clone();
                }
                assert!(Instant::now() < deadline, "the agent was sent no session/new {id}: {text}");
                std::thread::yield_now();
            }
        };
        let note = |params: &Value| params["_meta"]["systemPrompt"]["append"].as_str().unwrap_or_default().to_string();
        let cwd = std::env::temp_dir().to_string_lossy().into_owned();

        let log = dir.join("built-in");
        let mut backend = AcpBackend::start_builtin(&agent(&log), ctx(), &dir).unwrap();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut Vec::new());
        backend.set_project(Some(project.as_path()));
        ChatBackend::new_session(&mut backend);
        let handed = session_new(&log, backend.next_id);
        assert_eq!(handed["_meta"]["intcad"]["project"], json!(project.to_string_lossy()), "{handed}");
        assert_eq!(handed["cwd"], cwd.as_str(), "the session's own folder stays the work folder");
        let handed_note = note(&handed);
        assert!(handed_note.contains(acp::PROJECT_FILE_TOOLS) && !handed_note.contains(acp::NO_FILE_TOOLS), "{handed_note}");

        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"s1"})) }, &mut Vec::new());
        backend.set_project(None);
        ChatBackend::new_session(&mut backend);
        let taken_back = session_new(&log, backend.next_id);
        assert!(taken_back["_meta"].get("intcad").is_none(), "{taken_back}");
        let taken_back_note = note(&taken_back);
        assert!(taken_back_note.contains(acp::NO_FILE_TOOLS) && !taken_back_note.contains(acp::PROJECT_FILE_TOOLS), "{taken_back_note}");
        backend.shutdown(true);

        // Any other agent opens its session as soon as `initialize` is answered: a project it
        // was given before goes nowhere.
        let log = dir.join("other");
        let mut other = AcpBackend::start(&agent(&log), ctx()).unwrap();
        other.set_project(Some(project.as_path()));
        other.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut Vec::new());
        let ignored = session_new(&log, other.next_id);
        assert!(ignored["_meta"].get("intcad").is_none(), "{ignored}");
        let ignored_note = note(&ignored);
        assert!(ignored_note.contains(acp::NO_FILE_TOOLS) && !ignored_note.contains(acp::PROJECT_FILE_TOOLS), "{ignored_note}");
        other.shutdown(true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn snapshot(session: &str, value: &str) -> Value {
        json!({"sessionId":session,"configOptions":[{"id":"provider.model","category":"model","name":"Model","type":"select","currentValue":value,"options":[{"value":"fast","name":"Fast"},{"value":"smart","name":"Smart"}]}]})
    }

    #[test]
    fn config_updates_and_replies_are_isolated_to_the_active_session() {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "while IFS= read -r line; do :; done".into()], ctx).unwrap();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(snapshot("s1", "fast")) }, &mut events);
        assert_eq!(backend.session.as_deref(), Some("s1"));
        events.clear();
        backend.set_config_option("provider.model", "smart");
        let old_request = backend.config_pending.as_ref().map(|p| p.0).unwrap();
        ChatBackend::new_session(&mut backend);
        let new_session_request = backend.next_id;
        assert!(backend.config_pending.is_none());
        let old_notification = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"config_option_update","configOptions":[]}}}"#;
        backend.handle(acp::parse(old_notification).unwrap(), &mut events);
        assert!(events.is_empty());
        backend.handle(Incoming::Reply { id: new_session_request, result: Ok(snapshot("s2", "fast")) }, &mut events);
        assert_eq!(backend.session.as_deref(), Some("s2"));
        events.clear();
        backend.set_config_option("provider.model", "smart");
        let new_request = backend.config_pending.as_ref().map(|p| p.0).unwrap();
        backend.handle(Incoming::Reply { id: old_request, result: Ok(snapshot("s1", "smart")) }, &mut events);
        backend.handle(Incoming::Reply { id: old_request, result: Err((-32001, "old rejected request".into())) }, &mut events);
        backend.handle(acp::parse(old_notification).unwrap(), &mut events);
        assert!(events.is_empty(), "old requests and session notifications are ignored");
        assert_eq!(backend.config_pending.as_ref().map(|p| p.0), Some(new_request), "an old reply cannot clear the new session's pending request");
        let new_notification = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s2","update":{"sessionUpdate":"config_option_update","configOptions":[]}}}"#;
        backend.handle(acp::parse(new_notification).unwrap(), &mut events);
        assert!(matches!(events.as_slice(), [Event::ConfigOptions(options)] if options.is_empty()));
        assert_eq!(backend.config_pending.as_ref().map(|p| p.0), Some(new_request), "a notification is not an acknowledgement");
        events.clear();
        backend.handle(Incoming::Reply { id: new_request, result: Ok(snapshot("s2", "smart")) }, &mut events);
        assert!(backend.config_pending.is_none());
        assert!(matches!(events.as_slice(), [Event::ConfigOptionFinished { result: Ok(options), .. }] if options[0].current_value == "smart"));
        backend.shutdown(true);
    }

    /// The agent's advertised commands belong to the open session: a late update from a retired
    /// one cannot repopulate the new chat's menu.
    #[test]
    fn available_commands_are_isolated_to_the_active_session() {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "while IFS= read -r line; do :; done".into()], ctx).unwrap();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        assert_eq!(backend.session.as_deref(), Some("s1"));
        events.clear();
        let s1 = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"skill/solidcraft-modeling","description":"Build a part"}]}}}"#;
        backend.handle(acp::parse(s1).unwrap(), &mut events);
        assert!(
            matches!(events.as_slice(), [Event::Commands { session, commands }] if session == "s1" && commands.len() == 1
                && commands[0].name == "skill/solidcraft-modeling" && commands[0].description == "Build a part"),
            "{events:?}"
        );
        ChatBackend::new_session(&mut backend);
        let new_session_request = backend.next_id;
        backend.handle(Incoming::Reply { id: new_session_request, result: Ok(json!({"sessionId":"s2"})) }, &mut events);
        assert_eq!(backend.session.as_deref(), Some("s2"));
        events.clear();
        backend.handle(acp::parse(s1).unwrap(), &mut events);
        assert!(events.is_empty(), "a retired session's commands are ignored");
        let s2 = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s2","update":{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"skill/editing"}]}}}"#;
        backend.handle(acp::parse(s2).unwrap(), &mut events);
        assert!(
            matches!(events.as_slice(), [Event::Commands { session, commands }] if session == "s2" && commands.len() == 1
                && commands[0].name == "skill/editing"),
            "{events:?}"
        );
        backend.shutdown(true);
    }

    /// A refused model is the picker's business, not an error of the chat; signing in stays as it was.
    #[test]
    fn a_refused_model_is_reported_as_such() {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "while IFS= read -r line; do :; done".into()], ctx).unwrap();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        events.clear();
        backend.set_model("zzz");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((-32602, "no such model".into())) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::ModelRejected(message)] if message == "no such model"), "{events:?}");
        events.clear();
        backend.set_model("fast");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({})) }, &mut events);
        assert!(events.is_empty(), "{events:?}");
        backend.set_model("zzz");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((acp::AUTH_REQUIRED, "sign in".into())) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::AuthRequired { .. }]), "{events:?}");
        backend.shutdown(true);
    }
}
