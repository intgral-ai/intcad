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
mod skill;
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
pub use skill::{
    LiveDocument, SkillCheck, SkillFailure, SkillFailureKind, SkillValidationReport, SkillValidationRequest, TEXT_TO_CAD_PROFILE_ID,
    TEXT_TO_CAD_PROFILE_VERSION, TEXT_TO_CAD_REQUIRED_CHECKS, request_key, summary_fields, text_to_cad_profile, text_to_cad_profile_ref,
    validate_beta_profile, validate_skill_snapshot,
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

/// Contract §4 ceilings. A host may lower these, never raise them.
pub const MAX_WORKERS: usize = 2;
pub const MAX_QUEUE: usize = 8;
pub const MAX_EXECUTION: Duration = Duration::from_secs(600);
pub const MAX_QUEUE_ADMISSION: Duration = Duration::from_secs(300);
pub const MAX_CHECKS_PER_PROFILE: usize = MAX_CHECKS;
pub const MAX_EVIDENCE_ARTIFACTS: usize = 64;
pub const MAX_EVIDENCE_BYTES: usize = 16 << 20;
pub const MAX_EVIDENCE_TOTAL_BYTES: usize = 64 << 20;
pub const MAX_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const MAX_JOB_RECORDS: usize = 10_000;

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
            workers: MAX_WORKERS,
            queue: MAX_QUEUE,
            execution: MAX_EXECUTION,
            queue_admission: MAX_QUEUE_ADMISSION,
            report_bytes: MAX_REPORT_BYTES,
            checks: MAX_CHECKS_PER_PROFILE,
            evidence_artifacts: MAX_EVIDENCE_ARTIFACTS,
            evidence_bytes: MAX_EVIDENCE_BYTES,
            evidence_total_bytes: MAX_EVIDENCE_TOTAL_BYTES,
            retention: MAX_RETENTION,
            job_records: MAX_JOB_RECORDS,
        }
    }
}

impl Limits {
    /// Refuse a host that asks for more than the contract allows. Values at or below the ceiling
    /// are accepted unchanged.
    pub fn validate(&self) -> Result<()> {
        let reject = |resource: &str, limit: u64| ValidationError::resource_exhausted("the host limit exceeds the contract ceiling", resource, limit);
        if self.workers == 0 || self.workers > MAX_WORKERS {
            return Err(reject("validationWorkers", MAX_WORKERS as u64));
        }
        if self.queue == 0 || self.queue > MAX_QUEUE {
            return Err(reject("validationQueue", MAX_QUEUE as u64));
        }
        if self.execution > MAX_EXECUTION {
            return Err(reject("validationSeconds", MAX_EXECUTION.as_secs()));
        }
        if self.queue_admission > MAX_QUEUE_ADMISSION {
            return Err(reject("validationQueueSeconds", MAX_QUEUE_ADMISSION.as_secs()));
        }
        if self.report_bytes == 0 || self.report_bytes > MAX_REPORT_BYTES {
            return Err(reject("reportBytes", MAX_REPORT_BYTES as u64));
        }
        if self.checks == 0 || self.checks > MAX_CHECKS_PER_PROFILE {
            return Err(reject("checks", MAX_CHECKS_PER_PROFILE as u64));
        }
        if self.evidence_artifacts == 0 || self.evidence_artifacts > MAX_EVIDENCE_ARTIFACTS {
            return Err(reject("evidenceArtifacts", MAX_EVIDENCE_ARTIFACTS as u64));
        }
        if self.evidence_bytes == 0 || self.evidence_bytes > MAX_EVIDENCE_BYTES {
            return Err(reject("evidenceBytes", MAX_EVIDENCE_BYTES as u64));
        }
        if self.evidence_total_bytes == 0 || self.evidence_total_bytes > MAX_EVIDENCE_TOTAL_BYTES {
            return Err(reject("evidenceTotalBytes", MAX_EVIDENCE_TOTAL_BYTES as u64));
        }
        if self.retention > MAX_RETENTION {
            return Err(reject("retentionSeconds", MAX_RETENTION.as_secs()));
        }
        if self.job_records == 0 || self.job_records > MAX_JOB_RECORDS {
            return Err(reject("jobRecords", MAX_JOB_RECORDS as u64));
        }
        Ok(())
    }
}

