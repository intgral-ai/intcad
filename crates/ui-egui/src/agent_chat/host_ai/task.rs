//! Workflow-owned ACP tasks. A task gets a fresh ACP session and never the direct-chat history.
//! It may reuse only this executor's idle, healthy process, and every update is checked against
//! its upstream session before it becomes progress or proposal text.
//!
//! Delegation is proposal-only: the task process gets no CAD MCP bridge and no host control token.
//! Its answer is validated here before any caller could schedule a write. This is not an OS
//! sandbox; the agent still runs with the user's own account.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use super::super::acp::{self, AuthMethod, Incoming, PermissionOption, ToolStatus};
use super::super::process::{AgentProcess, FromAgent};
use super::{AcpBinding, AiError, Usage};

/// Time an agent gets to acknowledge `session/cancel` before its process tree is stopped.
pub(crate) const TASK_CANCEL_GRACE: Duration = Duration::from_secs(5);
/// Most answer text retained from one task.
pub(crate) const MAX_TASK_TEXT: usize = 256 << 10;
/// The contract's proposal bounds.
pub(crate) const MAX_COMMANDS: usize = 64;
/// Most bytes in one command's parameter object.
pub(crate) const MAX_COMMAND_BYTES: usize = 64 << 10;
/// ACP session start and model selection have their own 30-second ceiling, independent of the
/// task's remaining wall time.
pub(crate) const SESSION_START: Duration = Duration::from_secs(30);
/// Poll interval while waiting for ACP output, permission answers and cancellation.
const TICK: Duration = Duration::from_millis(20);

/// Process-local document identity used by a proposal's `source`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskStamp {
    pub document_id: String,
    pub revision: u64,
}

/// One command the agent proposed. It is not applied here and is not evidence of a CAD change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TaskCommand {
    pub command: String,
    pub params: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TaskProposal {
    pub source: TaskStamp,
    pub commands: Vec<TaskCommand>,
}

/// Bounded ACP information mapped by the panel adapter; never an arbitrary ACP object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskProgress {
    Text { session: String, text: String },
    Tool { session: String, call_id: String, title: String, state: TaskToolState },
    Plan { session: String, steps: Vec<(String, String)> },
    Permission { session: String, key: u64, title: String, state: TaskPermissionState },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskToolState {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskPermissionState {
    Waiting,
    Allowed,
    Denied,
}

/// What the user answered for one permission request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PermissionAnswer {
    Selected(String),
    Cancelled,
}

/// Owns one task's cancellation flag and permission answers. Answers are scoped to this task,
/// so a stale panel answer cannot reach a later task's session.
#[derive(Default)]
pub(crate) struct TaskControl {
    cancelled: AtomicBool,
    issued: Mutex<std::collections::HashSet<u64>>,
    answers: Mutex<HashMap<u64, PermissionAnswer>>,
}

impl TaskControl {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn answer(&self, key: u64, option: Option<&str>) -> Result<(), AiError> {
        // Only a key this task actually asked the panel about can be answered. Keys are
        // process-wide unique, so an answer for a retired task is refused instead of reaching a
        // later task's permission map.
        if !self.issued.lock().map_err(|_| AiError::internal("task_permission"))?.contains(&key) {
            return Err(AiError::invalid("permission"));
        }
        let answer = match option {
            Some(option) => PermissionAnswer::Selected(option.to_string()),
            None => PermissionAnswer::Cancelled,
        };
        self.answers.lock().map_err(|_| AiError::internal("task_permission"))?.insert(key, answer);
        Ok(())
    }

    pub(crate) fn next_key(&self) -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let key = NEXT.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut issued) = self.issued.lock() {
            issued.insert(key);
        }
        key
    }

    pub(crate) fn take_answers(&self) -> Result<Vec<(u64, PermissionAnswer)>, AiError> {
        let mut answers = self.answers.lock().map_err(|_| AiError::internal("task_permission"))?;
        Ok(answers.drain().collect())
    }
}

