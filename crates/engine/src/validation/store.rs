//! Durable validation records. Admission, worker identity, cancellation marker, report and
//! terminal state are written atomically and never silently replaced.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::profile::{CheckProfile, ProfileRef};
use super::report::{RunnerIdentity, ValidationState};
use super::{ValidationError, ids};
use crate::schema::snapshot::CandidateRef;

/// The record schema version.
pub const RECORD_SCHEMA: &str = "1.0";
/// The report file ceiling (contract §4).
pub const MAX_REPORT_BYTES: usize = 4 << 20;

/// What an idempotency key file holds. It is written first, so a retried submit finds the run
/// even when the process stopped before writing the admission.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SubmissionRecord {
    pub schema_version: String,
    pub submission_key: String,
    pub payload_digest: String,
    pub run_id: String,
    pub candidate: CandidateRef,
    pub profile: ProfileRef,
    pub owner: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

/// The immutable job admission.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AdmissionRecord {
    pub schema_version: String,
    pub run_id: String,
    pub submission_key: String,
    pub payload_digest: String,
    pub candidate: CandidateRef,
    pub profile: ProfileRef,
    pub owner: String,
    pub runner: RunnerIdentity,
    pub created_at_ms: u64,
    pub queued_deadline_ms: u64,
}

/// The current durable state. A terminal state is never replaced.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StateRecord {
    pub schema_version: String,
    pub run_id: String,
    pub state: ValidationState,
    pub updated_at_ms: u64,
    pub report_available: bool,
}

/// The identity and deadline of the worker that claimed a running job.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WorkerRecord {
    pub schema_version: String,
    pub run_id: String,
    pub runner: RunnerIdentity,
    pub worker_id: String,
    pub pid: u32,
    pub started_at_ms: u64,
    pub deadline_ms: u64,
}

