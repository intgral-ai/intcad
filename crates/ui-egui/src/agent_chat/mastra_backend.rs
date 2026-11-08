//! The Mastra workflow backend: one [`ChatBackend`] that hands a turn to the standalone Mastra
//! runtime over the versioned local protocol (`super::mastra_protocol`), while the model calls
//! stay with the AI connection the user already has (`HostAi`). The runtime understands the
//! workflow; the host owns the document, the chat's MCP bridge, permissions, cancellation and
//! undo.
//!
//! Everything slow (process start, IPC, host model calls) happens on the runtime client's worker
//! or a spawned host-call thread. [`MastraBackend::poll`] only drains already-decoded notices, so
//! no egui frame waits on the runtime.
//!
//! The turn binding is frozen when [`ChatBackend::prompt`] is called: workflow, connection,
//! model, session and turn ids are copied then. Picker changes reach the next turn only.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::acp::McpBridge;
use super::backend::{ChatBackend, Event, PromptPart, ToolStatus};
use super::host_ai::{
    self, AiError, AiErrorCode, AiErrorDetails, AiLimits, CallContext, CallScope, ContinuationId, Delivery, FinishReason, GenerateOptions,
    GenerateRequest, HostAi, MessageRole, ModelCallBudget, ModelMessage, SessionKey, TaskControl, TaskPermissionState, TaskProgress, TaskRequest,
    TaskStamp, TaskToolState, TurnControl,
};
use super::mastra_protocol::*;
use super::runtime_client::{
    CallbackCall, ClientError, ClientNotice, CloseReason, HostDispatch, IdSource, ReplyPort, RuntimeAdmission, RuntimeClient, RuntimeLaunch,
    RuntimeSender, RuntimeServices, ScopeLifecycle, TransportCause,
};

/// The runtime protocol's own default workflow when the preference names none.
pub(crate) const DEFAULT_WORKFLOW: &str = "modeling";
/// The fixed validation profile reference used until #11/#12 provide candidate profiles.
pub(crate) const NATIVE_PROFILE: &str = "solidcraft-native";
/// SHA-256 of the empty input, a stable placeholder digest for that profile reference.
pub(crate) const EMPTY_DIGEST: &str = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// Most tool output text kept for the conversation card.
const MAX_TOOL_OUTPUT: usize = 64 << 10;
/// How long a session or turn call may take before the runtime machine expires it.
const SESSION_TIMEOUT: Duration = Duration::from_secs(30);
const TURN_TIMEOUT: Duration = Duration::from_secs(1200);
/// How long quitting waits for the runtime transport to close.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);
/// The model calls one workflow turn may make through this host.
const MODEL_CALLS_PER_TURN: u32 = 20;
/// The ACP tasks one workflow turn may delegate. This says nothing about the agent's own hidden
/// model calls; it bounds only host delegations.
const AGENT_TASKS_PER_TURN: u32 = 4;

/// What a Mastra backend is told before it starts. The execution choice is already made; this is
/// only the connection, workflow and launch identity it uses.
pub struct MastraConfig {
    /// The existing AI connection (`AgentConfig.id`) the workflow's model calls use.
    pub connection_id: String,
    /// The model selected for that connection when the backend started.
    pub model: String,
    /// The workflow to ask the runtime to run.
    pub workflow_id: String,
    /// The chat's existing MCP bridge, if the window could open one.
    pub bridge: Option<McpBridge>,
    /// Chat instructions copied into every new session.
    pub instructions: String,
    /// Set on the host worker when a `connection.describe` call answers; polled by the panel as
    /// [`Event::ConnectionAuth`], never as runtime readiness.
    pub connection_auth: Arc<Mutex<Option<bool>>>,
    /// The connection/model pair frozen for the turn in progress; host calls must match it.
    pub frozen_binding: Arc<Mutex<Option<(String, String)>>>,
    /// Wakes the window when a runtime notice arrives.
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

/// A frozen turn identity: created at [`ChatBackend::prompt`] and never changed by a later picker
/// or preference edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenBinding {
    pub workflow_id: String,
    pub connection_id: String,
    pub model_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub workflow_run_id: String,
}

struct ActiveTurn {
    binding: FrozenBinding,
    scope: TurnScope,
}

/// A decoded notice for the backend; tests can inject host-origin progress the same way the live
/// port delivers runtime notices.
pub(crate) enum PortNotice {
    Client(Box<ClientNotice>),
    #[cfg(test)]
    Progress(AgentProgress),
}

impl PortNotice {
    fn client(notice: ClientNotice) -> Self {
        Self::Client(Box::new(notice))
    }
}

/// The narrow runtime seam: one implementation drives the real client, a deterministic one in
/// tests records the exact protocol values that would have gone on the wire.
pub(crate) trait Port: Send {
    fn request(&mut self, scope: Scope, request: HostRequest, timeout: Duration) -> Result<RequestId, ClientError>;
    fn cancel_turn(&mut self, scope: &TurnScope, reason: CancelReason) -> Result<(), ClientError>;
    fn shutdown(&mut self) -> Result<(), ClientError>;
    fn drain(&mut self) -> Vec<PortNotice>;
    fn wait_stopped(&mut self, _timeout: Duration) {}
}

/// The live port over [`RuntimeClient`]. All calls are bounded nonblocking admissions; none waits
/// for I/O.
struct LivePort {
    client: RuntimeClient,
    sender: RuntimeSender,
    /// One notice is at most one framed protocol value. Draining below `frame_bytes` would leave
    /// a legal large event at the queue front and wedge every notice behind it.
    drain_bytes: usize,
}

impl Port for LivePort {
    fn request(&mut self, scope: Scope, request: HostRequest, timeout: Duration) -> Result<RequestId, ClientError> {
        let deadline = Instant::now().checked_add(timeout).unwrap_or_else(Instant::now);
        let prepared = self.sender.prepare_request(scope, request, deadline)?;
        self.sender.try_submit(prepared)
    }

