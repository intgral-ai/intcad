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

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;

use solidcraft_engine::validation as validation_port;

use super::acp::{AuthMethod, McpBridge};
use super::backend::{ChatBackend, Event, PromptPart, ToolStatus};
use super::host_ai::{
    self, AiError, AiErrorCode, AiErrorDetails, AiLimits, CallContext, CallScope, ContinuationId, Delivery, FinishReason, GenerateOptions,
    GenerateRequest, HostAi, MessageRole, ModelCallBudget, ModelMessage, SessionKey, TaskControl, TaskPermissionState, TaskProgress, TaskRequest,
    TaskStamp, TaskToolState, TurnControl,
};
use super::host_bridge::{HostBridge, capability_argument};
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
/// Contract §4: four composer attachments per turn, 4 MiB each, 16 MiB total.
const MAX_PROMPT_ATTACHMENTS: usize = 4;
const MAX_PROMPT_ATTACHMENT_BYTES: usize = 4 << 20;
const MAX_PROMPT_ATTACHMENT_TOTAL_BYTES: usize = 16 << 20;

/// One in-flight ACP task. `agent.run` is serialized per window so two runtime calls cannot
/// replace each other's permission channel. A guard clears only the control it installed.
#[derive(Default)]
pub(crate) struct TaskControlSlot {
    serial: Mutex<()>,
    current: Mutex<Option<Arc<TaskControl>>>,
}

pub(crate) struct TaskControlGuard<'a> {
    slot: &'a TaskControlSlot,
    control: Arc<TaskControl>,
    _serial: std::sync::MutexGuard<'a, ()>,
}

impl TaskControlSlot {
    pub(crate) fn acquire(&self, control: Arc<TaskControl>) -> Result<TaskControlGuard<'_>, AiError> {
        let serial = self.serial.lock().map_err(|_| AiError::internal("acp_task_serial"))?;
        *self.current.lock().map_err(|_| AiError::internal("acp_task_slot"))? = Some(Arc::clone(&control));
        Ok(TaskControlGuard { slot: self, control, _serial: serial })
    }

    pub(crate) fn answer(&self, key: u64, option: Option<&str>) -> Result<(), AiError> {
        let current = self.current.lock().map_err(|_| AiError::internal("acp_task_slot"))?.clone();
        match current {
            Some(control) => control.answer(key, option),
            None => Err(AiError::invalid("permission")),
        }
    }

    /// Whether the task currently holding the slot issued this permission key.
    pub(crate) fn issued(&self, key: u64) -> bool {
        self.current.lock().ok().and_then(|current| current.as_ref().map(|control| control.issued(key))).unwrap_or(false)
    }

    pub(crate) fn cancel(&self) {
        if let Ok(current) = self.current.lock()
            && let Some(control) = current.as_ref()
        {
            control.cancel();
        }
    }

    fn clear(&self, control: &Arc<TaskControl>) {
        if let Ok(mut current) = self.current.lock()
            && current.as_ref().is_some_and(|current| Arc::ptr_eq(current, control))
        {
            *current = None;
        }
    }
}

impl Drop for TaskControlGuard<'_> {
    fn drop(&mut self) {
        self.slot.clear(&self.control);
    }
}

/// Most ACP task progress items kept for the next frame. The queue is bounded so a talkative
/// agent cannot grow memory without limit; the oldest item is dropped first.
const MAX_PANEL_PROGRESS: usize = 512;

/// Host-worker to panel hand-off for ACP task progress. `TaskProgress` is the bounded ACP
/// information the task executor emits; this queue never carries a raw ACP object.
pub(crate) struct ProgressQueue {
    queue: Mutex<VecDeque<AgentProgress>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl ProgressQueue {
    pub(crate) fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self { queue: Mutex::new(VecDeque::new()), wake })
    }

    /// Convert and queue one task update, and return the wire event for the runtime.
    pub(crate) fn publish(&self, progress: TaskProgress) -> AgentProgress {
        let event = task_progress_event(progress);
        if let Ok(mut queue) = self.queue.lock() {
            if queue.len() >= MAX_PANEL_PROGRESS {
                queue.pop_front();
            }
            queue.push_back(event.clone());
        }
        (self.wake)();
        event
    }

    fn drain(&self) -> Vec<AgentProgress> {
        self.queue.lock().map(|mut queue| queue.drain(..).collect()).unwrap_or_default()
    }
}

/// What a connection probe found, kept apart from runtime readiness. `acp` distinguishes an
/// ACP agent (whose own sign-in methods the panel can show) from an endpoint connection.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConnectionAuthReport {
    pub available: bool,
    pub acp: bool,
    pub methods: Vec<AuthMethod>,
}

impl ConnectionAuthReport {
    fn from_description(acp: bool, description: host_ai::AgentDescription) -> Self {
        Self { available: description.state == host_ai::ConnectionState::Available, acp, methods: description.auth_methods }
    }
}

/// The bounded ACP task progress shapes mapped onto the panel's existing text, tool, plan and
/// permission events. No arbitrary ACP object crosses this boundary.
fn task_progress_event(progress: TaskProgress) -> AgentProgress {
    let session = match &progress {
        TaskProgress::Text { session, .. }
        | TaskProgress::Tool { session, .. }
        | TaskProgress::Plan { session, .. }
        | TaskProgress::Permission { session, .. } => session.clone(),
    };
    let agent_session_id = Id::parse(&session).unwrap_or_else(|_| Id::literal("session"));
    let content = match progress {
        TaskProgress::Text { text, .. } => AgentProgressContent::Text(ProgressText { text }),
        TaskProgress::Tool { call_id, title, state, .. } => AgentProgressContent::Tool(ToolProgress {
            call_id: Id::parse(&call_id).unwrap_or_else(|_| Id::literal("tool")),
            title,
            state: match state {
                TaskToolState::Running => StageState::Running,
                TaskToolState::Completed => StageState::Completed,
                TaskToolState::Failed => StageState::Failed,
            },
        }),
        TaskProgress::Plan { steps, .. } => AgentProgressContent::Plan(WorkflowPlan {
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
        TaskProgress::Permission { key, title, state, .. } => AgentProgressContent::Permission(PermissionProgress {
            permission_id: Id::parse(&format!("p{key}")).unwrap_or_else(|_| Id::literal("permission")),
            title,
            state: match state {
                TaskPermissionState::Waiting => PermissionState::Waiting,
                TaskPermissionState::Allowed => PermissionState::Allowed,
                TaskPermissionState::Denied => PermissionState::Denied,
            },
        }),
    };
    AgentProgress { agent_session_id, content }
}

/// The host-owned local validation service. It routes the runtime's `validation.*` calls to the
/// Rust `ValidationPort`, caches the exact report bytes reference it returned, and owns the final
/// §9.3 acceptance decision. The runtime never supplies a report body or a verdict.
pub(crate) struct HostValidation {
    port: Arc<validation_port::ValidationPort>,
    profile: validation_port::ProfileRef,
    profile_checks: Vec<validation_port::CheckRequirement>,
    runner: validation_port::RunnerIdentity,
    book: Mutex<ValidationBook>,
    worker_stop: Arc<AtomicBool>,
}

#[derive(Default)]
struct ValidationBook {
    runs: HashMap<String, ValidationRunRecord>,
    run_order: VecDeque<String>,
    reports: HashMap<String, CachedValidationReport>,
    report_order: VecDeque<String>,
}

#[derive(Clone)]
struct ValidationRunRecord {
    run_id: String,
    session_id: String,
    turn_id: String,
    workflow_run_id: String,
    candidate: solidcraft_engine::schema::snapshot::CandidateRef,
    profile: validation_port::ProfileRef,
    state: validation_port::ValidationState,
}

#[derive(Clone)]
struct CachedValidationReport {
    reference: validation_port::ArtifactRef,
    state: validation_port::ValidationState,
    verdict: Result<(), String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidationIdentity {
    pub run_id: String,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub manifest_digest: String,
    pub source_document_id: String,
    pub source_revision: String,
    pub profile_id: String,
    pub profile_version: String,
    pub profile_digest: String,
    /// The real validation state. Never replaced by a presentation verdict.
    pub state: String,
    /// `accepted`, `rejected`, `history` or `cancelled` — distinct from the state.
    pub outcome: String,
    pub report_artifact_id: String,
    pub report_digest: String,
    pub live_document_id: Option<String>,
    pub live_revision: Option<String>,
    pub live_changed: bool,
    pub historical: bool,
}

impl ValidationIdentity {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "stage": "validate",
            "runId": self.run_id,
            "candidateId": self.candidate_id,
            "candidateDigest": self.candidate_digest,
            "manifestDigest": self.manifest_digest,
            "snapshot": {"documentId": self.source_document_id, "revision": self.source_revision},
            "profile": {"profileId": self.profile_id, "version": self.profile_version, "digest": self.profile_digest},
            "validationState": self.state,
            "outcome": self.outcome,
            "report": {"artifactId": self.report_artifact_id, "digest": self.report_digest},
            "liveDocument": self.live_document_id.as_ref().zip(self.live_revision.as_ref()).map(|(document, revision)| serde_json::json!({"documentId": document, "revision": revision})),
            "liveChanged": self.live_changed,
            "historical": self.historical,
        })
    }
}

