#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;

use super::*;
use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, Capability, EntryCategory, HostSessionBinding, InstallationId, OperationId, OperationRecord, OperationStatus,
    OperationStore, RetryDecision, RetryScopeKey, ReviewOperation, RunId, SaveOutcome, TransportCorrelation,
};

struct TempStore {
    store: FileOperationStore,
    root: PathBuf,
}

impl TempStore {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
        let root = std::env::temp_dir().join(format!("solidcraft-account-ledger-{tag}-{}-{nanos}", std::process::id()));
        let store = FileOperationStore::open(&root).expect("open ledger");
        Self { store, root }
    }

    fn reopen(&self) -> FileOperationStore {
        FileOperationStore::open(&self.root).expect("reopen ledger")
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Deref for TempStore {
    type Target = FileOperationStore;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl DerefMut for TempStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.store
    }
}

fn operation(raw: &str) -> OperationId {
    OperationId::parse(raw).expect("valid operation id")
}

fn held_record(raw: &str) -> OperationRecord {
    OperationRecord::held(operation(raw), AccountId::new("acct_example_0001"), Capability::Mcp, EntryCategory::HeadlessMcp, 1_000)
}

fn scope() -> AuthenticatedHostScope {
    AuthenticatedHostScope::new(
        AccountId::new("acct_example_0001"),
        InstallationId::new("inst_example_0001"),
        HostSessionBinding::new("test_session_binding"),
    )
}

fn correlation(raw: &str) -> TransportCorrelation {
    TransportCorrelation::new(raw)
}

fn key(scope: AuthenticatedHostScope, correlation: TransportCorrelation) -> RetryScopeKey {
    RetryScopeKey { host_scope: scope, transport_correlation: correlation }
}

#[test]
fn duplicate_save_reports_existing_without_grant() {
    let mut store = TempStore::new("duplicate-save");
    let record = held_record("op_example_0001");
    assert_eq!(store.save_new(&record).unwrap(), SaveOutcome::Saved);
    assert_eq!(store.save_new(&record).unwrap(), SaveOutcome::Existing);
    assert!(store.load(&record.operation).unwrap().unwrap().grant_id.is_none());
}

#[test]
fn retry_by_scope_and_correlation_without_operation_id_recovers_original() {
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let mut store = InMemoryOperationStore::default();
    let original = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    let original_id = original.operation.clone();
    drop(original); // the retry deliberately carries no original operation id
    let retry = store.lookup_original_retry(&key(scope.clone(), correlation.clone()));
    assert!(matches!(retry, RetryDecision::Original(record) if record.operation == original_id));
    assert_eq!(store.admissions, 1);
    assert_eq!(store.commits, 1);
}

#[test]
fn same_scope_lost_response_and_reconnect_preserves_mapping() {
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let mut store = InMemoryOperationStore::default();
    let original = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    store.simulate_lost_response(original.operation.clone());
    store.reconnect(&scope);
    let retry = store.lookup_original_retry(&key(scope, correlation));
    assert!(matches!(retry, RetryDecision::Original(record) if record.operation == original.operation));
    assert_eq!(store.admissions, 1);
    assert_eq!(store.commits, 1);
}

#[test]
fn retry_mapping_collision_conflicts_and_does_not_overwrite() {
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let mut store = InMemoryOperationStore::default();
    let original = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    assert_eq!(store.persist_intent(&scope, &correlation, Capability::Control, EntryCategory::DirectControl), Err(SaveOutcome::Conflict));
    let retry = store.lookup_original_retry(&key(scope, correlation));
    assert!(matches!(retry, RetryDecision::Original(record) if record.operation == original.operation));
}

#[test]
fn unprovable_cross_session_retry_fails_closed() {
    let scope = scope();
    let other = AuthenticatedHostScope::new(
        AccountId::new("acct_example_0001"),
        InstallationId::new("inst_example_0002"),
        HostSessionBinding::new("test_session_binding"),
    );
    let correlation = correlation("req_example_0001");
    let mut store = InMemoryOperationStore::default();
    store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    assert!(matches!(store.lookup_original_retry(&key(other, correlation)), RetryDecision::Unprovable));
    assert_eq!(store.admissions, 0);
}

#[test]
fn unproven_retry_is_not_treated_as_fresh() {
    let scope = scope();
    let correlation = correlation("req_unknown_0001");
    let mut store = InMemoryOperationStore::default();
    assert!(matches!(store.lookup_original_retry(&key(scope, correlation)), RetryDecision::Unprovable));
    assert_eq!(store.fresh_intents, 0);
    assert_eq!(store.admissions, 0);
}