/// The local validation port.
pub struct ValidationPort {
    store: Store,
    candidate_root: PathBuf,
    runner: Arc<dyn Runner>,
    limits: Limits,
    clock: Arc<dyn Clock>,
    host_id: String,
    profiles: Mutex<BTreeMap<(String, String), CheckProfile>>,
    /// Serializes admission so concurrent same-key submits observe the first writer's key.
    admit: Mutex<()>,
    /// Serializes terminal commits and cancellation markers.
    commit: Mutex<()>,
    /// Serializes pin/unpin with the candidate sweep in `cleanup`.
    pins: Mutex<()>,
}

impl ValidationPort {
    /// Open the local port as a worker host: reconcile owned or dead workers, then use
    /// [`LocalRunner`].
    pub fn open(store: PathBuf, candidate_root: PathBuf) -> Result<Self> {
        Self::open_with(store, candidate_root, Arc::new(LocalRunner), Limits::default(), Arc::new(SystemClock))
    }

    /// Open with a host-provided runner, limits and clock. Reconciliation runs before return.
    pub fn open_with(store: PathBuf, candidate_root: PathBuf, runner: Arc<dyn Runner>, limits: Limits, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::open_mode(store, candidate_root, runner, limits, clock, true)
    }

    /// Open without reconciliation. Status, report and cancel clients use this: reading another
    /// process's live job must never turn it into a restart.
    pub fn open_read_only(store: PathBuf, candidate_root: PathBuf) -> Result<Self> {
        Self::open_mode(store, candidate_root, Arc::new(LocalRunner), Limits::default(), Arc::new(SystemClock), false)
    }

    /// Open for read-only operations with an explicit clock (tests and host clients).
    pub fn open_read_only_with(store: PathBuf, candidate_root: PathBuf, limits: Limits, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::open_mode(store, candidate_root, Arc::new(LocalRunner), limits, clock, false)
    }