    fn cancel_turn(&mut self, scope: &TurnScope, reason: CancelReason) -> Result<(), ClientError> {
        self.sender.cancel_turn(scope, reason)?;
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ClientError> {
        self.sender.shutdown(ShutdownReason::AppExit)?;
        Ok(())
    }

    fn drain(&mut self) -> Vec<PortNotice> {
        self.client.drain(128, self.drain_bytes).into_iter().map(PortNotice::client).collect()
    }

    fn wait_stopped(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let notices = self.client.drain(128, self.drain_bytes);
            if notices.iter().any(|n| matches!(n, ClientNotice::TransportClosed { .. })) {
                return;
            }
            if !notices.is_empty() {
                continue;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The backend. Construct with [`MastraBackend::start`]; it starts the runtime off the window
/// thread and reports readiness through [`Event::RuntimeReady`].
pub struct MastraBackend {
    port: Box<dyn Port>,
    ids: Arc<dyn IdSource>,
    config: MastraConfig,
    host: Option<Arc<HostAi>>,
    runtime_instance: Option<Arc<Mutex<Option<String>>>>,
    model_budget: Option<Arc<AtomicU32>>,
    agent_budget: Option<Arc<AtomicU32>>,
    task_control: Arc<Mutex<Option<Arc<TaskControl>>>>,
    next_model: String,
    next_connection: String,
    runtime_ready: bool,
    session: Option<Id>,
    session_ready: bool,
    awaiting_session: Option<RequestId>,
    reset_target: Option<Id>,
    active: Option<ActiveTurn>,
    stages: Vec<(String, String, String)>,
    plan_set: bool,
    document: Option<DocumentStamp>,
    events: VecDeque<Event>,
    stopped: bool,
    models_sent: bool,
}

impl MastraBackend {
    /// Start the runtime and its bounded client. Returns an error text suitable for the panel
    /// when the process cannot start at all; protocol failures arrive as events later.
    pub fn start(
        config: MastraConfig,
        launch: RuntimeLaunch,
        host: Arc<HostAi>,
        ids: Arc<dyn IdSource>,
        admission: Arc<RuntimeAdmission>,
    ) -> Result<Self, String> {
        let wake = Arc::clone(&config.wake);
        // One framed value plus the notice's scope/enum overhead; never below one MiB.
        let drain_bytes = usize::try_from(launch.limits.frame_bytes).unwrap_or(1 << 20).saturating_add(1 << 20).max(1 << 20);
        let runtime_instance = Arc::new(Mutex::new(None::<String>));
        let model_budget = Arc::new(AtomicU32::new(MODEL_CALLS_PER_TURN));
        let agent_budget = Arc::new(AtomicU32::new(AGENT_TASKS_PER_TURN));
        let task_control: Arc<Mutex<Option<Arc<TaskControl>>>> = Arc::new(Mutex::new(None));
        let runtime = RuntimeClient::start(
            launch,
            runtime_services(
                Arc::clone(&host),
                ids.clone(),
                admission,
                Arc::clone(&config.connection_auth),
                Arc::clone(&runtime_instance),
                Arc::clone(&model_budget),
                Arc::clone(&agent_budget),
                Arc::clone(&task_control),
                Arc::clone(&config.frozen_binding),
            ),
            Arc::new(move || wake()),
        )
        .map_err(|error| crate::i18n::format("ui.mastra_runtime_failed", &[("error", &format!("{:?}", error.code))]))?;
        let sender = runtime.sender();
        let connection = config.connection_id.clone();
        let model = config.model.clone();
        let auth = Arc::clone(&config.connection_auth);
        let mut backend = Self::with_port(config, Box::new(LivePort { client: runtime, sender, drain_bytes }), ids);
        backend.host = Some(host);
        backend.runtime_instance = Some(runtime_instance);
        backend.model_budget = Some(model_budget);
        backend.agent_budget = Some(agent_budget);
        backend.task_control = task_control;
        backend.events.push_back(Event::Documents(false));
        backend.events.push_back(Event::ToolsAvailable(backend.config.bridge.is_some()));
        // The connection's sign-in state is checked once, off the window thread, and kept apart
        // from runtime readiness. A failure leaves it unknown rather than guessing.
        let probe = backend.host.as_ref().map(Arc::clone);
        let _ = std::thread::Builder::new().name("mastra-connection-probe".into()).spawn(move || {
            let Some(host) = probe else { return };
            let Ok(binding) = host.bind(&connection, &model) else { return };
            let Ok(description) = host.describe(&binding) else { return };
            if let Ok(mut reported) = auth.lock() {
                *reported = Some(description.state == super::host_ai::ConnectionState::Available);
            }
        });
        Ok(backend)
    }

    pub(crate) fn with_port(config: MastraConfig, port: Box<dyn Port>, ids: Arc<dyn IdSource>) -> Self {
        let next_model = config.model.clone();
        let next_connection = config.connection_id.clone();
        Self {
            port,
            ids,
            config,
            host: None,
            runtime_instance: Some(Arc::new(Mutex::new(None))),
            model_budget: None,
            agent_budget: None,
            task_control: Arc::new(Mutex::new(None)),
            next_model,
            next_connection,
            runtime_ready: false,
            session: None,
            session_ready: false,
            awaiting_session: None,
            reset_target: None,
            active: None,
            stages: Vec::new(),
            plan_set: false,
            document: None,
            events: VecDeque::new(),
            stopped: false,
            models_sent: false,
        }
    }

    /// The binding of the turn in progress, for tests and a future status line.
    #[cfg(test)]
    pub fn active_binding(&self) -> Option<FrozenBinding> {
        self.active.as_ref().map(|turn| turn.binding.clone())
    }

    /// The binding the next prompt would use.
    #[cfg(test)]
    pub fn next_binding(&self) -> Option<FrozenBinding> {
        Some(FrozenBinding {
            workflow_id: self.config.workflow_id.clone(),
            connection_id: self.next_connection.clone(),
            model_id: self.next_model.clone(),
            session_id: self.session.as_ref()?.to_string(),
            turn_id: String::new(),
            workflow_run_id: String::new(),
        })
    }

    /// Change the connection used by the next turn. A turn already accepted keeps its binding.
    #[cfg(test)]
    pub(crate) fn set_connection(&mut self, connection_id: &str) {
        self.next_connection = connection_id.to_owned();
    }

    /// Whether the runtime transport is ready (not whether its AI connection is authenticated).
    #[cfg(test)]
    pub fn runtime_ready(&self) -> bool {
        self.runtime_ready && !self.stopped
    }

    /// Test seam: the same callback-side pin check `handle_call` uses.
    #[cfg(test)]
    pub(crate) fn retarget_error_for_test(&self, connection: &str, model: &str) -> Option<ErrorCode> {
        let frozen = self.config.frozen_binding.lock().ok().and_then(|frozen| frozen.clone());
        reject_retarget(&frozen, connection, model).err().map(|error| error.code)
    }

    #[cfg(test)]
    pub(crate) fn attach_host_for_test(&mut self, host: Arc<HostAi>) {
        self.host = Some(host);
    }

    #[cfg(test)]
    pub(crate) fn limits_for_test(limits: &Limits) -> Option<AiLimits> {
        ai_limits(limits).ok()
    }

    #[cfg(test)]
    pub(crate) fn budget_error_for_test() -> ErrorCode {
        resource_exhausted("model_calls", MODEL_CALLS_PER_TURN).code
    }

    fn push(&mut self, event: Event) {
        self.events.push_back(event);
    }

    fn start_session(&mut self) {
        if !self.runtime_ready || self.awaiting_session.is_some() {
            return;
        }
        let Ok(session) = self.ids.next_id() else {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
            return;
        };
        if !self.rebind_host_session(self.session.as_ref(), &session) {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
            return;
        }
        let bridge = if self.config.bridge.is_some() {
            let Ok(bridge_id) = self.ids.next_id() else {
                self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
                return;
            };
            Some(BridgeRef { name: BridgeName::SolidCraftApp, bridge_id })
        } else {
            None
        };
        let params = SessionStartParams {
            workflow_id: Id::parse(&self.config.workflow_id).unwrap_or_else(|_| session.clone()),
            instructions: self.config.instructions.chars().take(4000).collect(),
            bridge: RequiredNullable(bridge),
        };
        match self.port.request(Scope::Session(session.clone()), HostRequest::SessionStart(params), SESSION_TIMEOUT) {
            Ok(request) => {
                self.awaiting_session = Some(request);
                self.reset_target = None;
                self.session = Some(session);
                self.session_ready = false;
            }
            Err(_) => self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string())),
        }
    }

    fn begin_turn(&mut self, parts: &[PromptPart]) {
        if !self.session_ready || self.stopped {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_start_before_prompt").to_string()));
            self.push(Event::TurnEnded { reason: "error".into() });
            return;
        }
        if self.active.is_some() {
            return;
        }
        if let Some(budget) = self.model_budget.as_ref() {
            budget.store(MODEL_CALLS_PER_TURN, Ordering::Release);
        }
        if let Some(budget) = self.agent_budget.as_ref() {
            budget.store(AGENT_TASKS_PER_TURN, Ordering::Release);
        }
        let (Some(session), Some(document)) = (self.session.clone(), self.document.clone()) else {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_document_missing").to_string()));
            self.push(Event::TurnEnded { reason: "error".into() });
            return;
        };
        let (Ok(turn_id), Ok(workflow_run_id), Ok(workflow_id), Ok(connection_id)) =
            (self.ids.next_id(), self.ids.next_id(), Id::parse(&self.config.workflow_id), Id::parse(&self.next_connection))
        else {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
            self.push(Event::TurnEnded { reason: "error".into() });
            return;
        };
        let mut blocks = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                PromptPart::Text(text) => blocks.push(ContentBlock::Text { text: text.clone() }),
                // Runtime attachments are host artifact references (#11). Until that store
                // exists, refusing is honest; dropping one silently would change the prompt.
                PromptPart::Image { .. } | PromptPart::Document { .. } => {
                    self.push(Event::Error(crate::i18n::tr("ui.mastra_attachments_unsupported").to_string()));
                    self.push(Event::TurnEnded { reason: "error".into() });
                    return;
                }
            }
        }
        let binding = FrozenBinding {
            workflow_id: workflow_id.to_string(),
            connection_id: connection_id.to_string(),
            model_id: self.next_model.clone(),
            session_id: session.to_string(),
            turn_id: turn_id.to_string(),
            workflow_run_id: workflow_run_id.to_string(),
        };
        let scope = TurnScope { session_id: session.clone(), turn_id, workflow_run_id };
        let Ok(profile_id) = Id::parse(NATIVE_PROFILE) else {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
            self.push(Event::TurnEnded { reason: "error".into() });
            return;
        };
        let Ok(digest) = Digest::parse(EMPTY_DIGEST) else {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
            self.push(Event::TurnEnded { reason: "error".into() });
            return;
        };
        let params = TurnStartParams {
            binding: TurnBinding {
                workflow_id,
                connection_id,
                model_id: self.next_model.clone(),
                document,
                profile: ProfileRef { profile_id, version: "1.0".into(), digest },
            },
            parts: blocks,
            context: TurnContext { selection: Vec::new(), requirements: RequiredNullable(None) },
            budgets: Budgets::default(),
        };
        match self.port.request(Scope::Turn(scope.clone()), HostRequest::TurnStart(Box::new(params)), TURN_TIMEOUT) {
            Ok(_) => {
                if let Ok(mut frozen) = self.config.frozen_binding.lock() {
                    *frozen = Some((binding.connection_id.clone(), binding.model_id.clone()));
                }
                self.active = Some(ActiveTurn { binding, scope });
                self.stages.clear();
                self.plan_set = false;
            }
            Err(_) => {
                self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
                self.push(Event::TurnEnded { reason: "error".into() });
            }
        }
    }