pub(crate) struct ValidationAcceptance {
    pub accepted: bool,
    pub historical: bool,
    pub reason: String,
    pub identity: ValidationIdentity,
}

impl HostValidation {
    /// Open the local port and start its single worker loop. The store and candidate directories
    /// are the same ones `document.snapshot.capture` writes to.
    pub(crate) fn open(store: PathBuf, candidate_root: PathBuf) -> Result<Self, String> {
        let port = Arc::new(validation_port::ValidationPort::open(store, candidate_root).map_err(|error| format!("{error}"))?);
        let native = validation_port::native_basic_profile();
        let profile = validation_port::profile_ref(&native).map_err(|error| format!("{error}"))?;
        let host = HostValidation {
            port: Arc::clone(&port),
            profile,
            profile_checks: native.checks,
            runner: validation_port::local_runner_identity(),
            book: Mutex::new(ValidationBook::default()),
            worker_stop: Arc::new(AtomicBool::new(false)),
        };
        let stop = Arc::clone(&host.worker_stop);
        let _ = std::thread::Builder::new().name("mastra-validation-worker".into()).spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if port.pump().is_err() {
                    // A failed job is already a persisted terminal record; never restart it as
                    // new work. The next queued job can still run.
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        Ok(host)
    }

    /// Open the port on this build's normal app-data directories. A failure leaves the backend
    /// without verification rather than claiming success from an unverified runtime event.
    pub(crate) fn local() -> Option<Arc<Self>> {
        let candidates = solidcraft_engine::cmd::candidate_store_root()?;
        let store = std::env::var_os("SOLIDCRAFT_VALIDATION_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| solidcraft_engine::recovery::data_dir().map(|dir| dir.join("validation")))?;
        HostValidation::open(store, candidates).ok().map(Arc::new)
    }

    #[cfg(test)]
    pub(crate) fn worker_stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.worker_stop)
    }

    /// The profile reference frozen in every turn binding.
    pub(crate) fn profile(&self) -> Option<ProfileRef> {
        encode_wire(&self.profile).ok()
    }

    pub(crate) fn submit(&self, scope: &Scope, params: ValidationSubmitParams) -> Result<ValidationSubmitResult, WireError> {
        let Scope::Turn(turn) = scope else {
            return Err(local_error(ErrorCode::InvalidRequest));
        };
        let candidate: solidcraft_engine::schema::snapshot::CandidateRef =
            decode_wire(&params.candidate).map_err(|_| local_error(ErrorCode::InvalidRequest))?;
        let profile: validation_port::ProfileRef = decode_wire(&params.profile).map_err(|_| local_error(ErrorCode::InvalidRequest))?;
        if profile != self.profile {
            return Err(validation_wire(validation_port::ValidationError::profile_unsupported(
                "the submitted profile is not the host-frozen turn profile",
            )));
        }
        let result =
            self.port.submit_for(&candidate, &profile, params.request_id.as_str(), turn.workflow_run_id.as_str()).map_err(validation_wire)?;
        {
            let mut book = self.book.lock().map_err(|_| local_error(ErrorCode::InternalError))?;
            let record = ValidationRunRecord {
                run_id: result.run_id.to_string(),
                session_id: turn.session_id.to_string(),
                turn_id: turn.turn_id.to_string(),
                workflow_run_id: turn.workflow_run_id.to_string(),
                candidate: result.candidate.clone(),
                profile: result.profile.clone(),
                state: result.state,
            };
            book.insert_run(result.run_id.as_str(), record);
        }
        encode_wire(&result)
    }

    pub(crate) fn status(&self, scope: &Scope, params: ValidationStatusParams) -> Result<ValidationStatusResult, WireError> {
        let run = self.authorized(scope, params.run_id.as_str())?;
        let status = self.port.status(params.run_id.as_str()).map_err(validation_wire)?;
        if status.candidate != run.candidate || status.profile != run.profile {
            return Err(validation_wire(validation_port::ValidationError::report_mismatch("the status response names another candidate or profile")));
        }
        if let Ok(mut book) = self.book.lock()
            && let Some(record) = book.runs.get_mut(params.run_id.as_str())
        {
            record.state = status.state;
        }
        encode_wire(&status)
    }

    pub(crate) fn report(&self, scope: &Scope, params: ValidationReportParams) -> Result<ValidationReportResult, WireError> {
        let run = self.authorized(scope, params.run_id.as_str())?;
        let result = self.port.report(params.run_id.as_str()).map_err(validation_wire)?;
        let verdict = self.verify_report(&run, &result.report);
        if let Ok(mut book) = self.book.lock() {
            book.remember(
                params.run_id.as_str(),
                CachedValidationReport { reference: result.reference.clone(), state: result.report.state, verdict },
            );
        }
        encode_wire(&result)
    }

    pub(crate) fn cancel(&self, scope: &Scope, params: ValidationCancelParams) -> Result<ValidationCancelResult, WireError> {
        let _run = self.authorized(scope, params.run_id.as_str())?;
        let result = self.port.cancel(params.run_id.as_str()).map_err(validation_wire)?;
        if let Ok(mut book) = self.book.lock()
            && let Some(record) = book.runs.get_mut(params.run_id.as_str())
        {
            record.state = result.state;
        }
        encode_wire(&result)
    }

    fn authorized(&self, scope: &Scope, run_id: &str) -> Result<ValidationRunRecord, WireError> {
        let book = self.book.lock().map_err(|_| local_error(ErrorCode::InternalError))?;
        let Some(run) = book.runs.get(run_id) else {
            return Err(local_error(ErrorCode::NotFound));
        };
        match scope {
            Scope::Turn(turn) => {
                if run.session_id == turn.session_id.as_str()
                    && run.turn_id == turn.turn_id.as_str()
                    && run.workflow_run_id == turn.workflow_run_id.as_str()
                {
                    Ok(run.clone())
                } else {
                    Err(local_error(ErrorCode::NotFound))
                }
            }
            Scope::Session(session) => {
                if run.session_id == session.as_str() {
                    Ok(run.clone())
                } else {
                    Err(local_error(ErrorCode::NotFound))
                }
            }
            Scope::Runtime => Err(local_error(ErrorCode::InvalidRequest)),
        }
    }

