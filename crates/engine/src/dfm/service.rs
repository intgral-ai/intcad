//! The host-side DFM service: capture, freeze, run beside the window, cancel, inspect.
//!
//! `dfm.check` runs this on the UI thread only for the two cheap, coherent reads: the model
//! snapshot (`document.snapshot.capture`) and the project freeze. The validation port and the
//! rule layer run on one background worker, so saving and editing stay usable. The worker writes
//! the project record through [`run_once`] and the window reads it with `inspect.dfm`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dfm::compile::{RunInput, SheetRef};
use crate::dfm::run::{Freshness, Reservation, RunOutcome, RunRequest, RunResult, RunStore, freshness, run_once_reserved};
use crate::project::Project;
use crate::schema::snapshot::CandidateRef;
use crate::validation::{ValidationPort, timestamp};
use crate::{EngineError, Session};

/// What `dfm.check` accepted. The run id is not known until the worker submits it, so the window
/// follows `dfm.status` and reads the result with `inspect.dfm`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckTicket {
    pub project_id: String,
    pub project_name: String,
    pub candidate_digest: String,
    pub config_digest: String,
    pub sheets: usize,
    /// `true` when the checks could only run after an earlier check stopped.
    pub replaces_running: bool,
}

/// The live state `dfm.status` reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub running: bool,
    pub active_run: Option<String>,
    /// The last finished check, from this session.
    pub last_run: Option<String>,
    pub last_outcome: Option<String>,
    pub last_reason: Option<String>,
    pub error: Option<String>,
}

/// `inspect.dfm`'s answer. `findings` is the exact CONTRACT §2 object A renders and B locates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub available: bool,
    pub running: bool,
    pub run_id: Option<String>,
    pub outcome: Option<String>,
    pub reason: Option<String>,
    pub model_version: Option<String>,
    pub candidate_digest: Option<String>,
    pub config_digest: Option<String>,
    pub freshness: Option<String>,
    /// The whole `findings.json` object, unchanged.
    pub findings: Option<Value>,
    pub summary: Option<String>,
    pub source_revision: Option<String>,
}

struct Job {
    request: RunRequest,
    /// The candidate store the worker must read. Captured on the window thread so the worker does
    /// not depend on thread-local or later host state.
    candidates: Option<PathBuf>,
    store: Option<PathBuf>,
    /// A controlled Review run: exact host run identity, billed operation and measured stop.
    reservation: Option<Reservation>,
}

struct Last {
    run_id: String,
    outcome: RunOutcome,
    reason: Option<String>,
}

#[derive(Default)]
struct Shared {
    cancel: AtomicBool,
    running: AtomicBool,
    active: Mutex<Option<String>>,
    last: Mutex<Option<Last>>,
    error: Mutex<Option<String>>,
    port: Mutex<Option<Arc<ValidationPort>>>,
    /// The captured document revision of the live job, for a cheap staleness cancel.
    started_revision: Mutex<Option<String>>,
}

struct Service {
    tx: Sender<Job>,
    shared: Arc<Shared>,
}

static SERVICE: OnceLock<Mutex<Option<Service>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<Service>> {
    SERVICE.get_or_init(|| Mutex::new(None))
}

