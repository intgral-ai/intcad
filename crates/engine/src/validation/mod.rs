//! Asynchronous validation of immutable design candidates (Mastra contract §9).
//!
//! The port is vendor-neutral: it admits a `CandidateRef` against a versioned, digest-frozen
//! [`CheckProfile`], hands the job to a [`Runner`], and persists every transition. The built-in
//! [`LocalRunner`] opens the captured design in a fresh headless session and evaluates exact
//! geometry; it never touches the live document or the user's file.
//!
//! Wire operations:
//! - [`ValidationPort::submit`] (same submission key + same JCS payload is idempotent)
//! - [`ValidationPort::status`]
//! - [`ValidationPort::report`]
//! - [`ValidationPort::cancel`]
//!
//! A host with a background scheduler may call [`ValidationPort::start_next`] and
//! [`ValidationPort::complete`] instead of the synchronous [`ValidationPort::pump`].

mod error;
mod ids;
mod jcs;
mod keys;
mod profile;
mod report;
mod runner;
mod store;

#[cfg(test)]
mod tests;

pub use error::{ErrorCode, ValidationError, WireError};
pub use keys::{SubmissionKey, generate as generate_submission_key, run_id as generate_run_id, uuid_v4};
pub use profile::{
    CheckProfile, CheckRequirement, MAX_CHECKS, MAX_REQUIREMENTS, MeasurementRequirement, NATIVE_BASIC_ID, NATIVE_BASIC_VERSION,
    ParameterRequirement, ProfileRef, measurement_requirements, native_basic_profile, parameter_requirements, profile_digest, profile_ref,
    task_profile, validate_profile, verify_profile_ref,
};
pub use report::{
    ArtifactRef, CancelResult, CheckResult, CheckState, ReportResult, RunStatus, RunnerIdentity, SubmitResult, ValidationReport, ValidationState,
};
pub use runner::{
    CheckOutcome, EvidenceBytes, LOCAL_RUNNER_ID, LOCAL_RUNNER_VERSION, LocalRunner, RunControl, RunJob, Runner, RunnerOutcome, local_runner_identity,
};
pub use store::MAX_REPORT_BYTES;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use store::{AdmissionRecord, RECORD_SCHEMA, StateRecord, Store, SubmissionRecord, WorkerRecord};

use crate::schema::snapshot::CandidateRef;

type Result<T> = std::result::Result<T, ValidationError>;

/// A monotonic-enough wall clock, injectable so expiry and deadlines are deterministic in tests.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// The system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
    }
}

/// A clock tests and deterministic hosts can move by hand.
#[derive(Clone, Debug)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn new(now_ms: u64) -> Self {
        ManualClock(Arc::new(AtomicU64::new(now_ms)))
    }

    pub fn set(&self, now_ms: u64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }

    pub fn advance(&self, milliseconds: u64) {
        self.0.fetch_add(milliseconds, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The contract's finite validation resources (contract §4). A host may lower them, never raise
/// them.
#[derive(Clone, Debug)]
pub struct Limits {
    pub workers: usize,
    pub queue: usize,
    pub execution: Duration,
    pub queue_admission: Duration,
    pub report_bytes: usize,
    pub checks: usize,
    pub evidence_artifacts: usize,
    pub evidence_bytes: usize,
    pub evidence_total_bytes: usize,
    pub retention: Duration,
    pub job_records: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            workers: 2,
            queue: 8,
            execution: Duration::from_secs(600),
            queue_admission: Duration::from_secs(300),
            report_bytes: MAX_REPORT_BYTES,
            checks: MAX_CHECKS,
            evidence_artifacts: 64,
            evidence_bytes: 16 << 20,
            evidence_total_bytes: 64 << 20,
            retention: Duration::from_secs(7 * 24 * 60 * 60),
            job_records: 10_000,
        }
    }
}

/// The local validation port.
pub struct ValidationPort {
    store: Store,
    candidate_root: PathBuf,
    runner: Arc<dyn Runner>,
    limits: Limits,
    clock: Arc<dyn Clock>,
    profiles: Mutex<BTreeMap<(String, String), CheckProfile>>,
    commit: Mutex<()>,
}

impl ValidationPort {
    /// Open the local port, reconcile persisted state and use [`LocalRunner`].
    pub fn open(store: PathBuf, candidate_root: PathBuf) -> Result<Self> {
        Self::open_with(store, candidate_root, Arc::new(LocalRunner), Limits::default(), Arc::new(SystemClock))
    }

    /// Open with a host-provided runner, limits and clock. Reconciliation runs before return.
    pub fn open_with(store: PathBuf, candidate_root: PathBuf, runner: Arc<dyn Runner>, limits: Limits, clock: Arc<dyn Clock>) -> Result<Self> {
        if limits.workers == 0 || limits.queue == 0 || limits.checks == 0 || limits.report_bytes == 0 {
            return Err(ValidationError::internal("validation limits must be positive"));
        }
        let store = Store::open(store)?;
        let mut profiles = BTreeMap::new();
        let native = native_basic_profile();
        profiles.insert((native.profile_id.clone(), native.version.clone()), native);
        let port = ValidationPort { store, candidate_root, runner, limits, clock, profiles: Mutex::new(profiles), commit: Mutex::new(()) };
        port.reconcile()?;
        Ok(port)
    }