    fn end_active(&mut self) -> Option<ActiveTurn> {
        if let Ok(mut frozen) = self.config.frozen_binding.lock() {
            *frozen = None;
        }
        self.active.take()
    }

    /// A new runtime session needs its own HostAi session; the old key is retired so a late
    /// model call for the previous conversation cannot resolve.
    fn rebind_host_session(&self, old: Option<&Id>, next: &Id) -> bool {
        let Some(host) = self.host.as_ref() else { return true };
        let instance = self.runtime_instance.as_ref().and_then(|id| id.lock().ok().and_then(|id| id.clone())).unwrap_or_default();
        if let Some(old) = old {
            let _ = host.retire_session(&SessionKey { runtime_instance_id: instance.clone(), session_id: old.to_string() });
        }
        host.open_session(SessionKey { runtime_instance_id: instance, session_id: next.to_string() }, AiLimits::contract_defaults()).is_ok()
    }

    fn map_runtime_event(&mut self, scope: &Scope, event: RuntimeEvent) {
        let Some(active) = self.active.as_ref() else { return };
        if *scope != Scope::Turn(active.scope.clone()) {
            return;
        }
        match event {
            RuntimeEvent::RuntimeReady(_) => {}
            RuntimeEvent::WorkflowStage(stage) => {
                let status = match stage.state {
                    StageState::Running => "in_progress",
                    StageState::Completed => "completed",
                    StageState::Failed => "failed",
                };
                let title = stage.title.chars().take(200).collect::<String>();
                if let Some((_, _, state)) = self.stages.iter_mut().find(|(id, _, _)| id == stage.stage_id.as_str()) {
                    *state = status.to_string();
                } else if self.stages.len() < 64 {
                    self.stages.push((stage.stage_id.to_string(), title, status.to_string()));
                }
                // A stage list is shown through the existing plan presentation; an explicit plan
                // from the workflow, once it arrives, replaces it.
                if !self.plan_set {
                    self.push(Event::Plan(self.stages.iter().map(|(_, title, state)| (title.clone(), state.clone())).collect()));
                }
            }
            RuntimeEvent::WorkflowText(text) => {
                let thought = text.channel == TextChannel::Progress;
                self.push(Event::Text { thought, text: text.text });
            }
            RuntimeEvent::WorkflowPlan(plan) => {
                self.plan_set = true;
                let steps = plan
                    .steps
                    .into_iter()
                    .take(64)
                    .map(|step| {
                        let status = match step.state {
                            StepState::Pending => "pending",
                            StepState::Running => "in_progress",
                            StepState::Completed => "completed",
                            StepState::Failed => "failed",
                        };
                        (step.title.chars().take(200).collect::<String>(), status.to_string())
                    })
                    .collect();
                self.push(Event::Plan(steps));
            }
            RuntimeEvent::WorkflowTool(tool) => {
                let status = match tool.state {
                    StageState::Running => Some(ToolStatus::InProgress),
                    StageState::Completed => Some(ToolStatus::Completed),
                    StageState::Failed => Some(ToolStatus::Failed),
                };
                let output =
                    tool.output.0.and_then(|value| serde_json::to_string(&value).ok().map(|text| text.chars().take(MAX_TOOL_OUTPUT).collect()));
                self.push(Event::Tool {
                    id: tool.call_id.to_string(),
                    title: Some(tool.name.chars().take(200).collect()),
                    kind: None,
                    status,
                    input: None,
                    output,
                });
            }
            RuntimeEvent::TurnFinished(finished) => {
                let Some(turn) = self.end_active() else { return };
                log::debug!("mastra workflow turn {} ended", turn.binding.workflow_id);
                let reason = terminal_reason(finished.state);
                if finished.state == TurnTerminal::AuthRequired {
                    // A terminal auth failure belongs to the AI connection, never to runtime
                    // readiness, so it updates the separate connection state.
                    if let Ok(mut reported) = self.config.connection_auth.lock() {
                        *reported = Some(false);
                    }
                    self.push(Event::ConnectionAuth { available: false });
                }
                if finished.state == TurnTerminal::ValidatedSuccess {
                    // Host verification (§9.3 / #12) does not exist in this release, even when a
                    // validationRunId is present, so never claim that a report passed.
                    self.push(Event::Text { thought: true, text: crate::i18n::tr("ui.mastra_validation_not_available").to_string() });
                }
                let _ = turn;
                self.push(Event::TurnEnded { reason });
            }
        }
    }

