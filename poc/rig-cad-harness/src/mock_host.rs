//! In-memory mock of the CAD host, for the task/tool state-machine tests and
//! fault injection. Every result this host produces is labelled `mock` in the
//! task result/CLI; it is not CAD evidence.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use crate::types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    Apply,
    StaleOnce,
    FailOnce,
    UnknownOnce,
    PartialOnce,
    CancelDuringWrite,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationMode {
    Pass,
    FailOnce,
    FailAlways,
    Infrastructure,
}

#[derive(Clone, Debug)]
pub enum PermissionMode {
    Auto,
    Ask(VecDeque<PermissionDecision>),
    AllowList(Vec<String>),
    DenyAll,
}

struct MockState {
    document_id: String,
    revision: u64,
    thickness: f64,
    hole_diameter: f64,
    hole_count: usize,
    parameters: Value,
    write_mode: WriteMode,
    validation_mode: ValidationMode,
    permission_mode: PermissionMode,
    permits: HashSet<String>,
    permission_requests: Vec<PermissionRequest>,
    applied: HashMap<String, WriteOutcome>,
    applied_after: HashMap<String, DocumentStamp>,
    snapshots: HashMap<String, SnapshotRef>,
    undo: Vec<MockUndo>,
    validation_runs: u64,
    write_history: Vec<(String, Vec<CadOp>)>,
}

#[derive(Clone)]
struct MockUndo {
    thickness: f64,
    hole_diameter: f64,
    hole_count: usize,
    parameters: Value,
    revision: u64,
}

pub struct MockHost {
    state: Mutex<MockState>,
    cancel: AtomicBool,
    started: Arc<(Mutex<bool>, Condvar)>,
    candidate_seq: AtomicU64,
}

impl MockHost {
    pub fn l_bracket() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(MockState {
                document_id: "doc-l-bracket".into(),
                revision: 1,
                thickness: 5.0,
                hole_diameter: 4.0,
                hole_count: 4,
                parameters: json!({"thickness": {"expression": "5 mm", "value_mm": 5.0}, "hole_diameter": {"expression": "4 mm", "value_mm": 4.0}, "width": {"expression": "60 mm", "value_mm": 60.0}}),
                write_mode: WriteMode::Apply,
                validation_mode: ValidationMode::Pass,
                permission_mode: PermissionMode::Auto,
                permits: HashSet::new(),
                permission_requests: Vec::new(),
                applied: HashMap::new(),
                applied_after: HashMap::new(),
                snapshots: HashMap::new(),
                undo: Vec::new(),
                validation_runs: 0,
                write_history: Vec::new(),
            }),
            cancel: AtomicBool::new(false),
            started: Arc::new((Mutex::new(false), Condvar::new())),
            candidate_seq: AtomicU64::new(0),
        })
    }

    fn lock(&self) -> MutexGuard<'_, MockState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_write_mode(&self, mode: WriteMode) {
        self.lock().write_mode = mode;
    }

    pub fn set_validation_mode(&self, mode: ValidationMode) {
        self.lock().validation_mode = mode;
    }

    pub fn set_permission_mode(&self, mode: PermissionMode) {
        self.lock().permission_mode = mode;
    }

    pub fn clear_cancel(&self) {
        self.cancel.store(false, Ordering::SeqCst);
    }

    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    pub fn thickness(&self) -> f64 {
        self.lock().thickness
    }

    pub fn hole_count(&self) -> usize {
        self.lock().hole_count
    }

    pub fn permission_requests(&self) -> Vec<PermissionRequest> {
        self.lock().permission_requests.clone()
    }

    pub fn applied_writes(&self) -> Vec<(String, Vec<CadOp>)> {
        self.lock().write_history.clone()
    }

    /// Wait until a `CancelDuringWrite` write has started, for the cancel test.
    pub fn wait_until_write_started(&self, timeout: Duration) -> bool {
        let (lock, cvar) = &*self.started;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let (guard, _) = cvar.wait_timeout_while(guard, timeout, |started| !*started).unwrap_or_else(PoisonError::into_inner);
        *guard
    }

    fn apply_ops(state: &mut MockState, ops: &[CadOp]) -> Result<usize, String> {
        let mut count = 0;
        for op in ops {
            match op.command.as_str() {
                "parameters.change" => {
                    let name = op.params.get("name").and_then(Value::as_str).ok_or_else(|| "parameters.change needs name".to_string())?;
                    let expression = op.params.get("expression").and_then(Value::as_str).ok_or_else(|| "parameters.change needs expression".to_string())?;
                    let value = parse_mm(expression).ok_or_else(|| format!("parameters.change: `{expression}` is not a length"))?;
                    match name {
                        "thickness" => state.thickness = value,
                        "hole_diameter" => state.hole_diameter = value,
                        other => {
                            if let Some(object) = state.parameters.as_object_mut() {
                                object.insert(other.to_string(), json!({"expression": expression, "value_mm": value}));
                            }
                        }
                    }
                    if let Some(object) = state.parameters.as_object_mut() {
                        object.insert(name.to_string(), json!({"expression": expression, "value_mm": value}));
                    }
                    count += 1;
                }
                other => return Err(format!("mock host refuses `{other}`: only parameters.change is modelled")),
            }
        }
        Ok(count)
    }

}