/// A small crash-safe file store below one host directory.
#[derive(Clone, Debug)]
pub(super) struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self, ValidationError> {
        let store = Store { root };
        store.ensure_dirs()?;
        Ok(store)
    }

    fn ensure_dirs(&self) -> Result<(), ValidationError> {
        for dir in [self.runs_dir(), self.keys_dir(), self.evidence_root()] {
            fs::create_dir_all(&dir).map_err(|error| ValidationError::internal(format!("validation store: {error}")))?;
        }
        Ok(())
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    pub fn keys_dir(&self) -> PathBuf {
        self.root.join("keys")
    }

    pub fn evidence_root(&self) -> PathBuf {
        self.root.join("evidence")
    }

    pub fn admission_path(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(format!("{run_id}.json"))
    }

    pub fn profile_path(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(format!("{run_id}.profile.json"))
    }

    pub fn state_path(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(format!("{run_id}.state.json"))
    }

    pub fn worker_path(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(format!("{run_id}.worker.json"))
    }

    pub fn cancel_path(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(format!("{run_id}.cancel"))
    }

    pub fn report_path(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(format!("{run_id}.report.json"))
    }

    pub fn key_path(&self, file_name: &str) -> PathBuf {
        self.keys_dir().join(format!("{file_name}.json"))
    }

    pub fn evidence_dir(&self, run_id: &str) -> PathBuf {
        self.evidence_root().join(run_id)
    }

    pub fn evidence_path(&self, run_id: &str, artifact_id: &str) -> PathBuf {
        self.evidence_dir(run_id).join(format!("{artifact_id}.json"))
    }

    pub fn run_ids(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.runs_dir()) else { return Vec::new() };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".state.json").map(str::to_string)
            })
            .filter(|id| ids::valid_id(id))
            .collect();
        ids.sort();
        ids
    }

    pub fn admission_ids(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.runs_dir()) else { return Vec::new() };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".json")
                    && !name.contains(".state.")
                    && !name.contains(".profile.")
                    && !name.contains(".worker.")
                    && !name.contains(".report.")
                {
                    name.strip_suffix(".json").map(str::to_string)
                } else {
                    None
                }
            })
            .filter(|id| ids::valid_id(id))
            .collect();
        ids.sort();
        ids
    }

    pub fn count_admissions(&self) -> usize {
        self.admission_ids().len()
    }

    pub fn read_json<T: for<'de> Deserialize<'de>>(&self, path: &Path, limit: usize) -> Result<T, ValidationError> {
        let bytes = self.read_bounded(path, limit)?;
        serde_json::from_slice(&bytes).map_err(|error| ValidationError::internal(format!("{}: {error}", path.display())))
    }

    pub fn read_optional_json<T: for<'de> Deserialize<'de>>(&self, path: &Path, limit: usize) -> Result<Option<T>, ValidationError> {
        match self.read_bounded(path, limit) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| ValidationError::internal(format!("{}: {error}", path.display()))),
            Err(error) if error.code == super::ErrorCode::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn read_bounded(&self, path: &Path, limit: usize) -> Result<Vec<u8>, ValidationError> {
        let metadata = fs::metadata(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ValidationError::not_found(format!("{}", path.display()))
            } else {
                ValidationError::internal(format!("{}: {error}", path.display()))
            }
        })?;
        if !metadata.is_file() || metadata.len() > limit as u64 {
            return Err(ValidationError::internal(format!("{}: not a file within {limit} bytes", path.display())));
        }
        let file = fs::File::open(path).map_err(|error| ValidationError::internal(format!("{}: {error}", path.display())))?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|error| ValidationError::internal(format!("{}: {error}", path.display())))?;
        if bytes.len() > limit {
            return Err(ValidationError::internal(format!("{} exceeds {limit} bytes", path.display())));
        }
        Ok(bytes)
    }

    pub fn write_json_new<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), ValidationError> {
        let bytes = serde_json::to_vec(value).map_err(|error| ValidationError::internal(format!("record: {error}")))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ValidationError::internal(format!("validation store: {error}")))?;
        }
        solidcraft_io::safe_write::create_atomic(path, &bytes).map_err(|error| ValidationError::internal(format!("{}: {error}", path.display())))
    }

    pub fn write_json_replace<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), ValidationError> {
        let bytes = serde_json::to_vec(value).map_err(|error| ValidationError::internal(format!("record: {error}")))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ValidationError::internal(format!("validation store: {error}")))?;
        }
        solidcraft_io::safe_write::write_atomic(path, &bytes, false)
            .map_err(|error| ValidationError::internal(format!("{}: {error}", path.display())))
    }

    pub fn write_bytes_new(&self, path: &Path, bytes: &[u8]) -> Result<(), ValidationError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ValidationError::internal(format!("validation store: {error}")))?;
        }
        solidcraft_io::safe_write::create_atomic(path, bytes).map_err(|error| ValidationError::internal(format!("{}: {error}", path.display())))
    }

    pub fn marker_exists(&self, run_id: &str) -> bool {
        self.cancel_path(run_id).is_file()
    }

    pub fn report_exists(&self, run_id: &str) -> bool {
        self.report_path(run_id).is_file()
    }

    /// Remove every record for a completed run older than the retention window. Active
    /// (nonterminal) runs are never touched.
    pub fn purge_older_than(&self, now_ms: u64, retention_ms: u64) -> usize {
        let mut removed = 0;
        for run_id in self.run_ids() {
            let state_path = self.state_path(&run_id);
            let Ok(bytes) = self.read_bounded(&state_path, 64 << 10) else { continue };
            let Ok(state) = serde_json::from_slice::<StateRecord>(&bytes) else { continue };
            if !state.state.is_terminal() {
                continue;
            }
            if now_ms.saturating_sub(state.updated_at_ms) < retention_ms {
                continue;
            }
            for path in [
                self.admission_path(&run_id),
                self.profile_path(&run_id),
                self.state_path(&run_id),
                self.worker_path(&run_id),
                self.cancel_path(&run_id),
                self.report_path(&run_id),
            ] {
                let _ = fs::remove_file(path);
            }
            let _ = fs::remove_dir_all(self.evidence_dir(&run_id));
            removed += 1;
        }
        self.purge_keys(now_ms, retention_ms);
        removed
    }

    fn purge_keys(&self, now_ms: u64, retention_ms: u64) {
        let Ok(entries) = fs::read_dir(self.keys_dir()) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(bytes) = self.read_bounded(&path, 64 << 10) else { continue };
            let Ok(record) = serde_json::from_slice::<SubmissionRecord>(&bytes) else { continue };
            if record.expires_at_ms.saturating_add(retention_ms) > now_ms {
                continue;
            }
            let run_terminal_old = self
                .read_optional_json::<StateRecord>(&self.state_path(&record.run_id), 64 << 10)
                .ok()
                .flatten()
                .is_some_and(|state| state.state.is_terminal() && now_ms.saturating_sub(state.updated_at_ms) >= retention_ms);
            if run_terminal_old || !self.state_path(&record.run_id).is_file() {
                let _ = fs::remove_file(path);
            }
        }
    }

    /// The frozen profile bytes were persisted at admission so later verification never depends
    /// on a registry entry changing.
    pub fn write_profile(&self, run_id: &str, profile: &CheckProfile) -> Result<(), ValidationError> {
        self.write_json_new(&self.profile_path(run_id), profile)
    }

    pub fn read_profile(&self, run_id: &str) -> Result<CheckProfile, ValidationError> {
        self.read_json(&self.profile_path(run_id), 1 << 20)
    }
}