    #[cfg(test)]
    fn map_progress(&mut self, progress: AgentProgress) {
        match progress.content {
            AgentProgressContent::Text(text) => self.push(Event::Text { thought: false, text: text.text }),
            AgentProgressContent::Tool(tool) => {
                let status = match tool.state {
                    StageState::Running => Some(ToolStatus::InProgress),
                    StageState::Completed => Some(ToolStatus::Completed),
                    StageState::Failed => Some(ToolStatus::Failed),
                };
                self.push(Event::Tool { id: tool.call_id.to_string(), title: Some(tool.title), kind: None, status, input: None, output: None });
            }
            AgentProgressContent::Plan(plan) => {
                let steps = plan
                    .steps
                    .into_iter()
                    .take(64)
                    .map(|step| {
                        let status = match step.state {
                            StepState::Pending => "pending",
                            StepState::Running => "in_progress",
                            StepState::Completed => "completed",
                            StepState::Failed => "failed",
                        };
                        (step.title, status.to_string())
                    })
                    .collect();
                self.push(Event::Plan(steps));
            }
            AgentProgressContent::Permission(permission) => {
                if permission.state == PermissionState::Waiting
                    && let Some(key) = permission.permission_id.as_str().strip_prefix('p').and_then(|k| k.parse::<u64>().ok())
                {
                    // The ACP task owner maps its own option ids to the panel's two answers.
                    self.push(Event::Permission {
                        key,
                        title: permission.title,
                        kind: Some("task".into()),
                        options: vec![
                            super::acp::PermissionOption {
                                id: "allow".into(),
                                name: crate::i18n::tr("ui.agent_chat_allow").to_string(),
                                kind: "allow_once".into(),
                            },
                            super::acp::PermissionOption {
                                id: "deny".into(),
                                name: crate::i18n::tr("ui.agent_chat_deny").to_string(),
                                kind: "reject_once".into(),
                            },
                        ],
                    });
                    return;
                }
                // Approval itself belongs to the ACP task owner; a no-longer-waiting progress is
                // shown as a card, never as a live button.
                let status = match permission.state {
                    PermissionState::Waiting => Some(ToolStatus::InProgress),
                    PermissionState::Allowed | PermissionState::Denied => Some(ToolStatus::Completed),
                };
                self.push(Event::Tool {
                    id: permission.permission_id.to_string(),
                    title: Some(permission.title),
                    kind: Some("permission".into()),
                    status,
                    input: None,
                    output: None,
                });
            }
        }
    }

    fn on_notice(&mut self, notice: PortNotice) {
        match notice {
            #[cfg(test)]
            PortNotice::Progress(progress) => self.map_progress(progress),
            PortNotice::Client(notice) => self.on_client_notice(*notice),
        }
    }

    fn on_client_notice(&mut self, notice: ClientNotice) {
        match notice {
            ClientNotice::RuntimeReady(info) => {
                self.runtime_ready = true;
                if let Some(instance) = self.runtime_instance.as_ref()
                    && let Ok(mut id) = instance.lock()
                {
                    *id = Some(info.runtime_instance_id.to_string());
                }
                self.push(Event::RuntimeReady { version: info.runtime_version });
                self.start_session();
            }
            ClientNotice::RequestFinished { request_id, scope, result } => {
                if self.awaiting_session.as_ref() == Some(&request_id) {
                    self.awaiting_session = None;
                    match result {
                        Ok(_) => {
                            if let Some(next) = self.reset_target.take() {
                                let old = self.session.replace(next.clone());
                                if !self.rebind_host_session(old.as_ref(), &next) {
                                    self.session_ready = false;
                                    self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
                                    return;
                                }
                            }
                            self.session_ready = true;
                            self.emit_ready();
                        }
                        Err(error) => {
                            self.session_ready = false;
                            self.push(Event::Error(protocol_note(error.code)));
                        }
                    }
                } else if self.active.as_ref().is_some_and(|active| Scope::Turn(active.scope.clone()) == scope) && result.is_err() {
                    self.end_active();
                    self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
                    self.push(Event::TurnEnded { reason: "error".into() });
                }
            }
            ClientNotice::Event { scope, event } => self.map_runtime_event(&scope, event),
            ClientNotice::TurnFinishedProposal { scope, proposal } => {
                if self.active.as_ref().is_some_and(|active| active.scope == scope) {
                    self.map_runtime_event(&Scope::Turn(scope), RuntimeEvent::TurnFinished(proposal));
                }
            }
            ClientNotice::TurnClosed { scope, cause } => {
                if self.active.as_ref().is_some_and(|active| active.scope == scope) {
                    self.end_active();
                    self.push(Event::TurnEnded { reason: close_reason(cause).into() });
                }
            }
            ClientNotice::TransportClosed { cause, stderr_tail } => {
                self.runtime_ready = false;
                self.session_ready = false;
                self.stopped = true;
                if let Some(turn) = self.end_active() {
                    let _ = turn;
                    self.push(Event::TurnEnded { reason: "transport_error".into() });
                }
                self.push(Event::Exited { tail: stderr_tail });
                if let TransportCause::Error(error) = cause {
                    self.push(Event::Error(protocol_note(error.code)));
                }
            }
        }
    }

    fn emit_ready(&mut self) {
        if !self.models_sent {
            self.models_sent = true;
            let model = self.next_model.clone();
            self.push(Event::Models {
                current: model.clone(),
                available: vec![super::acp::ModelInfo { id: model.clone(), name: model, description: String::new() }],
            });
        }
        self.push(Event::Ready { agent: format!("Mastra · {}", self.config.workflow_id), images: false });
    }
}

fn terminal_reason(state: TurnTerminal) -> String {
    match state {
        TurnTerminal::ValidatedSuccess => String::new(),
        TurnTerminal::AuthRequired => "auth".into(),
        TurnTerminal::Cancelled => "cancelled".into(),
        TurnTerminal::BudgetExhausted => "budget_exhausted".into(),
        TurnTerminal::TransportError | TurnTerminal::OutcomeUnknown => "transport_error".into(),
        TurnTerminal::QualityFailed | TurnTerminal::TaskFailed => "error".into(),
    }
}

fn close_reason(cause: CloseReason) -> &'static str {
    match cause {
        CloseReason::User | CloseReason::Reset | CloseReason::Shutdown => "cancelled",
        CloseReason::TransportLoss => "transport_error",
        CloseReason::Deadline | CloseReason::TerminalProposal | CloseReason::HostTerminal => "error",
    }
}

fn protocol_note(code: ErrorCode) -> String {
    crate::i18n::format("ui.mastra_protocol_error", &[("code", &format!("{code:?}"))])
}

/// Host services: clock, ids, admission, lifecycle and the callback dispatcher. The fixed
/// validation profile used on the wire names what it is; it is not a claim that a report exists.
fn runtime_services(
    host: Arc<HostAi>,
    ids: Arc<dyn IdSource>,
    admission: Arc<RuntimeAdmission>,
    connection_auth: Arc<Mutex<Option<bool>>>,
    runtime_instance: Arc<Mutex<Option<String>>>,
    budget: Arc<AtomicU32>,
    agent_budget: Arc<AtomicU32>,
    task_control: Arc<Mutex<Option<Arc<TaskControl>>>>,
    frozen_binding: Arc<Mutex<Option<(String, String)>>>,
) -> RuntimeServices {
    RuntimeServices {
        clock: Arc::new(WallClock),
        ids,
        admission,
        lifecycle: Arc::new(ClosedLifecycle),
        dispatch: Arc::new(HostCalls { host, connection_auth, budget, agent_budget, task_control, runtime_instance, frozen_binding }),
    }
}

