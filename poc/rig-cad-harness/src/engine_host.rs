//! Real CAD host: the solidcraft engine on a dedicated worker thread.
//!
//! The engine is never touched from the Rig/UI thread. Every write is checked
//! against a pinned revision, wrapped in a session checkpoint and rolled back
//! on failure or cancellation. Validation runs the real `ValidationPort` on
//! the captured candidate. This is the acceptance path; the mock host is only
//! for the state-machine tests.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use solidcraft_engine::schema::snapshot::{CandidateArtifact, CandidateRef, SnapshotSource};
use solidcraft_engine::validation::{
    CheckProfile, MeasurementRequirement, ParameterRequirement, ValidationPort, generate_submission_key, native_basic_profile, task_profile,
};
use solidcraft_engine::{ConditionedError, Session};

use crate::scenario;
use crate::types::*;

#[derive(Clone, Debug)]
pub enum EnginePermission {
    Auto,
    Ask(VecDeque<PermissionDecision>),
    AllowList(Vec<String>),
    DenyAll,
}

struct EngineState {
    permits: HashSet<String>,
    permission: EnginePermission,
    permission_requests: Vec<PermissionRequest>,
    applied: std::collections::HashMap<String, WriteOutcome>,
}

pub struct EngineCadHost {
    tx: Option<Sender<EngineCall>>,
    join: Mutex<Option<JoinHandle<()>>>,
    cancel: Arc<AtomicBool>,
    write_started: Arc<(Mutex<bool>, Condvar)>,
    state: Mutex<EngineState>,
}

enum EngineCall {
    Document(Sender<Result<DocumentStamp, HostError>>),
    Inspect(Sender<Result<InspectView, HostError>>),
    Capture { expected: DocumentStamp, reply: Sender<Result<SnapshotRef, HostError>> },
    Write { expected: DocumentStamp, ops: Vec<CadOp>, cancel: Arc<AtomicBool>, reply: Sender<Result<WorkerWrite, String>> },
    Undo { expected: DocumentStamp, reply: Sender<Result<WorkerWrite, String>> },
    Validate { request: Box<ValidationRequest>, reply: Sender<Result<ValidationReport, String>> },
}

enum WorkerWrite {
    Applied { before: DocumentStamp, after: DocumentStamp, applied_ops: usize },
    Failed { reason: String },
    Stale { actual: DocumentStamp },
    Cancelled { reason: String },
    Unknown { message: String },
}

