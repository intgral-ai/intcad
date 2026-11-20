#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::time::Duration;

const HOUR: Duration = Duration::from_secs(3_600);

fn online_available() -> Availability {
    Availability { online: true, version_supported: true, model_available: true, balance: 1_000, held: 0, reasons: Vec::new() }
}

fn verified_budget() -> BudgetTable {
    let mut table = BudgetTable::unverified_default();
    table.set_verified(Capability::Control, EntryCategory::DirectControl, HOUR);
    table.set_verified(Capability::Mcp, EntryCategory::HeadlessMcp, HOUR);
    table
}

fn verified_chat_budget() -> BudgetTable {
    BudgetTable::unverified_default()
}

#[test]
fn default_budget_refuses_every_controlled_path_with_unsafe_to_start() {
    let table = BudgetTable::unverified_default();
    for capability in Capability::ALL {
        for entry in EntryCategory::CONTROLLED {
            assert_eq!(
                table.require(capability, entry, &online_available(), HOUR).unwrap_err().code,
                RefusalCode::UnsafeToStart,
                "capability {capability:?}, entry {entry:?}"
            );
        }
    }
}

#[test]
fn unknown_entry_is_refused_before_the_budget_lookup() {
    let mut table = BudgetTable::unverified_default();
    table.set_verified(Capability::Control, EntryCategory::Unknown, HOUR);
    assert_eq!(table.require(Capability::Control, EntryCategory::Unknown, &online_available(), HOUR).unwrap_err().code, RefusalCode::UnsafeToStart);
}

#[test]
fn availability_reason_is_preserved_by_the_budget() {
    let mut availability = online_available();
    availability.reasons = vec![RefusalCode::EmailUnverified];
    assert_eq!(
        verified_budget().require(Capability::Control, EntryCategory::DirectControl, &availability, HOUR).unwrap_err().code,
        RefusalCode::EmailUnverified
    );
}

#[test]
fn offline_and_unsupported_version_refuse_controlled_paths() {
    let mut offline = online_available();
    offline.online = false;
    assert_eq!(
        verified_budget().require(Capability::Control, EntryCategory::DirectControl, &offline, HOUR).unwrap_err().code,
        RefusalCode::AuthUnavailable
    );
    let mut unsupported = online_available();
    unsupported.version_supported = false;
    assert_eq!(
        verified_budget().require(Capability::Control, EntryCategory::DirectControl, &unsupported, HOUR).unwrap_err().code,
        RefusalCode::VersionUnsupported
    );
}

#[test]
fn remaining_shorter_than_verified_bound_is_unsafe_to_start() {
    let short = HOUR - Duration::from_millis(1);
    assert_eq!(
        verified_budget().require(Capability::Control, EntryCategory::DirectControl, &online_available(), short).unwrap_err().code,
        RefusalCode::UnsafeToStart
    );
    assert_eq!(verified_budget().require(Capability::Control, EntryCategory::DirectControl, &online_available(), HOUR), Ok(HOUR));
}

#[test]
fn free_human_entry_does_not_need_network_or_a_verified_bound() {
    let table = BudgetTable::unverified_default();
    let offline = Availability {
        online: false,
        version_supported: false,
        model_available: false,
        balance: 0,
        held: 0,
        reasons: vec![RefusalCode::AuthUnavailable],
    };
    assert_eq!(table.require(Capability::Chat, EntryCategory::HumanGuiFree, &offline, HOUR), Ok(HOUR));
    assert_eq!(table.require(Capability::Control, EntryCategory::WebGuiFree, &offline, HOUR), Ok(HOUR));
}

#[test]
fn chat_with_zero_credits_is_allowed_when_online_authorized() {
    let availability = Availability { online: true, version_supported: true, model_available: true, balance: 0, held: 0, reasons: vec![] };
    assert!(matches!(admit_chat(&availability, &verified_chat_budget()), Admission::Allowed));
}

#[test]
fn new_billing_operation_requires_positive_available() {
    let availability = Availability { online: true, version_supported: true, model_available: true, balance: 1, held: 1, reasons: vec![] };
    let mut store = InMemoryOperationStore::default();
    let outcome = admit_billing(Capability::Control, EntryCategory::DirectControl, &availability, &verified_budget(), &mut store);
    assert_eq!(outcome.refusal_code(), Some(RefusalCode::CreditsHeld));
}

#[test]
fn zero_balance_without_held_credits_is_exhausted() {
    let availability = Availability { online: true, version_supported: true, model_available: true, balance: 0, held: 0, reasons: vec![] };
    let mut store = InMemoryOperationStore::default();
    let outcome = admit_billing(Capability::Control, EntryCategory::DirectControl, &availability, &verified_budget(), &mut store);
    assert_eq!(outcome.refusal_code(), Some(RefusalCode::CreditsExhausted));
}