/// Start (or reuse) the background worker. The worker opens the validation port on its first job,
/// so opening a project never blocks on the store.
fn service() -> Result<(Sender<Job>, Arc<Shared>), EngineError> {
    let mut slot = slot().lock().map_err(|_| internal("the DFM service lock was poisoned"))?;
    if let Some(service) = slot.as_ref() {
        return Ok((service.tx.clone(), Arc::clone(&service.shared)));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let shared = Arc::new(Shared::default());
    let worker_shared = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("dfm-check-worker".into())
        .spawn(move || worker(rx, worker_shared))
        .map_err(|error| internal(format!("the DFM worker could not start: {error}")))?;
    *slot = Some(Service { tx: tx.clone(), shared: Arc::clone(&shared) });
    Ok((tx, shared))
}

fn worker(rx: Receiver<Job>, shared: Arc<Shared>) {
    while let Ok(job) = rx.recv() {
        shared.running.store(true, Ordering::Release);
        shared.cancel.store(false, Ordering::Release);
        if let Ok(mut revision) = shared.started_revision.lock() {
            *revision = Some(job.request.candidate.source.revision.clone());
        }
        if let Ok(mut error) = shared.error.lock() {
            *error = None;
        }
        let result = run_job(&job, &shared);
        let last = match result {
            Ok(result) => Last { run_id: result.run_id.clone(), outcome: result.outcome, reason: result.reason },
            Err(message) => {
                if let Ok(mut error) = shared.error.lock() {
                    *error = Some(message.clone());
                }
                Last { run_id: String::new(), outcome: RunOutcome::Failed, reason: Some(message) }
            }
        };
        if let Ok(mut last_slot) = shared.last.lock() {
            *last_slot = Some(last);
        }
        if let Ok(mut active) = shared.active.lock() {
            *active = None;
        }
        if let Ok(mut revision) = shared.started_revision.lock() {
            *revision = None;
        }
        shared.running.store(false, Ordering::Release);
    }
}

fn run_job(job: &Job, shared: &Arc<Shared>) -> Result<RunResult, String> {
    let port = match shared.port.lock() {
        Ok(mut slot) => match slot.as_ref() {
            Some(port) => Arc::clone(port),
            None => {
                let opened = Arc::new(open_port(job.candidates.clone(), job.store.clone())?);
                *slot = Some(Arc::clone(&opened));
                opened
            }
        },
        Err(_) => return Err("the DFM service port lock was poisoned".into()),
    };
    let shared_for_run = Arc::clone(shared);
    let on_submitted = move |run_id: &str| {
        if let Ok(mut active) = shared_for_run.active.lock() {
            *active = Some(run_id.to_string());
        }
    };
    run_once_reserved(&port, &job.request, &shared.cancel, &on_submitted, job.reservation.as_ref()).map_err(|error| error.to_string())
}

/// The validation store root a check uses unless a host chooses another.
pub fn validation_store_root() -> Option<PathBuf> {
    if let Some(root) = HOST_STORE_ROOT.lock().ok().and_then(|root| root.clone()) {
        return Some(root);
    }
    std::env::var_os("SOLIDCRAFT_VALIDATION_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::recovery::data_dir().map(|dir| dir.join("validation")))
}

static HOST_STORE_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Choose the validation store for this process (`None`: the default location). Hosts and tests
/// use it instead of the environment variable; it is not part of a design.
pub fn set_store_root(root: Option<PathBuf>) {
    if let Ok(mut slot) = HOST_STORE_ROOT.lock() {
        *slot = root;
    }
}

fn open_port(candidates: Option<PathBuf>, store: Option<PathBuf>) -> Result<ValidationPort, String> {
    let candidates = candidates.or_else(crate::cmd::candidate_store_root).ok_or_else(|| "this build has no candidate store".to_string())?;
    let store = store.or_else(validation_store_root).ok_or_else(|| "this build has no app data folder for checks".to_string())?;
    ValidationPort::open(store, candidates).map_err(|error| error.to_string())
}

fn internal(message: impl Into<String>) -> EngineError {
    EngineError::Internal("dfm service".into(), message.into())
}

fn open_project(session: &Session) -> Result<Project, EngineError> {
    let path = session.path.as_deref().ok_or_else(|| internal("the open design is not saved in a project"))?;
    Project::of(std::path::Path::new(path))
        .map_err(|error| internal(error.to_string()))?
        .ok_or_else(|| internal("the open design is not in a project; make or open a project first"))
}

/// Capture the snapshot and freeze the project in one operation, then queue a free check.
pub fn start(session: &mut Session) -> Result<CheckTicket, EngineError> {
    start_with_reservation(session, None)
}

/// Queue one controlled Review check under the host's exact reservation. The host persists the
/// run-to-operation mapping before calling; the worker submits and claims only that run.
pub fn start_reserved(session: &mut Session, reservation: Reservation) -> Result<CheckTicket, EngineError> {
    if reservation.run_id.is_empty() || reservation.operation_id.is_empty() {
        return Err(internal("the controlled Review reservation has no run or operation identity"));
    }
    if crate::validation::now_ms() >= reservation.stop_at_ms {
        return Err(internal("the controlled Review safe-stop bound has already passed"));
    }
    start_with_reservation(session, Some(reservation))
}

fn start_with_reservation(session: &mut Session, reservation: Option<Reservation>) -> Result<CheckTicket, EngineError> {
    let project = open_project(session)?;
    let frozen = project.freeze().map_err(|error| internal(error.to_string()))?;
    let captured = session.execute("document.snapshot.capture", &json!({}))?;
    let candidate: CandidateRef =
        serde_json::from_value(captured.get("candidate").cloned().ok_or_else(|| internal("the snapshot reply has no candidate"))?)
            .map_err(|error| internal(format!("the snapshot reply is malformed: {error}")))?;
    let sheets: Vec<SheetRef> = session
        .model
        .state()
        .sheets
        .iter()
        .enumerate()
        .map(|(index, sheet)| SheetRef { sheet: index, body: sheet.body.clone(), thickness_mm: Some(sheet.t), converted: false })
        .collect();
    let input = RunInput::from_frozen(&frozen, "native");
    let candidate_digest = candidate.candidate_digest.clone();
    let request = RunRequest {
        project_root: project.root().to_path_buf(),
        candidate,
        input,
        sheets: sheets.clone(),
        started_at: timestamp(crate::validation::now_ms()),
    };
    let (tx, shared) = service()?;
    let replaces_running = shared.running.load(Ordering::Acquire);
    if replaces_running {
        cancel_shared(&shared);
    }
    tx.send(Job { request, candidates: crate::cmd::candidate_store_root(), store: validation_store_root(), reservation })
        .map_err(|_| internal("the DFM worker is not running"))?;
    Ok(CheckTicket {
        project_id: frozen.project_id,
        project_name: frozen.project_name,
        candidate_digest,
        config_digest: frozen.config_digest,
        sheets: sheets.len(),
        replaces_running,
    })
}

fn cancel_shared(shared: &Shared) {
    shared.cancel.store(true, Ordering::Release);
    let active = shared.active.lock().ok().and_then(|active| active.clone());
    let port = shared.port.lock().ok().and_then(|port| port.clone());
    if let (Some(run_id), Some(port)) = (active, port) {
        let _ = port.cancel(&run_id);
    }
}

/// Cancel the live check. Its record, if it commits, is `cancelled` and never becomes latest.
pub fn cancel() -> bool {
    let Ok(slot) = slot().lock() else { return false };
    let Some(service) = slot.as_ref() else { return false };
    let running = service.shared.running.load(Ordering::Acquire);
    cancel_shared(&service.shared);
    running
}

/// A person's edit or undo: stop a live check whose captured revision is no longer the model.
/// Cheap enough for every frame: it only reads the service state and the document revision.
pub fn cancel_if_stale(session: &mut Session) {
    let Some(shared) = slot().lock().ok().and_then(|slot| slot.as_ref().map(|service| Arc::clone(&service.shared))) else { return };
    if !shared.running.load(Ordering::Acquire) {
        return;
    }
    let current = session.document_stamp().revision.to_string();
    let stale = shared.started_revision.lock().ok().is_some_and(|started| started.as_deref().is_some_and(|started| started != current));
    if stale {
        cancel_shared(&shared);
    }
}

/// The live job's state, for a running/cancel/error entry in the window.
pub fn status() -> ServiceStatus {
    let empty = || ServiceStatus { running: false, active_run: None, last_run: None, last_outcome: None, last_reason: None, error: None };
    let shared = match slot().lock() {
        Ok(slot) => match slot.as_ref() {
            Some(service) => Arc::clone(&service.shared),
            None => return empty(),
        },
        Err(_) => return empty(),
    };
    let active_run = shared.active.lock().ok().and_then(|active| active.clone());
    let last = shared.last.lock().ok().and_then(|last| last.as_ref().map(|last| (last.run_id.clone(), last.outcome, last.reason.clone())));
    ServiceStatus {
        running: shared.running.load(Ordering::Acquire),
        active_run,
        last_run: last.as_ref().map(|last| last.0.clone()).filter(|run| !run.is_empty()),
        last_outcome: last.as_ref().map(|last| outcome_name(last.1).to_string()),
        last_reason: last.and_then(|last| last.2),
        error: shared.error.lock().ok().and_then(|error| error.clone()),
    }
}

fn outcome_name(outcome: RunOutcome) -> &'static str {
    match outcome {
        RunOutcome::Current => "current",
        RunOutcome::Superseded => "superseded",
        RunOutcome::Cancelled => "cancelled",
        RunOutcome::Failed => "failed",
    }
}

/// Read one run's record (the newest by default) against the live model and configuration.
pub fn inspect(session: &mut Session, run: Option<&str>, model_digest: Option<&str>) -> Result<Inspection, EngineError> {
    let project = open_project(session)?;
    let store = RunStore::new(project.root());
    let live_config = project.freeze().ok().map(|frozen| frozen.config_digest);
    let latest = store.latest().map_err(|error| internal(error.to_string()))?;
    let run_id = match run {
        Some(run) => Some(run.to_string()),
        None => latest.as_ref().map(|latest| latest.run_id.clone()),
    };
    let Some(run_id) = run_id else {
        return Ok(Inspection {
            available: false,
            running: status().running,
            run_id: None,
            outcome: None,
            reason: Some("no DFM check has finished for this project yet".into()),
            model_version: None,
            candidate_digest: None,
            config_digest: None,
            freshness: None,
            findings: None,
            summary: None,
            source_revision: None,
        });
    };
    let meta = store.load_meta(&run_id).map_err(|error| internal(error.to_string()))?;
    let written = store.load_findings(&run_id).map_err(|error| internal(error.to_string()))?;
    let summary = store.load_summary(&run_id).ok();
    let current_revision = session.document_stamp().revision.to_string();
    let digest_owned = model_digest.map(str::to_string);
    let current_model: Option<&str> = digest_owned.as_deref().or_else(|| {
        // The snapshot revision is the cheap in-session identity; it only compares with itself.
        (!meta.source_revision.is_empty() && meta.source_revision == current_revision).then_some(meta.candidate_digest.as_str())
    });
    let fresh = freshness(&meta, current_model, live_config.as_deref());
    let findings_value = serde_json::to_value(&written).map_err(|error| internal(error.to_string()))?;
    Ok(Inspection {
        available: true,
        running: status().running,
        run_id: Some(run_id),
        outcome: Some(outcome_name(meta.outcome).to_string()),
        reason: meta.reason,
        model_version: Some(meta.model_version),
        candidate_digest: Some(meta.candidate_digest),
        config_digest: Some(meta.config_digest),
        freshness: Some(freshness_name(fresh).to_string()),
        findings: Some(findings_value),
        summary,
        source_revision: (!meta.source_revision.is_empty()).then_some(meta.source_revision),
    })
}

fn freshness_name(fresh: Freshness) -> &'static str {
    match fresh {
        Freshness::Current => "current",
        Freshness::StaleModel => "staleModel",
        Freshness::StaleConfig => "staleConfig",
        Freshness::StaleBoth => "staleBoth",
        Freshness::Unknown => "unknown",
    }
}

/// One run's findings, unchanged, for a caller that already knows its id.
/// One run's findings, unchanged, for a caller that already knows its id.
pub fn findings_for(session: &Session, run: &str) -> Result<crate::dfm::findings::RunFindings, EngineError> {
    let project = open_project(session)?;
    RunStore::new(project.root()).load_findings(run).map_err(|error| internal(error.to_string()))
}