    fn open_mode(
        store: PathBuf,
        candidate_root: PathBuf,
        runner: Arc<dyn Runner>,
        limits: Limits,
        clock: Arc<dyn Clock>,
        reconcile: bool,
    ) -> Result<Self> {
        limits.validate()?;
        let store = Store::open(store)?;
        let mut profiles = BTreeMap::new();
        let native = native_basic_profile();
        profiles.insert((native.profile_id.clone(), native.version.clone()), native);
        let port = ValidationPort {
            store,
            candidate_root,
            runner,
            limits,
            clock,
            host_id: format!("host-{}", keys::uuid_v4()),
            profiles: Mutex::new(profiles),
            admit: Mutex::new(()),
            commit: Mutex::new(()),
            pins: Mutex::new(()),
        };
        if reconcile {
            port.reconcile()?;
        }
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
        let _guard = self.admit.lock().map_err(|_| ValidationError::internal("validation admission lock was poisoned"))?;
        let now = self.clock.now_ms();
        let key = SubmissionKey::parse(request_id, now)?;
        if owner.is_empty() || owner.len() > 128 {
            return Err(ValidationError::invalid_request("owner must be 1..=128 bytes"));
        }
        let payload = json!({"candidate": candidate, "profile": profile});
        let payload_digest = ids_value_digest(&payload)?;
        let key_path = self.store.key_path(&key.file_name());
        // Idempotency comes before profile/candidate verification: a replay of an admitted job
        // returns the original run even when its profile is no longer registered or its candidate
        // is no longer readable.
        if let Some(submission) = self.store.read_optional_json::<SubmissionRecord>(&key_path, 1 << 20)? {
            if submission.payload_digest != payload_digest {
                return Err(ValidationError::idempotency_conflict("this submission key was used with a different candidate/profile payload"));
            }
            if submission.owner != owner {
                return Err(ValidationError::invalid_request("this submission key belongs to another owner"));
            }
            return self.resume_submission(&key, &submission, profile, now);
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
            if !key_path.is_file() {
                self.store.write_json_new(&key_path, &record)?;
            }
            self.repair_state(&run_id, &admission, now)?;
            let state = self.load_state(&run_id)?;
            if !state.state.is_terminal() {
                self.pin_recovered(&run_id, &admission.candidate.candidate_id)?;
            }
            return self.submit_result(&run_id, &admission.candidate, &admission.profile);
        }
        // New work: resolve and freeze the profile, then verify the exact candidate bytes.
        let resolved = self.resolve_profile(profile)?;
        let verified = runner::verify_candidate(&self.candidate_root, candidate)?;
        let runner_engine = self.runner.identity().engine_version;
        if verified.manifest.get("engineVersion").and_then(Value::as_str) != Some(runner_engine.as_str()) {
            return Err(ValidationError::invalid_candidate("the runner engine version does not match the captured candidate manifest"));
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
        let admission = AdmissionRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.clone(),
            submission_key: key.text.clone(),
            payload_digest,
            candidate: candidate.clone(),
            profile: profile.clone(),
            owner: owner.into(),
            runner: self.runner.identity(),
            host_id: self.host_id.clone(),
            host_pid: std::process::id(),
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
        // The key file is the atomic cross-process claim. If another writer won it, read its
        // record and return that run; never delete or unpin another run's work.
        match self.store.write_json_new(&key_path, &record) {
            Ok(()) => {}
            Err(_) if key_path.is_file() => {
                let submission: SubmissionRecord = self.store.read_json(&key_path, 1 << 20)?;
                if submission.payload_digest != record.payload_digest {
                    return Err(ValidationError::idempotency_conflict("this submission key was used with a different candidate/profile payload"));
                }
                if submission.owner != record.owner {
                    return Err(ValidationError::invalid_request("this submission key belongs to another owner"));
                }
                return self.resume_submission(&key, &submission, profile, now);
            }
            Err(error) => return Err(error),
        }
        // This attempt owns the key. Admission and state are written before the candidate is
        // pinned, so a crash can never leave a pin with no visible run. A retry sees the
        // admission and completes the pin.
        let written = (|| -> Result<()> {
            self.store.write_profile(&run_id, &resolved)?;
            self.store.ensure_json(&self.store.admission_path(&run_id), &admission)?;
            self.store.ensure_json(&self.store.state_path(&run_id), &state)?;
            Ok(())
        })();
        if let Err(error) = written {
            // A concurrent helper may already have completed this run from the same key. Leave
            // its key, admission and state in place; only a wholly unclaimed run is rolled back.
            if !self.store.admission_path(&run_id).is_file() && !self.store.state_path(&run_id).is_file() {
                let _ = std::fs::remove_file(&key_path);
                let _ = std::fs::remove_file(self.store.profile_path(&run_id));
            }
            return Err(error);
        }
        self.pin_run(&run_id, &candidate.candidate_id)?;
        self.submit_result(&run_id, candidate, profile)
    }

    /// A key-mapped submission is resumed without touching the profile registry or candidate
    /// store unless the admission itself has to be repaired.
    fn resume_submission(&self, key: &SubmissionKey, submission: &SubmissionRecord, profile: &ProfileRef, now: u64) -> Result<SubmitResult> {
        match self.load_admission(&submission.run_id) {
            Ok(admission) => {
                self.repair_state(&submission.run_id, &admission, now)?;
                let state = self.load_state(&submission.run_id)?;
                if !state.state.is_terminal() {
                    self.pin_recovered(&submission.run_id, &admission.candidate.candidate_id)?;
                }
                self.submit_result(&submission.run_id, &admission.candidate, &admission.profile)
            }
            Err(error) if error.code == ErrorCode::NotFound => {
                // The process stopped after writing the key but before the admission. Complete
                // the admission from the key; this is the only replay that needs the profile.
                let resolved = self.resolve_profile(profile)?;
                let admission = AdmissionRecord {
                    schema_version: RECORD_SCHEMA.into(),
                    run_id: submission.run_id.clone(),
                    submission_key: key.text.clone(),
                    payload_digest: submission.payload_digest.clone(),
                    candidate: submission.candidate.clone(),
                    profile: submission.profile.clone(),
                    owner: submission.owner.clone(),
                    runner: self.runner.identity(),
                    host_id: self.host_id.clone(),
                    host_pid: std::process::id(),
                    created_at_ms: submission.created_at_ms,
                    queued_deadline_ms: submission.created_at_ms.saturating_add(duration_ms(self.limits.queue_admission)),
                };
                self.store.write_profile(&submission.run_id, &resolved)?;
                self.store.ensure_json(&self.store.admission_path(&submission.run_id), &admission)?;
                self.repair_state(&submission.run_id, &admission, now)?;
                let state = self.load_state(&submission.run_id)?;
                if !state.state.is_terminal() {
                    self.pin_recovered(&submission.run_id, &submission.candidate.candidate_id)?;
                }
                self.submit_result(&submission.run_id, &admission.candidate, &admission.profile)
            }
            Err(error) => Err(error),
        }
    }

    /// Repair a lost state file from the admission (queued) or an already committed report.
    fn repair_state(&self, run_id: &str, admission: &AdmissionRecord, now: u64) -> Result<()> {
        if self.store.state_path(run_id).is_file() {
            return Ok(());
        }
        if self.store.report_exists(run_id)
            && let Ok(bytes) = self.store.read_bounded(&self.store.report_path(run_id), self.limits.report_bytes)
            && let Ok(report) = serde_json::from_slice::<ValidationReport>(&bytes)
            && report.state.is_terminal()
        {
            let prospective = StateRecord {
                schema_version: RECORD_SCHEMA.into(),
                run_id: run_id.into(),
                state: report.state,
                updated_at_ms: now,
                report_available: true,
            };
            if self.verify_report(run_id, admission, &prospective, &report).is_ok() {
                self.store.write_json_replace(&self.store.state_path(run_id), &prospective)?;
                self.unpin(run_id)?;
                return Ok(());
            }
        }
        let state = StateRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.into(),
            state: ValidationState::Queued,
            updated_at_ms: now,
            report_available: false,
        };
        match self.store.write_json_new(&self.store.state_path(run_id), &state) {
            Ok(()) => Ok(()),
            Err(_) if self.store.state_path(run_id).is_file() => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn submit_result(&self, run_id: &str, candidate: &CandidateRef, profile: &ProfileRef) -> Result<SubmitResult> {
        let state = self.load_state(run_id)?.state;
        Ok(SubmitResult { run_id: run_id.into(), state, candidate: candidate.clone(), profile: profile.clone() })
    }

    fn pin_run(&self, run_id: &str, candidate_id: &str) -> Result<()> {
        let _guard = self.pins.lock().map_err(|_| ValidationError::internal("validation pin lock was poisoned"))?;
        self.pin_run_unlocked(run_id, candidate_id)
    }

    /// A replay of an already admitted run may find its candidate expired. The original run is
    /// still returned; only pin it when the inputs are still there.
    fn pin_recovered(&self, run_id: &str, candidate_id: &str) -> Result<()> {
        match self.pin_run(run_id, candidate_id) {
            Err(error) if error.code == ErrorCode::InvalidCandidate => Ok(()),
            other => other,
        }
    }

    fn pin_run_unlocked(&self, run_id: &str, candidate_id: &str) -> Result<()> {
        // A concurrent cleanup may have removed the candidate before this pin took the lock.
        if !self.candidate_root.join(format!("{candidate_id}.zip")).is_file() {
            return Err(ValidationError::invalid_candidate("the candidate inputs are no longer available"));
        }
        let first = self.store.pinned_runs(candidate_id).is_empty();
        if first {
            solidcraft_io::candidate::pin(&self.candidate_root, candidate_id)
                .map_err(|error| ValidationError::invalid_candidate(error.to_string()))?;
        }
        match self.store.write_run_pin(candidate_id, run_id) {
            Ok(_) => Ok(()),
            Err(error) => {
                if first && self.store.pinned_runs(candidate_id).is_empty() {
                    let _ = solidcraft_io::candidate::unpin(&self.candidate_root, candidate_id);
                }
                Err(error)
            }
        }
    }

    fn unpin_run(&self, run_id: &str, candidate_id: &str) -> Result<()> {
        let _guard = self.pins.lock().map_err(|_| ValidationError::internal("validation pin lock was poisoned"))?;
        self.unpin_run_unlocked(run_id, candidate_id)
    }

    fn unpin_run_unlocked(&self, run_id: &str, candidate_id: &str) -> Result<()> {
        let last = self.store.remove_run_pin(candidate_id, run_id)?;
        if last {
            solidcraft_io::candidate::unpin(&self.candidate_root, candidate_id).map_err(|error| ValidationError::internal(error.to_string()))?;
        }
        Ok(())
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
            self.purge_records(now);
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
        if !self.store.state_path(run_id).is_file() {
            self.repair_state(run_id, &admission, self.clock.now_ms())?;
        }
        let state = self.load_state(run_id)?;
        let report_available = self.report_available(run_id, &admission, &state);
        Ok(RunStatus {
            run_id: run_id.into(),
            state: state.state,
            candidate: admission.candidate,
            profile: admission.profile,
            updated_at: ids::timestamp(state.updated_at_ms),
            report_available,
        })
    }

    /// Availability is a read-only verification, not file existence: a corrupt or unverifiable
    /// terminal report is not offered, even before a worker port reconciles it.
    fn report_available(&self, run_id: &str, admission: &AdmissionRecord, state: &StateRecord) -> bool {
        if !state.state.is_terminal() || !self.store.report_exists(run_id) {
            return false;
        }
        let Ok(bytes) = self.store.read_bounded(&self.store.report_path(run_id), self.limits.report_bytes) else { return false };
        let Ok(report) = serde_json::from_slice::<ValidationReport>(&bytes) else { return false };
        self.verify_report(run_id, admission, state, &report).is_ok()
    }

    /// `cancel(runId)`: persist the marker before any signal, then finalize a queued job. A
    /// terminal state is preserved.
    pub fn cancel(&self, run_id: &str) -> Result<CancelResult> {
        let _guard = self.commit.lock().map_err(|_| ValidationError::internal("validation commit lock was poisoned"))?;
        let admission = self.load_admission(run_id)?;
        if !self.store.state_path(run_id).is_file() {
            self.repair_state(run_id, &admission, self.clock.now_ms())?;
        }
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
        let _guard = self.commit.lock().map_err(|_| ValidationError::internal("validation commit lock was poisoned"))?;
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
            let admission = self.load_admission(&run_id)?;
            queued.push((admission.created_at_ms, run_id));
        }
        queued.sort();
        for (_, run_id) in queued {
            let state = match self.load_state(&run_id) {
                Ok(state) => state,
                Err(_) => continue,
            };
            if state.state != ValidationState::Queued {
                continue;
            }
            let admission = self.load_admission(&run_id)?;
            let profile = self.store.read_profile(&run_id)?;
            if profile_digest(&profile).ok().as_deref() != Some(admission.profile.digest.as_str()) {
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::InfrastructureError,
                    Some(ValidationError::report_mismatch("the frozen profile does not match its admitted digest")),
                );
                self.commit_report(&run_id, report, ValidationState::InfrastructureError)?;
                continue;
            }
            if self.store.marker_exists(&run_id) {
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::Cancelled,
                    Some(ValidationError::cancelled("the job was cancelled before a worker started")),
                );
                self.commit_report(&run_id, report, ValidationState::Cancelled)?;
                continue;
            }
            if now >= admission.queued_deadline_ms {
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::InfrastructureError,
                    Some(ValidationError::timeout("the job expired in the admission queue")),
                );
                self.commit_report(&run_id, report, ValidationState::InfrastructureError)?;
                continue;
            }
            if profile.checks.len() > self.limits.checks {
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::InfrastructureError,
                    Some(ValidationError::resource_exhausted("the profile exceeds the host check limit", "checks", self.limits.checks as u64)),
                );
                self.commit_report(&run_id, report, ValidationState::InfrastructureError)?;
                continue;
            }
            if let Err(error) = self.pin_run(&run_id, &admission.candidate.candidate_id) {
                let report = self.recovery_report(
                    &admission,
                    &profile,
                    ValidationState::InfrastructureError,
                    Some(ValidationError::invalid_candidate(format!("the candidate inputs could not be pinned: {error}"))),
                );
                self.commit_report(&run_id, report, ValidationState::InfrastructureError)?;
                continue;
            }
            let worker = WorkerRecord {
                schema_version: RECORD_SCHEMA.into(),
                run_id: run_id.clone(),
                runner: admission.runner.clone(),
                worker_id: format!("worker-{}", keys::uuid_v4()),
                pid: std::process::id(),
                host_id: self.host_id.clone(),
                host_pid: std::process::id(),
                started_at_ms: now,
                deadline_ms: now.saturating_add(duration_ms(self.limits.execution)),
            };
            // The worker record is the claim token; a second host cannot start the same run.
            match self.store.write_json_new(&self.store.worker_path(&run_id), &worker) {
                Ok(()) => {}
                Err(_) if self.store.worker_path(&run_id).is_file() => continue,
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
                continue;
            }
            let state = StateRecord {
                schema_version: RECORD_SCHEMA.into(),
                run_id: run_id.clone(),
                state: ValidationState::Running,
                updated_at_ms: now,
                report_available: false,
            };
            self.store.write_json_replace(&self.store.state_path(&run_id), &state)?;
            return Ok(Some(RunJob {
                run_id,
                candidate: admission.candidate,
                candidate_root: self.candidate_root.clone(),
                profile,
                started_at_ms: now,
            }));
        }
        Ok(None)
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
        if !self.store.state_path(run_id).is_file() {
            self.repair_state(run_id, &admission, self.clock.now_ms())?;
        }
        let state = self.load_state(run_id)?;
        if !state.state.is_terminal() {
            return Err(ValidationError::not_ready("the validation run has no terminal report yet"));
        }
        let bytes = self.store.read_bounded(&self.store.report_path(run_id), self.limits.report_bytes).map_err(|error| {
            if error.code == ErrorCode::NotFound { ValidationError::not_ready("the validation run has no report") } else { error }
        })?;
        let report: ValidationReport =
            serde_json::from_slice(&bytes).map_err(|_| ValidationError::not_ready("the validation run's report is not verifiable"))?;
        if let Err(error) = self.verify_report(run_id, &admission, &state, &report) {
            // Candidate/input-byte integrity failures keep their permanent §3.2 codes. Only
            // report/profile/evidence/state verification failures mean "no report available".
            return Err(match error.code {
                ErrorCode::InvalidCandidate | ErrorCode::DependencyChanged | ErrorCode::NotFound => error,
                _ => ValidationError::not_ready(format!("the validation run's report is not verifiable: {}", error.message)),
            });
        }
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
        let profile = self
            .store
            .read_profile(run_id)
            .map_err(|error| ValidationError::report_mismatch(format!("the frozen profile is unavailable: {error}")))?;
        let frozen_digest =
            profile_digest(&profile).map_err(|error| ValidationError::report_mismatch(format!("the frozen profile is not digestible: {error}")))?;
        if profile.profile_id != report.profile.profile_id || profile.version != report.profile.version || frozen_digest != report.profile.digest {
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
                if !ids::valid_id(&evidence.artifact_id) {
                    return Err(ValidationError::report_mismatch("a report evidence artifact id is not a valid Id"));
                }
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

    /// Remove completed records and unpinned candidate inputs older than the retention window.
    /// Active jobs and their pins are never evicted.
    pub fn cleanup(&self, now_ms: u64) -> usize {
        let Ok(_guard) = self.pins.lock() else { return 0 };
        self.cleanup_locked(now_ms)
    }

    fn cleanup_locked(&self, now_ms: u64) -> usize {
        let removed = self.purge_records_locked(now_ms);
        let now = UNIX_EPOCH.checked_add(Duration::from_millis(now_ms)).unwrap_or(SystemTime::UNIX_EPOCH);
        let _ = solidcraft_io::candidate::cleanup(&self.candidate_root, now, self.limits.retention);
        removed
    }

    /// Remove expired records and their per-run pins, leaving candidate files for `cleanup`'s
    /// explicit candidate sweep.
    fn purge_records(&self, now_ms: u64) -> usize {
        let Ok(_guard) = self.pins.lock() else { return 0 };
        self.purge_records_locked(now_ms)
    }

    fn purge_records_locked(&self, now_ms: u64) -> usize {
        let removed = self.store.purge_older_than(now_ms, duration_ms(self.limits.retention));
        for (run_id, candidate) in &removed {
            if let Some(candidate) = candidate {
                let _ = self.unpin_run_unlocked(run_id, candidate);
            }
        }
        removed.len()
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
        let current = self.load_state(run_id)?;
        if current.state.is_terminal() {
            if current.state != state {
                return Err(ValidationError::report_mismatch("a terminal state cannot be paired with a report for another state"));
            }
            // A concurrent writer already committed this terminal state. Keep its report: a
            // cancelled run's report is stable, and a second cancel must not fail with
            // REPORT_MISMATCH (#88). A missing report still needs to be written.
            if self.store.report_exists(run_id) {
                return Ok(());
            }
        }
        let mut bytes = self.serialize_report(&report)?;
        let path = self.store.report_path(run_id);
        let mut created = false;
        match self.store.write_bytes_new(&path, &bytes) {
            Ok(()) => created = true,
            Err(error) => {
                let existing = self.store.read_bounded(&path, self.limits.report_bytes).map_err(|_| error)?;
                if existing != bytes {
                    // The other writer may have committed the same terminal state between our
                    // state read and the report write. Its report is authoritative.
                    let current = self.load_state(run_id)?;
                    if current.state.is_terminal() && current.state == state {
                        return Ok(());
                    }
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

    /// Keep an immutable terminal state but advertise that its report is not available. A
    /// missing or invalid report is never replaced by one whose state cannot bind the terminal
    /// state.
    fn mark_report_unavailable(&self, run_id: &str) -> Result<()> {
        let mut current = self.load_state(run_id)?;
        if !current.state.is_terminal() || !current.report_available {
            return Ok(());
        }
        current.report_available = false;
        current.updated_at_ms = self.clock.now_ms();
        self.store.write_json_replace(&self.store.state_path(run_id), &current)
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
        self.unpin_run(run_id, &admission.candidate.candidate_id)
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
        let _guard = self.commit.lock().map_err(|_| ValidationError::internal("validation commit lock was poisoned"))?;
        let now = self.clock.now_ms();
        for run_id in self.store.all_run_ids() {
            let admission = match self.load_admission(&run_id) {
                Ok(admission) => admission,
                // A state without an admission is not a run this host can reconcile.
                Err(_) => continue,
            };
            if !self.store.state_path(&run_id).is_file() {
                self.repair_state(&run_id, &admission, now)?;
            }
            let state = self.load_state(&run_id)?;
            let worker = self.store.read_optional_json::<WorkerRecord>(&self.store.worker_path(&run_id), 1 << 20).ok().flatten();
            let owned_or_dead = if state.state == ValidationState::Running {
                worker
                    .as_ref()
                    .map(|worker| self.host_owned_or_dead(&worker.host_id, worker.host_pid))
                    .unwrap_or_else(|| self.host_owned_or_dead(&admission.host_id, admission.host_pid))
            } else {
                self.host_owned_or_dead(&admission.host_id, admission.host_pid)
            };
            if state.state.is_terminal() {
                if self.adopt_report(&run_id, &admission)? {
                    self.unpin(&run_id)?;
                } else {
                    // The terminal state is immutable. Never write a report whose state cannot
                    // bind it: remove an unverifiable report and publish reportAvailable=false.
                    if self.store.report_exists(&run_id) {
                        let _ = std::fs::remove_file(self.store.report_path(&run_id));
                    }
                    self.mark_report_unavailable(&run_id)?;
                    self.unpin(&run_id)?;
                }
                continue;
            }
            if !owned_or_dead {
                continue;
            }
            let profile = self.store.read_profile(&run_id)?;
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
            // Running: never rerun. A verified completed report is adopted; anything else is
            // unknown (or cancelled), never a pass.
            if self.adopt_report(&run_id, &admission)? {
                self.unpin(&run_id)?;
                continue;
            }
            if self.store.report_exists(&run_id) {
                let _ = std::fs::remove_file(self.store.report_path(&run_id));
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

    /// Adopt a committed terminal report only when it verifies against this run. Returns whether
    /// the state is terminal after adoption.
    fn adopt_report(&self, run_id: &str, admission: &AdmissionRecord) -> Result<bool> {
        if !self.store.report_exists(run_id) {
            return Ok(false);
        }
        let Ok(bytes) = self.store.read_bounded(&self.store.report_path(run_id), self.limits.report_bytes) else { return Ok(false) };
        let Ok(report) = serde_json::from_slice::<ValidationReport>(&bytes) else { return Ok(false) };
        if !report.state.is_terminal() {
            return Ok(false);
        }
        let prospective = StateRecord {
            schema_version: RECORD_SCHEMA.into(),
            run_id: run_id.into(),
            state: report.state,
            updated_at_ms: self.clock.now_ms(),
            report_available: true,
        };
        if self.verify_report(run_id, admission, &prospective, &report).is_err() {
            return Ok(false);
        }
        let current = self.load_state(run_id).ok();
        if current.as_ref().is_none_or(|state| !state.state.is_terminal()) {
            self.store.write_json_replace(&self.store.state_path(run_id), &prospective)?;
        }
        Ok(true)
    }

    fn host_owned_or_dead(&self, host_id: &str, host_pid: u32) -> bool {
        if host_id == self.host_id {
            return true;
        }
        if host_pid == 0 {
            return true;
        }
        // A live process keeps its worker even when a second port is opened in this process.
        !process_alive(host_pid)
    }
}

/// Whether a recorded host process is still alive. Same-process/unknown answers are handled by
/// the caller; this only asks the operating system.
fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        return std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        return output.is_ok_and(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            output.status.success() && text.contains(&pid.to_string())
        });
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
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