impl EngineCadHost {
    /// Start a real engine session with the parametric L-bracket document.
    /// `root` holds the candidate and validation stores for this spike.
    pub fn l_bracket(root: impl AsRef<Path>) -> Result<Arc<Self>, String> {
        let root = root.as_ref().to_path_buf();
        let candidate_root = root.join("candidates");
        let validation_root = root.join("validation");
        std::fs::create_dir_all(&candidate_root).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&validation_root).map_err(|error| error.to_string())?;
        solidcraft_engine::cmd::set_candidate_store(Some(candidate_root.clone()));

        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("cad-engine-host".into())
            .spawn(move || worker(rx, ready_tx, candidate_root, validation_root))
            .map_err(|error| error.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err("engine worker stopped before it was ready".into()),
        }
        Ok(Arc::new(Self {
            tx: Some(tx),
            join: Mutex::new(Some(join)),
            cancel: Arc::new(AtomicBool::new(false)),
            write_started: Arc::new((Mutex::new(false), Condvar::new())),
            state: Mutex::new(EngineState {
                permits: HashSet::new(),
                permission: EnginePermission::Auto,
                permission_requests: Vec::new(),
                applied: std::collections::HashMap::new(),
            }),
        }))
    }

    pub fn set_permission(&self, permission: EnginePermission) {
        self.lock().permission = permission;
    }

    pub fn permission_requests(&self) -> Vec<PermissionRequest> {
        self.lock().permission_requests.clone()
    }

    pub fn clear_cancel(&self) {
        self.cancel.store(false, Ordering::SeqCst);
    }

    /// Wait until a write has been sent to the engine worker.
    pub fn wait_until_write_started(&self, timeout: std::time::Duration) -> bool {
        let (lock, cvar) = &*self.write_started;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let (guard, _) = cvar.wait_timeout_while(guard, timeout, |started| !*started).unwrap_or_else(PoisonError::into_inner);
        *guard
    }

    fn lock(&self) -> MutexGuard<'_, EngineState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, call: EngineCall) -> Result<(), HostError> {
        self.tx.as_ref().ok_or_else(|| HostError::Unavailable("engine worker is gone".into()))?.send(call).map_err(|_| HostError::Unavailable("engine worker stopped".into()))
    }

    fn permit_ok(&self, permit: Option<&WritePermit>) -> bool {
        permit.is_some_and(|permit| self.lock().permits.contains(&permit.key()))
    }

    fn permission(&self, request: &PermissionRequest) -> PermissionDecision {
        let mut state = self.lock();
        state.permission_requests.push(request.clone());
        match &mut state.permission {
            EnginePermission::Auto => PermissionDecision::Allow,
            EnginePermission::Ask(queue) => queue.pop_front().unwrap_or_else(|| PermissionDecision::Deny("REAL: approval expired".into())),
            EnginePermission::AllowList(commands) => {
                if commands.iter().any(|command| request.command.starts_with(command)) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Deny(format!("REAL: `{}` is not on the allow list", request.command))
                }
            }
            EnginePermission::DenyAll => PermissionDecision::Deny("REAL: the user denied every write".into()),
        }
    }
}

impl Drop for EngineCadHost {
    fn drop(&mut self) {
        self.tx.take();
        if let Ok(mut join) = self.join.lock()
            && let Some(handle) = join.take()
        {
            let _ = handle.join();
        }
    }
}

impl CadHost for EngineCadHost {
    fn document(&self) -> Result<DocumentStamp, HostError> {
        let (tx, rx) = mpsc::channel();
        self.send(EngineCall::Document(tx))?;
        rx.recv().map_err(|_| HostError::Unavailable("engine worker stopped".into()))?
    }

    fn inspect(&self) -> Result<InspectView, HostError> {
        let (tx, rx) = mpsc::channel();
        self.send(EngineCall::Inspect(tx))?;
        rx.recv().map_err(|_| HostError::Unavailable("engine worker stopped".into()))?
    }

    fn capture_snapshot(&self, expected: &DocumentStamp) -> Result<SnapshotRef, HostError> {
        if self.is_cancelled() {
            return Err(HostError::Invalid("cancelled".into()));
        }
        let (tx, rx) = mpsc::channel();
        self.send(EngineCall::Capture { expected: expected.clone(), reply: tx })?;
        rx.recv().map_err(|_| HostError::Unavailable("engine worker stopped".into()))?
    }