    /// The frozen minimum profile's reference.
    pub fn native_basic_reference(&self) -> Result<ProfileRef> {
        profile_ref(&native_basic_profile())
    }

    /// Register a host-formed profile. A profile id/version is immutable once registered.
    pub fn register_profile(&self, profile: CheckProfile) -> Result<ProfileRef> {
        validate_profile(&profile)?;
        if profile.checks.len() > self.limits.checks {
            return Err(ValidationError::resource_exhausted("the profile exceeds the host's check limit", "checks", self.limits.checks as u64));
        }
        let reference = profile_ref(&profile)?;
        let mut profiles = self.profiles.lock().map_err(|_| ValidationError::internal("profile registry lock was poisoned"))?;
        let key = (profile.profile_id.clone(), profile.version.clone());
        match profiles.get(&key) {
            Some(existing) => {
                if profile_digest(existing)? != reference.digest {
                    return Err(ValidationError::profile_unsupported("this profile id/version is already registered with another digest"));
                }
            }
            None => {
                profiles.insert(key, profile);
            }
        }
        Ok(reference)
    }

    /// `submit(candidate, profile, requestId)`.
    pub fn submit(&self, candidate: &CandidateRef, profile: &ProfileRef, request_id: &str) -> Result<SubmitResult> {
        self.submit_for(candidate, profile, request_id, "local")
    }