/// One delegated task. `text` is already plain text: the caller refuses images/documents before
/// starting a proposal-only ACP session.
pub(crate) struct TaskRequest {
    pub text: String,
    pub model: Option<String>,
    pub source: TaskStamp,
    pub timeout: Duration,
    pub remaining_commands: u32,
    pub allowed_commands: Option<Vec<String>>,
}

/// A validated proposal and its origin. The session id is the ACP session, never the direct chat's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TaskOutcome {
    pub agent_session_id: String,
    pub text: String,
    pub stop_reason: String,
    pub proposal: TaskProposal,
    pub usage: Option<Usage>,
}

/// What a real ACP session start proved about the connection.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TaskProbe {
    pub state: super::ConnectionState,
    pub auth: Vec<AuthMethod>,
    pub images: bool,
    pub documents: bool,
    pub model_selection: bool,
}

struct LiveProcess {
    connection_id: String,
    cwd: std::path::PathBuf,
    proc: AgentProcess,
    init: Value,
    auth: Vec<AuthMethod>,
    session: Option<String>,
    next_id: u64,
    images: bool,
    documents: bool,
    model_selection: bool,
    /// False when the peer omitted session identities or the process was cancelled; such a
    /// process is never reused by a different task.
    reusable: bool,
}

impl LiveProcess {
    fn request(&mut self, method: &str, params: Value) -> u64 {
        self.next_id = self.next_id.saturating_add(1);
        self.proc.send(&acp::request(self.next_id, method, params));
        self.next_id
    }

    fn shutdown(&mut self) {
        self.proc.shutdown(true);
        let _ = std::fs::remove_dir_all(&self.cwd);
    }
}

/// The executor behind `HostAi::agent_run`. A sequential task may reuse one healthy process; a
/// concurrent or unverifiable peer gets a fresh one so histories can never cross.
#[derive(Default)]
pub(crate) struct AcpTaskExecutor {
    state: Mutex<Option<LiveProcess>>,
    generation: AtomicU64,
}