    fn dispatch_write(&self, request: WriteRequest) -> Result<WriteOutcome, HostError> {
        if !self.permit_ok(request.permit.as_ref()) {
            return Ok(WriteOutcome::Denied { reason: "write has no active task permit (external bypass refused)".into() });
        }
        if self.is_cancelled() {
            return Ok(WriteOutcome::Cancelled { reason: "cancelled before the write".into() });
        }
        if let Some(previous) = self.lock().applied.get(&request.write_id).cloned() {
            return Ok(match previous {
                WriteOutcome::Applied { write_id, before, after, applied_ops, .. } => {
                    WriteOutcome::Applied { write_id, before, after, applied_ops, replayed: true }
                }
                other => other,
            });
        }
        let command = request.ops.first().map(|op| op.command.clone()).unwrap_or_default();
        if let PermissionDecision::Deny(reason) =
            self.permission(&PermissionRequest {
                task_id: request.task_id.clone(),
                run_id: request.run_id.clone(),
                tool: "cad_apply".into(),
                command: command.clone(),
                summary: format!("apply {} op(s) on {}", request.ops.len(), request.expected.document_id),
            })
        {
            return Ok(WriteOutcome::Denied { reason });
        }
        let (lock, cvar) = &*self.write_started;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
        cvar.notify_all();
        let (tx, rx) = mpsc::channel();
        self.send(EngineCall::Write {
            expected: request.expected.clone(),
            ops: request.ops.clone(),
            cancel: Arc::clone(&self.cancel),
            reply: tx,
        })?;
        let outcome = match rx.recv() {
            Ok(Ok(WorkerWrite::Applied { before, after, applied_ops })) => {
                WriteOutcome::Applied { write_id: request.write_id.clone(), before, after, applied_ops, replayed: false }
            }
            Ok(Ok(WorkerWrite::Failed { reason })) => WriteOutcome::Failed { reason, unchanged: true },
            Ok(Ok(WorkerWrite::Stale { actual })) => WriteOutcome::Stale { expected: request.expected.clone(), actual },
            Ok(Ok(WorkerWrite::Cancelled { reason })) => WriteOutcome::Cancelled { reason },
            Ok(Ok(WorkerWrite::Unknown { message })) => {
                WriteOutcome::Unknown { write_id: request.write_id.clone(), expected: request.expected.clone(), message }
            }
            Ok(Err(error)) => WriteOutcome::Failed { reason: error, unchanged: false },
            Err(_) => WriteOutcome::Unknown {
                write_id: request.write_id.clone(),
                expected: request.expected.clone(),
                message: "the engine worker stopped after the write was sent".into(),
            },
        };
        if matches!(outcome, WriteOutcome::Applied { .. } | WriteOutcome::Unknown { .. }) {
            self.lock().applied.insert(request.write_id, outcome.clone());
        }
        Ok(outcome)
    }

    fn undo(&self, request: UndoRequest) -> Result<WriteOutcome, HostError> {
        if !self.permit_ok(request.permit.as_ref()) {
            return Ok(WriteOutcome::Denied { reason: "undo has no active task permit".into() });
        }
        if self.is_cancelled() {
            return Ok(WriteOutcome::Cancelled { reason: "cancelled before undo".into() });
        }
        let (tx, rx) = mpsc::channel();
        self.send(EngineCall::Undo { expected: request.expected.clone(), reply: tx })?;
        let outcome = match rx.recv() {
            Ok(Ok(WorkerWrite::Applied { before, after, applied_ops })) => {
                WriteOutcome::Applied { write_id: request.write_id, before, after, applied_ops, replayed: false }
            }
            Ok(Ok(WorkerWrite::Failed { reason })) => WriteOutcome::Failed { reason, unchanged: true },
            Ok(Ok(WorkerWrite::Stale { actual })) => WriteOutcome::Stale { expected: request.expected, actual },
            Ok(Ok(WorkerWrite::Cancelled { reason })) => WriteOutcome::Cancelled { reason },
            Ok(Ok(WorkerWrite::Unknown { message })) => {
                WriteOutcome::Unknown { write_id: "undo".into(), expected: request.expected, message }
            }
            Ok(Err(error)) => WriteOutcome::Failed { reason: error, unchanged: false },
            Err(_) => WriteOutcome::Unknown { write_id: "undo".into(), expected: request.expected, message: "engine worker stopped".into() },
        };
        Ok(outcome)
    }

    fn validate(&self, request: ValidationRequest) -> Result<ValidationReport, HostError> {
        if self.is_cancelled() {
            return Ok(ValidationReport {
                run_id: "cancelled".into(),
                candidate_id: request.snapshot.candidate_id.clone(),
                candidate_digest: request.snapshot.candidate_digest.clone(),
                profile_id: request.profile.profile_id.clone(),
                profile_version: request.profile.version.clone(),
                state: "cancelled".into(),
                checks: vec![],
                error: Some("cancelled".into()),
            });
        }
        let (tx, rx) = mpsc::channel();
        self.send(EngineCall::Validate { request: Box::new(request), reply: tx })?;
        match rx.recv() {
            Ok(Ok(report)) => Ok(report),
            Ok(Err(error)) => Err(HostError::Invalid(error)),
            Err(_) => Err(HostError::Unavailable("engine worker stopped".into())),
        }
    }