    /// Execute the authoritative §9.3 check at the terminal transition.
    pub(crate) fn accept(&self, scope: &TurnScope, run_id: &Id, claimed: &ArtifactRef, live: Option<&DocumentStamp>) -> ValidationAcceptance {
        let run_id = run_id.as_str();
        let (run, report) = {
            let book = match self.book.lock() {
                Ok(book) => book,
                Err(_) => return self.rejected(run_id, live, "The host validation state is unavailable.".into(), false),
            };
            (book.runs.get(run_id).cloned(), book.reports.get(run_id).cloned())
        };
        let Some(run) = run else {
            return self.rejected(run_id, live, "This run was not submitted by the current turn.".into(), false);
        };
        if run.session_id != scope.session_id.as_str()
            || run.turn_id != scope.turn_id.as_str()
            || run.workflow_run_id != scope.workflow_run_id.as_str()
        {
            return self.rejected(run_id, live, "This run belongs to another turn; it cannot approve this one.".into(), true);
        }
        if run.state == validation_port::ValidationState::Cancelled {
            return self.rejected(run_id, live, "The run was cancelled; a late report cannot promote this turn.".into(), true);
        }
        if run.state == validation_port::ValidationState::Unknown {
            return self.rejected(run_id, live, "The run's outcome is unknown; it cannot approve this turn.".into(), true);
        }
        let Some(report) = report else {
            return self.rejected(run_id, live, "The host has not fetched and verified this run's report.".into(), false);
        };
        let expected = match encode_wire::<_, ArtifactRef>(&report.reference) {
            Ok(value) => value,
            Err(_) => return self.rejected(run_id, live, "The host report reference is malformed.".into(), false),
        };
        if expected != *claimed {
            return self.rejected(run_id, live, "The runtime's report reference does not match the host's own report.".into(), false);
        }
        if let Err(reason) = &report.verdict {
            return self.rejected(run_id, live, format!("The report failed host verification: {reason}."), false);
        }
        if report.state != validation_port::ValidationState::Passed {
            return self.rejected(run_id, live, "The run is terminal but did not pass.".into(), true);
        }
        let Some(live) = live else {
            return self.rejected(run_id, live, "The current live document stamp is unavailable.".into(), false);
        };
        if live.document_id.as_str() != run.candidate.source.document_id || live.revision.as_u64().to_string() != run.candidate.source.revision {
            let acceptance = self.rejected(
                run_id,
                Some(live),
                "The live document changed after the captured snapshot; the passing report is historical evidence, not approval of current content."
                    .into(),
                true,
            );
            return acceptance;
        }
        ValidationAcceptance {
            accepted: true,
            historical: false,
            reason: format!("Required checks passed for candidate {}.", run.candidate.candidate_id),
            identity: self.identity(&run, Some(&report), Some(live), "accepted", false),
        }
    }

    fn rejected(&self, run_id: &str, live: Option<&DocumentStamp>, reason: String, historical: bool) -> ValidationAcceptance {
        let book = self.book.lock().ok();
        let record = book.as_ref().and_then(|book| book.runs.get(run_id));
        ValidationAcceptance {
            accepted: false,
            historical,
            reason,
            identity: match record {
                Some(run) => {
                    let report = book.as_ref().and_then(|book| book.reports.get(run_id));
                    self.identity(run, report, live, "rejected", historical)
                }
                None => ValidationIdentity {
                    run_id: run_id.to_owned(),
                    candidate_id: String::new(),
                    candidate_digest: String::new(),
                    manifest_digest: String::new(),
                    source_document_id: String::new(),
                    source_revision: String::new(),
                    profile_id: String::new(),
                    profile_version: String::new(),
                    profile_digest: String::new(),
                    state: "unknown".into(),
                    outcome: "rejected".into(),
                    report_artifact_id: String::new(),
                    report_digest: String::new(),
                    live_document_id: live.map(|stamp| stamp.document_id.to_string()),
                    live_revision: live.map(|stamp| stamp.revision.to_string()),
                    live_changed: live.is_some(),
                    historical,
                },
            },
        }
    }

    fn identity(
        &self,
        run: &ValidationRunRecord,
        report: Option<&CachedValidationReport>,
        live: Option<&DocumentStamp>,
        outcome: &str,
        historical: bool,
    ) -> ValidationIdentity {
        let state = report.map_or(run.state, |report| report.state);
        ValidationIdentity {
            run_id: run.run_id.clone(),
            candidate_id: run.candidate.candidate_id.clone(),
            candidate_digest: run.candidate.candidate_digest.clone(),
            manifest_digest: run.candidate.manifest_digest.clone(),
            source_document_id: run.candidate.source.document_id.clone(),
            source_revision: run.candidate.source.revision.clone(),
            profile_id: run.profile.profile_id.clone(),
            profile_version: run.profile.version.clone(),
            profile_digest: run.profile.digest.clone(),
            state: state.as_str().to_owned(),
            outcome: outcome.to_owned(),
            report_artifact_id: report.map(|report| report.reference.artifact_id.clone()).unwrap_or_default(),
            report_digest: report.map(|report| report.reference.digest.clone()).unwrap_or_default(),
            live_document_id: live.map(|stamp| stamp.document_id.to_string()),
            live_revision: live.map(|stamp| stamp.revision.to_string()),
            live_changed: live.is_some_and(|stamp| {
                stamp.document_id.as_str() != run.candidate.source.document_id || stamp.revision.as_u64().to_string() != run.candidate.source.revision
            }),
            historical,
        }
    }