struct WallClock;
impl super::runtime_client::Clock for WallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct ClosedLifecycle;
impl ScopeLifecycle for ClosedLifecycle {
    fn close_turn(&self, _scope: &TurnScope, _reason: CloseReason) {}
    fn retire_session(&self, _session_id: &Id) {}
}

/// Runs each host call on its own bounded thread. The IPC worker never waits for a model call,
/// and no egui frame is involved.
struct HostCalls {
    host: Arc<HostAi>,
    connection_auth: Arc<Mutex<Option<bool>>>,
    budget: Arc<AtomicU32>,
    agent_budget: Arc<AtomicU32>,
    task_control: Arc<Mutex<Option<Arc<TaskControl>>>>,
    runtime_instance: Arc<Mutex<Option<String>>>,
    frozen_binding: Arc<Mutex<Option<(String, String)>>>,
}

impl HostDispatch for HostCalls {
    fn dispatch(&self, call: CallbackCall, reply: ReplyPort) {
        let host = Arc::clone(&self.host);
        let auth = Arc::clone(&self.connection_auth);
        let budget = Arc::clone(&self.budget);
        let agent_budget = Arc::clone(&self.agent_budget);
        let task_control = Arc::clone(&self.task_control);
        let instance = Arc::clone(&self.runtime_instance);
        let frozen = Arc::clone(&self.frozen_binding);
        let spawned = std::thread::Builder::new().name("mastra-host-call".into()).spawn(move || {
            let cancelled = call.cancellation.is_cancelled();
            let result = if cancelled {
                Err(cancelled_error("operation"))
            } else {
                handle_call(&host, &auth, &budget, &agent_budget, &task_control, &instance, &frozen, call, &reply)
            };
            let _ = reply.finish(result);
        });
        // On a failed spawn, dropping the reply port resolves the call with the machine's
        // cancellation completion.
        let _ = spawned;
    }
}