#[test]
fn atomic_mapping_is_saved_before_first_wire() {
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let mut store = InMemoryOperationStore::default();
    store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    assert_eq!(store.events, vec!["mapping_persisted", "wire_attempt"]);
}

#[test]
fn durable_mapping_is_readable_before_persist_intent_returns() {
    let mut store = TempStore::new("before-wire");
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let record = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    assert!(store.load(&record.operation).unwrap().is_some());
    assert!(matches!(store.lookup_original_retry(&key(scope, correlation)), RetryDecision::Original(found) if found.operation == record.operation));
}

#[test]
fn retry_after_restart_by_scope_and_correlation_recovers_original() {
    let mut store = TempStore::new("restart-retry");
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let record = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    let mut reopened = store.reopen();
    assert!(matches!(
        reopened.lookup_original_retry(&key(scope, correlation)),
        RetryDecision::Original(found) if found.operation == record.operation
    ));
}

#[test]
fn restart_loads_held_record_and_never_returns_execution_grant() {
    let mut store = TempStore::new("restart-held");
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let record = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    let reopened = store.reopen();
    let loaded = reopened.load(&record.operation).unwrap().expect("held row survives restart");
    assert_eq!(loaded.status, OperationStatus::Held);
    assert!(loaded.grant_id.is_none());
}

#[test]
fn crash_between_admit_and_dispatch_leaves_no_started_record() {
    let store = TempStore::new("crash-before-publish");
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let staged = store.root().join("mappings").join(".staged.json.1.1.tmp");
    std::fs::write(&staged, br#"{"version":1,"account":"acct_example_0001"}"#).unwrap();
    let mut reopened = store.reopen();
    assert!(matches!(reopened.lookup_original_retry(&key(scope, correlation)), RetryDecision::Unprovable));
    assert!(reopened.load(&operation("op_example_0099")).unwrap().is_none());
}

#[test]
fn external_correlation_id_cannot_elevate() {
    let mut store = TempStore::new("external-id");
    let scope = scope();
    let correlation = correlation("req_request_id_do_not_trust");
    assert!(matches!(store.lookup_original_retry(&key(scope, correlation)), RetryDecision::Unprovable));
    assert!(store.load(&operation("op_example_0099")).unwrap().is_none());
    assert!(OperationId::parse("req_request_id_do_not_trust").is_err());
}

#[test]
fn ambiguous_transport_fails_closed() {
    let mut store = TempStore::new("ambiguous");
    let empty = AuthenticatedHostScope::new(AccountId::new("acct_example_0001"), InstallationId::new(""), HostSessionBinding::new(""));
    let correlation = correlation("req_example_0001");
    assert!(!empty.is_provable());
    assert_eq!(store.persist_intent(&empty, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp), Err(SaveOutcome::Conflict));
    assert!(matches!(store.lookup_original_retry(&key(empty, correlation)), RetryDecision::Unprovable));
}

#[test]
fn review_run_operation_survives_restart() {
    let mut store = TempStore::new("review");
    let record = held_record("op_example_0001");
    store.save_new(&record).unwrap();
    store.record_review(&ReviewOperation::new(RunId::new("run_example_0001"), record.operation.clone())).unwrap();
    let reopened = store.reopen();
    assert_eq!(reopened.review_operation(&RunId::new("run_example_0001")).unwrap(), Some(record.operation.clone()));
    assert_eq!(reopened.load(&record.operation).unwrap().unwrap().review_run_id, Some(RunId::new("run_example_0001")));
}

#[test]
fn immutable_duplicate_with_different_capability_conflicts() {
    let mut store = TempStore::new("immutable");
    let record = held_record("op_example_0001");
    store.save_new(&record).unwrap();
    let mut other = record.clone();
    other.capability = Capability::Control;
    other.entry = EntryCategory::DirectControl;
    assert_eq!(store.save_new(&other).unwrap(), SaveOutcome::Conflict);
    let loaded = store.load(&record.operation).unwrap().unwrap();
    assert_eq!(loaded.capability, Capability::Mcp);
    assert_eq!(loaded.entry, EntryCategory::HeadlessMcp);
}

#[test]
fn terminal_state_is_monotonic() {
    let mut store = TempStore::new("terminal");
    let charged = held_record("op_example_0001");
    store.save_new(&charged).unwrap();
    store.mark_terminal(&charged.operation, OperationStatus::Charged).unwrap();
    assert_eq!(store.mark_terminal(&charged.operation, OperationStatus::Released), Err(OperationStoreError::TerminalState));
    assert_eq!(store.mark_terminal(&charged.operation, OperationStatus::Charged), Err(OperationStoreError::TerminalState));
    assert_eq!(store.load(&charged.operation).unwrap().unwrap().status, OperationStatus::Charged);

    let expired = held_record("op_example_0002");
    store.save_new(&expired).unwrap();
    store.mark_terminal(&expired.operation, OperationStatus::Expired).unwrap();
    assert_eq!(store.mark_terminal(&expired.operation, OperationStatus::Charged), Err(OperationStoreError::TerminalState));
    assert_eq!(store.mark_terminal(&expired.operation, OperationStatus::Pending), Err(OperationStoreError::InvalidState));
}

#[test]
fn charged_mapping_retry_is_unknown_not_replayed() {
    let mut store = TempStore::new("terminal-mapping");
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let record = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    store.mark_terminal(&record.operation, OperationStatus::Charged).unwrap();
    assert!(matches!(store.lookup_original_retry(&key(scope, correlation)), RetryDecision::Unknown));
}

#[test]
fn concurrent_intents_publish_one_mapping_without_overwrite() {
    use std::sync::{Arc, Barrier};

    let store = TempStore::new("concurrent-intent");
    let root = store.root().to_path_buf();
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = FileOperationStore::open(root).unwrap();
                barrier.wait();
                store.persist_intent(&scope(), &correlation("req_same_0001"), Capability::Mcp, EntryCategory::HeadlessMcp)
            })
        })
        .collect();
    let results: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
    assert_eq!(results.len(), 2);
    let first = results[0].as_ref().unwrap();
    for result in &results {
        let record = result.as_ref().unwrap();
        assert_eq!(record.operation, first.operation, "both writers must agree on one original operation");
        assert_eq!(record.grant_id, None);
    }
    let mappings = std::fs::read_dir(store.root().join("mappings"))
        .unwrap()
        .filter(|entry| entry.as_ref().is_ok_and(|entry| entry.path().extension().is_some_and(|extension| extension == "json")));
    assert_eq!(mappings.count(), 1, "one immutable mapping file, never two overwriting rows");
    let mut reopened = store.reopen();
    assert!(matches!(
        reopened.lookup_original_retry(&key(scope(), correlation("req_same_0001"))),
        RetryDecision::Original(record) if record.operation == first.operation
    ));
}

