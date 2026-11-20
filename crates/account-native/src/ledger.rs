//! Durable, at-most-once operation journal.
//!
//! Identity comes only from [`AuthenticatedHostScope`], which the host authentication path owns.
//! A transport correlation is a lookup hint, never authority. The ledger publishes immutable files
//! (create-only, synced, then hard-linked into place), so a retry can never overwrite an admitted
//! operation and a crash leaves either the whole entry or no entry.
//!
//! The caller injects the ledger root; this crate never reads `$HOME`. The ledger stores no
//! token, password or session secret.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, Capability, EntryCategory, FreshIntent, GrantId, OperationId, OperationRecord, OperationStatus,
    OperationStore, OperationStoreError, RetryDecision, RetryMapping, RetryScopeKey, ReviewOperation, RunId, SaveOutcome, TransportCorrelation,
};

const MAPPINGS_DIR: &str = "mappings";
const RECORDS_DIR: &str = "records";

/// The ledger could not read or publish a file. Callers must fail closed.
#[derive(Debug)]
enum LedgerError {
    Io(std::io::Error),
    Corrupt,
}

impl From<std::io::Error> for LedgerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Corrupt => formatter.write_str("corrupt ledger entry"),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedRecord {
    operation: String,
    account: String,
    capability: String,
    entry: String,
    started_at_ms: u64,
    deadline_ms: Option<u64>,
    grant_id: Option<String>,
    review_run_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PersistedMapping {
    version: u8,
    account: String,
    installation: String,
    /// The host session binding is a non-secret scope label; store only its digest so a host that
    /// uses a sensitive binding value never leaves it at rest.
    session_binding_digest: String,
    correlation: String,
    operation: String,
    capability: String,
    entry: String,
    created_at_ms: u64,
    record: PersistedRecord,
}

#[derive(Serialize, Deserialize)]
struct PersistedReview {
    run_id: String,
    operation_id: String,
}

#[derive(Serialize, Deserialize)]
struct StartedMarker {
    started_at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct TerminalMarker {
    status: String,
}

/// A durable operation ledger rooted at a caller-supplied directory.
pub struct FileOperationStore {
    root: PathBuf,
}

impl FileOperationStore {
    /// Open (and create if needed) a ledger directory.
    ///
    /// The caller must pass a directory the host has already made private to the current user.
    /// On Unix this method tightens the root and its folders to `0700` and files to `0600`. On
    /// Windows it applies no ACL: the caller must use a private root such as the per-user app-data
    /// directory, and Windows ACL/durability behaviour remains unverified until Task 13. This crate
    /// never changes system-wide permissions and never uses unsafe code.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let store = Self { root: root.into() };
        store.ensure_layout()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ensure_layout(&self) -> std::io::Result<()> {
        for directory in [&self.root, &self.root.join(RECORDS_DIR), &self.root.join(MAPPINGS_DIR)] {
            fs::create_dir_all(directory)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(())
    }

    fn records_dir(&self) -> PathBuf {
        self.root.join(RECORDS_DIR)
    }

    fn mappings_dir(&self) -> PathBuf {
        self.root.join(MAPPINGS_DIR)
    }

    fn record_path(&self, operation: &OperationId) -> PathBuf {
        self.records_dir().join(format!("{}.json", operation.as_str()))
    }

    fn started_path(&self, operation: &OperationId) -> PathBuf {
        self.records_dir().join(format!("{}.started.json", operation.as_str()))
    }

    fn terminal_path(&self, operation: &OperationId) -> PathBuf {
        self.records_dir().join(format!("{}.terminal.json", operation.as_str()))
    }

    fn operation_review_path(&self, operation: &OperationId) -> PathBuf {
        self.records_dir().join(format!("{}.review.json", operation.as_str()))
    }

    fn mapping_path(&self, key: &RetryScopeKey) -> PathBuf {
        self.mappings_dir().join(format!("{}.json", scope_digest(key)))
    }

    /// The base row: the standalone record file first, then the record embedded in a mapping.
    fn load_base(&self, operation: &OperationId) -> Result<Option<PersistedRecord>, LedgerError> {
        if let Some(record) = read_json::<PersistedRecord>(&self.record_path(operation))? {
            if record.operation != operation.as_str() {
                return Err(LedgerError::Corrupt);
            }
            return Ok(Some(record));
        }
        let entries = match fs::read_dir(self.mappings_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(LedgerError::Io(error)),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !is_json_file(&path) {
                continue;
            }
            match read_json::<PersistedMapping>(&path) {
                Ok(Some(mapping)) if mapping.record.operation == operation.as_str() => return Ok(Some(mapping.record)),
                Ok(_) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    fn load_and_apply_markers(&self, operation: &OperationId) -> Result<Option<OperationRecord>, LedgerError> {
        let Some(base) = self.load_base(operation)? else {
            return Ok(None);
        };
        let mut record = from_persisted(base)?;
        if let Some(started) = read_json::<StartedMarker>(&self.started_path(operation))? {
            record.started_at_ms = started.started_at_ms;
            if record.status == OperationStatus::Held {
                record.status = OperationStatus::Pending;
            }
        }
        if let Some(terminal) = read_json::<TerminalMarker>(&self.terminal_path(operation))? {
            let status = status_from_name(&terminal.status).ok_or(LedgerError::Corrupt)?;
            if !status.is_terminal() {
                return Err(LedgerError::Corrupt);
            }
            record.status = status;
        }
        if let Some(review) = read_json::<PersistedReview>(&self.operation_review_path(operation))? {
            if review.operation_id != operation.as_str() {
                return Err(LedgerError::Corrupt);
            }
            record.review_run_id = Some(RunId::new(review.run_id));
        }
        Ok(Some(record))
    }

    fn mapping_for(&self, key: &RetryScopeKey) -> Result<Option<PersistedMapping>, LedgerError> {
        read_json::<PersistedMapping>(&self.mapping_path(key))
    }

    /// Whether a correlation already appears in any stored mapping. A seen correlation with no
    /// matching scope is an unprovable retry, so it must not silently mint a fresh operation.
    fn correlation_seen(&self, correlation: &TransportCorrelation) -> Result<bool, LedgerError> {
        let entries = match fs::read_dir(self.mappings_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(LedgerError::Io(error)),
        };
        for entry in entries {
            let path = entry?.path();
            if !is_json_file(&path) {
                continue;
            }
            if let Some(mapping) = read_json::<PersistedMapping>(&path)?
                && mapping.correlation == correlation.as_str()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Publish the canonical mapping plus embedded record. Create-only, so concurrent writers
    /// publish exactly one row and the loser reads that row instead of overwriting it.
    fn publish_intent(
        &self,
        record: &OperationRecord,
        scope: &AuthenticatedHostScope,
        correlation: &TransportCorrelation,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        let key = RetryScopeKey { host_scope: scope.clone(), transport_correlation: correlation.clone() };
        if let Ok(Some(mapping)) = self.mapping_for(&key) {
            return self.mapping_decision(&mapping, &key, capability, entry);
        }
        let canonical = match self.load_and_apply_markers(&record.operation) {
            Ok(Some(existing)) if existing.common_identity_matches(record) && !existing.status.is_terminal() => existing,
            Ok(Some(_)) | Err(_) => return Err(SaveOutcome::Conflict),
            Ok(None) => record.clone(),
        };
        let persisted = to_persisted(&canonical).map_err(|_| SaveOutcome::Conflict)?;
        let mapping = PersistedMapping {
            version: 1,
            account: scope.account().as_str().to_owned(),
            installation: scope.installation().as_str().to_owned(),
            session_binding_digest: digest(&[scope.session_binding().as_str()]),
            correlation: correlation.as_str().to_owned(),
            operation: canonical.operation.as_str().to_owned(),
            capability: capability.wire().to_owned(),
            entry: entry_name(entry).to_owned(),
            created_at_ms: now_ms(),
            record: persisted,
        };
        let bytes = serde_json::to_vec(&mapping).map_err(|_| SaveOutcome::Conflict)?;
        match publish_create_only(&self.mapping_path(&key), &bytes) {
            Ok(true) => Ok(canonical),
            Ok(false) => match self.mapping_for(&key) {
                Ok(Some(mapping)) => self.mapping_decision(&mapping, &key, capability, entry),
                Ok(None) | Err(_) => Err(SaveOutcome::Conflict),
            },
            Err(_) => Err(SaveOutcome::Conflict),
        }
    }

    fn mapping_scope_matches(mapping: &PersistedMapping, key: &RetryScopeKey) -> bool {
        mapping.account == key.host_scope.account().as_str()
            && mapping.installation == key.host_scope.installation().as_str()
            && mapping.session_binding_digest == digest(&[key.host_scope.session_binding().as_str()])
            && mapping.correlation == key.transport_correlation.as_str()
    }

    fn mapping_decision(
        &self,
        mapping: &PersistedMapping,
        key: &RetryScopeKey,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        if !Self::mapping_scope_matches(mapping, key)
            || mapping.capability != capability.wire()
            || mapping.entry != entry_name(entry)
            || mapping.record.capability != mapping.capability
            || mapping.record.entry != mapping.entry
        {
            return Err(SaveOutcome::Conflict);
        }
        let operation = OperationId::parse(&mapping.operation).map_err(|_| SaveOutcome::Conflict)?;
        match self.load_and_apply_markers(&operation) {
            Ok(Some(record)) if record.capability == capability && record.entry == entry && !record.status.is_terminal() => Ok(record),
            Ok(_) | Err(_) => Err(SaveOutcome::Conflict),
        }
    }

    fn retry_decision(&self, key: &RetryScopeKey) -> RetryDecision {
        let mapping = match self.mapping_for(key) {
            Ok(Some(mapping)) => mapping,
            Ok(None) => return RetryDecision::Unprovable,
            Err(_) => return RetryDecision::Unknown,
        };
        if !Self::mapping_scope_matches(&mapping, key) {
            return RetryDecision::Conflict;
        }
        let Some(capability) = Capability::from_wire(&mapping.capability) else {
            return RetryDecision::Unknown;
        };
        let Some(entry) = entry_from_persisted_name(&mapping.entry) else {
            return RetryDecision::Unknown;
        };
        if mapping.record.capability != mapping.capability || mapping.record.entry != mapping.entry {
            return RetryDecision::Conflict;
        }
        let operation = match OperationId::parse(&mapping.operation) {
            Ok(operation) => operation,
            Err(_) => return RetryDecision::Unknown,
        };
        match self.load_and_apply_markers(&operation) {
            Ok(Some(record)) if record.capability == capability && record.entry == entry => {
                if record.status.is_terminal() {
                    RetryDecision::Unknown
                } else {
                    RetryDecision::Original(record)
                }
            }
            Ok(_) | Err(_) => RetryDecision::Unknown,
        }
    }
}

impl OperationStore for FileOperationStore {
    fn save_new(&mut self, record: &OperationRecord) -> Result<SaveOutcome, OperationStoreError> {
        if record.status != OperationStatus::Held || record.grant_id.is_some() || record.entry.is_free() {
            return Err(OperationStoreError::InvalidState);
        }
        match self.save_record_once(&record.operation, record) {
            Ok(outcome) => Ok(outcome),
            Err(_) => Err(OperationStoreError::Unavailable),
        }
    }

    fn load(&self, operation: &OperationId) -> Result<Option<OperationRecord>, OperationStoreError> {
        self.load_and_apply_markers(operation).map_err(|_| OperationStoreError::Unavailable)
    }

    fn mark_started(&mut self, operation: &OperationId, started_at_ms: u64) -> Result<(), OperationStoreError> {
        let Some(record) = self.load_and_apply_markers(operation).map_err(|_| OperationStoreError::Unavailable)? else {
            return Err(OperationStoreError::UnknownOperation);
        };
        if record.status.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        let bytes = serde_json::to_vec(&StartedMarker { started_at_ms }).map_err(|_| OperationStoreError::Unavailable)?;
        publish_create_only(&self.started_path(operation), &bytes).map(|_| ()).map_err(|_| OperationStoreError::Unavailable)
    }

    fn mark_terminal(&mut self, operation: &OperationId, status: OperationStatus) -> Result<(), OperationStoreError> {
        if !status.is_terminal() {
            return Err(OperationStoreError::InvalidState);
        }
        let Some(record) = self.load_and_apply_markers(operation).map_err(|_| OperationStoreError::Unavailable)? else {
            return Err(OperationStoreError::UnknownOperation);
        };
        if record.status.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        let bytes = serde_json::to_vec(&TerminalMarker { status: status_name(status).to_owned() }).map_err(|_| OperationStoreError::Unavailable)?;
        match publish_create_only(&self.terminal_path(operation), &bytes) {
            Ok(true) => Ok(()),
            Ok(false) => Err(OperationStoreError::TerminalState),
            Err(_) => Err(OperationStoreError::Unavailable),
        }
    }

    fn persist_intent(
        &mut self,
        scope: &AuthenticatedHostScope,
        correlation: &TransportCorrelation,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        if !scope.is_provable() || entry.is_free() {
            return Err(SaveOutcome::Conflict);
        }
        let key = RetryScopeKey { host_scope: scope.clone(), transport_correlation: correlation.clone() };
        if let Ok(Some(mapping)) = self.mapping_for(&key) {
            return self.mapping_decision(&mapping, &key, capability, entry);
        }
        match self.correlation_seen(correlation) {
            Ok(false) => {}
            Ok(true) | Err(_) => return Err(SaveOutcome::Conflict),
        }
        let operation = mint_operation().map_err(|_| SaveOutcome::Conflict)?;
        let record = OperationRecord::held(operation, scope.account().clone(), capability, entry, now_ms());
        self.publish_intent(&record, scope, correlation, capability, entry)
    }

    fn persist_fresh_intent(&mut self, intent: &FreshIntent, capability: Capability, entry: EntryCategory) -> Result<OperationRecord, SaveOutcome> {
        if !intent.is_provable() || entry.is_free() {
            return Err(SaveOutcome::Conflict);
        }
        let record = OperationRecord::held(intent.host_minted_operation().clone(), intent.scope().account().clone(), capability, entry, now_ms());
        self.publish_intent(&record, intent.scope(), intent.transport_correlation(), capability, entry)
    }

    fn lookup_original_retry(&self, key: &RetryScopeKey) -> RetryDecision {
        if !key.host_scope.is_provable() {
            return RetryDecision::Unprovable;
        }
        self.retry_decision(key)
    }

    fn record_review(&mut self, review: &ReviewOperation) -> Result<(), OperationStoreError> {
        match self.load_and_apply_markers(&review.operation_id) {
            Ok(Some(_)) => {}
            Ok(None) => return Err(OperationStoreError::UnknownOperation),
            Err(_) => return Err(OperationStoreError::Unavailable),
        }
        match review_operation_by_run(self, &review.run_id).map_err(|_| OperationStoreError::Unavailable)? {
            Some(existing) if existing != review.operation_id => return Err(OperationStoreError::InvalidState),
            _ => {}
        }
        // One canonical, operation-keyed association. run -> operation is derived by scanning this
        // file set, so a crash between two independent indices cannot leave a half-linked identity.
        let persisted = PersistedReview { run_id: review.run_id.as_str().to_owned(), operation_id: review.operation_id.as_str().to_owned() };
        let bytes = serde_json::to_vec(&persisted).map_err(|_| OperationStoreError::Unavailable)?;
        let path = self.operation_review_path(&review.operation_id);
        match read_json::<PersistedReview>(&path).map_err(|_| OperationStoreError::Unavailable)? {
            Some(existing) => {
                if existing.run_id == review.run_id.as_str() && existing.operation_id == review.operation_id.as_str() {
                    Ok(())
                } else {
                    Err(OperationStoreError::InvalidState)
                }
            }
            None => match publish_create_only(&path, &bytes) {
                Ok(true) => Ok(()),
                Ok(false) => match read_json::<PersistedReview>(&path) {
                    Ok(Some(existing)) if existing.run_id == review.run_id.as_str() && existing.operation_id == review.operation_id.as_str() => {
                        Ok(())
                    }
                    _ => Err(OperationStoreError::InvalidState),
                },
                Err(_) => Err(OperationStoreError::Unavailable),
            },
        }
    }

    fn review_operation(&self, run_id: &RunId) -> Result<Option<OperationId>, OperationStoreError> {
        let Some(operation) = review_operation_by_run(self, run_id).map_err(|_| OperationStoreError::Unavailable)? else {
            return Ok(None);
        };
        match self.load_and_apply_markers(&operation).map_err(|_| OperationStoreError::Unavailable)? {
            Some(_) => Ok(Some(operation)),
            None => Err(OperationStoreError::Unavailable),
        }
    }
}

/// An in-memory ledger for tests only. It never grants execution authority.
#[derive(Debug, Default)]
pub struct InMemoryOperationStore {
    records: BTreeMap<OperationId, OperationRecord>,
    mappings: BTreeMap<RetryScopeKey, RetryMapping>,
    reviews: BTreeMap<RunId, OperationId>,
    next: u64,
    pub admissions: usize,
    pub commits: usize,
    pub fresh_intents: usize,
    pub events: Vec<&'static str>,
}

impl InMemoryOperationStore {
    fn mint(&mut self) -> Result<OperationId, OperationStoreError> {
        self.next = self.next.saturating_add(1);
        let mut entropy = [0u8; 16];
        entropy[..8].copy_from_slice(&self.next.to_le_bytes());
        OperationId::mint_from_entropy(&entropy).map_err(|_| OperationStoreError::Unavailable)
    }

    /// The test drives a lost response: the original row stays non-terminal.
    pub fn simulate_lost_response(&mut self, operation: OperationId) {
        if let Some(record) = self.records.get_mut(&operation)
            && !record.status.is_terminal()
        {
            record.status = OperationStatus::Pending;
        }
    }

    /// Reconnect keeps the same scope; the mapping is unchanged.
    pub fn reconnect(&mut self, _scope: &AuthenticatedHostScope) {}
}

impl OperationStore for InMemoryOperationStore {
    fn save_new(&mut self, record: &OperationRecord) -> Result<SaveOutcome, OperationStoreError> {
        if record.status != OperationStatus::Held || record.grant_id.is_some() || record.entry.is_free() {
            return Err(OperationStoreError::InvalidState);
        }
        if let Some(existing) = self.records.get(&record.operation) {
            return Ok(if existing.common_identity_matches(record) { SaveOutcome::Existing } else { SaveOutcome::Conflict });
        }
        self.records.insert(record.operation.clone(), record.clone());
        self.admissions = self.admissions.saturating_add(1);
        Ok(SaveOutcome::Saved)
    }

    fn load(&self, operation: &OperationId) -> Result<Option<OperationRecord>, OperationStoreError> {
        Ok(self.records.get(operation).cloned())
    }

    fn mark_started(&mut self, operation: &OperationId, started_at_ms: u64) -> Result<(), OperationStoreError> {
        let record = self.records.get_mut(operation).ok_or(OperationStoreError::UnknownOperation)?;
        if record.status.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        record.status = OperationStatus::Pending;
        record.started_at_ms = started_at_ms;
        Ok(())
    }

    fn mark_terminal(&mut self, operation: &OperationId, status: OperationStatus) -> Result<(), OperationStoreError> {
        if !status.is_terminal() {
            return Err(OperationStoreError::InvalidState);
        }
        let record = self.records.get_mut(operation).ok_or(OperationStoreError::UnknownOperation)?;
        if record.status.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        record.status = status;
        if status == OperationStatus::Charged {
            self.commits = self.commits.saturating_add(1);
        }
        Ok(())
    }

    fn persist_intent(
        &mut self,
        scope: &AuthenticatedHostScope,
        correlation: &TransportCorrelation,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        if !scope.is_provable() || entry.is_free() {
            return Err(SaveOutcome::Conflict);
        }
        let key = RetryScopeKey { host_scope: scope.clone(), transport_correlation: correlation.clone() };
        if let Some(mapping) = self.mappings.get(&key) {
            if mapping.capability != capability || mapping.entry != entry {
                return Err(SaveOutcome::Conflict);
            }
            let record = self.records.get(&mapping.operation).ok_or(SaveOutcome::Conflict)?;
            if record.status.is_terminal() {
                return Err(SaveOutcome::Conflict);
            }
            return Ok(record.clone());
        }
        if self.mappings.keys().any(|existing| existing.transport_correlation == *correlation) {
            return Err(SaveOutcome::Conflict);
        }
        let operation = self.mint().map_err(|_| SaveOutcome::Conflict)?;
        let record = OperationRecord::held(operation.clone(), scope.account().clone(), capability, entry, 0);
        self.records.insert(operation.clone(), record.clone());
        self.mappings.insert(
            key,
            RetryMapping {
                scope: RetryScopeKey { host_scope: scope.clone(), transport_correlation: correlation.clone() },
                operation,
                capability,
                entry,
                created_at_ms: 0,
            },
        );
        self.admissions = self.admissions.saturating_add(1);
        self.events.push("mapping_persisted");
        self.events.push("wire_attempt");
        Ok(record)
    }

    fn persist_fresh_intent(&mut self, intent: &FreshIntent, capability: Capability, entry: EntryCategory) -> Result<OperationRecord, SaveOutcome> {
        if !intent.is_provable() || entry.is_free() {
            return Err(SaveOutcome::Conflict);
        }
        let key = RetryScopeKey { host_scope: intent.scope().clone(), transport_correlation: intent.transport_correlation().clone() };
        if let Some(mapping) = self.mappings.get(&key) {
            if mapping.operation != *intent.host_minted_operation() || mapping.capability != capability || mapping.entry != entry {
                return Err(SaveOutcome::Conflict);
            }
            return self.records.get(&mapping.operation).cloned().ok_or(SaveOutcome::Conflict);
        }
        if let Some(existing) = self.records.get(intent.host_minted_operation()) {
            return if existing.common_identity_matches(&OperationRecord::held(
                intent.host_minted_operation().clone(),
                intent.scope().account().clone(),
                capability,
                entry,
                0,
            )) {
                Ok(existing.clone())
            } else {
                Err(SaveOutcome::Conflict)
            };
        }
        let record = OperationRecord::held(intent.host_minted_operation().clone(), intent.scope().account().clone(), capability, entry, 0);
        self.records.insert(record.operation.clone(), record.clone());
        self.mappings.insert(
            key,
            RetryMapping {
                scope: RetryScopeKey { host_scope: intent.scope().clone(), transport_correlation: intent.transport_correlation().clone() },
                operation: record.operation.clone(),
                capability,
                entry,
                created_at_ms: 0,
            },
        );
        self.admissions = self.admissions.saturating_add(1);
        self.fresh_intents = self.fresh_intents.saturating_add(1);
        self.events.push("mapping_persisted");
        self.events.push("wire_attempt");
        Ok(record)
    }

    fn lookup_original_retry(&self, key: &RetryScopeKey) -> RetryDecision {
        if !key.host_scope.is_provable() {
            return RetryDecision::Unprovable;
        }
        let Some(mapping) = self.mappings.get(key) else {
            return RetryDecision::Unprovable;
        };
        let Some(record) = self.records.get(&mapping.operation) else {
            return RetryDecision::Unknown;
        };
        if record.capability != mapping.capability || record.entry != mapping.entry {
            return RetryDecision::Conflict;
        }
        if record.status.is_terminal() {
            return RetryDecision::Unknown;
        }
        RetryDecision::Original(record.clone())
    }

    fn record_review(&mut self, review: &ReviewOperation) -> Result<(), OperationStoreError> {
        let record = self.records.get_mut(&review.operation_id).ok_or(OperationStoreError::UnknownOperation)?;
        if let Some(existing) = self.reviews.get(&review.run_id)
            && existing != &review.operation_id
        {
            return Err(OperationStoreError::InvalidState);
        }
        match &record.review_run_id {
            Some(existing) if existing != &review.run_id => return Err(OperationStoreError::InvalidState),
            Some(_) => {}
            None => record.review_run_id = Some(review.run_id.clone()),
        }
        self.reviews.insert(review.run_id.clone(), review.operation_id.clone());
        Ok(())
    }

    fn review_operation(&self, run_id: &RunId) -> Result<Option<OperationId>, OperationStoreError> {
        Ok(self.reviews.get(run_id).cloned())
    }
}

impl FileOperationStore {
    fn save_record_once(&self, operation: &OperationId, record: &OperationRecord) -> Result<SaveOutcome, LedgerError> {
        let path = self.record_path(operation);
        if let Some(existing) = self.load_base(operation)? {
            return Ok(if persisted_identity_matches(&existing, record) { SaveOutcome::Existing } else { SaveOutcome::Conflict });
        }
        let persisted = to_persisted(record)?;
        let bytes = serde_json::to_vec(&persisted).map_err(|_| LedgerError::Corrupt)?;
        if publish_create_only(&path, &bytes)? {
            return Ok(SaveOutcome::Saved);
        }
        match self.load_base(operation)? {
            Some(existing) if persisted_identity_matches(&existing, record) => Ok(SaveOutcome::Existing),
            _ => Ok(SaveOutcome::Conflict),
        }
    }
}

/// Derive run -> operation from the single canonical op -> run marker set.
fn review_operation_by_run(store: &FileOperationStore, run_id: &RunId) -> Result<Option<OperationId>, LedgerError> {
    let entries = fs::read_dir(store.records_dir())?;
    let mut found: Option<OperationId> = None;
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(operation_name) = name.strip_suffix(".review.json") else {
            continue;
        };
        let Some(review) = read_json::<PersistedReview>(&path)? else {
            continue;
        };
        if review.operation_id != operation_name {
            return Err(LedgerError::Corrupt);
        }
        if review.run_id == run_id.as_str() {
            let operation = OperationId::parse(&review.operation_id).map_err(|_| LedgerError::Corrupt)?;
            if found.as_ref().is_some_and(|existing| existing != &operation) {
                return Err(LedgerError::Corrupt);
            }
            found = Some(operation);
        }
    }
    Ok(found)
}

fn persisted_identity_matches(persisted: &PersistedRecord, record: &OperationRecord) -> bool {
    persisted.operation == record.operation.as_str()
        && persisted.account == record.account.as_str()
        && persisted.capability == record.capability.wire()
        && persisted.entry == entry_name(record.entry)
}

fn to_persisted(record: &OperationRecord) -> Result<PersistedRecord, LedgerError> {
    Ok(PersistedRecord {
        operation: record.operation.as_str().to_owned(),
        account: record.account.as_str().to_owned(),
        capability: record.capability.wire().to_owned(),
        entry: entry_persisted_name(record.entry)?.to_owned(),
        started_at_ms: record.started_at_ms,
        deadline_ms: record.deadline_ms,
        grant_id: record.grant_id.as_ref().map(|id| id.as_str().to_owned()),
        review_run_id: record.review_run_id.as_ref().map(|id| id.as_str().to_owned()),
    })
}

fn from_persisted(persisted: PersistedRecord) -> Result<OperationRecord, LedgerError> {
    Ok(OperationRecord {
        operation: OperationId::parse(&persisted.operation).map_err(|_| LedgerError::Corrupt)?,
        account: AccountId::new(persisted.account),
        capability: Capability::from_wire(&persisted.capability).ok_or(LedgerError::Corrupt)?,
        entry: entry_from_persisted_name(&persisted.entry).ok_or(LedgerError::Corrupt)?,
        status: OperationStatus::Held,
        started_at_ms: persisted.started_at_ms,
        deadline_ms: persisted.deadline_ms,
        grant_id: persisted.grant_id.map(GrantId::new),
        review_run_id: persisted.review_run_id.map(RunId::new),
    })
}

fn entry_persisted_name(entry: EntryCategory) -> Result<&'static str, LedgerError> {
    if entry.is_free() {
        return Err(LedgerError::Corrupt);
    }
    Ok(entry_name(entry))
}

fn entry_name(entry: EntryCategory) -> &'static str {
    match entry {
        EntryCategory::HumanGuiFree => "human_gui_free",
        EntryCategory::WebGuiFree => "web_gui_free",
        EntryCategory::Mastra => "mastra",
        EntryCategory::ExternalAcpMcp => "external_acp_mcp",
        EntryCategory::HeadlessMcp => "headless_mcp",
        EntryCategory::GuiBridgeMcp => "gui_bridge_mcp",
        EntryCategory::DirectControl => "direct_control",
        EntryCategory::CliDfm => "cli_dfm",
        EntryCategory::ReviewPanel => "review_panel",
        EntryCategory::ExternalUiAutomation => "external_ui_automation",
        EntryCategory::Unknown => "unknown",
    }
}

fn entry_from_persisted_name(name: &str) -> Option<EntryCategory> {
    match name {
        "mastra" => Some(EntryCategory::Mastra),
        "external_acp_mcp" => Some(EntryCategory::ExternalAcpMcp),
        "headless_mcp" => Some(EntryCategory::HeadlessMcp),
        "gui_bridge_mcp" => Some(EntryCategory::GuiBridgeMcp),
        "direct_control" => Some(EntryCategory::DirectControl),
        "cli_dfm" => Some(EntryCategory::CliDfm),
        "review_panel" => Some(EntryCategory::ReviewPanel),
        "external_ui_automation" => Some(EntryCategory::ExternalUiAutomation),
        "unknown" => Some(EntryCategory::Unknown),
        _ => None,
    }
}

fn status_name(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::Held => "held",
        OperationStatus::Pending => "pending",
        OperationStatus::Charged => "charged",
        OperationStatus::Released => "released",
        OperationStatus::Expired => "expired",
    }
}

fn status_from_name(name: &str) -> Option<OperationStatus> {
    match name {
        "held" => Some(OperationStatus::Held),
        "pending" => Some(OperationStatus::Pending),
        "charged" => Some(OperationStatus::Charged),
        "released" => Some(OperationStatus::Released),
        "expired" => Some(OperationStatus::Expired),
        _ => None,
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, LedgerError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(LedgerError::Io(error)),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|_| LedgerError::Corrupt)
}

fn is_json_file(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|extension| extension == "json")
}

/// Publish a whole file without replacing an existing destination.
///
/// The content is written to a temporary file in the same directory, flushed, then hard-linked to
/// the final name. A crash therefore leaves the complete entry or only an ignored temporary file;
/// two concurrent writers publish exactly one file, and no writer can overwrite an admitted row.
/// File systems that refuse hard links fail closed instead of falling back to a torn create.
fn publish_create_only(path: &Path, bytes: &[u8]) -> Result<bool, LedgerError> {
    let parent = path.parent().ok_or_else(|| LedgerError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "ledger path has no parent")))?;
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| LedgerError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "ledger path has no file name")))?;
    let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), next_temp_id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(LedgerError::Io(error));
    }
    let linked = fs::hard_link(&temporary, path);
    let _ = fs::remove_file(&temporary);
    match linked {
        Ok(()) => {
            sync_dir(parent);
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(LedgerError::Io(error)),
    }
}