    fn reconcile(&self, pending: &PendingWrite) -> Result<ReconcileOutcome, HostError> {
        if let Some(WriteOutcome::Applied { after, .. }) = self.lock().applied.get(&pending.write_id) {
            return Ok(ReconcileOutcome::Applied { write_id: pending.write_id.clone(), after: after.clone() });
        }
        let current = self.document()?;
        if current.revision == pending.expected.revision {
            Ok(ReconcileOutcome::NotApplied { write_id: pending.write_id.clone(), current })
        } else {
            Ok(ReconcileOutcome::Unknown {
                write_id: pending.write_id.clone(),
                message: format!("REAL: revision moved from {} to {} and no applied write is recorded", pending.expected.revision, current.revision),
            })
        }
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.permission(request)
    }

    fn register_permit(&self, permit: &WritePermit) -> Result<(), HostError> {
        self.lock().permits.insert(permit.key());
        Ok(())
    }

    fn revoke_permit(&self, permit: &WritePermit) -> Result<(), HostError> {
        self.lock().permits.remove(&permit.key());
        Ok(())
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

fn worker(rx: Receiver<EngineCall>, ready: Sender<Result<(), String>>, candidate_root: PathBuf, validation_root: PathBuf) {
    let mut session = Session::default();
    if let Err(error) = session.run_script(&scenario::l_bracket_script()) {
        let _ = ready.send(Err(format!("L-bracket script: {error}")));
        return;
    }
    let port = match ValidationPort::open(validation_root, candidate_root) {
        Ok(port) => port,
        Err(error) => {
            let _ = ready.send(Err(format!("validation port: {error}")));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    for call in rx {
        match call {
            EngineCall::Document(reply) => {
                let _ = reply.send(Ok(our_stamp(&session.document_stamp())));
            }
            EngineCall::Inspect(reply) => {
                let _ = reply.send(inspect(&mut session));
            }
            EngineCall::Capture { expected, reply } => {
                let _ = reply.send(capture(&mut session, &expected));
            }
            EngineCall::Write { expected, ops, cancel, reply } => {
                let _ = reply.send(write(&mut session, &expected, &ops, &cancel).map_err(|error| error.to_string()));
            }
            EngineCall::Undo { expected, reply } => {
                let _ = reply.send(undo(&mut session, &expected).map_err(|error| error.to_string()));
            }
            EngineCall::Validate { request, reply } => {
                let _ = reply.send(validate(&port, &request));
            }
        }
    }
}

fn inspect(session: &mut Session) -> Result<InspectView, HostError> {
    let document = our_stamp(&session.document_stamp());
    let measure = session.execute("inspect.measure", &json!({})).map_err(|error| HostError::Invalid(error.to_string()))?;
    let body_count = measure.get("body_count").and_then(Value::as_u64).unwrap_or(0) as usize;
    let volume_mm3 = measure.get("total").and_then(|total| total.get("volume_mm3")).and_then(Value::as_f64);
    let parameters = session.execute("parameters.list", &json!({})).unwrap_or(Value::Null);
    let hole_count = session
        .execute("model.faces", &json!({"body": "Bracket"}))
        .ok()
        .and_then(|faces| faces.get("faces").and_then(Value::as_array).map(|faces| {
            faces
                .iter()
                .filter(|face| {
                    face.get("cylinder")
                        .and_then(|cylinder| cylinder.get("radius"))
                        .and_then(Value::as_f64)
                        .is_some_and(|radius| (radius - 2.0).abs() < 1e-3)
                })
                .count()
        }));
    Ok(InspectView {
        document: document.clone(),
        body_count,
        volume_mm3,
        hole_count,
        parameters,
        summary: format!(
            "REAL ENGINE: document {} revision {}, bodies {}, volume {:?} mm3, mounting holes {:?}",
            document.document_id, document.revision, body_count, volume_mm3, hole_count
        ),
    })
}

fn capture(session: &mut Session, expected: &DocumentStamp) -> Result<SnapshotRef, HostError> {
    let expected_engine = engine_stamp(expected);
    let (reply, _) = session
        .execute_if_current("document.snapshot.capture", &json!({}), &expected_engine)
        .map_err(|error| match error {
            ConditionedError::Changed { expected, current } => HostError::Stale { expected: our_stamp(&expected), actual: our_stamp(&current) },
            ConditionedError::Command(error) => HostError::Invalid(error.to_string()),
        })?;
    let candidate = reply.get("candidate").ok_or_else(|| HostError::Invalid("snapshot reply has no candidate".into()))?;
    let source = candidate.get("source").ok_or_else(|| HostError::Invalid("candidate has no source".into()))?;
    let artifact = candidate.get("artifact").ok_or_else(|| HostError::Invalid("candidate has no artifact".into()))?;
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    Ok(SnapshotRef {
        candidate_id: text(candidate, "candidateId").ok_or_else(|| HostError::Invalid("candidateId".into()))?,
        source: DocumentStamp::new(
            text(source, "documentId").ok_or_else(|| HostError::Invalid("source.documentId".into()))?,
            text(source, "revision").and_then(|revision| revision.parse().ok()).unwrap_or(0),
        ),
        candidate_digest: text(candidate, "candidateDigest").ok_or_else(|| HostError::Invalid("candidateDigest".into()))?,
        manifest_digest: text(candidate, "manifestDigest").ok_or_else(|| HostError::Invalid("manifestDigest".into()))?,
        artifact_id: text(artifact, "artifactId").ok_or_else(|| HostError::Invalid("artifact.artifactId".into()))?,
        artifact_digest: text(artifact, "digest").ok_or_else(|| HostError::Invalid("artifact.digest".into()))?,
        size_bytes: artifact.get("sizeBytes").and_then(Value::as_u64).ok_or_else(|| HostError::Invalid("artifact.sizeBytes".into()))?,
        media_type: text(artifact, "mediaType").ok_or_else(|| HostError::Invalid("artifact.mediaType".into()))?,
    })
}

fn engine_stamp(stamp: &DocumentStamp) -> solidcraft_engine::DocumentStamp {
    solidcraft_engine::DocumentStamp { document_id: stamp.document_id.clone(), revision: stamp.revision }
}

fn our_stamp(stamp: &solidcraft_engine::DocumentStamp) -> DocumentStamp {
    DocumentStamp::new(stamp.document_id.clone(), stamp.revision)
}

fn write(session: &mut Session, expected: &DocumentStamp, ops: &[CadOp], cancel: &AtomicBool) -> Result<WorkerWrite, HostError> {
    let expected_engine = engine_stamp(expected);
    if let Err(error) = session.check_document_stamp(&expected_engine) {
        return Ok(match error {
            ConditionedError::Changed { current, .. } => WorkerWrite::Stale { actual: our_stamp(&current) },
            ConditionedError::Command(error) => WorkerWrite::Failed { reason: error.to_string() },
        });
    }
    let checkpoint = session.checkpoint();
    for op in ops {
        if cancel.load(Ordering::SeqCst) {
            return rollback(session, checkpoint, WorkerWrite::Cancelled { reason: "cancelled while the write was in flight; rolled back".into() });
        }
        if let Err(error) = session.execute(&op.command, &op.params) {
            return rollback(session, checkpoint, WorkerWrite::Failed { reason: error.to_string() });
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return rollback(session, checkpoint, WorkerWrite::Cancelled { reason: "cancelled before commit; rolled back".into() });
    }
    Ok(WorkerWrite::Applied { before: expected.clone(), after: our_stamp(&session.document_stamp()), applied_ops: ops.len() })
}

fn rollback(session: &mut Session, checkpoint: u64, outcome: WorkerWrite) -> Result<WorkerWrite, HostError> {
    match session.restore_checkpoint(checkpoint) {
        Ok(()) => Ok(outcome),
        Err(error) => Ok(WorkerWrite::Unknown { message: format!("rollback failed: {error}") }),
    }
}

fn undo(session: &mut Session, expected: &DocumentStamp) -> Result<WorkerWrite, HostError> {
    let expected_engine = engine_stamp(expected);
    if let Err(error) = session.check_document_stamp(&expected_engine) {
        return Ok(match error {
            ConditionedError::Changed { current, .. } => WorkerWrite::Stale { actual: our_stamp(&current) },
            ConditionedError::Command(error) => WorkerWrite::Failed { reason: error.to_string() },
        });
    }
    match session.execute("edit.undo", &json!({})) {
        Ok(_) => Ok(WorkerWrite::Applied { before: expected.clone(), after: our_stamp(&session.document_stamp()), applied_ops: 1 }),
        Err(error) => Ok(WorkerWrite::Failed { reason: error.to_string() }),
    }
}

fn validate(port: &ValidationPort, request: &ValidationRequest) -> Result<ValidationReport, String> {
    let candidate = candidate_ref(&request.snapshot)?;
    let profile = profile_for(&request.profile)?;
    let reference = port.register_profile(profile).map_err(|error| error.to_string())?;
    let key = generate_submission_key(now_ms());
    let submitted = port.submit_for(&candidate, &reference, &key, &request.task_id).map_err(|error| error.to_string())?;
    port.pump().map_err(|error| error.to_string())?;
    let result = port.report(&submitted.run_id).map_err(|error| error.to_string())?;
    Ok(map_report(result.report))
}

fn candidate_ref(snapshot: &SnapshotRef) -> Result<CandidateRef, String> {
    Ok(CandidateRef {
        candidate_id: snapshot.candidate_id.clone(),
        source: SnapshotSource { document_id: snapshot.source.document_id.clone(), revision: snapshot.source.revision.to_string() },
        candidate_digest: snapshot.candidate_digest.clone(),
        manifest_digest: snapshot.manifest_digest.clone(),
        artifact: CandidateArtifact {
            artifact_id: snapshot.artifact_id.clone(),
            digest: snapshot.artifact_digest.clone(),
            size_bytes: snapshot.size_bytes,
            media_type: snapshot.media_type.clone(),
        },
    })
}

fn profile_for(profile: &ValidationProfile) -> Result<CheckProfile, String> {
    if profile.profile_id == "native-basic" {
        return Ok(native_basic_profile());
    }
    if profile.profile_id == "l-bracket-task" {
        let parameters = vec![
            ParameterRequirement { name: "thickness".into(), value: 8.0, abs_tolerance: 1e-6 },
            ParameterRequirement { name: "hole_diameter".into(), value: 4.0, abs_tolerance: 1e-6 },
        ];
        let measurements = vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: scenario::expected_l_bracket_volume(8.0, 4.0),
            abs_tolerance: 5.0,
            rel_tolerance: 0.01,
        }];
        return task_profile(&profile.profile_id, &profile.version, Some(parameters), Some(measurements)).map_err(|error| error.to_string());
    }
    Err(format!("PROFILE_UNSUPPORTED: {} {}", profile.profile_id, profile.version))
}

fn map_report(report: solidcraft_engine::validation::ValidationReport) -> ValidationReport {
    ValidationReport {
        run_id: report.run_id,
        candidate_id: report.candidate_id,
        candidate_digest: report.candidate_digest,
        profile_id: report.profile.profile_id,
        profile_version: report.profile.version,
        state: report.state.as_str().into(),
        checks: report
            .checks
            .into_iter()
            .map(|check| CheckResult {
                check_id: check.check_id,
                required: check.required,
                state: check.state.as_str().into(),
                expected: check.expected,
                actual: check.actual,
            })
            .collect(),
        error: report.error.map(|error| format!("{error:?}")),
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}