    /// `submit` with the original turn/session owner binding.
    pub fn submit_for(&self, candidate: &CandidateRef, profile: &ProfileRef, request_id: &str, owner: &str) -> Result<SubmitResult> {
        let now = self.clock.now_ms();
        let key = SubmissionKey::parse(request_id, now)?;
        if owner.is_empty() || owner.len() > 128 {
            return Err(ValidationError::invalid_request("owner must be 1..=128 bytes"));
        }
        // Resolve and freeze the profile before any durable work is admitted.
        let resolved = self.resolve_profile(profile)?;
        // Verify the candidate now: changed input bytes are refused before admission.
        runner::verify_candidate(&self.candidate_root, candidate)?;
        let payload = json!({"candidate": candidate, "profile": profile});
        let payload_digest = ids_value_digest(&payload)?;
        let key_path = self.store.key_path(&key.file_name());
        let submission = self.store.read_optional_json::<SubmissionRecord>(&key_path, 1 << 20)?;
        if let Some(submission) = submission {
            if submission.payload_digest != payload_digest {
                return Err(ValidationError::idempotency_conflict("this submission key was used with a different candidate/profile payload"));
            }
            if submission.owner != owner {
                return Err(ValidationError::invalid_request("this submission key belongs to another owner"));
            }
            return self.existing_submission(&key, &submission, &resolved, now);
        }
        // A crash between admission and the key file must not create a second job.
        if let Some((run_id, admission)) = self.find_admission_by_key(&key.text)? {
            if admission.payload_digest != payload_digest {
                return Err(ValidationError::idempotency_conflict("this submission key was used with a different candidate/profile payload"));
            }
            if admission.owner != owner {
                return Err(ValidationError::invalid_request("this submission key belongs to another owner"));
            }
            let record = SubmissionRecord {
                schema_version: RECORD_SCHEMA.into(),
                submission_key: key.text.clone(),
                payload_digest,
                run_id: run_id.clone(),
                candidate: candidate.clone(),
                profile: profile.clone(),
                owner: owner.into(),
                created_at_ms: key.timestamp_ms,
                expires_at_ms: key.expires_at_ms(),
            };
            self.store.write_json_new(&key_path, &record)?;
            return self.submit_result(&run_id, &admission.candidate, &admission.profile);
        }
        self.enforce_admission(now)?;
        let run_id = keys::run_id(now);
        let record = SubmissionRecord {
            schema_version: RECORD_SCHEMA.into(),
            submission_key: key.text.clone(),
            payload_digest: payload_digest.clone(),
            run_id: run_id.clone(),
            candidate: candidate.clone(),
            profile: profile.clone(),
            owner: owner.into(),
            created_at_ms: key.timestamp_ms,
            expires_at_ms: key.expires_at_ms(),
        };
        // Pin the immutable inputs while the job is active.
        solidcraft_io::candidate::pin(&self.candidate_root, &candidate.candidate_id)
            .map_err(|error| ValidationError::invalid_candidate(error.to_string()))?;
        let admission = AdmissionRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.clone(),
            submission_key: key.text.clone(),
            payload_digest,
            candidate: candidate.clone(),
            profile: profile.clone(),
            owner: owner.into(),
            runner: self.runner.identity(),
            created_at_ms: now,
            queued_deadline_ms: now.saturating_add(duration_ms(self.limits.queue_admission)),
        };
        let state = StateRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.clone(),
            state: ValidationState::Queued,
            updated_at_ms: now,
            report_available: false,
        };
        let written = (|| -> Result<()> {
            self.store.write_profile(&run_id, &resolved)?;
            self.store.write_json_new(&key_path, &record)?;
            self.store.write_json_new(&self.store.admission_path(&run_id), &admission)?;
            self.store.write_json_new(&self.store.state_path(&run_id), &state)?;
            Ok(())
        })();
        if let Err(error) = written {
            let _ = std::fs::remove_file(self.store.key_path(&key.file_name()));
            let _ = std::fs::remove_file(self.store.admission_path(&run_id));
            let _ = std::fs::remove_file(self.store.state_path(&run_id));
            let _ = std::fs::remove_file(self.store.profile_path(&run_id));
            let _ = solidcraft_io::candidate::unpin(&self.candidate_root, &candidate.candidate_id);
            return Err(error);
        }
        self.submit_result(&run_id, candidate, profile)
    }

    fn existing_submission(&self, key: &SubmissionKey, submission: &SubmissionRecord, profile: &CheckProfile, now: u64) -> Result<SubmitResult> {
        // A mapping whose admission write was lost is completed without creating another run.
        match self.load_admission(&submission.run_id) {
            Ok(admission) => self.submit_result(&submission.run_id, &admission.candidate, &admission.profile),
            Err(error) if error.code == ErrorCode::NotFound => {
                let candidate_verified = runner::verify_candidate(&self.candidate_root, &submission.candidate)?;
                let _ = candidate_verified;
                let admission = AdmissionRecord {
                    schema_version: RECORD_SCHEMA.into(),
                    run_id: submission.run_id.clone(),
                    submission_key: key.text.clone(),
                    payload_digest: submission.payload_digest.clone(),
                    candidate: submission.candidate.clone(),
                    profile: submission.profile.clone(),
                    owner: submission.owner.clone(),
                    runner: self.runner.identity(),
                    created_at_ms: submission.created_at_ms,
                    queued_deadline_ms: submission.created_at_ms.saturating_add(duration_ms(self.limits.queue_admission)),
                };
                self.store.write_profile(&submission.run_id, profile)?;
                self.store.write_json_new(&self.store.admission_path(&submission.run_id), &admission)?;
                let state = StateRecord {
                    schema_version: RECORD_SCHEMA.into(),
                    run_id: submission.run_id.clone(),
                    state: ValidationState::Queued,
                    updated_at_ms: now,
                    report_available: false,
                };
                self.store.write_json_new(&self.store.state_path(&submission.run_id), &state)?;
                solidcraft_io::candidate::pin(&self.candidate_root, &submission.candidate.candidate_id)
                    .map_err(|error| ValidationError::invalid_candidate(error.to_string()))?;
                self.submit_result(&submission.run_id, &submission.candidate, &submission.profile)
            }
            Err(error) => Err(error),
        }
    }

    fn submit_result(&self, run_id: &str, candidate: &CandidateRef, profile: &ProfileRef) -> Result<SubmitResult> {
        let state = self.load_state(run_id)?.state;
        Ok(SubmitResult { run_id: run_id.into(), state, candidate: candidate.clone(), profile: profile.clone() })
    }

    fn find_admission_by_key(&self, key: &str) -> Result<Option<(String, AdmissionRecord)>> {
        for run_id in self.store.admission_ids() {
            if let Ok(admission) = self.load_admission(&run_id)
                && admission.submission_key == key
            {
                return Ok(Some((run_id, admission)));
            }
        }
        Ok(None)
    }

    fn enforce_admission(&self, now: u64) -> Result<()> {
        if self.store.count_admissions() >= self.limits.job_records {
            self.cleanup(now);
            if self.store.count_admissions() >= self.limits.job_records {
                return Err(ValidationError::resource_exhausted(
                    "the validation job-record cap is full",
                    "jobRecords",
                    self.limits.job_records as u64,
                ));
            }
        }
        let (queued, running) = self.counts()?;
        if queued >= self.limits.queue && running >= self.limits.workers {
            return Err(ValidationError::resource_exhausted("the validation queue is full", "validationQueue", self.limits.queue as u64));
        }
        if queued >= self.limits.queue {
            return Err(ValidationError::resource_exhausted("the validation queue is full", "validationQueue", self.limits.queue as u64));
        }
        Ok(())
    }

    fn counts(&self) -> Result<(usize, usize)> {
        let mut queued = 0;
        let mut running = 0;
        for run_id in self.store.run_ids() {
            if let Ok(state) = self.load_state(&run_id) {
                match state.state {
                    ValidationState::Queued => queued += 1,
                    ValidationState::Running => running += 1,
                    _ => {}
                }
            }
        }
        Ok((queued, running))
    }

    /// `status(runId)`.
    pub fn status(&self, run_id: &str) -> Result<RunStatus> {
        let admission = self.load_admission(run_id)?;
        let state = self.load_state(run_id)?;
        Ok(RunStatus {
            run_id: run_id.into(),
            state: state.state,
            candidate: admission.candidate,
            profile: admission.profile,
            updated_at: ids::timestamp(state.updated_at_ms),
            report_available: state.state.is_terminal() && self.store.report_exists(run_id),
        })
    }

    /// `cancel(runId)`: persist the marker before any signal, then finalize a queued job. A
    /// terminal state is preserved.
    pub fn cancel(&self, run_id: &str) -> Result<CancelResult> {
        let _guard = self.commit.lock().map_err(|_| ValidationError::internal("validation commit lock was poisoned"))?;
        let admission = self.load_admission(run_id)?;
        let state = self.load_state(run_id)?;
        if state.state.is_terminal() {
            return Ok(CancelResult { run_id: run_id.into(), state: state.state });
        }
        let marker = self.store.cancel_path(run_id);
        self.store.write_bytes_new(&marker, b"cancelled\n").or_else(|error| if marker.is_file() { Ok(()) } else { Err(error) })?;
        let state = self.load_state(run_id)?;
        if !state.state.is_terminal() {
            let profile = self.store.read_profile(run_id)?;
            let report = self.recovery_report(
                &admission,
                &profile,
                ValidationState::Cancelled,
                Some(ValidationError::cancelled("the job was cancelled before its result committed")),
            );
            self.commit_report(run_id, report, ValidationState::Cancelled)?;
            return Ok(CancelResult { run_id: run_id.into(), state: ValidationState::Cancelled });
        }
        Ok(CancelResult { run_id: run_id.into(), state: state.state })
    }

    /// Run every queued job to a terminal state with the configured runner (the CLI's scheduler).
    pub fn pump(&self) -> Result<()> {
        while let Some(job) = self.start_next()? {
            let control = RunControl {
                clock: Arc::clone(&self.clock),
                deadline_ms: job.started_at_ms.saturating_add(duration_ms(self.limits.execution)),
                cancel_path: self.store.cancel_path(&job.run_id),
            };
            let outcome = self.runner.run(&job, &control);
            self.complete(&job.run_id, outcome)?;
        }
        Ok(())
    }

    /// Claim the oldest queued job and persist its worker identity. `None` when no worker slot or
    /// no queued job is available.
    pub fn start_next(&self) -> Result<Option<RunJob>> {
        let (_, running) = self.counts()?;
        if running >= self.limits.workers {
            return Ok(None);
        }
        let now = self.clock.now_ms();
        let mut queued: Vec<(u64, String)> = Vec::new();
        for run_id in self.store.run_ids() {
            let state = match self.load_state(&run_id) {
                Ok(state) => state,
                Err(_) => continue,
            };
            if state.state != ValidationState::Queued {
                continue;
            }
            if self.store.marker_exists(&run_id) {
                let admission = self.load_admission(&run_id)?;
                let profile = self.store.read_profile(&run_id)?;
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::Cancelled,
                    Some(ValidationError::cancelled("the job was cancelled before a worker started")),
                );
                self.commit_report(&run_id, report, ValidationState::Cancelled)?;
                continue;
            }
            let admission = self.load_admission(&run_id)?;
            if now >= admission.queued_deadline_ms {
                let profile = self.store.read_profile(&run_id)?;
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::InfrastructureError,
                    Some(ValidationError::timeout("the job expired in the admission queue")),
                );
                self.commit_report(&run_id, report, ValidationState::InfrastructureError)?;
                continue;
            }
            queued.push((admission.created_at_ms, run_id));
        }
        queued.sort();
        let Some((_, run_id)) = queued.into_iter().next() else { return Ok(None) };
        let admission = self.load_admission(&run_id)?;
        let profile = self.store.read_profile(&run_id)?;
        if profile.checks.len() > self.limits.checks {
            let report = self.recovery_report(
                &admission,
                &profile,
                ValidationState::InfrastructureError,
                Some(ValidationError::resource_exhausted("the profile exceeds the host check limit", "checks", self.limits.checks as u64)),
            );
            self.commit_report(&run_id, report, ValidationState::InfrastructureError)?;
            return Ok(None);
        }
        let worker = WorkerRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.clone(),
            runner: admission.runner.clone(),
            worker_id: format!("worker-{}", keys::uuid_v4()),
            pid: std::process::id(),
            started_at_ms: now,
            deadline_ms: now.saturating_add(duration_ms(self.limits.execution)),
        };
        // The worker record is the claim token; a second host cannot start the same run.
        match self.store.write_json_new(&self.store.worker_path(&run_id), &worker) {
            Ok(()) => {}
            Err(_) if self.store.worker_path(&run_id).is_file() => return Ok(None),
            Err(error) => return Err(error),
        }
        if self.store.marker_exists(&run_id) {
            let report = self.recovery_report(
                &admission,
                &profile,
                ValidationState::Cancelled,
                Some(ValidationError::cancelled("the job was cancelled before the worker started")),
            );
            self.commit_report(&run_id, report, ValidationState::Cancelled)?;
            return Ok(None);
        }
        let state = StateRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.clone(),
            state: ValidationState::Running,
            updated_at_ms: now,
            report_available: false,
        };
        self.store.write_json_replace(&self.store.state_path(&run_id), &state)?;
        Ok(Some(RunJob { run_id, candidate: admission.candidate, candidate_root: self.candidate_root.clone(), profile, started_at_ms: now }))
    }

    /// Finish a claimed job. Terminal states cannot be replaced and a cancellation marker always
    /// wins before the terminal commit.
    pub fn complete(&self, run_id: &str, outcome: RunnerOutcome) -> Result<ValidationState> {
        self.complete_by(run_id, outcome, &self.runner.identity())
    }

    pub(crate) fn complete_by(&self, run_id: &str, outcome: RunnerOutcome, runner: &RunnerIdentity) -> Result<ValidationState> {
        let _guard = self.commit.lock().map_err(|_| ValidationError::internal("validation commit lock was poisoned"))?;
        let admission = self.load_admission(run_id)?;
        let state = self.load_state(run_id)?;
        if state.state.is_terminal() {
            return Ok(state.state);
        }
        let profile = self.store.read_profile(run_id)?;
        if runner != &admission.runner {
            let report = self.recovery_report(
                &admission,
                &profile,
                ValidationState::InfrastructureError,
                Some(ValidationError::report_mismatch("the worker identity does not match the admitted runner")),
            );
            self.commit_report(run_id, report, ValidationState::InfrastructureError)?;
            return Ok(ValidationState::InfrastructureError);
        }
        if self.store.marker_exists(run_id) {
            let checks = match &outcome {
                RunnerOutcome::Finished { checks, .. } | RunnerOutcome::Cancelled { checks, .. } => checks.clone(),
                _ => Vec::new(),
            };
            let report = self.report_with_checks(
                &admission,
                &profile,
                ValidationState::Cancelled,
                Some(ValidationError::cancelled("the job was cancelled before its result committed")),
                checks,
                worker_started(run_id, &self.store).map(ids::timestamp),
                self.clock.now_ms(),
            )?;
            self.commit_report(run_id, report, ValidationState::Cancelled)?;
            return Ok(ValidationState::Cancelled);
        }
        let started_at = worker_started(run_id, &self.store).map(ids::timestamp);
        match outcome {
            RunnerOutcome::Finished { finished_at_ms, checks, .. } => {
                let report = match self.report_with_checks(&admission, &profile, ValidationState::Passed, None, checks, started_at, finished_at_ms) {
                    Ok(report) => report,
                    Err(error) => self.recovery_report(
                        &admission,
                        &profile,
                        ValidationState::InfrastructureError,
                        Some(ValidationError::report_mismatch(format!("the worker's output is malformed: {error}"))),
                    ),
                };
                let desired = aggregate_state(&report);
                let report = finalize_state(report, desired);
                self.commit_report(run_id, report, desired)?;
                Ok(desired)
            }
            RunnerOutcome::Cancelled { finished_at_ms, checks, .. } => {
                let report = match self.report_with_checks(
                    &admission,
                    &profile,
                    ValidationState::Cancelled,
                    Some(ValidationError::cancelled("the worker observed cancellation")),
                    checks,
                    started_at,
                    finished_at_ms,
                ) {
                    Ok(report) => report,
                    Err(error) => self.recovery_report(
                        &admission,
                        &profile,
                        ValidationState::Cancelled,
                        Some(ValidationError::cancelled(format!("the cancelled worker's output is malformed: {error}"))),
                    ),
                };
                self.commit_report(run_id, report, ValidationState::Cancelled)?;
                Ok(ValidationState::Cancelled)
            }
            RunnerOutcome::Infrastructure { finished_at_ms, message, .. } => {
                let report = self.report_with_checks(
                    &admission,
                    &profile,
                    ValidationState::InfrastructureError,
                    Some(ValidationError::internal(format!("the worker failed: {message}"))),
                    Vec::new(),
                    started_at,
                    finished_at_ms,
                )?;
                self.commit_report(run_id, report, ValidationState::InfrastructureError)?;
                Ok(ValidationState::InfrastructureError)
            }
            RunnerOutcome::TimedOut { finished_at_ms, message, .. } => {
                let report = self.report_with_checks(
                    &admission,
                    &profile,
                    ValidationState::InfrastructureError,
                    Some(ValidationError::timeout(format!("the worker timed out: {message}"))),
                    Vec::new(),
                    started_at,
                    finished_at_ms,
                )?;
                self.commit_report(run_id, report, ValidationState::InfrastructureError)?;
                Ok(ValidationState::InfrastructureError)
            }
            RunnerOutcome::Unknown { finished_at_ms, message, .. } => {
                let report = self.report_with_checks(
                    &admission,
                    &profile,
                    ValidationState::Unknown,
                    Some(ValidationError::outcome_unknown(format!("the worker outcome is unknown: {message}"))),
                    Vec::new(),
                    started_at,
                    finished_at_ms,
                )?;
                self.commit_report(run_id, report, ValidationState::Unknown)?;
                Ok(ValidationState::Unknown)
            }
        }
    }

    /// `report(runId)` with the persisted bytes verified and re-digested.
    pub fn report(&self, run_id: &str) -> Result<ReportResult> {
        let admission = self.load_admission(run_id)?;
        let state = self.load_state(run_id)?;
        if !state.state.is_terminal() {
            return Err(ValidationError::not_ready("the validation run has no terminal report yet"));
        }
        let bytes = self.store.read_bounded(&self.store.report_path(run_id), self.limits.report_bytes).map_err(|error| {
            if error.code == ErrorCode::NotFound { ValidationError::not_ready("the validation run has no report") } else { error }
        })?;
        let report: ValidationReport =
            serde_json::from_slice(&bytes).map_err(|error| ValidationError::report_mismatch(format!("the report is malformed: {error}")))?;
        self.verify_report(run_id, &admission, &state, &report)?;
        let reference = ArtifactRef {
            artifact_id: format!("report-{run_id}"),
            digest: ids::digest(&bytes),
            size_bytes: bytes.len() as u64,
            media_type: "application/json".into(),
        };
        Ok(ReportResult { report, reference })
    }

    fn verify_report(&self, run_id: &str, admission: &AdmissionRecord, state: &StateRecord, report: &ValidationReport) -> Result<()> {
        if report.schema_version != "1.0"
            || report.run_id != run_id
            || report.state != state.state
            || report.candidate_id != admission.candidate.candidate_id
            || report.candidate_digest != admission.candidate.candidate_digest
            || report.manifest_digest != admission.candidate.manifest_digest
            || report.profile != admission.profile
            || report.runner != admission.runner
        {
            return Err(ValidationError::report_mismatch("the report does not bind this run's admission"));
        }
        let profile = self.store.read_profile(run_id)?;
        if profile.profile_id != report.profile.profile_id
            || profile.version != report.profile.version
            || profile_digest(&profile)? != report.profile.digest
        {
            return Err(ValidationError::report_mismatch("the frozen profile does not match the report"));
        }
        if report.checks.len() != profile.checks.len() {
            return Err(ValidationError::report_mismatch("the report does not cover every profile check"));
        }
        for (result, requirement) in report.checks.iter().zip(&profile.checks) {
            if result.check_id != requirement.check_id || result.required != requirement.required {
                return Err(ValidationError::report_mismatch("the report's checks are missing, duplicated or reordered"));
            }
            for evidence in &result.evidence {
                let bytes = self
                    .store
                    .read_bounded(&self.store.evidence_path(run_id, &evidence.artifact_id), self.limits.evidence_bytes)
                    .map_err(|_| ValidationError::report_mismatch("a report evidence artifact is missing"))?;
                if ids::digest(&bytes) != evidence.digest || bytes.len() as u64 != evidence.size_bytes {
                    return Err(ValidationError::report_mismatch("a report evidence artifact was changed"));
                }
            }
        }
        let verified = runner::verify_candidate(&self.candidate_root, &admission.candidate)?;
        if verified.manifest.get("engineVersion").and_then(Value::as_str) != Some(report.runner.engine_version.as_str()) {
            return Err(ValidationError::report_mismatch("the runner engine version does not match the captured manifest"));
        }
        if report.state == ValidationState::Passed {
            if report.error.is_some() {
                return Err(ValidationError::report_mismatch("a passing report carries an error"));
            }
            for (result, requirement) in report.checks.iter().zip(&profile.checks) {
                if requirement.required && (result.state != CheckState::Passed || result.evidence.is_empty()) {
                    return Err(ValidationError::report_mismatch("a passing report has a required check without a pass and evidence"));
                }
            }
        }
        Ok(())
    }

    /// Remove completed records older than the retention window. Active jobs are never evicted.
    pub fn cleanup(&self, now_ms: u64) -> usize {
        self.store.purge_older_than(now_ms, duration_ms(self.limits.retention))
    }

    fn load_admission(&self, run_id: &str) -> Result<AdmissionRecord> {
        if !keys::valid_run_id(run_id) {
            return Err(ValidationError::not_found("no such validation run"));
        }
        self.store.read_json(&self.store.admission_path(run_id), 1 << 20)
    }

    fn load_state(&self, run_id: &str) -> Result<StateRecord> {
        if !keys::valid_run_id(run_id) {
            return Err(ValidationError::not_found("no such validation run"));
        }
        self.store.read_json(&self.store.state_path(run_id), 1 << 20)
    }

    fn resolve_profile(&self, reference: &ProfileRef) -> Result<CheckProfile> {
        if !ids::valid_id(&reference.profile_id) || !ids::valid_digest(&reference.digest) {
            return Err(ValidationError::profile_unsupported("profileId or digest is malformed"));
        }
        let profiles = self.profiles.lock().map_err(|_| ValidationError::internal("profile registry lock was poisoned"))?;
        let profile = profiles
            .get(&(reference.profile_id.clone(), reference.version.clone()))
            .ok_or_else(|| ValidationError::profile_unsupported("the profile is not in this host's registry"))?;
        verify_profile_ref(profile, reference)?;
        Ok(profile.clone())
    }

    fn commit_report(&self, run_id: &str, report: ValidationReport, state: ValidationState) -> Result<()> {
        let mut bytes = self.serialize_report(&report)?;
        let path = self.store.report_path(run_id);
        let mut created = false;
        match self.store.write_bytes_new(&path, &bytes) {
            Ok(()) => created = true,
            Err(error) => {
                let existing = self.store.read_bounded(&path, self.limits.report_bytes).map_err(|_| error)?;
                if existing != bytes {
                    return Err(ValidationError::report_mismatch("a different terminal report already exists for this run"));
                }
            }
        }
        // A cancellation marker written while finishing wins before the terminal commit.
        if created && state != ValidationState::Cancelled && self.store.marker_exists(run_id) {
            let _ = std::fs::remove_file(&path);
            let mut cancelled = report;
            cancelled.state = ValidationState::Cancelled;
            cancelled.error = Some(ValidationError::cancelled("the job was cancelled before its result committed").wire());
            bytes = self.serialize_report(&cancelled)?;
            self.store.write_bytes_new(&path, &bytes)?;
            self.write_terminal_state(run_id, ValidationState::Cancelled)?;
            self.unpin(run_id)?;
            return Ok(());
        }
        self.write_terminal_state(run_id, state)?;
        self.unpin(run_id)?;
        Ok(())
    }

    fn serialize_report(&self, report: &ValidationReport) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(report).map_err(|error| ValidationError::internal(format!("report: {error}")))?;
        if bytes.len() > self.limits.report_bytes {
            return Err(ValidationError::resource_exhausted(
                "the validation report exceeds the host's size limit",
                "reportBytes",
                self.limits.report_bytes as u64,
            ));
        }
        Ok(bytes)
    }

    fn write_terminal_state(&self, run_id: &str, state: ValidationState) -> Result<()> {
        let current = self.load_state(run_id)?;
        if current.state.is_terminal() && current.state != state {
            return Ok(());
        }
        let record = StateRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.into(),
            state,
            updated_at_ms: self.clock.now_ms(),
            report_available: true,
        };
        self.store.write_json_replace(&self.store.state_path(run_id), &record)
    }

    fn unpin(&self, run_id: &str) -> Result<()> {
        let admission = self.load_admission(run_id)?;
        solidcraft_io::candidate::unpin(&self.candidate_root, &admission.candidate.candidate_id)
            .map_err(|error| ValidationError::internal(error.to_string()))
    }

    #[allow(clippy::too_many_arguments)]
    fn report_with_checks(
        &self,
        admission: &AdmissionRecord,
        profile: &CheckProfile,
        state: ValidationState,
        error: Option<ValidationError>,
        checks: Vec<CheckOutcome>,
        started_at: Option<String>,
        finished_at_ms: u64,
    ) -> Result<ValidationReport> {
        // Reject unknown or duplicate ids before any evidence is written.
        let mut by_id: BTreeMap<&str, &CheckOutcome> = BTreeMap::new();
        for check in &checks {
            if !profile.checks.iter().any(|requirement| requirement.check_id == check.check_id) {
                return Err(ValidationError::report_mismatch(format!("the worker reported unknown check {}", check.check_id)));
            }
            if by_id.insert(check.check_id.as_str(), check).is_some() {
                return Err(ValidationError::report_mismatch(format!("the worker reported duplicate check {}", check.check_id)));
            }
        }
        let mut evidence_count = 0_usize;
        let mut evidence_total = 0_usize;
        let mut results = Vec::with_capacity(profile.checks.len());
        for requirement in &profile.checks {
            let Some(outcome) = by_id.get(requirement.check_id.as_str()) else {
                results.push(CheckResult {
                    check_id: requirement.check_id.clone(),
                    required: requirement.required,
                    state: CheckState::NotRun,
                    expected: requirement.parameters.clone(),
                    actual: json!({}),
                    evidence: Vec::new(),
                });
                continue;
            };
            let mut result = CheckResult {
                check_id: requirement.check_id.clone(),
                required: requirement.required,
                state: outcome.state,
                expected: outcome.expected.clone(),
                actual: outcome.actual.clone(),
                evidence: Vec::new(),
            };
            for evidence in &outcome.evidence {
                evidence_count += 1;
                if evidence_count > self.limits.evidence_artifacts
                    || evidence.bytes.len() > self.limits.evidence_bytes
                    || evidence_total.saturating_add(evidence.bytes.len()) > self.limits.evidence_total_bytes
                {
                    result.state = CheckState::Error;
                    result.actual = json!({"error": "evidence exceeds the host's limits", "reported": result.actual});
                    break;
                }
                let artifact_id = format!("evidence-{}-{evidence_count:03}", admission.run_id);
                let path = self.store.evidence_path(&admission.run_id, &artifact_id);
                if path.is_file() {
                    let existing = self.store.read_bounded(&path, self.limits.evidence_bytes)?;
                    if existing != evidence.bytes {
                        return Err(ValidationError::report_mismatch("an evidence artifact was written twice with different bytes"));
                    }
                } else {
                    self.store.write_bytes_new(&path, &evidence.bytes)?;
                }
                evidence_total = evidence_total.saturating_add(evidence.bytes.len());
                result.evidence.push(ArtifactRef {
                    artifact_id,
                    digest: ids::digest(&evidence.bytes),
                    size_bytes: evidence.bytes.len() as u64,
                    media_type: evidence.media_type.clone(),
                });
            }
            results.push(result);
        }
        let report = ValidationReport::new(
            &admission.run_id,
            &admission.candidate,
            &admission.profile,
            &admission.runner,
            state,
            started_at,
            ids::timestamp(finished_at_ms),
            results,
            error.map(|error| error.wire()),
        );
        // Keep the report under the host ceiling. The evidence is dropped and the run cannot
        // pass, rather than persisting a truncated or over-budget record.
        if serde_json::to_vec(&report).map(|bytes| bytes.len() > self.limits.report_bytes).unwrap_or(true) {
            let fallback_state = if state == ValidationState::Cancelled { ValidationState::Cancelled } else { ValidationState::InfrastructureError };
            let fallback_error = if fallback_state == ValidationState::Cancelled {
                ValidationError::cancelled("the cancelled report exceeded the host's size limit")
            } else {
                ValidationError::resource_exhausted(
                    "the validation report exceeded the host's size limit",
                    "reportBytes",
                    self.limits.report_bytes as u64,
                )
            };
            let minimal = self.recovery_report(admission, profile, fallback_state, Some(fallback_error));
            return Ok(minimal);
        }
        Ok(report)
    }

    fn recovery_report(
        &self,
        admission: &AdmissionRecord,
        profile: &CheckProfile,
        state: ValidationState,
        error: Option<ValidationError>,
    ) -> ValidationReport {
        let checks = profile
            .checks
            .iter()
            .map(|requirement| CheckResult {
                check_id: requirement.check_id.clone(),
                required: requirement.required,
                state: CheckState::NotRun,
                expected: requirement.parameters.clone(),
                actual: json!({"reason": "the host restarted before this check completed"}),
                evidence: Vec::new(),
            })
            .collect();
        ValidationReport::new(
            &admission.run_id,
            &admission.candidate,
            &admission.profile,
            &admission.runner,
            state,
            None,
            ids::timestamp(self.clock.now_ms()),
            checks,
            error.map(|error| error.wire()),
        )
    }

    fn reconcile(&self) -> Result<()> {
        for run_id in self.store.run_ids() {
            let admission = self.load_admission(&run_id)?;
            let state = self.load_state(&run_id)?;
            let profile = self.store.read_profile(&run_id)?;
            if state.state.is_terminal() {
                if !self.store.report_exists(&run_id) {
                    let report = self.recovery_report(
                        &admission,
                        &profile,
                        ValidationState::Unknown,
                        Some(ValidationError::outcome_unknown("the terminal report is missing after a restart")),
                    );
                    self.commit_report(&run_id, report, ValidationState::Unknown)?;
                }
                continue;
            }
            if state.state == ValidationState::Queued {
                if self.store.marker_exists(&run_id) {
                    let report = self.recovery_report(
                        &admission,
                        &profile,
                        ValidationState::Cancelled,
                        Some(ValidationError::cancelled("the job was cancelled before a worker started")),
                    );
                    self.commit_report(&run_id, report, ValidationState::Cancelled)?;
                } else {
                    let report = self.recovery_report(
                        &admission,
                        &profile,
                        ValidationState::InfrastructureError,
                        Some(ValidationError::internal("the host restarted before the queued job started; it was not restarted")),
                    );
                    self.commit_report(&run_id, report, ValidationState::InfrastructureError)?;
                }
                continue;
            }
            // Running: never rerun. A completed report is reconciled; anything else is unknown.
            if self.store.report_exists(&run_id)
                && let Ok(bytes) = self.store.read_bounded(&self.store.report_path(&run_id), self.limits.report_bytes)
                && let Ok(report) = serde_json::from_slice::<ValidationReport>(&bytes)
                && report.state.is_terminal()
                && (report.state == ValidationState::Cancelled || !self.store.marker_exists(&run_id))
            {
                self.write_terminal_state(&run_id, report.state)?;
                self.unpin(&run_id)?;
                continue;
            }
            if self.store.marker_exists(&run_id) {
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::Cancelled,
                    Some(ValidationError::cancelled("the job was cancelled while running")),
                );
                self.commit_report(&run_id, report, ValidationState::Cancelled)?;
            } else {
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::Unknown,
                    Some(ValidationError::outcome_unknown("the worker outcome is unknown after a host restart")),
                );
                self.commit_report(&run_id, report, ValidationState::Unknown)?;
            }
        }
        Ok(())
    }
}