#[test]
fn billing_preflight_refuses_unverified_budget_without_admission() {
    let mut store = InMemoryOperationStore::default();
    let outcome =
        admit_billing(Capability::Control, EntryCategory::DirectControl, &online_available(), &BudgetTable::unverified_default(), &mut store);
    assert_eq!(outcome.refusal_code(), Some(RefusalCode::UnsafeToStart));
    assert_eq!(store.admissions, 0);
}

#[test]
fn billing_preflight_preserves_account_refusals_without_admission() {
    let mut offline = online_available();
    offline.online = false;
    let mut store = InMemoryOperationStore::default();
    assert_eq!(
        admit_billing(Capability::Control, EntryCategory::DirectControl, &offline, &verified_budget(), &mut store).refusal_code(),
        Some(RefusalCode::AuthUnavailable)
    );
    let mut disabled = online_available();
    disabled.reasons = vec![RefusalCode::AccountDisabled];
    assert_eq!(
        admit_billing(Capability::Control, EntryCategory::DirectControl, &disabled, &verified_budget(), &mut store).refusal_code(),
        Some(RefusalCode::AccountDisabled)
    );
    assert_eq!(store.admissions, 0);
}

#[test]
fn local_affordability_never_mints_an_execution_grant() {
    let availability = Availability { online: true, version_supported: true, model_available: true, balance: 100, held: 0, reasons: vec![] };
    let mut store = InMemoryOperationStore::default();
    let outcome = admit_billing(Capability::Control, EntryCategory::DirectControl, &availability, &verified_budget(), &mut store);
    assert_eq!(outcome.refusal_code(), Some(RefusalCode::AuthUnavailable));
    assert_eq!(store.admissions, 0);
}

#[test]
fn deadline_anchors_at_send_not_receive() {
    let clock = FakeMonotonicClock::at_millis(1_000);
    let deadline = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
    clock.advance(Duration::from_millis(500));
    assert_eq!(deadline.remaining(clock.now()), Some(Duration::from_millis(59_500)));
    clock.advance(Duration::from_millis(59_500));
    assert_eq!(deadline.remaining(clock.now()), None);
}

#[test]
fn network_renewal_does_not_extend_past_original_deadline() {
    let clock = FakeMonotonicClock::at_millis(1_000);
    let old = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
    clock.advance(Duration::from_secs(61));
    let attempt = RenewalAttempt { sent_at: clock.now(), received_at: clock.now(), old_deadline: old.clone(), still_valid: true };
    assert_eq!(old.renewed(&attempt, Duration::from_secs(60)), Err(DeadlineError::Expired));
    assert_eq!(old.remaining(clock.now()), None);
}

#[test]
fn renewal_sent_before_old_deadline_but_received_after_is_expired() {
    let clock = FakeMonotonicClock::at_millis(1_000);
    let old = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
    clock.advance(Duration::from_millis(59_500));
    let sent_at = clock.now();
    clock.advance(Duration::from_millis(1_000));
    let received_at = clock.now();
    let attempt = RenewalAttempt { sent_at, received_at, old_deadline: old.clone(), still_valid: true };
    assert_eq!(old.renewed(&attempt, Duration::from_secs(60)), Err(DeadlineError::Expired));
}

#[test]
fn renewal_with_still_valid_false_is_rejected() {
    let clock = FakeMonotonicClock::at_millis(1_000);
    let old = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
    clock.advance(Duration::from_secs(30));
    let sent_at = clock.now();
    clock.advance(Duration::from_millis(500));
    let attempt = RenewalAttempt { sent_at, received_at: clock.now(), old_deadline: old.clone(), still_valid: false };
    assert_eq!(old.renewed(&attempt, Duration::from_secs(60)), Err(DeadlineError::Rejected));
}

#[test]
fn valid_renewal_anchors_at_send() {
    let clock = FakeMonotonicClock::at_millis(1_000);
    let old = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
    clock.advance(Duration::from_secs(30));
    let sent_at = clock.now();
    clock.advance(Duration::from_millis(500));
    let attempt = RenewalAttempt { sent_at, received_at: clock.now(), old_deadline: old.clone(), still_valid: true };
    let renewed = old.renewed(&attempt, Duration::from_secs(60)).expect("valid renewal");
    assert_eq!(renewed.remaining(clock.now()), Some(Duration::from_millis(59_500)));
}