fn handle_call(
    host: &HostAi,
    auth: &Mutex<Option<bool>>,
    budget: &AtomicU32,
    agent_budget: &AtomicU32,
    task_control: &Mutex<Option<Arc<TaskControl>>>,
    runtime_instance: &Mutex<Option<String>>,
    frozen_binding: &Mutex<Option<(String, String)>>,
    call: CallbackCall,
    reply: &ReplyPort,
) -> Result<HostReply, WireError> {
    let CallbackCall { scope, call, cancellation, deadline, limits, token: _token } = call;
    let frozen = frozen_binding.lock().ok().and_then(|binding| binding.clone());
    match call {
        HostCall::ConnectionDescribe(params) => {
            reject_retarget(&frozen, &params.connection_id.to_string(), &params.model_id)?;
            let connection = params.connection_id.to_string();
            let description = host.describe_agent(&connection, &params.model_id).map_err(|error| ai_error(&connection, &error))?;
            if let Ok(mut reported) = auth.lock() {
                *reported = Some(description.state == host_ai::ConnectionState::Available);
            }
            let capabilities = description.capabilities;
            Ok(HostReply::ConnectionDescribe(ConnectionDescribeResult {
                state: match description.state {
                    host_ai::ConnectionState::Available => ConnectionAvailability::Available,
                    host_ai::ConnectionState::AuthRequired => ConnectionAvailability::AuthRequired,
                    host_ai::ConnectionState::Unavailable => ConnectionAvailability::Unavailable,
                },
                capabilities: AiCapabilities {
                    model_generate: capabilities.model_generate,
                    agent_run: capabilities.agent_run,
                    tools: capabilities.tools,
                    images: capabilities.images,
                    documents: capabilities.documents,
                    structured_output: capabilities.structured_output,
                    model_selection: capabilities.model_selection,
                    usage: capabilities.usage,
                    delivery: super::mastra_protocol::Delivery::Buffered,
                },
                auth_methods: description
                    .auth_methods
                    .into_iter()
                    .filter_map(|method| Some(AuthMethod { id: Id::parse(&method.id).ok()?, name: method.name, description: method.description }))
                    .collect(),
            }))
        }
        HostCall::ModelGenerate(params) => {
            let ModelGenerateParams { connection_id, model_id, messages, tools, options, continuation } = params;
            reject_retarget(&frozen, &connection_id.to_string(), &model_id)?;
            if budget.try_update(Ordering::AcqRel, Ordering::Acquire, |left| left.checked_sub(1)).is_err() {
                return Err(resource_exhausted("model_calls", MODEL_CALLS_PER_TURN));
            }
            let connection = connection_id.to_string();
            let binding = host.bind(&connection, &model_id).map_err(|error| ai_error(&connection, &error))?;
            let messages = host_messages(&messages)?;
            let tools = tools
                .into_iter()
                .map(|tool| host_ai::ToolDefinition { name: tool.name, description: tool.description, input_schema: tool.input_schema })
                .collect();
            let Some(session) = session_from_scope(&scope, runtime_instance) else {
                return Err(invalid_error("scope"));
            };
            if options.delivery == super::mastra_protocol::Delivery::Streaming {
                return Err(unsupported("streaming"));
            }
            let host_limits = ai_limits(&limits)?;
            let timeout_seconds = options.timeout_seconds.clamp(1, limits.call_max_seconds.max(1)).min(600);
            let request = GenerateRequest {
                connection_id: connection.clone(),
                model_id,
                messages,
                tools,
                options: GenerateOptions {
                    delivery: Delivery::Buffered,
                    timeout_seconds,
                    max_output_tokens: options.max_output_tokens.0,
                    temperature: options.temperature.0,
                    reasoning_effort: options.reasoning_effort.0,
                    output_schema: options.output_schema.0,
                },
                continuation: match continuation.0 {
                    Some(id) => Some(ContinuationId::parse(id.as_str()).map_err(|_| invalid_error("continuation"))?),
                    None => None,
                },
            };
            let control = TurnControl::new().new_call().map_err(|error| ai_error(&connection, &error))?;
            let context = CallContext {
                scope: CallScope { session, turn_id: turn_id(&scope), workflow_run_id: workflow_run_id(&scope) },
                control: Arc::clone(&control),
                deadline: host.deadline_after(deadline.saturating_duration_since(Instant::now())),
                limits: host_limits,
                model_budget: Arc::new(ModelCallBudget::new(MODEL_CALLS_PER_TURN).map_err(|error| ai_error(&connection, &error))?),
            };
            let cancelled = cancellation.clone();
            let result = host.generate(&binding, &context, request);
            if cancelled.is_cancelled() {
                control.cancel();
                return Err(cancelled_error("model.generate"));
            }
            match result {
                Ok(reply) => {
                    let mut tool_calls = Vec::with_capacity(reply.tool_calls.len());
                    for tool in reply.tool_calls {
                        tool_calls.push(ToolCall {
                            id: Id::parse(&tool.id).map_err(|_| invalid_error("tool_call_id"))?,
                            name: tool.name,
                            arguments: tool.arguments,
                        });
                    }
                    let continuation = match reply.continuation {
                        Some(id) => Some(Id::parse(id.as_str()).map_err(|_| invalid_error("continuation"))?),
                        None => None,
                    };
                    Ok(HostReply::ModelGenerate(ModelGenerateResult {
                        content: reply.text.into_iter().map(|text| ContentBlock::Text { text }).collect(),
                        tool_calls,
                        finish_reason: match reply.finish_reason {
                            FinishReason::Stop => FinishReasonWire::Stop,
                            FinishReason::ToolCalls => FinishReasonWire::ToolCalls,
                            FinishReason::Length => FinishReasonWire::Length,
                            FinishReason::Refusal => FinishReasonWire::Refusal,
                            FinishReason::Unknown => FinishReasonWire::Unknown,
                        },
                        usage: RequiredNullable(reply.usage.map(|usage| Usage {
                            input_tokens: RequiredNullable(usage.input_tokens),
                            output_tokens: RequiredNullable(usage.output_tokens),
                            total_tokens: RequiredNullable(usage.total_tokens),
                        })),
                        continuation: RequiredNullable(continuation),
                        delivery: super::mastra_protocol::Delivery::Buffered,
                    }))
                }
                Err(error) => Err(ai_error(&connection, &error)),
            }
        }
        HostCall::AgentRun(params) => {
            let AgentRunParams { connection_id, model_id, task, context, timeout_seconds } = params;
            reject_retarget(&frozen, &connection_id.to_string(), &model_id)?;
            if context.delegation_mode != DelegationMode::ProposalOnly {
                return Err(unsupported("delegation_mode"));
            }
            if agent_budget.try_update(Ordering::AcqRel, Ordering::Acquire, |left| left.checked_sub(1)).is_err() {
                return Err(resource_exhausted("agent_tasks", AGENT_TASKS_PER_TURN));
            }
            let connection = connection_id.to_string();
            let binding = host.bind_acp(&connection).map_err(|error| ai_error(&connection, &error))?;
            let mut text = String::new();
            for block in task {
                match block {
                    ContentBlock::Text { text: part } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        let room = (256usize << 10).saturating_sub(text.len());
                        text.extend(part.chars().take(room));
                    }
                    ContentBlock::Image { .. } | ContentBlock::Document { .. } => return Err(unsupported("attachments")),
                }
            }
            let timeout = Duration::from_secs(u64::from(timeout_seconds.clamp(1, 600)));
            let request = TaskRequest {
                text,
                model: if model_id.is_empty() { None } else { Some(model_id) },
                source: TaskStamp { document_id: context.document.document_id.as_str().to_string(), revision: context.document.revision.as_u64() },
                timeout,
                remaining_commands: 64,
                allowed_commands: None,
            };
            let control = Arc::new(TaskControl::default());
            if let Ok(mut slot) = task_control.lock() {
                *slot = Some(Arc::clone(&control));
            }
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let watch_control = Arc::clone(&control);
            let watch_done = Arc::clone(&done);
            let watch_cancel = cancellation.clone();
            let _watch = std::thread::Builder::new().name("mastra-task-cancel".into()).spawn(move || {
                while !watch_done.load(Ordering::Acquire) {
                    if watch_cancel.is_cancelled() {
                        watch_control.cancel();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            let mut emit = |progress: TaskProgress| {
                let event = match progress {
                    TaskProgress::Text { session, text } => AgentProgress {
                        agent_session_id: Id::parse(&session).unwrap_or_else(|_| Id::literal("session")),
                        content: AgentProgressContent::Text(ProgressText { text }),
                    },
                    TaskProgress::Tool { session, call_id, title, state } => AgentProgress {
                        agent_session_id: Id::parse(&session).unwrap_or_else(|_| Id::literal("session")),
                        content: AgentProgressContent::Tool(ToolProgress {
                            call_id: Id::parse(&call_id).unwrap_or_else(|_| Id::literal("tool")),
                            title,
                            state: match state {
                                TaskToolState::Running => StageState::Running,
                                TaskToolState::Completed => StageState::Completed,
                                TaskToolState::Failed => StageState::Failed,
                            },
                        }),
                    },
                    TaskProgress::Plan { session, steps } => AgentProgress {
                        agent_session_id: Id::parse(&session).unwrap_or_else(|_| Id::literal("session")),
                        content: AgentProgressContent::Plan(WorkflowPlan {
                            steps: steps
                                .into_iter()
                                .take(64)
                                .enumerate()
                                .filter_map(|(index, (title, state))| {
                                    Some(PlanStep {
                                        id: Id::parse(&format!("s{}", index + 1)).ok()?,
                                        title,
                                        state: match state.as_str() {
                                            "completed" => StepState::Completed,
                                            "failed" => StepState::Failed,
                                            "in_progress" | "running" => StepState::Running,
                                            _ => StepState::Pending,
                                        },
                                    })
                                })
                                .collect(),
                        }),
                    },
                    TaskProgress::Permission { session, key, title, state } => AgentProgress {
                        agent_session_id: Id::parse(&session).unwrap_or_else(|_| Id::literal("session")),
                        content: AgentProgressContent::Permission(PermissionProgress {
                            permission_id: Id::parse(&format!("p{key}")).unwrap_or_else(|_| Id::literal("permission")),
                            title,
                            state: match state {
                                TaskPermissionState::Waiting => PermissionState::Waiting,
                                TaskPermissionState::Allowed => PermissionState::Allowed,
                                TaskPermissionState::Denied => PermissionState::Denied,
                            },
                        }),
                    },
                };
                let _ = reply.emit(CallProgress::AgentProgress(event));
            };
            let result = host.agent_run(&binding, request, &control, &mut emit);
            done.store(true, Ordering::Release);
            if let Ok(mut slot) = task_control.lock() {
                *slot = None;
            }
            if cancellation.is_cancelled() || control.is_cancelled() {
                return Err(cancelled_error("agent.run"));
            }
            match result {
                Ok(outcome) => Ok(HostReply::AgentRun(AgentRunResult {
                    agent_session_id: Id::parse(&outcome.agent_session_id).map_err(|_| invalid_error("agent_session_id"))?,
                    state: CompletedState::Completed,
                    content: vec![ContentBlock::Text { text: outcome.text }],
                    stop_reason: outcome.stop_reason,
                    usage: RequiredNullable(None),
                })),
                Err(error) => Err(ai_error(&connection, &error)),
            }
        }
        HostCall::ValidationSubmit(_) | HostCall::ValidationStatus(_) | HostCall::ValidationReport(_) | HostCall::ValidationCancel(_) => {
            Err(unsupported("validation"))
        }
    }
}

fn host_messages(messages: &[ModelMessageWire]) -> Result<Vec<ModelMessage>, WireError> {
    messages
        .iter()
        .map(|message| {
            let content = message
                .content
                .iter()
                .map(|block| match block {
                    ContentBlock::Text { text } => Ok(host_ai::ContentPart::Text(text.clone())),
                    // The runtime sends host artifact references; the direct host has no artifact
                    // store yet (#11), so this fails closed instead of inventing bytes.
                    ContentBlock::Image { .. } | ContentBlock::Document { .. } => Err(unsupported("artifacts")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ModelMessage {
                role: match message.role {
                    Role::System => MessageRole::System,
                    Role::User => MessageRole::User,
                    Role::Assistant => MessageRole::Assistant,
                    Role::Tool => MessageRole::Tool,
                },
                content,
                tool_calls: message
                    .tool_calls
                    .iter()
                    .map(|call| host_ai::ToolCall { id: call.id.to_string(), name: call.name.clone(), arguments: call.arguments.clone() })
                    .collect(),
                tool_call_id: message.tool_call_id.0.as_ref().map(ToString::to_string),
            })
        })
        .collect()
}

fn session_from_scope(scope: &Scope, runtime_instance: &Mutex<Option<String>>) -> Option<SessionKey> {
    let session_id = match scope {
        Scope::Session(id) => id,
        Scope::Turn(turn) => &turn.session_id,
        Scope::Runtime => return None,
    };
    let runtime_instance_id = runtime_instance.lock().ok().and_then(|id| id.clone()).unwrap_or_default();
    Some(SessionKey { runtime_instance_id, session_id: session_id.to_string() })
}

fn turn_id(scope: &Scope) -> String {
    match scope {
        Scope::Turn(turn) => turn.turn_id.to_string(),
        _ => String::new(),
    }
}

fn workflow_run_id(scope: &Scope) -> String {
    match scope {
        Scope::Turn(turn) => turn.workflow_run_id.to_string(),
        _ => String::new(),
    }
}

/// The host pins the turn's connection and model. A runtime call naming another pair is a
/// retarget attempt, never a picker change: the picker only affects the next turn.
fn reject_retarget(frozen: &Option<(String, String)>, connection: &str, model: &str) -> Result<(), WireError> {
    if let Some((pinned_connection, pinned_model)) = frozen
        && (pinned_connection != connection || pinned_model != model)
    {
        return Err(connection_changed(pinned_connection));
    }
    Ok(())
}

fn connection_changed(connection_id: &str) -> WireError {
    let details = match Id::parse(connection_id) {
        Ok(id) => ErrorDetails::Connection(ConnectionDetails { connection_id: id }),
        Err(_) => ErrorDetails::Empty(EmptyDetails {}),
    };
    WireError {
        code: ErrorCode::ConnectionChanged,
        message: "The frozen AI connection or model does not match this call.".into(),
        retry: Retry::Never,
        details,
    }
}

fn resource_exhausted(resource: &str, limit: u32) -> WireError {
    WireError {
        code: ErrorCode::ResourceExhausted,
        message: "The turn's budget is exhausted.".into(),
        retry: Retry::Never,
        details: ErrorDetails::Resource(ResourceDetails { resource: resource.into(), limit }),
    }
}

/// The runtime negotiates lower limits in hello; a host model call must not widen them again to
/// the contract defaults.
fn ai_limits(limits: &Limits) -> Result<AiLimits, WireError> {
    let limits = AiLimits {
        model_messages: limits.model_messages,
        tool_definitions: limits.tool_definitions,
        tool_calls: limits.tool_calls,
        text_bytes: limits.text_bytes,
        tool_json_bytes: limits.tool_json_bytes,
        attachments: limits.attachments,
        attachment_bytes: limits.attachment_bytes,
        attachment_total_bytes: limits.attachment_total_bytes,
        continuation_handles: limits.continuation_handles,
        continuation_bytes: limits.continuation_bytes,
        continuation_ttl_seconds: limits.continuation_ttl_seconds,
        http_per_connection: limits.http_per_connection,
        http_per_host: limits.http_per_host,
        call_max_seconds: limits.call_max_seconds,
    };
    limits.validate().map_err(|_| invalid_error("limits"))?;
    Ok(limits)
}

fn unsupported(capability: &str) -> WireError {
    WireError {
        code: ErrorCode::UnsupportedCapability,
        message: "This host capability is not available yet.".into(),
        retry: Retry::Never,
        details: ErrorDetails::Capability(CapabilityDetails { capability: capability.into() }),
    }
}

fn invalid_error(field: &str) -> WireError {
    WireError {
        code: ErrorCode::InvalidRequest,
        message: "The runtime request is not valid.".into(),
        retry: Retry::Never,
        details: ErrorDetails::FieldMethod(FieldMethodDetails { field: Some(field.into()), method: None }),
    }
}

fn cancelled_error(operation: &str) -> WireError {
    WireError {
        code: ErrorCode::Cancelled,
        message: "The call was cancelled.".into(),
        retry: Retry::Never,
        details: ErrorDetails::Operation(OperationDetails { operation: Some(operation.into()) }),
    }
}

fn ai_error(connection_id: &str, error: &AiError) -> WireError {
    let (code, details) = match error.code() {
        AiErrorCode::InvalidRequest => (ErrorCode::InvalidRequest, ErrorDetails::Empty(EmptyDetails {})),
        AiErrorCode::UnsupportedCapability => {
            (ErrorCode::UnsupportedCapability, ErrorDetails::Capability(CapabilityDetails { capability: "model_generate".into() }))
        }
        AiErrorCode::AuthRequired | AiErrorCode::ConnectionChanged => {
            let connection = match error.details() {
                AiErrorDetails::Connection { connection_id, .. } => connection_id.clone(),
                _ => connection_id.to_owned(),
            };
            let details = match Id::parse(&connection) {
                Ok(id) => ErrorDetails::Connection(ConnectionDetails { connection_id: id }),
                Err(_) => ErrorDetails::Empty(EmptyDetails {}),
            };
            (if error.code() == AiErrorCode::AuthRequired { ErrorCode::AuthRequired } else { ErrorCode::ConnectionChanged }, details)
        }
        AiErrorCode::ModelError | AiErrorCode::MalformedResponse => {
            let (connection, status) = match error.details() {
                AiErrorDetails::Connection { connection_id, http_status } => (connection_id.clone(), http_status.map(u32::from)),
                _ => (connection_id.to_owned(), None),
            };
            let details = match Id::parse(&connection) {
                Ok(id) => ErrorDetails::Model(ModelErrorDetails { connection_id: id, http_status: status }),
                Err(_) => ErrorDetails::Empty(EmptyDetails {}),
            };
            (if error.code() == AiErrorCode::ModelError { ErrorCode::ModelError } else { ErrorCode::MalformedResponse }, details)
        }
        AiErrorCode::PermissionDenied => {
            let permission = match error.details() {
                AiErrorDetails::Permission { permission_id } => permission_id.clone(),
                _ => "permission".into(),
            };
            let details = match Id::parse(&permission) {
                Ok(id) => ErrorDetails::Permission(PermissionDetails { permission_id: id }),
                Err(_) => ErrorDetails::Empty(EmptyDetails {}),
            };
            (ErrorCode::PermissionDenied, details)
        }
        AiErrorCode::InvalidProposal | AiErrorCode::TaskFailed => {
            let details = match Id::parse(connection_id) {
                Ok(id) => ErrorDetails::Connection(ConnectionDetails { connection_id: id }),
                Err(_) => ErrorDetails::Empty(EmptyDetails {}),
            };
            (if error.code() == AiErrorCode::InvalidProposal { ErrorCode::InvalidProposal } else { ErrorCode::TaskFailed }, details)
        }
        AiErrorCode::ResourceExhausted => {
            let (resource, limit) = match error.details() {
                AiErrorDetails::Resource { resource, limit } => ((*resource).to_owned(), *limit),
                _ => ("resource".into(), 0),
            };
            (ErrorCode::ResourceExhausted, ErrorDetails::Resource(ResourceDetails { resource, limit }))
        }
        AiErrorCode::Timeout => (ErrorCode::Timeout, ErrorDetails::Operation(OperationDetails { operation: Some("model.generate".into()) })),
        AiErrorCode::Cancelled => (ErrorCode::Cancelled, ErrorDetails::Operation(OperationDetails { operation: Some("model.generate".into()) })),
        AiErrorCode::TransportError => {
            (ErrorCode::TransportError, ErrorDetails::Operation(OperationDetails { operation: Some("model.generate".into()) }))
        }
        AiErrorCode::InternalError => (ErrorCode::InternalError, ErrorDetails::Empty(EmptyDetails {})),
    };
    WireError { code, message: error.message().chars().take(2048).collect(), retry: Retry::Never, details }
}

// The wire and host modules both use the name `FinishReason`; alias the wire one where the host
// own type is already in scope.
use super::mastra_protocol::FinishReason as FinishReasonWire;
use super::mastra_protocol::ModelMessage as ModelMessageWire;

impl ChatBackend for MastraBackend {
    fn poll(&mut self) -> Vec<Event> {
        for notice in self.port.drain() {
            self.on_notice(notice);
        }
        let reported = self.config.connection_auth.lock().ok().and_then(|mut reported| reported.take());
        if let Some(available) = reported {
            self.push(Event::ConnectionAuth { available });
        }
        self.events.drain(..).collect()
    }

    fn prompt(&mut self, parts: &[PromptPart]) {
        self.begin_turn(parts);
    }

    fn cancel(&mut self) {
        if let Ok(slot) = self.task_control.lock()
            && let Some(control) = slot.as_ref()
        {
            control.cancel();
        }
        let Some(turn) = self.end_active() else { return };
        let _ = self.port.cancel_turn(&turn.scope, CancelReason::User);
        // The host owns the terminal transition: a runtime that never answers cannot leave the
        // panel busy, and a late turn.finished is ignored by the empty active slot.
        self.push(Event::TurnEnded { reason: "cancelled".into() });
    }

    fn new_session(&mut self) {
        self.cancel();
        if !self.runtime_ready || self.stopped {
            return;
        }
        if !self.session_ready {
            if self.awaiting_session.is_none() {
                self.start_session();
            }
            return;
        }
        let Ok(next) = self.ids.next_id() else {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string()));
            return;
        };
        let params = SessionResetParams { next_session_id: next.clone(), instructions: self.config.instructions.chars().take(4000).collect() };
        let scope = self.session.clone().map(Scope::Session).unwrap_or(Scope::Runtime);
        match self.port.request(scope, HostRequest::SessionReset(params), SESSION_TIMEOUT) {
            Ok(request) => {
                self.awaiting_session = Some(request);
                self.reset_target = Some(next);
                self.session_ready = false;
                self.stages.clear();
                self.plan_set = false;
                self.models_sent = false;
            }
            Err(_) => self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_failed").to_string())),
        }
    }

    fn set_instructions(&mut self, text: &str) {
        self.config.instructions = text.to_owned();
    }

    fn set_document(&mut self, document_id: &str, revision: u64) {
        self.document = document_stamp_from_native(document_id, revision).ok();
    }

    fn answer_permission(&mut self, key: u64, option: Option<&str>) {
        if let Ok(slot) = self.task_control.lock()
            && let Some(control) = slot.as_ref()
        {
            let _ = control.answer(key, option);
        }
    }

    fn authenticate(&mut self, method: &str) {
        // The selected connection owns its sign-in. ACP tasks forward the agent's own methods and
        // invoke its authenticate request; no key is stored here.
        let Some(host) = self.host.as_ref().map(Arc::clone) else { return };
        let connection = self.config.connection_id.clone();
        let method = method.to_string();
        let auth = Arc::clone(&self.config.connection_auth);
        let wake = Arc::clone(&self.config.wake);
        let _ = std::thread::Builder::new().name("mastra-agent-auth".into()).spawn(move || {
            let available = host.authenticate_agent(&connection, &method).is_ok_and(|d| d.state == super::host_ai::ConnectionState::Available);
            if let Ok(mut reported) = auth.lock() {
                *reported = Some(available);
            }
            wake();
        });
    }

    fn set_model(&mut self, model: &str) {
        if !model.is_empty() && !model.chars().any(char::is_control) && model.chars().count() <= super::MODEL_ID_CAP {
            self.next_model = model.to_owned();
        }
    }

    fn shutdown(&mut self, wait: bool) {
        self.stopped = true;
        let _ = self.port.shutdown();
        if wait {
            self.port.wait_stopped(SHUTDOWN_WAIT);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::super::runtime_client::RuntimeInfo;
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// One recorded backend request, with the exact protocol scope and method.
    #[derive(Clone, Debug)]
    pub struct Recorded {
        pub scope: Scope,
        pub request: HostRequest,
        pub id: RequestId,
    }

    #[derive(Default)]
    pub struct Log {
        pub requests: Vec<Recorded>,
        pub notices: VecDeque<PortNotice>,
        pub cancellations: Vec<(TurnScope, CancelReason)>,
        pub shutdowns: usize,
        next: u32,
    }

    impl Log {
        pub fn turn_start(&self) -> Option<&TurnStartParams> {
            self.requests.iter().find_map(|r| match &r.request {
                HostRequest::TurnStart(params) => Some(params.as_ref()),
                _ => None,
            })
        }
        pub fn inject_client(&mut self, notice: ClientNotice) {
            self.notices.push_back(PortNotice::client(notice));
        }
        pub fn inject(&mut self, notice: PortNotice) {
            self.notices.push_back(notice);
        }
    }

    pub struct TestPort {
        log: Arc<Mutex<Log>>,
    }

    impl TestPort {
        pub fn new(log: Arc<Mutex<Log>>) -> Self {
            Self { log }
        }
    }

    impl Port for TestPort {
        fn request(&mut self, scope: Scope, request: HostRequest, _timeout: Duration) -> Result<RequestId, ClientError> {
            let mut log = self.log.lock().map_err(|_| ClientError { code: ErrorCode::InternalError })?;
            log.next += 1;
            let id = RequestId::parse(&format!("h:test{}", log.next), Sender::Host).map_err(|_| ClientError { code: ErrorCode::InternalError })?;
            log.requests.push(Recorded { scope, request, id: id.clone() });
            Ok(id)
        }

        fn cancel_turn(&mut self, scope: &TurnScope, reason: CancelReason) -> Result<(), ClientError> {
            self.log.lock().map_err(|_| ClientError { code: ErrorCode::InternalError })?.cancellations.push((scope.clone(), reason));
            Ok(())
        }

        fn shutdown(&mut self) -> Result<(), ClientError> {
            self.log.lock().map_err(|_| ClientError { code: ErrorCode::InternalError })?.shutdowns += 1;
            Ok(())
        }

        fn drain(&mut self) -> Vec<PortNotice> {
            self.log.lock().map(|mut log| log.notices.drain(..).collect()).unwrap_or_default()
        }
    }

    pub fn runtime_info(version: &str) -> RuntimeInfo {
        RuntimeInfo {
            selected_version: "1.0".into(),
            runtime_instance_id: Id::literal("instance"),
            runtime_version: version.into(),
            capabilities: RuntimeCapabilities {
                workflow_ids: Vec::new(),
                host_model_calls: true,
                host_agent_tasks: false,
                guarded_execute: false,
                guarded_batch: false,
                snapshot_validation: false,
            },
            limits: Limits::default(),
        }
    }
}