#[test]
fn concurrent_save_with_different_capability_has_one_winner_and_no_overwrite() {
    use std::sync::{Arc, Barrier};

    let store = TempStore::new("concurrent-save");
    let root = store.root().to_path_buf();
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = [Capability::Mcp, Capability::Control]
        .into_iter()
        .map(|capability| {
            let root = root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = FileOperationStore::open(root).unwrap();
                let entry = if capability == Capability::Mcp { EntryCategory::HeadlessMcp } else { EntryCategory::DirectControl };
                let mut record = held_record("op_example_0001");
                record.capability = capability;
                record.entry = entry;
                barrier.wait();
                store.save_new(&record)
            })
        })
        .collect();
    let outcomes: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|outcome| **outcome == SaveOutcome::Saved).count(), 1);
    assert_eq!(outcomes.iter().filter(|outcome| **outcome == SaveOutcome::Conflict).count(), 1);
    let loaded = store.reopen().load(&operation("op_example_0001")).unwrap().unwrap();
    assert!(matches!(loaded.capability, Capability::Mcp | Capability::Control));
    assert_eq!(loaded.grant_id, None);
}

#[test]
fn ledger_never_stores_credentials() {
    let mut store = TempStore::new("credentials");
    let scope = scope();
    let correlation = correlation("req_example_0001");
    let record = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).unwrap();
    store.record_review(&ReviewOperation::new(RunId::new("run_example_0001"), record.operation.clone())).unwrap();
    for path in files_under(store.root()) {
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for forbidden in ["access_token", "refresh_token", "session_token", "password", "secret", "credential"] {
            assert!(!text.contains(forbidden), "{} must not store {forbidden}", path.display());
        }
    }
}

fn files_under(root: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

#[cfg(unix)]
#[test]
fn ledger_files_are_private_to_the_host_user() {
    use std::os::unix::fs::PermissionsExt;
    let mut store = TempStore::new("permissions");
    let record = held_record("op_example_0001");
    store.save_new(&record).unwrap();
    let mode = |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(store.root()), 0o700);
    assert_eq!(mode(&store.root().join("records")), 0o700);
    let record_file = store.root().join("records").join("op_example_0001.json");
    assert_eq!(mode(&record_file), 0o600);
}