#[test]
fn renewal_at_old_deadline_is_expired() {
    let clock = FakeMonotonicClock::at_millis(1_000);
    let old = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
    clock.advance(Duration::from_secs(60));
    let attempt = RenewalAttempt { sent_at: clock.now(), received_at: clock.now(), old_deadline: old.clone(), still_valid: true };
    assert_eq!(old.renewed(&attempt, Duration::from_secs(60)), Err(DeadlineError::Expired));
}

#[test]
fn suspend_or_clock_anomaly_is_treated_as_expired() {
    let clock = FakeMonotonicClock::at_millis(1_000);
    let deadline = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
    clock.backward(Duration::from_millis(1));
    assert_eq!(deadline.remaining(clock.now()), None);
    let clock2 = FakeMonotonicClock::at_millis(1_000);
    let d2 = ConservativeDeadline::new(clock2.now(), Duration::from_secs(60));
    clock2.suspend(Duration::from_secs(3_600));
    assert_eq!(d2.remaining(clock2.now()), None);
}

#[test]
fn trusted_context_can_only_be_minted_by_host() {
    let scope = AuthenticatedHostScope::test_fixture("acct_example_0001", "inst_example_0001");
    let ctx = TrustedContext::mint(&scope, "op_example_0001", Capability::Control, EntryCategory::DirectControl).expect("host mint");
    assert_eq!(ctx.operation().as_str(), "op_example_0001");
}

#[test]
fn operation_id_matches_candidate_wire_grammar() {
    assert!(OperationId::parse("op_example_0001").is_ok());
    assert!(OperationId::parse("op_short").is_err());
    assert!(OperationId::parse("op-1_2:3").is_err());
    assert!(OperationId::parse("bad space").is_err());
}

#[test]
fn operation_id_mint_is_valid_rev3_grammar() {
    let minted = OperationId::mint_from_entropy(&[7_u8; 16]).expect("mint");
    assert!(OperationId::parse(minted.as_str()).is_ok());
    assert_eq!(minted.as_str().trim_start_matches("op_").len(), 16);
    assert!(OperationId::mint_from_entropy(&[7_u8; 8]).is_err());
}

#[test]
fn entry_and_capability_wire_values_match_frozen_rev3() {
    assert_eq!(EntryCategory::HumanGuiFree.wire_entry_category(), None);
    assert_eq!(EntryCategory::DirectControl.wire_entry_category(), Some(WireEntryCategory::DirectControl));
    assert_eq!(EntryCategory::Unknown.wire_entry_category(), Some(WireEntryCategory::Unknown));
    assert_eq!(Capability::DesignIntent.wire(), "design_intent");
}

#[test]
fn transport_error_is_not_a_business_refusal() {
    assert!(matches!(Refusal::from_wire(ErrorCode::InternalError), Err(TransportError::InternalError)));
    assert!(matches!(Refusal::from_wire(ErrorCode::InvalidRequest), Err(TransportError::InvalidRequest)));
    assert_eq!(Refusal::from_wire(ErrorCode::AuthUnavailable).unwrap(), RefusalCode::AuthUnavailable);
}

#[test]
fn in_memory_store_persists_once_and_has_no_grant() {
    let scope = AuthenticatedHostScope::test_fixture("acct_example_0001", "inst_example_0001");
    let correlation = TransportCorrelation::new("req_example_0001");
    let mut store = InMemoryOperationStore::default();
    let record = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).expect("first intent");
    let operation = record.operation.clone();
    assert!(record.grant_id.is_none());
    assert_eq!(store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).expect("existing intent"), record);
    assert_eq!(store.persist_intent(&scope, &correlation, Capability::Control, EntryCategory::DirectControl), Err(SaveOutcome::Conflict));
    let key = RetryScopeKey { host_scope: scope, transport_correlation: correlation };
    assert!(matches!(store.lookup_original_retry(&key), RetryDecision::Original(found) if found.operation == operation));
    assert_eq!(store.admissions, 1);
}

#[test]
fn in_memory_store_commits_only_on_the_first_charged_terminal() {
    let scope = AuthenticatedHostScope::test_fixture("acct_example_0001", "inst_example_0001");
    let correlation = TransportCorrelation::new("req_example_0001");
    let mut store = InMemoryOperationStore::default();
    let record = store.persist_intent(&scope, &correlation, Capability::Mcp, EntryCategory::HeadlessMcp).expect("intent");
    assert_eq!(store.admissions, 1);
    assert_eq!(store.commits, 0);
    store.mark_terminal(&record.operation, OperationStatus::Charged).expect("charge");
    assert_eq!(store.commits, 1);
    assert_eq!(store.mark_terminal(&record.operation, OperationStatus::Charged), Err(OperationStoreError::TerminalState));
    assert_eq!(store.commits, 1);
}