impl AcpTaskExecutor {
    fn lock(&self) -> Result<MutexGuard<'_, Option<LiveProcess>>, AiError> {
        self.state.lock().map_err(|_| AiError::internal("acp_task"))
    }

    /// Start a task and return exactly one terminal outcome. Updates from any other session are
    /// dropped; their permission requests are answered as cancelled.
    pub(crate) fn run(
        &self,
        binding: &AcpBinding,
        request: TaskRequest,
        control: &TaskControl,
        emit: &mut dyn FnMut(TaskProgress),
    ) -> Result<TaskOutcome, AiError> {
        if control.is_cancelled() {
            return Err(AiError::cancelled());
        }
        let deadline = Instant::now().checked_add(request.timeout).unwrap_or_else(Instant::now);
        let mut live = self.take_or_start(binding, control)?;
        let result = self.run_live(&mut live, request, control, emit, deadline);
        let keep = result.is_ok() && live.reusable && !control.is_cancelled();
        if keep {
            let mut slot = self.lock()?;
            if slot.as_ref().is_some_and(|old| old.connection_id != live.connection_id)
                && let Some(mut old) = slot.take()
            {
                old.shutdown();
            }
            *slot = Some(live);
        } else {
            live.shutdown();
        }
        result
    }

    /// A bounded authentication/capability probe. It starts a real session and closes it, so a
    /// ready runtime by itself never authenticates anything.
    pub(crate) fn probe(&self, binding: &AcpBinding) -> Result<TaskProbe, AiError> {
        let existing = self.lock()?.take();
        if let Some(mut live) = existing {
            if live.connection_id == binding.connection_id() && live.reusable && live.proc.pid().is_some() {
                // The idle process already has verified identities; reuse it for this probe.
                let state = if live.session.is_some() { super::ConnectionState::Available } else { super::ConnectionState::Unavailable };
                let probe = TaskProbe {
                    state,
                    auth: live.auth.clone(),
                    images: live.images,
                    documents: live.documents,
                    model_selection: live.model_selection,
                };
                *self.lock()? = Some(live);
                return Ok(probe);
            }
            live.shutdown();
        }
        let control = TaskControl::default();
        let mut live = self.start(binding, &control)?;
        let auth = live.auth.clone();
        let (images, documents) = (live.images, live.documents);
        match self.open_session(&mut live, Instant::now() + Duration::from_secs(30), &control) {
            Ok((session, _, reply)) => {
                live.session = Some(session);
                let model_selection = acp::models(&reply).is_some();
                live.shutdown();
                Ok(TaskProbe { state: super::ConnectionState::Available, auth, images, documents, model_selection })
            }
            Err(error) => {
                let state = if error.code() == super::AiErrorCode::AuthRequired {
                    super::ConnectionState::AuthRequired
                } else {
                    super::ConnectionState::Unavailable
                };
                live.shutdown();
                Ok(TaskProbe { state, auth, images, documents, model_selection: false })
            }
        }
    }

    /// Invoke the agent's own authentication method, then verify with a real session. Credentials
    /// stay with the agent; this function stores none.
    pub(crate) fn authenticate(&self, binding: &AcpBinding, method: &str) -> Result<TaskProbe, AiError> {
        let control = TaskControl::default();
        let mut live = self.start(binding, &control)?;
        if !live.auth.iter().any(|m| m.id == method) {
            live.shutdown();
            return Err(AiError::unsupported("auth_method"));
        }
        let id = live.request("authenticate", serde_json::json!({"methodId": method}));
        if let Err(error) = reply(&live, id, Instant::now() + Duration::from_secs(300), &control) {
            live.shutdown();
            return Err(error);
        }
        let auth = live.auth.clone();
        let (images, documents) = (live.images, live.documents);
        match self.open_session(&mut live, Instant::now() + Duration::from_secs(30), &control) {
            Ok((session, _, session_reply)) => {
                let model_selection = acp::models(&session_reply).is_some();
                live.session = Some(session);
                live.shutdown();
                Ok(TaskProbe { state: super::ConnectionState::Available, auth, images, documents, model_selection })
            }
            Err(error) => {
                let state = if error.code() == super::AiErrorCode::AuthRequired {
                    super::ConnectionState::AuthRequired
                } else {
                    super::ConnectionState::Unavailable
                };
                live.shutdown();
                Ok(TaskProbe { state, auth, images, documents, model_selection: false })
            }
        }
    }

    fn take_or_start(&self, binding: &AcpBinding, control: &TaskControl) -> Result<LiveProcess, AiError> {
        let existing = self.lock()?.take();
        if let Some(live) = existing {
            if live.connection_id == binding.connection_id() && live.reusable && live.proc.pid().is_some() {
                return Ok(live);
            }
            let mut live = live;
            live.shutdown();
        }
        self.start(binding, control)
    }

    fn start(&self, binding: &AcpBinding, control: &TaskControl) -> Result<LiveProcess, AiError> {
        let argv = binding.argv();
        if argv.is_empty() {
            return Err(AiError::invalid("command"));
        }
        let cwd = task_directory(self.generation.fetch_add(1, Ordering::AcqRel).saturating_add(1))?;
        let proc = AgentProcess::spawn(&argv, &cwd, &["SOLIDCRAFT_CONTROL_TOKEN"], || {}).map_err(|_| AiError::task_failed("agent_start"))?;
        let mut live = LiveProcess {
            connection_id: binding.connection_id().to_string(),
            cwd,
            proc,
            init: Value::Null,
            auth: Vec::new(),
            session: None,
            next_id: 0,
            images: false,
            documents: false,
            model_selection: false,
            reusable: false,
        };
        let initialize = live.request("initialize", acp::initialize_params(env!("CARGO_PKG_VERSION")));
        let init = match reply(&live, initialize, Instant::now() + Duration::from_secs(10), control) {
            Ok(value) => value,
            Err(error) => {
                live.shutdown();
                return Err(error);
            }
        };
        live.images = acp::takes_images(&init);
        live.documents = acp::takes_documents(&init);
        live.auth = acp::auth_methods(&init);
        live.init = init;
        Ok(live)
    }

    fn open_session(&self, live: &mut LiveProcess, deadline: Instant, control: &TaskControl) -> Result<(String, String, Value), AiError> {
        let session_deadline = (Instant::now() + SESSION_START).min(deadline);
        let params = acp::session_new_params(&live.cwd, None, &live.init, acp::PROPOSAL_NOTE);
        let id = live.request("session/new", params);
        let value = reply(live, id, session_deadline, control)?;
        let Some(session) = value.get("sessionId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string) else {
            return Err(AiError::task_failed("session_new"));
        };
        if live.session.as_deref() == Some(session.as_str()) {
            // A peer that hands back the previous session id has not opened a fresh history.
            return Err(AiError::task_failed("session_reuse"));
        }
        Ok((session, String::new(), value))
    }

    fn run_live(
        &self,
        live: &mut LiveProcess,
        request: TaskRequest,
        control: &TaskControl,
        emit: &mut dyn FnMut(TaskProgress),
        deadline: Instant,
    ) -> Result<TaskOutcome, AiError> {
        let (session, _, session_reply) = self.open_session(live, deadline, control)?;
        live.model_selection = acp::models(&session_reply).is_some();
        if let Some(model) = request.model.as_deref().filter(|m| !m.is_empty()) {
            select_model(live, &session, model, &session_reply, deadline, control)?;
        }
        live.session = Some(session.clone());
        live.reusable = true;
        if control.is_cancelled() {
            self.cancel_live(live, &session, 0, &mut HashMap::new(), control)?;
            return Err(AiError::cancelled());
        }
        let prompt = live.request("session/prompt", acp::prompt_params(&session, &[super::super::backend::PromptPart::Text(request.text.clone())]));
        let mut text = String::new();
        let usage;
        let mut open: HashMap<u64, (Value, Vec<PermissionOption>)> = HashMap::new();
        let stop_reason;
        loop {
            if control.is_cancelled() {
                self.cancel_live(live, &session, prompt, &mut open, control)?;
                return Err(AiError::cancelled());
            }
            for (key, answer) in control.take_answers()? {
                let Some((rpc, options)) = open.remove(&key) else { continue };
                let (selected, state, denied) = resolve_answer(&answer, &options);
                live.proc.send(&acp::permission_reply(rpc, selected.as_deref()));
                emit(TaskProgress::Permission { session: session.clone(), key, title: String::new(), state });
                if denied {
                    return Err(AiError::permission_denied(&format!("p{key}")));
                }
            }
            if Instant::now() >= deadline {
                self.cancel_live(live, &session, prompt, &mut open, control)?;
                return Err(AiError::timeout());
            }
            let wait = deadline.saturating_duration_since(Instant::now()).min(TICK);
            match live.proc.from_agent.recv_timeout(wait) {
                Ok(FromAgent::Line(line)) => {
                    let Some(message) = acp::parse(&line) else { continue };
                    match message {
                        Incoming::Reply { id, result } if id == prompt => {
                            let value = match result {
                                Ok(value) => value,
                                Err((code, message)) => {
                                    if code == acp::AUTH_REQUIRED || message.to_ascii_lowercase().contains("auth") {
                                        return Err(AiError::auth_required_task());
                                    }
                                    return Err(AiError::task_failed("prompt"));
                                }
                            };
                            stop_reason = Some(value.get("stopReason").and_then(Value::as_str).map(str::to_string).unwrap_or_default());
                            usage = parse_usage(&value);
                            break;
                        }
                        Incoming::Reply { .. } => {}
                        Incoming::Text { session: update_session, thought, text: chunk } => {
                            if !live.accepts(&update_session) || thought {
                                // The wire progress shape has no thought flag; dropping reasoning
                                // is honest and keeps it out of the proposal text.
                                continue;
                            }
                            let chunk = bounded_text(&chunk);
                            if text.len() < MAX_TASK_TEXT {
                                let room = MAX_TASK_TEXT.saturating_sub(text.len());
                                text.push_str(&chunk.chars().take(room).collect::<String>());
                            }
                            emit(TaskProgress::Text { session: update_session, text: chunk });
                        }
                        Incoming::Tool { session: update_session, id, title, status, .. } => {
                            if !live.accepts(&update_session) {
                                continue;
                            }
                            let Some(status) = status else { continue };
                            let state = match status {
                                ToolStatus::Pending | ToolStatus::InProgress => TaskToolState::Running,
                                ToolStatus::Completed => TaskToolState::Completed,
                                ToolStatus::Failed => TaskToolState::Failed,
                            };
                            emit(TaskProgress::Tool {
                                session: update_session,
                                call_id: bounded_id(&id),
                                title: title.as_deref().map(bounded_text).unwrap_or_default(),
                                state,
                            });
                        }
                        Incoming::Plan { session: update_session, steps } => {
                            if !live.accepts(&update_session) {
                                continue;
                            }
                            let steps = steps.into_iter().take(64).map(|(title, state)| (bounded_text(&title), state)).collect();
                            emit(TaskProgress::Plan { session: update_session, steps });
                        }
                        Incoming::Permission { session: update_session, rpc, title, options, .. } => {
                            if !live.accepts(&update_session) {
                                live.proc.send(&acp::permission_reply(rpc, None));
                                continue;
                            }
                            let key = control.next_key();
                            open.insert(key, (rpc, options));
                            emit(TaskProgress::Permission {
                                session: update_session,
                                key,
                                title: bounded_text(&title),
                                state: TaskPermissionState::Waiting,
                            });
                        }
                        Incoming::Unsupported { rpc, .. } => {
                            live.proc.send(&acp::reply_error(rpc, acp::METHOD_NOT_FOUND, "Intcad task sessions serve no host tools"));
                        }
                        Incoming::AuthStatus(_) | Incoming::Ignored => {}
                        Incoming::ModeChanged(_) | Incoming::ModelChanged(_) | Incoming::ConfigOptions { .. } => {}
                    }
                }
                Ok(FromAgent::Exited(_)) => return Err(AiError::task_failed("agent_exit")),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(AiError::task_failed("agent_exit")),
            }
        }
        for (_, (rpc, _)) in open.drain() {
            live.proc.send(&acp::permission_reply(rpc, None));
        }
        let proposal = parse_proposal(&text, &request.source, request.remaining_commands, request.allowed_commands.as_deref())?;
        Ok(TaskOutcome { agent_session_id: session, text, stop_reason: stop_reason.unwrap_or_default(), proposal, usage })
    }

    fn cancel_live(
        &self,
        live: &mut LiveProcess,
        session: &str,
        prompt: u64,
        open: &mut HashMap<u64, (Value, Vec<PermissionOption>)>,
        control: &TaskControl,
    ) -> Result<(), AiError> {
        for (_, (rpc, _)) in open.drain() {
            live.proc.send(&acp::permission_reply(rpc, None));
        }
        let _ = control.take_answers();
        live.proc.send(&acp::notification("session/cancel", serde_json::json!({"sessionId": session})));
        if prompt == 0 {
            live.reusable = false;
            live.shutdown();
            return Ok(());
        }
        let deadline = Instant::now() + TASK_CANCEL_GRACE;
        while Instant::now() < deadline {
            let wait = deadline.saturating_duration_since(Instant::now()).min(TICK);
            match live.proc.from_agent.recv_timeout(wait) {
                Ok(FromAgent::Line(line)) => {
                    if let Some(Incoming::Reply { id, .. }) = acp::parse(&line)
                        && id == prompt
                    {
                        break;
                    }
                }
                Ok(FromAgent::Exited(_)) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        // Cancellation is terminal for this process: do not reuse an ACP session whose
        // acknowledgement is uncertain.
        live.reusable = false;
        live.shutdown();
        Ok(())
    }
}

impl LiveProcess {
    fn accepts(&self, update_session: &str) -> bool {
        match self.session.as_deref() {
            Some(active) if !update_session.is_empty() => active == update_session,
            // An empty id is only accepted on a fresh process; a reused process requires
            // verifiable session identity and marks the peer non-reusable otherwise.
            Some(_) => false,
            None => false,
        }
    }
}

fn select_model(
    live: &mut LiveProcess,
    session: &str,
    model: &str,
    session_reply: &Value,
    deadline: Instant,
    control: &TaskControl,
) -> Result<(), AiError> {
    let deadline = (Instant::now() + SESSION_START).min(deadline);
    if let Some((current, available)) = acp::models(session_reply) {
        if model != current && !available.iter().any(|m| m.id == model) {
            return Err(AiError::unsupported("model_selection"));
        }
        if model == current {
            return Ok(());
        }
    }
    let id = live.request("session/set_model", acp::set_model_params(session, model));
    match reply(live, id, deadline, control) {
        Ok(_) => Ok(()),
        Err(error) if error.code() == super::AiErrorCode::UnsupportedCapability => Err(AiError::unsupported("model_selection")),
        Err(error) => Err(error),
    }
}

fn resolve_answer(answer: &PermissionAnswer, options: &[PermissionOption]) -> (Option<String>, TaskPermissionState, bool) {
    match answer {
        PermissionAnswer::Cancelled => (None, TaskPermissionState::Denied, true),
        PermissionAnswer::Selected(choice) => {
            if choice == "deny" || choice.starts_with("reject") {
                let selected = options.iter().find(|o| o.kind.starts_with("reject")).map(|o| o.id.clone());
                return (selected, TaskPermissionState::Denied, true);
            }
            let selected = if choice == "allow" {
                options.iter().find(|o| o.kind.starts_with("allow")).map(|o| o.id.clone())
            } else if options.iter().any(|o| o.id == *choice) {
                Some(choice.clone())
            } else {
                options.iter().find(|o| o.kind.starts_with("allow")).map(|o| o.id.clone())
            };
            (selected, TaskPermissionState::Allowed, false)
        }
    }
}

fn reply(live: &LiveProcess, id: u64, deadline: Instant, control: &TaskControl) -> Result<Value, AiError> {
    loop {
        if control.is_cancelled() {
            return Err(AiError::cancelled());
        }
        if Instant::now() >= deadline {
            return Err(AiError::timeout());
        }
        let wait = deadline.saturating_duration_since(Instant::now()).min(TICK);
        match live.proc.from_agent.recv_timeout(wait) {
            Ok(FromAgent::Line(line)) => {
                let Some(message) = acp::parse(&line) else { continue };
                match message {
                    Incoming::Reply { id: got, result } if got == id => {
                        return match result {
                            Ok(value) => Ok(value),
                            Err((code, message)) => {
                                if code == acp::METHOD_NOT_FOUND {
                                    Err(AiError::unsupported("acp_method"))
                                } else if code == acp::AUTH_REQUIRED || message.to_ascii_lowercase().contains("auth") {
                                    Err(AiError::auth_required_task())
                                } else {
                                    Err(AiError::task_failed("acp_request"))
                                }
                            }
                        };
                    }
                    Incoming::Permission { rpc, .. } => {
                        live.proc.send(&acp::permission_reply(rpc, None));
                    }
                    Incoming::Unsupported { rpc, .. } => {
                        live.proc.send(&acp::reply_error(rpc, acp::METHOD_NOT_FOUND, "Intcad task sessions serve no host tools"));
                    }
                    _ => {}
                }
            }
            Ok(FromAgent::Exited(_)) => return Err(AiError::task_failed("agent_exit")),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(AiError::task_failed("agent_exit")),
        }
    }
}

fn task_directory(generation: u64) -> Result<std::path::PathBuf, AiError> {
    // Unique across executors in one process: two windows/tests must not share or delete each
    // other's empty task folder.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let unique = NEXT.fetch_add(1, Ordering::AcqRel);
    let path = std::env::temp_dir().join(format!("solidcraft-acp-task-{}-{unique}-{generation}", std::process::id()));
    std::fs::create_dir_all(&path).map_err(|_| AiError::internal("task_workdir"))?;
    Ok(path)
}

fn parse_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?.as_object()?;
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let parsed = Usage { input_tokens: field("inputTokens"), output_tokens: field("outputTokens"), total_tokens: field("totalTokens") };
    (parsed.input_tokens.is_some() || parsed.output_tokens.is_some() || parsed.total_tokens.is_some()).then_some(parsed)
}