fn report_locked(state: &MockState, request: &ValidationRequest) -> ValidationReport {
    let (state_name, checks) = match state.validation_mode {
        ValidationMode::Pass => {
            let checks = mock_checks(state, true, false);
            let passed = checks.iter().filter(|check| check.required).all(|check| check.state == "passed");
            (if passed { "passed".to_string() } else { "quality_failed".to_string() }, checks)
        }
        ValidationMode::FailOnce | ValidationMode::FailAlways => {
            ("quality_failed".to_string(), mock_checks(state, true, true))
        }
        ValidationMode::Infrastructure => ("infrastructure_error".to_string(), vec![]),
    };
    ValidationReport {
        run_id: format!("mock-validation-{}", state.validation_runs.max(1)),
        candidate_id: request.snapshot.candidate_id.clone(),
        candidate_digest: request.snapshot.candidate_digest.clone(),
        profile_id: request.profile.profile_id.clone(),
        profile_version: request.profile.version.clone(),
        state: state_name,
        checks,
        error: None,
    }
}

fn mock_checks(state: &MockState, actual: bool, force_fail: bool) -> Vec<CheckResult> {
    let parameters_ok = actual && !force_fail && (state.thickness - 8.0).abs() < 1e-9 && (state.hole_diameter - 4.0).abs() < 1e-9;
    let geometry_ok = actual && !force_fail && state.hole_count == 4;
    vec![
        CheckResult { check_id: "input.integrity".into(), required: true, state: "passed".into(), expected: json!({}), actual: json!({}) },
        CheckResult { check_id: "native.rebuild".into(), required: true, state: "passed".into(), expected: json!({}), actual: json!({}) },
        CheckResult {
            check_id: "parameters.required".into(),
            required: true,
            state: if parameters_ok { "passed" } else { "failed" }.into(),
            expected: json!({"thickness": 8.0, "hole_diameter": 4.0}),
            actual: json!({"thickness": state.thickness, "hole_diameter": state.hole_diameter}),
        },
        CheckResult {
            check_id: "geometry.exact".into(),
            required: true,
            state: if geometry_ok { "passed" } else { "failed" }.into(),
            expected: json!({"hole_count": 4}),
            actual: json!({"hole_count": state.hole_count}),
        },
    ]
}

impl CadHost for MockHost {
    fn document(&self) -> Result<DocumentStamp, HostError> {
        let state = self.lock();
        Ok(DocumentStamp::new(state.document_id.clone(), state.revision))
    }

    fn inspect(&self) -> Result<InspectView, HostError> {
        let state = self.lock();
        Ok(InspectView {
            document: DocumentStamp::new(state.document_id.clone(), state.revision),
            body_count: 1,
            volume_mm3: Some(mock_volume(&state)),
            hole_count: Some(state.hole_count),
            parameters: state.parameters.clone(),
            summary: format!("MOCK: L bracket, thickness {} mm, {} holes, revision {}", state.thickness, state.hole_count, state.revision),
        })
    }