fn sync_dir(directory: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(directory) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = directory;
}

fn next_temp_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

fn mint_operation() -> Result<OperationId, LedgerError> {
    let mut first = std::collections::hash_map::RandomState::new().build_hasher();
    let mut second = std::collections::hash_map::RandomState::new().build_hasher();
    now_ms().hash(&mut first);
    std::process::id().hash(&mut first);
    std::thread::current().id().hash(&mut second);
    next_temp_id().hash(&mut second);
    let mut entropy = [0u8; 16];
    entropy[..8].copy_from_slice(&first.finish().to_le_bytes());
    entropy[8..].copy_from_slice(&second.finish().to_le_bytes());
    OperationId::mint_from_entropy(&entropy).map_err(|_| LedgerError::Corrupt)
}

/// A stable 128-bit filename digest. A collision is detected by comparing the stored identity and
/// fails closed (`Conflict`/`Unknown`); it never selects another account's operation.
fn digest(parts: &[&str]) -> String {
    let mut first = 0xcbf2_9ce4_8422_2325u64;
    let mut second = 0x9e37_79b9_7f4a_7c15u64;
    for part in parts {
        for byte in (part.len() as u64).to_le_bytes().iter().chain(part.as_bytes()) {
            first ^= u64::from(*byte);
            first = first.wrapping_mul(0x0000_0100_0000_01b3);
            second ^= u64::from(*byte).wrapping_add(0x9e37_79b9_7f4a_7c15);
            second = second.rotate_left(5).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }
    format!("{first:016x}{second:016x}")
}

fn scope_digest(key: &RetryScopeKey) -> String {
    digest(&[
        key.host_scope.account().as_str(),
        key.host_scope.installation().as_str(),
        key.host_scope.session_binding().as_str(),
        key.transport_correlation.as_str(),
    ])
}