fn bounded_text(text: &str) -> String {
    text.chars().take(4096).collect()
}

fn bounded_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len().min(128));
    for byte in id.bytes().take(128) {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':') {
            out.push(byte as char);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() || !out.as_bytes()[0].is_ascii_alphanumeric() {
        out.insert(0, 't');
    }
    out
}

/// Validate the exact proposal shape before a caller can schedule it. No proposal is applied here.
pub(crate) fn parse_proposal(
    text: &str,
    expected: &TaskStamp,
    remaining_commands: u32,
    allowed_commands: Option<&[String]>,
) -> Result<TaskProposal, AiError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_TASK_TEXT {
        return Err(AiError::invalid_proposal());
    }
    let value: Value = serde_json::from_str(trimmed).map_err(|_| AiError::invalid_proposal())?;
    let object = value.as_object().ok_or_else(AiError::invalid_proposal)?;
    if object.len() != 2 || !object.contains_key("source") || !object.contains_key("commands") {
        return Err(AiError::invalid_proposal());
    }
    let source = object.get("source").and_then(Value::as_object).ok_or_else(AiError::invalid_proposal)?;
    if source.len() != 2 {
        return Err(AiError::invalid_proposal());
    }
    let document_id = source.get("documentId").and_then(Value::as_str).ok_or_else(AiError::invalid_proposal)?;
    let revision_text = source.get("revision").and_then(Value::as_str).ok_or_else(AiError::invalid_proposal)?;
    if !valid_id(document_id) || revision_text.is_empty() || revision_text.len() > 20 {
        return Err(AiError::invalid_proposal());
    }
    let revision: u64 = revision_text.parse().map_err(|_| AiError::invalid_proposal())?;
    if revision.to_string() != revision_text {
        return Err(AiError::invalid_proposal());
    }
    let source = TaskStamp { document_id: document_id.to_string(), revision };
    if &source != expected {
        return Err(AiError::invalid_proposal());
    }
    let commands = object.get("commands").and_then(Value::as_array).ok_or_else(AiError::invalid_proposal)?;
    let ceiling = usize::try_from(remaining_commands).unwrap_or(0).min(MAX_COMMANDS);
    if commands.is_empty() || commands.len() > MAX_COMMANDS {
        return Err(AiError::invalid_proposal());
    }
    if commands.len() > ceiling {
        return Err(AiError::exhausted("cad_commands", remaining_commands));
    }
    let mut parsed = Vec::with_capacity(commands.len());
    for command in commands {
        let object = command.as_object().ok_or_else(AiError::invalid_proposal)?;
        if object.len() != 2 || !object.contains_key("command") || !object.contains_key("params") {
            return Err(AiError::invalid_proposal());
        }
        let name = object.get("command").and_then(Value::as_str).ok_or_else(AiError::invalid_proposal)?;
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(AiError::invalid_proposal());
        }
        if let Some(allowed) = allowed_commands
            && !allowed.iter().any(|allowed| allowed == name)
        {
            return Err(AiError::invalid_proposal());
        }
        let params = object.get("params").and_then(Value::as_object).ok_or_else(AiError::invalid_proposal)?.clone();
        let size = serde_json::to_vec(&params).map_err(|_| AiError::invalid_proposal())?.len();
        if size > MAX_COMMAND_BYTES {
            return Err(AiError::invalid_proposal());
        }
        parsed.push(TaskCommand { command: name.to_string(), params });
    }
    Ok(TaskProposal { source, commands: parsed })
}

fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    value.len() <= 128 && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}