    fn capture_snapshot(&self, expected: &DocumentStamp) -> Result<SnapshotRef, HostError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(HostError::Invalid("cancelled".into()));
        }
        let state = self.lock();
        let actual = DocumentStamp::new(state.document_id.clone(), state.revision);
        if actual != *expected {
            return Err(HostError::Stale { expected: expected.clone(), actual });
        }
        let seq = self.candidate_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let candidate_id = format!("mock-candidate-{seq}");
        let snapshot = SnapshotRef {
            candidate_id: candidate_id.clone(),
            source: actual,
            candidate_digest: format!("sha256:mock-candidate-{seq}"),
            manifest_digest: format!("sha256:mock-manifest-{seq}"),
            artifact_id: format!("mock-artifact-{seq}"),
            artifact_digest: format!("sha256:mock-artifact-{seq}"),
            size_bytes: 1024,
            media_type: "application/zip".into(),
        };
        drop(state);
        self.lock().snapshots.insert(candidate_id, snapshot.clone());
        Ok(snapshot)
    }

    fn dispatch_write(&self, request: WriteRequest) -> Result<WriteOutcome, HostError> {
        let permit_ok = request
            .permit
            .as_ref()
            .is_some_and(|permit| self.lock().permits.contains(&permit.key()));
        if !permit_ok {
            return Ok(WriteOutcome::Denied { reason: "write has no active task permit (external bypass refused)".into() });
        }
        if self.cancel.load(Ordering::SeqCst) {
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
        let (mode, current) = {
            let state = self.lock();
            (state.write_mode, DocumentStamp::new(state.document_id.clone(), state.revision))
        };
        if current != request.expected {
            return Ok(WriteOutcome::Stale { expected: request.expected, actual: current });
        }
        let command = request.ops.first().map(|op| op.command.clone()).unwrap_or_else(|| "parameters.change".into());
        let permission = self.request_permission(&PermissionRequest {
            task_id: request.task_id.clone(),
            run_id: request.run_id.clone(),
            tool: "cad_apply".into(),
            command,
            summary: format!("apply {} op(s)", request.ops.len()),
        });
        if let PermissionDecision::Deny(reason) = permission {
            return Ok(WriteOutcome::Denied { reason });
        }
        if mode == WriteMode::Deny {
            return Ok(WriteOutcome::Denied { reason: "host policy denied the write".into() });
        }
        if mode == WriteMode::StaleOnce {
            let mut state = self.lock();
            state.write_mode = WriteMode::Apply;
            state.revision += 1;
            let actual = DocumentStamp::new(state.document_id.clone(), state.revision);
            return Ok(WriteOutcome::Stale { expected: request.expected, actual });
        }
        if mode == WriteMode::FailOnce {
            self.lock().write_mode = WriteMode::Apply;
            return Ok(WriteOutcome::Failed { reason: "MOCK: command refused before changing the design".into(), unchanged: true });
        }
        if mode == WriteMode::CancelDuringWrite {
            let (lock, cvar) = &*self.started;
            *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
            cvar.notify_all();
            let deadline = SystemTime::now() + Duration::from_secs(5);
            while SystemTime::now() < deadline {
                if self.cancel.load(Ordering::SeqCst) {
                    return Ok(WriteOutcome::Cancelled { reason: "cancelled while the write was in flight; nothing applied".into() });
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let mut state = self.lock();
        let before = DocumentStamp::new(state.document_id.clone(), state.revision);
        let untouched = MockUndo {
            thickness: state.thickness,
            hole_diameter: state.hole_diameter,
            hole_count: state.hole_count,
            parameters: state.parameters.clone(),
            revision: state.revision,
        };
        let ops_to_apply: &[CadOp] = if mode == WriteMode::PartialOnce { &request.ops[..request.ops.len().min(1)] } else { &request.ops };
        let applied_ops = match Self::apply_ops(&mut state, ops_to_apply) {
            Ok(count) => count,
            Err(reason) => return Ok(WriteOutcome::Failed { reason, unchanged: true }),
        };
        state.revision += 1;
        state.undo.push(untouched);
        state.write_history.push((request.write_id.clone(), request.ops.clone()));
        let after = DocumentStamp::new(state.document_id.clone(), state.revision);
        let outcome = if mode == WriteMode::UnknownOnce || mode == WriteMode::PartialOnce {
            state.write_mode = WriteMode::Apply;
            let message = if mode == WriteMode::PartialOnce {
                format!("MOCK: only {applied_ops} of {} ops applied before the result was lost", request.ops.len())
            } else {
                "MOCK: host applied the ops but the result was lost".into()
            };
            WriteOutcome::Unknown {
                write_id: request.write_id.clone(),
                expected: request.expected.clone(),
                message,
            }
        } else {
            WriteOutcome::Applied { write_id: request.write_id.clone(), before, after: after.clone(), applied_ops, replayed: false }
        };
        state.applied_after.insert(request.write_id.clone(), after);
        state.applied.insert(request.write_id, outcome.clone());
        Ok(outcome)
    }

    fn undo(&self, request: UndoRequest) -> Result<WriteOutcome, HostError> {
        let permit_ok = request
            .permit
            .as_ref()
            .is_some_and(|permit| self.lock().permits.contains(&permit.key()));
        if !permit_ok {
            return Ok(WriteOutcome::Denied { reason: "undo has no active task permit".into() });
        }
        if self.cancel.load(Ordering::SeqCst) {
            return Ok(WriteOutcome::Cancelled { reason: "cancelled before undo".into() });
        }
        let mut state = self.lock();
        let current = DocumentStamp::new(state.document_id.clone(), state.revision);
        if current != request.expected {
            return Ok(WriteOutcome::Stale { expected: request.expected, actual: current });
        }
        let Some(previous) = state.undo.pop() else {
            return Ok(WriteOutcome::Denied { reason: "nothing to undo".into() });
        };
        let before = current;
        state.thickness = previous.thickness;
        state.hole_diameter = previous.hole_diameter;
        state.hole_count = previous.hole_count;
        state.parameters = previous.parameters;
        state.revision = previous.revision.max(state.revision + 1);
        let after = DocumentStamp::new(state.document_id.clone(), state.revision);
        Ok(WriteOutcome::Applied { write_id: request.write_id, before, after, applied_ops: 1, replayed: false })
    }

    fn validate(&self, request: ValidationRequest) -> Result<ValidationReport, HostError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Ok(ValidationReport {
                run_id: "mock-validation-cancelled".into(),
                candidate_id: request.snapshot.candidate_id.clone(),
                candidate_digest: request.snapshot.candidate_digest.clone(),
                profile_id: request.profile.profile_id.clone(),
                profile_version: request.profile.version.clone(),
                state: "cancelled".into(),
                checks: vec![],
                error: Some("cancelled".into()),
            });
        }
        let mut state = self.lock();
        state.validation_runs += 1;
        let mode = state.validation_mode;
        let report = report_locked(&state, &request);
        if mode == ValidationMode::FailOnce {
            state.validation_mode = ValidationMode::Pass;
        }
        Ok(report)
    }

    fn reconcile(&self, pending: &PendingWrite) -> Result<ReconcileOutcome, HostError> {
        let state = self.lock();
        if let Some(after) = state.applied_after.get(&pending.write_id) {
            return Ok(ReconcileOutcome::Applied { write_id: pending.write_id.clone(), after: after.clone() });
        }
        let current = DocumentStamp::new(state.document_id.clone(), state.revision);
        if current.revision == pending.expected.revision {
            Ok(ReconcileOutcome::NotApplied { write_id: pending.write_id.clone(), current })
        } else {
            Ok(ReconcileOutcome::Unknown {
                write_id: pending.write_id.clone(),
                message: format!("MOCK: revision moved from {} to {} and the write is not recorded; a human must reconcile", pending.expected.revision, current.revision),
            })
        }
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        let mut state = self.lock();
        state.permission_requests.push(request.clone());
        match &mut state.permission_mode {
            PermissionMode::Auto => PermissionDecision::Allow,
            PermissionMode::Ask(queue) => queue.pop_front().unwrap_or_else(|| PermissionDecision::Deny("MOCK: approval expired".into())),
            PermissionMode::AllowList(commands) => {
                if commands.iter().any(|command| request.command.starts_with(command)) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Deny(format!("MOCK: `{}` is not on the allow list", request.command))
                }
            }
            PermissionMode::DenyAll => PermissionDecision::Deny("MOCK: the user denied every write".into()),
        }
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

fn parse_mm(expression: &str) -> Option<f64> {
    let expression = expression.trim();
    let number = expression.strip_suffix("mm").unwrap_or(expression).trim();
    number.parse::<f64>().ok()
}

fn mock_volume(state: &MockState) -> f64 {
    // MOCK arithmetic for the L profile minus four through holes; not CAD evidence.
    let area = 40.0 * state.thickness + (30.0 - state.thickness) * state.thickness;
    let volume = area * 60.0;
    let holes = state.hole_count as f64 * std::f64::consts::PI * (state.hole_diameter / 2.0).powi(2) * state.thickness;
    (volume - holes * 0.0).max(0.0) - holes
}