    fn verify_report(&self, run: &ValidationRunRecord, report: &validation_port::ValidationReport) -> Result<(), String> {
        if report.run_id != run.run_id {
            return Err("the report belongs to another run".into());
        }
        if report.candidate_id != run.candidate.candidate_id {
            return Err("the report names another candidate".into());
        }
        if report.candidate_digest != run.candidate.candidate_digest || report.manifest_digest != run.candidate.manifest_digest {
            return Err("the report binds different candidate bytes".into());
        }
        if report.profile != run.profile {
            return Err("the report binds another profile".into());
        }
        if report.runner != self.runner {
            return Err("the report was produced by an unapproved runner or engine version".into());
        }
        if report.state != validation_port::ValidationState::Passed {
            return Err(format!("the report state is {}", report.state.as_str()));
        }
        if report.error.is_some() {
            return Err("a passing report carries an error".into());
        }
        if report.checks.len() != self.profile_checks.len() {
            return Err("the report does not cover the profile's required checks exactly".into());
        }
        let mut seen = HashSet::with_capacity(report.checks.len());
        for requirement in &self.profile_checks {
            let Some(result) = report.checks.iter().find(|result| result.check_id == requirement.check_id) else {
                return Err(format!("the report is missing check {}", requirement.check_id));
            };
            if !seen.insert(result.check_id.as_str()) {
                return Err(format!("the report repeats check {}", requirement.check_id));
            }
            if result.required != requirement.required {
                return Err(format!("check {} changed its required flag", requirement.check_id));
            }
            if requirement.required {
                if result.state != validation_port::CheckState::Passed {
                    return Err(format!("required check {} is {}", requirement.check_id, result.state.as_str()));
                }
                if result.evidence.is_empty() {
                    return Err(format!("required check {} has no evidence", requirement.check_id));
                }
                for evidence in &result.evidence {
                    if !valid_digest(&evidence.digest) || evidence.size_bytes == 0 || evidence.media_type.is_empty() {
                        return Err(format!("required check {} has an unusable evidence reference", requirement.check_id));
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn verify_report_for_test(&self, run_id: &str, report: &validation_port::ValidationReport) -> Result<(), String> {
        let book = self.book.lock().map_err(|_| "validation book is unavailable".to_string())?;
        let run = book.runs.get(run_id).ok_or_else(|| "no such run".to_string())?;
        self.verify_report(run, report)
    }

    /// Identity for a terminal run that did not propose success. It is history, not approval.
    pub(crate) fn history_identity(&self, run_id: &str, live: Option<&DocumentStamp>) -> Option<ValidationIdentity> {
        let book = self.book.lock().ok()?;
        let run = book.runs.get(run_id)?;
        let report = book.reports.get(run_id);
        Some(self.identity(run, report, live, "history", true))
    }

    /// Host cancellation: cancel this turn's outstanding local jobs, keep their records as
    /// history, and return the identities the panel should show as cancelled rather than
    /// approval. An already-passed report remains history because the active turn is gone.
    pub(crate) fn cancel_turn(&self, scope: &TurnScope, live: Option<&DocumentStamp>) -> Vec<ValidationIdentity> {
        let run_ids: Vec<String> = match self.book.lock() {
            Ok(book) => book
                .runs
                .iter()
                .filter(|(_, run)| {
                    run.session_id == scope.session_id.as_str()
                        && run.turn_id == scope.turn_id.as_str()
                        && run.workflow_run_id == scope.workflow_run_id.as_str()
                        && !run.state.is_terminal()
                })
                .map(|(run_id, _)| run_id.clone())
                .collect(),
            Err(_) => Vec::new(),
        };
        for run_id in &run_ids {
            if let Ok(result) = self.port.cancel(run_id)
                && let Ok(mut book) = self.book.lock()
                && let Some(record) = book.runs.get_mut(run_id)
            {
                record.state = result.state;
            }
        }
        let Ok(book) = self.book.lock() else {
            return Vec::new();
        };
        book.runs
            .iter()
            .filter(|(_, run)| {
                run.session_id == scope.session_id.as_str()
                    && run.turn_id == scope.turn_id.as_str()
                    && run.workflow_run_id == scope.workflow_run_id.as_str()
            })
            .map(|(run_id, run)| self.identity(run, book.reports.get(run_id), live, "cancelled", true))
            .collect()
    }
}

impl Drop for HostValidation {
    fn drop(&mut self) {
        // The worker checks this every idle interval. A running `pump()` finishes its current
        // job before observing it; the port's own cancellation still bounds that job. The flag
        // is what prevents a reconnect from leaving a second 25 ms pump loop alive.
        self.worker_stop.store(true, Ordering::Release);
    }
}

impl ValidationBook {
    fn insert_run(&mut self, run_id: &str, record: ValidationRunRecord) {
        self.runs.insert(run_id.to_owned(), record);
        if !self.run_order.iter().any(|known| known == run_id) {
            self.run_order.push_back(run_id.to_owned());
        }
        while self.run_order.len() > 64 {
            // Never evict the run that was just admitted; evict the oldest other record.
            let Some(index) = self.run_order.iter().position(|known| known != run_id) else {
                break;
            };
            if let Some(oldest) = self.run_order.remove(index) {
                self.runs.remove(&oldest);
            }
        }
    }

    fn remember(&mut self, run_id: &str, report: CachedValidationReport) {
        self.reports.insert(run_id.to_owned(), report);
        if !self.report_order.iter().any(|known| known == run_id) {
            self.report_order.push_back(run_id.to_owned());
        }
        while self.report_order.len() > 128 {
            let Some(index) = self.report_order.iter().position(|known| known != run_id) else {
                break;
            };
            if let Some(oldest) = self.report_order.remove(index) {
                self.reports.remove(&oldest);
            }
        }
    }
}

fn decode_wire<T: DeserializeOwned, U: Serialize>(value: &U) -> Result<T, WireError> {
    let json = serde_json::to_value(value).map_err(|_| local_error(ErrorCode::InvalidRequest))?;
    serde_json::from_value(json).map_err(|_| local_error(ErrorCode::InvalidRequest))
}

fn encode_wire<T: Serialize, U: DeserializeOwned>(value: &T) -> Result<U, WireError> {
    let json = serde_json::to_value(value).map_err(|_| local_error(ErrorCode::InternalError))?;
    serde_json::from_value(json).map_err(|_| local_error(ErrorCode::InternalError))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..].iter().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn local_error(code: ErrorCode) -> WireError {
    WireError {
        code,
        message: "The host validation service rejected this call.".into(),
        retry: Retry::Never,
        details: ErrorDetails::Empty(EmptyDetails {}),
    }
}

fn validation_wire(error: validation_port::ValidationError) -> WireError {
    let code = match error.code {
        validation_port::ErrorCode::InvalidRequest => ErrorCode::InvalidRequest,
        validation_port::ErrorCode::InvalidCandidate => ErrorCode::InvalidCandidate,
        validation_port::ErrorCode::DependencyChanged => ErrorCode::DependencyChanged,
        validation_port::ErrorCode::ProfileUnsupported => ErrorCode::ProfileUnsupported,
        validation_port::ErrorCode::ReportMismatch => ErrorCode::ReportMismatch,
        validation_port::ErrorCode::NotReady => ErrorCode::NotReady,
        validation_port::ErrorCode::NotFound => ErrorCode::NotFound,
        validation_port::ErrorCode::IdempotencyConflict => ErrorCode::IdempotencyConflict,
        validation_port::ErrorCode::RequestExpired => ErrorCode::RequestExpired,
        validation_port::ErrorCode::ResourceExhausted => ErrorCode::ResourceExhausted,
        validation_port::ErrorCode::Timeout => ErrorCode::Timeout,
        validation_port::ErrorCode::Cancelled => ErrorCode::Cancelled,
        validation_port::ErrorCode::OutcomeUnknown => ErrorCode::OutcomeUnknown,
        validation_port::ErrorCode::InternalError => ErrorCode::InternalError,
    };
    let retry = match code {
        ErrorCode::ResourceExhausted | ErrorCode::NotReady | ErrorCode::Timeout | ErrorCode::Cancelled | ErrorCode::OutcomeUnknown => {
            Retry::SameSubmission
        }
        _ => Retry::Never,
    };
    WireError { code, message: error.message, retry, details: ErrorDetails::Empty(EmptyDetails {}) }
}

/// What a Mastra backend is told before it starts. The execution choice is already made; this is
/// only the connection, workflow and launch identity it uses.
pub(crate) struct MastraConfig {
    /// The existing AI connection (`AgentConfig.id`) the workflow's model calls use.
    pub connection_id: String,
    /// The model selected for that connection when the backend started.
    pub model: String,
    /// The workflow to ask the runtime to run.
    pub workflow_id: String,
    /// The chat's existing MCP bridge, if the window could open one.
    pub bridge: Option<McpBridge>,
    /// The chat's control queue, used to serve the runtime's inherited MCP lane with the same
    /// gate, undo ownership and document state as every other chat tool call.
    pub tools: Option<std::sync::mpsc::Sender<crate::control::ControlRequest>>,
    /// Chat instructions copied into every new session.
    pub instructions: String,
    /// Set on the host worker when a connection probe or `connection.describe` call answers;
    /// polled by the panel as [`Event::AuthRequired`] / [`Event::ConnectionAuth`], never as
    /// runtime readiness.
    pub connection_auth: Arc<Mutex<Option<ConnectionAuthReport>>>,
    /// ACP task progress from the host worker, bounded and mapped to panel events.
    pub progress: Arc<ProgressQueue>,
    /// The connection/model pair frozen for the turn in progress; host calls must match it.
    pub frozen_binding: Arc<Mutex<Option<(String, String)>>>,
    /// The host-owned local validation port, when this build can run it. Without it the
    /// backend never publishes `validated_success`, even if the runtime proposes one.
    pub validation: Option<Arc<HostValidation>>,
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
pub(crate) struct MastraBackend {
    port: Box<dyn Port>,
    ids: Arc<dyn IdSource>,
    config: MastraConfig,
    host: Option<Arc<HostAi>>,
    runtime_instance: Option<Arc<Mutex<Option<String>>>>,
    model_budget: Option<Arc<AtomicU32>>,
    agent_budget: Option<Arc<AtomicU32>>,
    task_control: Arc<TaskControlSlot>,
    progress: Arc<ProgressQueue>,
    auth_methods: Vec<AuthMethod>,
    connection_signed_out: bool,
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
    packaged_verified: bool,
    models_sent: bool,
    host_bridge: Option<HostBridge>,
    bridge_generation: Option<u64>,
    turn_active: Option<Arc<AtomicBool>>,
}

impl MastraBackend {
    /// Start the runtime and its bounded client. Returns an error text suitable for the panel
    /// when the process cannot start at all; protocol failures arrive as events later.
    pub(crate) fn start(
        mut config: MastraConfig,
        mut launch: RuntimeLaunch,
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
        let task_control: Arc<TaskControlSlot> = Arc::new(TaskControlSlot::default());
        let mut host_bridge = None;
        let mut bridge_fallback: Option<String> = None;
        if config.bridge.is_some() {
            let tools = config.tools.clone().ok_or_else(|| crate::i18n::tr("ui.mastra_bridge_unavailable").to_string())?;
            match HostBridge::open(tools) {
                Ok(bridge) => {
                    let fd = bridge.child_fd();
                    launch.args.push(std::ffi::OsString::from(format!("--bridge-fd={fd}")));
                    if let Some(argument) = capability_argument() {
                        launch.args.push(std::ffi::OsString::from(argument));
                    }
                    host_bridge = Some(bridge);
                }
                Err(error) => {
                    // The prior no-bridge path is truthful and keeps Mastra chat available
                    // instead of making the whole backend fatal on a platform without a lane.
                    config.bridge = None;
                    bridge_fallback = Some(error);
                }
            }
        }
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
                Arc::clone(&config.progress),
                Arc::clone(&config.frozen_binding),
                config.validation.as_ref().map(Arc::clone),
            ),
            Arc::new(move || wake()),
        )
        .map_err(|error| crate::i18n::format("ui.mastra_runtime_failed", &[("error", &format!("{:?}", error.code))]))?;
        if let Some(bridge) = host_bridge.as_mut() {
            bridge.release_child();
        }
        let sender = runtime.sender();
        let connection = config.connection_id.clone();
        let model = config.model.clone();
        let auth = Arc::clone(&config.connection_auth);
        let probe_wake = Arc::clone(&config.wake);
        let mut backend = Self::with_port(config, Box::new(LivePort { client: runtime, sender, drain_bytes }), ids);
        backend.host = Some(host);
        backend.runtime_instance = Some(runtime_instance);
        backend.model_budget = Some(model_budget);
        backend.agent_budget = Some(agent_budget);
        backend.task_control = task_control;
        backend.host_bridge = host_bridge;
        if let Some(error) = bridge_fallback {
            backend.events.push_back(Event::Text { thought: true, text: crate::i18n::format("ui.mastra_bridge_fallback", &[("error", &error)]) });
        }
        backend.events.push_back(Event::Documents(false));
        backend.events.push_back(Event::ToolsAvailable(backend.config.bridge.is_some()));
        // The connection's sign-in state is checked once, off the window thread, and kept apart
        // from runtime readiness. For an ACP connection this is the real session probe, so the
        // panel gets the agent's own methods instead of a guessed key state.
        let probe = backend.host.as_ref().map(Arc::clone);
        let _ = std::thread::Builder::new().name("mastra-connection-probe".into()).spawn(move || {
            let Some(host) = probe else { return };
            let Ok(description) = host.describe_agent(&connection, &model) else { return };
            if let Ok(mut reported) = auth.lock() {
                *reported = Some(ConnectionAuthReport::from_description(host.is_acp(&connection), description));
            }
            (probe_wake)();
        });
        Ok(backend)
    }

    pub(crate) fn with_port(config: MastraConfig, port: Box<dyn Port>, ids: Arc<dyn IdSource>) -> Self {
        let next_model = config.model.clone();
        let next_connection = config.connection_id.clone();
        let progress = Arc::clone(&config.progress);
        Self {
            port,
            ids,
            config,
            host: None,
            runtime_instance: Some(Arc::new(Mutex::new(None))),
            model_budget: None,
            agent_budget: None,
            task_control: Arc::new(TaskControlSlot::default()),
            progress,
            auth_methods: Vec::new(),
            connection_signed_out: false,
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
            packaged_verified: false,
            models_sent: false,
            host_bridge: None,
            bridge_generation: None,
            turn_active: None,
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
    pub(crate) fn set_task_control_for_test(&mut self, slot: Arc<TaskControlSlot>) {
        self.task_control = slot;
    }

    #[cfg(test)]
    pub(crate) fn report_description_for_test(&mut self, description: host_ai::AgentDescription) {
        let acp = self.host.as_ref().is_some_and(|host| host.is_acp(&self.config.connection_id));
        if let Ok(mut reported) = self.config.connection_auth.lock() {
            *reported = Some(ConnectionAuthReport::from_description(acp, description));
        }
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
            self.push(Event::Error(crate::i18n::format("ui.mastra_runtime_failed", &[("error", "session")])));
            return;
        };
        if !self.rebind_host_session(self.session.as_ref(), &session) {
            self.push(Event::Error(crate::i18n::format("ui.mastra_runtime_failed", &[("error", "session")])));
            return;
        }
        let bridge = if self.config.bridge.is_some() {
            let Ok(bridge_id) = self.ids.next_id() else {
                self.push(Event::Error(crate::i18n::format("ui.mastra_runtime_failed", &[("error", "bridge")])));
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
            Err(_) => self.push(Event::Error(crate::i18n::format("ui.mastra_runtime_failed", &[("error", "session")]))),
        }
    }

    fn host_session_key(&self, session: &Id) -> Option<SessionKey> {
        let runtime_instance_id = self.runtime_instance.as_ref().and_then(|id| id.lock().ok().and_then(|id| id.clone())).unwrap_or_default();
        Some(SessionKey { runtime_instance_id, session_id: session.to_string() })
    }

    /// Turn the composer's parts into wire content blocks. Images and PDFs become host-owned
    /// artifact references; only the reference is sent to the runtime.
    fn turn_blocks(&self, session: &SessionKey, parts: &[PromptPart]) -> Result<Vec<ContentBlock>, &'static str> {
        let mut blocks = Vec::with_capacity(parts.len());
        let mut count = 0usize;
        let mut total = 0usize;
        for part in parts {
            match part {
                PromptPart::Text(text) => blocks.push(ContentBlock::Text { text: text.clone() }),
                PromptPart::Image { mime, base64 } => {
                    if !matches!(mime.as_str(), "image/png" | "image/jpeg") {
                        return Err("ui.mastra_attachment_type");
                    }
                    let bytes = super::backend::base64_decode(base64).map_err(|_| "ui.mastra_attachment_invalid")?;
                    let reference = self.store_attachment(session, mime, bytes, &mut count, &mut total)?;
                    blocks.push(ContentBlock::Image { artifact: wire_artifact(&reference)? });
                }
                PromptPart::Document { name, mime, base64, .. } => {
                    if mime != "application/pdf" {
                        return Err("ui.mastra_attachment_type");
                    }
                    let bytes = super::backend::base64_decode(base64).map_err(|_| "ui.mastra_attachment_invalid")?;
                    let reference = self.store_attachment(session, mime, bytes, &mut count, &mut total)?;
                    let name: String = name.trim().chars().take(200).collect();
                    if name.is_empty() {
                        return Err("ui.mastra_attachment_invalid");
                    }
                    blocks.push(ContentBlock::Document { name, artifact: wire_artifact(&reference)? });
                }
            }
        }
        Ok(blocks)
    }

    fn store_attachment(
        &self,
        session: &SessionKey,
        media_type: &str,
        bytes: Vec<u8>,
        count: &mut usize,
        total: &mut usize,
    ) -> Result<host_ai::ArtifactRef, &'static str> {
        if bytes.is_empty() || bytes.len() > MAX_PROMPT_ATTACHMENT_BYTES {
            return Err("ui.mastra_attachment_size");
        }
        *count = count.checked_add(1).ok_or("ui.mastra_attachment_count")?;
        if *count > MAX_PROMPT_ATTACHMENTS {
            return Err("ui.mastra_attachment_count");
        }
        *total = total.checked_add(bytes.len()).ok_or("ui.mastra_attachment_size")?;
        if *total > MAX_PROMPT_ATTACHMENT_TOTAL_BYTES {
            return Err("ui.mastra_attachment_size");
        }
        let host = self.host.as_ref().ok_or("ui.mastra_attachment_invalid")?;
        host.store_attachment(session, std::sync::Arc::from(bytes), media_type).map_err(|error| match error.code() {
            AiErrorCode::ResourceExhausted => {
                if *count > MAX_PROMPT_ATTACHMENTS {
                    "ui.mastra_attachment_count"
                } else {
                    "ui.mastra_attachment_size"
                }
            }
            AiErrorCode::UnsupportedCapability => "ui.mastra_attachment_type",
            _ => "ui.mastra_attachment_invalid",
        })
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
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
            self.push(Event::TurnEnded { reason: "error".into() });
            return;
        };
        let Some(host_session) = self.host_session_key(&session) else {
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
            self.push(Event::TurnEnded { reason: "error".into() });
            return;
        };
        if let Some(host) = self.host.as_ref() {
            // A turn owns its attachments; the previous turn's references stop resolving.
            host.clear_attachments(&host_session);
        }
        let blocks = match self.turn_blocks(&host_session, parts) {
            Ok(blocks) => blocks,
            Err(key) => {
                self.push(Event::Error(crate::i18n::tr(key).to_string()));
                self.push(Event::TurnEnded { reason: "error".into() });
                return;
            }
        };
        let binding = FrozenBinding {
            workflow_id: workflow_id.to_string(),
            connection_id: connection_id.to_string(),
            model_id: self.next_model.clone(),
            session_id: session.to_string(),
            turn_id: turn_id.to_string(),
            workflow_run_id: workflow_run_id.to_string(),
        };
        let scope = TurnScope { session_id: session.clone(), turn_id, workflow_run_id };
        let profile = match self.config.validation.as_ref() {
            Some(validation) => match validation.profile() {
                Some(profile) => profile,
                None => {
                    self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
                    self.push(Event::TurnEnded { reason: "error".into() });
                    return;
                }
            },
            None => {
                let Ok(profile_id) = Id::parse(NATIVE_PROFILE) else {
                    self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
                    self.push(Event::TurnEnded { reason: "error".into() });
                    return;
                };
                let Ok(digest) = Digest::parse(EMPTY_DIGEST) else {
                    self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
                    self.push(Event::TurnEnded { reason: "error".into() });
                    return;
                };
                ProfileRef { profile_id, version: "1.0".into(), digest }
            }
        };
        let params = TurnStartParams {
            binding: TurnBinding { workflow_id, connection_id, model_id: self.next_model.clone(), document, profile },
            parts: blocks,
            context: TurnContext { selection: Vec::new(), requirements: RequiredNullable(None) },
            budgets: Budgets::default(),
        };
        if let Some(bridge) = self.host_bridge.as_ref() {
            let Some(active) = self.turn_active.clone() else {
                self.push(Event::Error(crate::i18n::tr("ui.mastra_bridge_unavailable").to_string()));
                self.push(Event::TurnEnded { reason: "error".into() });
                return;
            };
            self.bridge_generation = Some(bridge.begin_turn(active));
        }
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
                if let Some(bridge) = self.host_bridge.as_ref()
                    && let Some(generation) = self.bridge_generation.take()
                {
                    bridge.end_turn(generation);
                }
                self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
                self.push(Event::TurnEnded { reason: "error".into() });
            }
        }
    }

    fn end_active(&mut self) -> Option<ActiveTurn> {
        if let Ok(mut frozen) = self.config.frozen_binding.lock() {
            *frozen = None;
        }
        if let Some(bridge) = self.host_bridge.as_ref()
            && let Some(generation) = self.bridge_generation.take()
        {
            bridge.end_turn(generation);
        }
        self.turn_active = None;
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

    fn push_validation_identity(&mut self, acceptance: &ValidationAcceptance, status: ToolStatus) {
        self.push_validation_card(&acceptance.identity, status);
    }

    fn push_validation_history(&mut self, identity: &ValidationIdentity, status: ToolStatus) {
        self.push_validation_card(identity, status);
    }

    fn push_validation_card(&mut self, identity: &ValidationIdentity, status: ToolStatus) {
        let state = validation_state_label(&identity.state);
        let title = crate::i18n::format("ui.mastra_validation_card", &[("state", &state)]);
        let output = serde_json::to_string(&identity.json()).ok();
        self.push(Event::Tool {
            id: format!("validation:{}", identity.run_id),
            title: Some(title),
            kind: Some("validation".into()),
            status: Some(status),
            input: None,
            output,
        });
    }

    fn validation_history(&self, run_id: &str) -> Option<ValidationIdentity> {
        self.config.validation.as_ref()?.history_identity(run_id, self.document.as_ref())
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
                let mut reason = terminal_reason(finished.state);
                if finished.state == TurnTerminal::AuthRequired {
                    // A terminal auth failure belongs to the AI connection, never to runtime
                    // readiness, so it updates the separate connection state.
                    if let Ok(mut reported) = self.config.connection_auth.lock() {
                        *reported = Some(ConnectionAuthReport {
                            available: false,
                            acp: self.host.as_ref().is_some_and(|host| host.is_acp(&self.config.connection_id)),
                            methods: self.auth_methods.clone(),
                        });
                    }
                }
                if finished.state == TurnTerminal::ValidatedSuccess {
                    // The runtime's proposal is not acceptance: this host must find the report it
                    // submitted for this turn, verify it and compare the current live stamp.
                    let run_id = finished.validation_run_id.0.as_ref();
                    let claimed = finished.report.0.as_ref();
                    let acceptance = match (&self.config.validation, run_id, claimed) {
                        (Some(service), Some(run_id), Some(claimed)) => Some(service.accept(&turn.scope, run_id, claimed, self.document.as_ref())),
                        (None, _, _) => {
                            self.push(Event::Text { thought: true, text: crate::i18n::tr("ui.mastra_validation_unavailable").to_string() });
                            None
                        }
                        _ => {
                            self.push(Event::Text { thought: true, text: crate::i18n::tr("ui.mastra_validation_no_report").to_string() });
                            None
                        }
                    };
                    match acceptance {
                        Some(acceptance) if acceptance.accepted => {
                            reason = String::new();
                            self.push_validation_identity(&acceptance, ToolStatus::Completed);
                        }
                        Some(acceptance) => {
                            reason = "error".into();
                            let key = if acceptance.historical { "ui.mastra_validation_historical" } else { "ui.mastra_validation_rejected" };
                            self.push(Event::Text { thought: true, text: crate::i18n::format(key, &[("reason", &acceptance.reason)]) });
                            self.push_validation_identity(&acceptance, ToolStatus::Failed);
                        }
                        None => {
                            reason = "error".into();
                        }
                    }
                } else if let Some(run_id) = finished.validation_run_id.0.as_ref() {
                    // A failed or cancelled run is shown as history, never as approval.
                    if let Some(identity) = self.validation_history(run_id.as_str()) {
                        self.push_validation_history(&identity, ToolStatus::Failed);
                    }
                }
                // Keep the report/reference for history and read-only queries; the active turn
                // slot is already cleared, so no later event can promote this turn.
                self.push(Event::TurnEnded { reason });
            }
        }
    }