fn ids_value_digest(value: &Value) -> Result<String> {
    ids::value_digest(value)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn worker_started(run_id: &str, store: &Store) -> Option<u64> {
    store.read_optional_json::<WorkerRecord>(&store.worker_path(run_id), 1 << 20).ok().flatten().map(|worker| worker.started_at_ms)
}

fn aggregate_state(report: &ValidationReport) -> ValidationState {
    let mut failed = false;
    let mut infrastructure = false;
    for result in &report.checks {
        if !result.required {
            continue;
        }
        match result.state {
            CheckState::Passed => {
                if result.evidence.is_empty() {
                    infrastructure = true;
                }
            }
            CheckState::Failed => failed = true,
            CheckState::Error | CheckState::Unsupported | CheckState::NotRun => infrastructure = true,
        }
    }
    if infrastructure {
        ValidationState::InfrastructureError
    } else if failed {
        ValidationState::QualityFailed
    } else {
        ValidationState::Passed
    }
}

fn finalize_state(mut report: ValidationReport, state: ValidationState) -> ValidationReport {
    report.state = state;
    if state == ValidationState::InfrastructureError && report.error.is_none() {
        report.error = Some(ValidationError::report_mismatch("the report did not prove every required check").wire());
    }
    if state == ValidationState::Passed {
        report.error = None;
    }
    report
}

/// The evidence bytes and digest a host can verify without trusting the runner's JSON.
pub fn evidence_reference(media_type: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef { artifact_id: "evidence".into(), digest: ids::digest(bytes), size_bytes: bytes.len() as u64, media_type: media_type.into() }
}