    fn map_progress(&mut self, progress: AgentProgress) {
        // Progress belongs to the turn that is running. A stopped or replaced turn's late
        // updates never become panel items, so its permission cannot leak into a later task.
        if self.active.is_none() {
            return;
        }
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
                let title =
                    if permission.title.trim().is_empty() { crate::i18n::tr("ui.host_ai_task_permission").to_string() } else { permission.title };
                if permission.state == PermissionState::Waiting
                    && let Some(key) = permission.permission_id.as_str().strip_prefix('p').and_then(|k| k.parse::<u64>().ok())
                    && self.task_control.issued(key)
                {
                    // The ACP task owner maps its own option ids to the panel's two answers.
                    self.push(Event::Permission {
                        key,
                        title,
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
                if permission.state == PermissionState::Waiting {
                    // Waiting for a task that no longer holds the slot: never a live button.
                    return;
                }
                // A resolved permission is shown as a card, never as a live button.
                let status = match permission.state {
                    PermissionState::Waiting => Some(ToolStatus::InProgress),
                    PermissionState::Allowed | PermissionState::Denied => Some(ToolStatus::Completed),
                };
                self.push(Event::Tool {
                    id: permission.permission_id.to_string(),
                    title: Some(title),
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
                                    self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
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
                    self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
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
        // A known unauthenticated ACP connection never becomes ready: the panel keeps the
        // sign-in box open until the agent proves a real session works.
        if self.connection_signed_out {
            return;
        }
        if !self.models_sent {
            self.models_sent = true;
            let model = self.next_model.clone();
            if !model.is_empty() {
                self.push(Event::Models {
                    current: model.clone(),
                    available: vec![super::acp::ModelInfo { id: model.clone(), name: model, description: String::new() }],
                });
            }
        }
        self.push(Event::Ready { agent: format!("Mastra · {}", self.config.workflow_id), images: false });
    }
}

fn validation_state_label(state: &str) -> String {
    let key = match state {
        "queued" => "ui.mastra_validation_state_queued",
        "running" => "ui.mastra_validation_state_running",
        "passed" => "ui.mastra_validation_state_passed",
        "quality_failed" => "ui.mastra_validation_state_quality_failed",
        "infrastructure_error" => "ui.mastra_validation_state_infrastructure_error",
        "cancelled" => "ui.mastra_validation_state_cancelled",
        "unknown" => "ui.mastra_validation_state_unknown",
        _ => return state.to_owned(),
    };
    crate::i18n::tr(key).to_string()
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
    connection_auth: Arc<Mutex<Option<ConnectionAuthReport>>>,
    runtime_instance: Arc<Mutex<Option<String>>>,
    budget: Arc<AtomicU32>,
    agent_budget: Arc<AtomicU32>,
    task_control: Arc<TaskControlSlot>,
    progress: Arc<ProgressQueue>,
    frozen_binding: Arc<Mutex<Option<(String, String)>>>,
    validation: Option<Arc<HostValidation>>,
) -> RuntimeServices {
    RuntimeServices {
        clock: Arc::new(WallClock),
        ids,
        admission,
        lifecycle: Arc::new(ClosedLifecycle),
        dispatch: Arc::new(HostCalls {
            host,
            connection_auth,
            budget,
            agent_budget,
            task_control,
            progress,
            runtime_instance,
            frozen_binding,
            validation: validation.map(|service| Arc::downgrade(&service)),
        }),
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
    connection_auth: Arc<Mutex<Option<ConnectionAuthReport>>>,
    budget: Arc<AtomicU32>,
    agent_budget: Arc<AtomicU32>,
    task_control: Arc<TaskControlSlot>,
    progress: Arc<ProgressQueue>,
    runtime_instance: Arc<Mutex<Option<String>>>,
    frozen_binding: Arc<Mutex<Option<(String, String)>>>,
    /// Weak so dropping the backend drops the port and stops its worker even while the runtime
    /// client thread still owns its services. A live callback upgrades for its own duration.
    validation: Option<std::sync::Weak<HostValidation>>,
}

impl HostDispatch for HostCalls {
    fn dispatch(&self, call: CallbackCall, reply: ReplyPort) {
        let host = Arc::clone(&self.host);
        let auth = Arc::clone(&self.connection_auth);
        let budget = Arc::clone(&self.budget);
        let agent_budget = Arc::clone(&self.agent_budget);
        let task_control = Arc::clone(&self.task_control);
        let progress = Arc::clone(&self.progress);
        let instance = Arc::clone(&self.runtime_instance);
        let frozen = Arc::clone(&self.frozen_binding);
        let validation = self.validation.as_ref().and_then(std::sync::Weak::upgrade);
        let spawned = std::thread::Builder::new().name("mastra-host-call".into()).spawn(move || {
            let cancelled = call.cancellation.is_cancelled();
            let result = if cancelled {
                Err(cancelled_error("operation"))
            } else {
                handle_call(&host, &auth, &budget, &agent_budget, &task_control, &progress, &instance, &frozen, validation.as_deref(), call, &reply)
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
    auth: &Mutex<Option<ConnectionAuthReport>>,
    budget: &AtomicU32,
    agent_budget: &AtomicU32,
    task_control: &TaskControlSlot,
    progress: &ProgressQueue,
    runtime_instance: &Mutex<Option<String>>,
    frozen_binding: &Mutex<Option<(String, String)>>,
    validation: Option<&HostValidation>,
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
                *reported = Some(ConnectionAuthReport::from_description(host.is_acp(&connection), description.clone()));
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
                    .filter_map(|method| {
                        Some(super::mastra_protocol::AuthMethod {
                            id: Id::parse(&method.id).ok()?,
                            name: method.name,
                            description: method.description,
                        })
                    })
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
            // A validated delegation now consumes the turn's ACP-task budget.
            if agent_budget.try_update(Ordering::AcqRel, Ordering::Acquire, |left| left.checked_sub(1)).is_err() {
                return Err(resource_exhausted("agent_tasks", AGENT_TASKS_PER_TURN));
            }
            let control = Arc::new(TaskControl::default());
            let _control_guard = task_control.acquire(Arc::clone(&control)).map_err(|error| ai_task_error(&connection, &error))?;
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
            let mut emit = |update: TaskProgress| {
                let event = progress.publish(update);
                let _ = reply.emit(CallProgress::AgentProgress(event));
            };
            let result = host.agent_run(&binding, request, &control, &mut emit);
            done.store(true, Ordering::Release);
            if cancellation.is_cancelled() || control.is_cancelled() {
                return Err(cancelled_error("agent.run"));
            }
            match result {
                Ok(outcome) => Ok(HostReply::AgentRun(AgentRunResult {
                    agent_session_id: Id::parse(&outcome.agent_session_id).map_err(|_| invalid_error("agent_session_id"))?,
                    state: CompletedState::Completed,
                    content: vec![ContentBlock::Text { text: outcome.text }],
                    stop_reason: outcome.stop_reason,
                    usage: RequiredNullable(outcome.usage.map(|usage| Usage {
                        input_tokens: RequiredNullable(usage.input_tokens),
                        output_tokens: RequiredNullable(usage.output_tokens),
                        total_tokens: RequiredNullable(usage.total_tokens),
                    })),
                })),
                Err(error) => Err(ai_task_error(&connection, &error)),
            }
        }
        HostCall::ValidationSubmit(params) => {
            let service = validation.ok_or_else(|| unsupported("validation"))?;
            service.submit(&scope, *params).map(HostReply::ValidationSubmit)
        }
        HostCall::ValidationStatus(params) => {
            let service = validation.ok_or_else(|| unsupported("validation"))?;
            service.status(&scope, params).map(HostReply::ValidationStatus)
        }
        HostCall::ValidationReport(params) => {
            let service = validation.ok_or_else(|| unsupported("validation"))?;
            service.report(&scope, params).map(HostReply::ValidationReport)
        }
        HostCall::ValidationCancel(params) => {
            let service = validation.ok_or_else(|| unsupported("validation"))?;
            service.cancel(&scope, params).map(HostReply::ValidationCancel)
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
                    ContentBlock::Image { artifact } => Ok(host_ai::ContentPart::Image(host_artifact(artifact)?)),
                    ContentBlock::Document { name, artifact } => {
                        Ok(host_ai::ContentPart::Document { name: name.clone(), artifact: host_artifact(artifact)? })
                    }
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

fn digest_text(digest: [u8; 32]) -> String {
    let mut text = String::with_capacity(71);
    text.push_str("sha256:");
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn wire_artifact(reference: &host_ai::ArtifactRef) -> Result<ArtifactRef, &'static str> {
    Ok(ArtifactRef {
        artifact_id: Id::parse(&reference.artifact_id).map_err(|_| "ui.mastra_attachment_invalid")?,
        digest: Digest::parse(&digest_text(reference.digest)).map_err(|_| "ui.mastra_attachment_invalid")?,
        size_bytes: reference.size_bytes,
        media_type: reference.media_type.clone(),
    })
}

fn host_artifact(reference: &ArtifactRef) -> Result<host_ai::ArtifactRef, WireError> {
    let text = reference.digest.to_string();
    let hex = text.strip_prefix("sha256:").ok_or_else(|| invalid_error("digest"))?;
    if hex.len() != 64 {
        return Err(invalid_error("digest"));
    }
    let mut digest = [0u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        let pair = hex.get(index * 2..index * 2 + 2).ok_or_else(|| invalid_error("digest"))?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| invalid_error("digest"))?;
    }
    let artifact_id = reference.artifact_id.as_str().to_string();
    Ok(host_ai::ArtifactRef { artifact_id, digest, size_bytes: reference.size_bytes, media_type: reference.media_type.clone() })
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
            let capability = match error.details() {
                AiErrorDetails::Capability { capability } => (*capability).to_owned(),
                _ => "model_generate".to_owned(),
            };
            (ErrorCode::UnsupportedCapability, ErrorDetails::Capability(CapabilityDetails { capability }))
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

/// Agent-task errors name the operation the runtime called, not `model.generate`.
fn ai_task_error(connection_id: &str, error: &AiError) -> WireError {
    let mut wire = ai_error(connection_id, error);
    if matches!(error.code(), AiErrorCode::Timeout | AiErrorCode::Cancelled | AiErrorCode::TransportError) {
        wire.details = ErrorDetails::Operation(OperationDetails { operation: Some("agent.run".into()) });
    }
    wire
}

// The wire and host modules both use the name `FinishReason`; alias the wire one where the host
// own type is already in scope.
use super::mastra_protocol::FinishReason as FinishReasonWire;
use super::mastra_protocol::ModelMessage as ModelMessageWire;

impl ChatBackend for MastraBackend {
    fn poll(&mut self) -> Vec<Event> {
        for event in self.host_bridge.as_ref().map(HostBridge::take_events).unwrap_or_default() {
            self.push(event);
        }
        for notice in self.port.drain() {
            self.on_notice(notice);
        }
        for progress in self.progress.drain() {
            self.on_notice(PortNotice::Progress(progress));
        }
        if let Some(error) = self.host_bridge.as_ref().and_then(HostBridge::take_error) {
            self.push(Event::Error(error));
        }
        let reported = self.config.connection_auth.lock().ok().and_then(|mut reported| reported.take());
        if let Some(report) = reported {
            self.connection_signed_out = report.acp && !report.available;
            if report.available {
                self.push(Event::ConnectionAuth { available: true });
                if self.session_ready && self.active.is_none() {
                    self.emit_ready();
                }
            } else {
                if report.acp {
                    self.auth_methods = report.methods.clone();
                    self.push(Event::AuthRequired { methods: report.methods });
                }
                self.push(Event::ConnectionAuth { available: false });
            }
        }
        self.events.drain(..).collect()
    }

    fn prompt(&mut self, parts: &[PromptPart]) {
        // The window thread checked the receipt shape when it connected. Hash the full artifact
        // once, when the first turn actually needs it, rather than freezing startup (or every
        // prompt) on a multi-hundred-megabyte digest pass.
        if !self.packaged_verified && super::packaged_runtime::installed() {
            if let Err(error) = super::packaged_runtime::verify_current(&super::packaged_runtime::expectations()) {
                let text = crate::i18n::format("ui.mastra_runtime_unusable", &[("reason", &error.to_string())]);
                self.push(Event::Error(text));
                return;
            }
            self.packaged_verified = true;
        }
        self.begin_turn(parts);
    }

    fn cancel(&mut self) {
        self.task_control.cancel();
        let Some(turn) = self.end_active() else { return };
        if let Some(bridge) = self.host_bridge.as_ref() {
            bridge.cancel_turn();
        }
        let _ = self.port.cancel_turn(&turn.scope, CancelReason::User);
        // Cancel this turn's local jobs but keep their records: the panel shows them as history,
        // and clearing the active slot makes a late terminal proposal unable to promote the turn.
        if let Some(service) = self.config.validation.as_ref() {
            for identity in service.cancel_turn(&turn.scope, self.document.as_ref()) {
                self.push_validation_history(&identity, ToolStatus::Failed);
            }
        }
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
            self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string()));
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
            Err(_) => self.push(Event::Error(crate::i18n::tr("ui.mastra_runtime_internal_failure").to_string())),
        }
    }

    fn set_instructions(&mut self, text: &str) {
        self.config.instructions = text.to_owned();
    }

    fn set_document(&mut self, document_id: &str, revision: u64) {
        self.document = document_stamp_from_native(document_id, revision).ok();
    }

    fn set_turn_control(&mut self, active: Arc<AtomicBool>) {
        self.turn_active = Some(active);
    }

    fn answer_permission(&mut self, key: u64, option: Option<&str>) {
        // The task owner refuses a key it did not issue; a stale panel answer must never reach a
        // later task's permission map.
        let _ = self.task_control.answer(key, option);
    }

    fn authenticate(&mut self, method: &str) {
        // The selected connection owns its sign-in. ACP tasks forward the agent's own methods and
        // invoke its authenticate request; no key is stored here.
        let Some(host) = self.host.as_ref().map(Arc::clone) else { return };
        let connection = self.config.connection_id.clone();
        let method = method.to_string();
        let auth = Arc::clone(&self.config.connection_auth);
        let methods = self.auth_methods.clone();
        let wake = Arc::clone(&self.config.wake);
        let _ = std::thread::Builder::new().name("mastra-agent-auth".into()).spawn(move || {
            let acp = host.is_acp(&connection);
            let report = match host.authenticate_agent(&connection, &method) {
                Ok(description) => ConnectionAuthReport::from_description(acp, description),
                Err(_) => ConnectionAuthReport { available: false, acp, methods },
            };
            if let Ok(mut reported) = auth.lock() {
                *reported = Some(report);
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
        // A task in flight must see cancellation before its process is reaped.
        self.task_control.cancel();
        self.host_bridge.take();
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
