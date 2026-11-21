//! The Review panel's own behaviour: the check entry, the report it shows, a row that chooses a
//! finding, and the card dragged onto the Assistant as an attachment.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::dfm_chat::{Card, Finding, Measure, Requirement};
use crate::scenario::{Harness, rect_of};
use crate::shell::LayoutMode;
use serde_json::json;
use solidcraft_engine::dfm::run::{RunMeta, RunOutcome, RunStore};
use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, BillingPort, BillingResult, BudgetTable, Capability, EntryCategory, FreshIntent, Grant, GrantId,
    MonotonicInstant, OperationId, OperationRecord, OperationStore, OperationStoreError, Refusal, RefusalCode, RetryDecision, RetryScopeKey,
    ReviewOperation, RunId, SaveOutcome, SettlementIntent, TransportCorrelation,
};

#[derive(Clone, Default)]
struct CountingBilling {
    admits: Arc<Mutex<usize>>,
    commits: Arc<Mutex<usize>>,
    releases: Arc<Mutex<usize>>,
    committed: Arc<Mutex<Vec<OperationId>>>,
}

impl CountingBilling {
    fn admits(&self) -> usize {
        *self.admits.lock().expect("count")
    }

    fn commits(&self) -> usize {
        *self.commits.lock().expect("count")
    }

    fn releases(&self) -> usize {
        *self.releases.lock().expect("count")
    }

    fn committed_operations(&self) -> Vec<OperationId> {
        self.committed.lock().expect("count").clone()
    }
}

impl BillingPort for CountingBilling {
    fn admit(&mut self, operation: &OperationId, capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
        *self.admits.lock().expect("count") += 1;
        Ok(Grant::from_server(GrantId::new(format!("grant_test_{}", operation.as_str())), capability, MonotonicInstant::from_millis(60_000)))
    }

    fn commit(&mut self, operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        *self.commits.lock().expect("count") += 1;
        self.committed.lock().expect("count").push(operation.clone());
        Ok(result)
    }

    fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
        *self.releases.lock().expect("count") += 1;
        Ok(())
    }
}

/// A local durable-journal double: review mappings and operation rows only. It never ships and
/// never performs file or network IO.
#[derive(Clone, Default)]
struct ReviewLedger {
    reviews: Arc<Mutex<BTreeMap<String, OperationId>>>,
    records: Arc<Mutex<BTreeMap<String, OperationRecord>>>,
    intents: Arc<Mutex<BTreeMap<String, SettlementIntent>>>,
    fail_intent: Arc<Mutex<bool>>,
    fail_terminal: Arc<Mutex<bool>>,
    fail_mapping: Arc<Mutex<bool>>,
    fail_lookup: Arc<Mutex<bool>>,
}

impl ReviewLedger {
    fn insert_record(&self, operation: OperationId) {
        let record = OperationRecord::held(operation.clone(), AccountId::new("review-account"), Capability::Review, EntryCategory::ReviewPanel, 0);
        self.records.lock().expect("records").insert(operation.as_str().to_string(), record);
    }

    fn operation_for(&self, run: &str) -> Option<OperationId> {
        self.reviews.lock().expect("reviews").get(run).cloned()
    }

    fn intent_for(&self, operation: &OperationId) -> Option<SettlementIntent> {
        self.intents.lock().expect("intents").get(operation.as_str()).copied()
    }

    fn status_for(&self, operation: &OperationId) -> Option<solidcraft_engine::entitlement::OperationStatus> {
        self.records.lock().expect("records").get(operation.as_str()).map(|record| record.status)
    }

    fn set_terminal(&self, operation: &OperationId, status: solidcraft_engine::entitlement::OperationStatus) {
        if let Some(record) = self.records.lock().expect("records").get_mut(operation.as_str()) {
            record.status = status;
        }
    }

    fn fail_next_intent(&self, fail: bool) {
        *self.fail_intent.lock().expect("flag") = fail;
    }

    fn fail_next_terminal(&self, fail: bool) {
        *self.fail_terminal.lock().expect("flag") = fail;
    }

    fn fail_next_mapping(&self, fail: bool) {
        *self.fail_mapping.lock().expect("flag") = fail;
    }

    fn fail_next_lookup(&self, fail: bool) {
        *self.fail_lookup.lock().expect("flag") = fail;
    }

    fn record_settlement_intent_claim(&self, operation: &OperationId, intent: SettlementIntent) -> Result<(), OperationStoreError> {
        let mut intents = self.intents.lock().expect("intents");
        if intents.get(operation.as_str()).is_some() {
            return Err(OperationStoreError::InvalidState);
        }
        intents.insert(operation.as_str().to_string(), intent);
        Ok(())
    }
}

struct LedgerHandle(ReviewLedger);

impl OperationStore for LedgerHandle {
    fn save_new(&mut self, record: &OperationRecord) -> Result<SaveOutcome, OperationStoreError> {
        self.0.records.lock().expect("records").insert(record.operation.as_str().to_string(), record.clone());
        Ok(SaveOutcome::Saved)
    }

    fn load(&self, operation: &OperationId) -> Result<Option<OperationRecord>, OperationStoreError> {
        Ok(self.0.records.lock().expect("records").get(operation.as_str()).cloned())
    }

    fn mark_started(&mut self, _operation: &OperationId, _started_at_ms: u64) -> Result<(), OperationStoreError> {
        Ok(())
    }

    fn mark_terminal(&mut self, operation: &OperationId, status: solidcraft_engine::entitlement::OperationStatus) -> Result<(), OperationStoreError> {
        if *self.0.fail_terminal.lock().expect("flag") {
            return Err(OperationStoreError::Unavailable);
        }
        let mut records = self.0.records.lock().expect("records");
        let record = records.get_mut(operation.as_str()).ok_or(OperationStoreError::UnknownOperation)?;
        if record.status.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        record.status = status;
        Ok(())
    }

    fn persist_intent(
        &mut self,
        _scope: &AuthenticatedHostScope,
        _correlation: &TransportCorrelation,
        _capability: Capability,
        _entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        Err(SaveOutcome::Conflict)
    }

    fn persist_fresh_intent(
        &mut self,
        _intent: &FreshIntent,
        _capability: Capability,
        _entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome> {
        Err(SaveOutcome::Conflict)
    }

    fn lookup_original_retry(&self, _key: &RetryScopeKey) -> RetryDecision {
        RetryDecision::Unknown
    }

    fn record_review(&mut self, review: &ReviewOperation) -> Result<(), OperationStoreError> {
        if *self.0.fail_mapping.lock().expect("flag") {
            return Err(OperationStoreError::Unavailable);
        }
        if !self.0.records.lock().expect("records").contains_key(review.operation_id.as_str()) {
            return Err(OperationStoreError::UnknownOperation);
        }
        self.0.reviews.lock().expect("reviews").insert(review.run_id.as_str().to_string(), review.operation_id.clone());
        Ok(())
    }

    fn review_operation(&self, run_id: &RunId) -> Result<Option<OperationId>, OperationStoreError> {
        if *self.0.fail_lookup.lock().expect("flag") {
            return Err(OperationStoreError::Unavailable);
        }
        Ok(self.0.reviews.lock().expect("reviews").get(run_id.as_str()).cloned())
    }

    fn record_settlement_intent(&mut self, operation: &OperationId, intent: SettlementIntent) -> Result<SaveOutcome, OperationStoreError> {
        if *self.0.fail_intent.lock().expect("flag") {
            return Err(OperationStoreError::Unavailable);
        }
        let records = self.0.records.lock().expect("records");
        let record = records.get(operation.as_str()).ok_or(OperationStoreError::UnknownOperation)?;
        if record.status.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        drop(records);
        let mut intents = self.0.intents.lock().expect("intents");
        match intents.get(operation.as_str()) {
            Some(existing) if existing != &intent => Err(OperationStoreError::InvalidState),
            Some(_) => Ok(SaveOutcome::Existing),
            None => {
                intents.insert(operation.as_str().to_string(), intent);
                Ok(SaveOutcome::Saved)
            }
        }
    }

    fn settlement_intent(&self, operation: &OperationId) -> Result<Option<SettlementIntent>, OperationStoreError> {
        Ok(self.0.intents.lock().expect("intents").get(operation.as_str()).copied())
    }
}

fn install(app: &mut crate::SolveApp, billing: &CountingBilling, ledger: &ReviewLedger) {
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Review, EntryCategory::ReviewPanel, Duration::from_secs(30));
    app.install_billing(Box::new(billing.clone()), budget);
    app.install_review_store(Box::new(LedgerHandle(ledger.clone())));
}

fn begin_with_record(h: &mut Harness, billing: &CountingBilling, ledger: &ReviewLedger) -> (OperationId, String) {
    install(&mut h.app, billing, ledger);
    let operation = crate::control::mint_operation().expect("host entropy");
    ledger.insert_record(operation.clone());
    let (run, _stop) = crate::dfm_review::prepare_review_with(&mut h.app, operation.clone()).expect("the Review press is prepared");
    (operation, run)
}

fn card_for(run: &str) -> Card {
    let mut value = card();
    value.run_id = run.into();
    value
}

fn window(mode: LayoutMode) -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true}));
    h.call("ui.resize", json!({"width": 1500, "height": 900}));
    h.call("ui.set", json!({"layout": mode}));
    h.app.run("solid.box", json!({"length": 40, "width": 30, "height": 20})).expect("a body");
    h.settle();
    h.step(&json!({"click": {"handle": "rail:item:review"}})).expect("the Review item");
    h.settle();
    h
}

/// One finished current run with a failed hole finding that has an anchor, and one passed finding
/// without one, as the project's report gives them.
fn card() -> Card {
    Card {
        run_id: "run-a".into(),
        candidate_digest: "candidate-a".into(),
        config_digest: "config-a".into(),
        model_version: "model-a".into(),
        project_id: "project-a".into(),
        freshness: "current".into(),
        status: "done".into(),
        available: true,
        not_checked: "Bend sequence was not checked.".into(),
        rules: vec![crate::dfm_chat::RuleRow { rule_id: "sm.hole_min_diameter".into(), state: "failed".into(), count: 1 }],
        findings: vec![
            Finding {
                id: "sheet-1/F10".into(),
                sheet: 0,
                body: "Body1".into(),
                from: "rules".into(),
                rules: vec!["sm.hole_min_diameter".into()],
                state: "failed".into(),
                status: "needs_review".into(),
                code: Some("HOLE_DIAMETER_BELOW_LIMIT".into()),
                measured: Some(Measure { value: 3.0, unit: "mm".into() }),
                requirement: Some(Requirement {
                    value: 3.5,
                    unit: "mm".into(),
                    layer: "factory".into(),
                    confirmed: Some("factory_confirmed".into()),
                    source: "shop".into(),
                }),
                feature: Some(crate::dfm_chat::Feature { kind: "hole".into(), number: Some(2) }),
                anchor: Some([16.5, 33.5, 1.0]),
                ..Finding::default()
            },
            Finding {
                id: "sheet-1/F11".into(),
                sheet: 0,
                body: "Body1".into(),
                from: "rules".into(),
                rules: vec!["sm.hole_to_edge".into()],
                state: "passed".into(),
                status: "pass".into(),
                feature: Some(crate::dfm_chat::Feature { kind: "hole".into(), number: Some(3) }),
                ..Finding::default()
            },
        ],
        ..Card::default()
    }
}

/// Show `card` for this design without reading a project.
fn show(h: &mut Harness, card: Card) {
    let document = h.app.session.document_stamp().document_id;
    h.app.dfm_review.document = Some(document);
    h.app.dfm_review.run = Some(card.run_id.clone());
    crate::dfm_chat::show_marks(&mut h.app, &card);
    h.app.dfm_chat.card = Some(card.clone());
    h.app.dfm_review.card = Some(card);
    h.settle();
}

/// With no report yet the panel offers the check itself, without the Assistant: a press starts
/// one run of the host's manual entry, and the panel says so while it runs.
#[test]
fn a_project_with_no_report_offers_the_check() {
    let h = window(LayoutMode::Docked);
    let run = rect_of("review:run").expect("the Run button");
    let panel = rect_of("rail:panel").expect("the Review panel is open");
    assert!(panel.contains_rect(run), "{run:?} in {panel:?}");
    assert!(rect_of("review:card").is_none(), "no report is on show");
    assert!(h.app.dfm_review.card.is_none());
}

/// A finished report lists its run, its counts and the findings that failed, with the measured and
/// required value, and a row chooses the same finding in the viewport.
#[test]
fn a_report_row_shows_the_values_and_chooses_the_finding() {
    let mut h = window(LayoutMode::Docked);
    show(&mut h, card());
    assert!(rect_of("review:card").is_some(), "the report's card");
    let failed = rect_of("review:finding:sheet-1/F10").expect("the failed finding's row");
    let passed = rect_of("review:finding:sheet-1/F11").expect("the passed finding's row");
    assert!(failed.top() < passed.top(), "failures stand above what passed: {failed:?} {passed:?}");
    let panel = rect_of("rail:panel").expect("the panel");
    assert!(panel.contains_rect(failed) && panel.contains_rect(passed), "{panel:?}");
    // The Assistant may be showing another run's card; the Review rows are this card's own.
    let mut other = card();
    other.run_id = "run-other".into();
    h.app.dfm_chat.card = Some(other);
    h.step(&json!({"click": {"handle": "review:finding:sheet-1/F10"}})).expect("the row");
    h.settle();
    assert_eq!(h.app.viewport.dfm.selected_key(), Some(("run-a", "sheet-1/F10")), "the row chooses the finding in the viewport");
    assert_eq!(h.app.dfm_chat.selected.as_ref().map(|(run, id)| (run.as_str(), id.as_str())), Some(("run-a", "sheet-1/F10")));
}

/// A finding no rule could judge is listed under its own heading, never dropped: a report that
/// hides what it could not check reads as if everything else were fine.
#[test]
fn a_finding_no_rule_could_judge_is_listed_as_not_covered() {
    let mut h = window(LayoutMode::Docked);
    let mut shown = card();
    shown.findings.push(Finding {
        id: "sheet-1/F5".into(),
        sheet: 0,
        body: "Body1".into(),
        rules: vec!["sm.countersink_depth".into()],
        state: "unsupported".into(),
        status: "not_checked".into(),
        feature: Some(crate::dfm_chat::Feature { kind: "countersink".into(), number: None }),
        reason: Some("the App did not measure countersinks".into()),
        ..Finding::default()
    });
    show(&mut h, shown);
    let row = rect_of("review:finding:sheet-1/F5").expect("the finding no rule judged still has a row");
    let panel = rect_of("rail:panel").expect("the panel");
    assert!(panel.contains_rect(row), "{row:?} in {panel:?}");
}

/// The card is dragged onto the Assistant: one report attachment, named after its run, and the
/// report is the one on show.
#[test]
fn the_card_dropped_on_the_assistant_becomes_an_attachment() {
    let mut h = window(LayoutMode::Docked);
    h.app.agent_chat.open = true;
    show(&mut h, card());
    let from = rect_of("review:card").expect("the report's card");
    let to = rect_of("agent_chat:panel").expect("the Assistant is open");
    h.step(&json!({"drag": [[from.center().x as f64, from.center().y as f64], [to.center().x as f64, to.center().y as f64]], "steps": 12}))
        .expect("the drag");
    h.settle();
    assert!(h.app.dfm_review.dragging.is_none(), "the drag ended");
    let reports: Vec<&crate::agent_chat::attach::Attachment> = h.app.agent_chat.attachments.iter().filter(|file| file.is_report()).collect();
    assert_eq!(reports.len(), 1, "one report attachment: {:?}", h.app.agent_chat.attachments);
    assert_eq!(reports[0].report_run(), Some("run-a"));
    assert!(reports[0].name.contains("run-a"), "the chip names the run: {}", reports[0].name);
}

/// A drag that ends away from the Assistant leaves nothing behind.
#[test]
fn a_card_dropped_elsewhere_leaves_no_attachment() {
    let mut h = window(LayoutMode::Docked);
    h.app.agent_chat.open = true;
    show(&mut h, card());
    let from = rect_of("review:card").expect("the report's card");
    let view = h.app.viewport.rect.expect("the view");
    h.step(&json!({"drag": [[from.center().x as f64, from.center().y as f64], [view.center().x as f64, view.center().y as f64]], "steps": 12}))
        .expect("the drag");
    h.settle();
    assert!(h.app.dfm_review.dragging.is_none(), "the drag ended");
    assert!(h.app.agent_chat.attachments.is_empty(), "{:?}", h.app.agent_chat.attachments);
}

/// A Review press mints and admits one operation. A second press while that operation has no
/// confirmed settlement does not admit another one.
#[test]
fn review_panel_press_is_one_operation() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    install(&mut h.app, &billing, &ledger);
    let first = crate::control::mint_operation().expect("host entropy");
    ledger.insert_record(first.clone());
    crate::dfm_review::prepare_review_with(&mut h.app, first.clone()).expect("the first press is prepared");
    let second = crate::control::mint_operation().expect("host entropy");
    ledger.insert_record(second.clone());
    let error = crate::dfm_review::prepare_review_with(&mut h.app, second).unwrap_err();
    assert!(error.starts_with("UNSAFE_TO_START:"), "a second press waits for settlement");
    assert_eq!(billing.admits(), 1, "one press is one admission");
    assert_eq!(billing.commits(), 0);
    crate::dfm_review::release_unstarted(&mut h.app, RefusalCode::UnsafeToStart).expect("clean up the hold");
    assert_eq!(billing.releases(), 1);
}

/// A finished report with zero violations still commits exactly once.
#[test]
fn review_panel_zero_violation_report_commits_once() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_, run) = begin_with_record(&mut h, &billing, &ledger);
    crate::dfm_review::settle_review_run(&mut h.app, &run, true).expect("the report settles");
    assert_eq!(billing.commits(), 1);
    assert_eq!(billing.releases(), 0);
    assert_eq!(ledger.operation_for(&run), h.app.dfm_review.operation.clone());
}

/// An incomplete or failed check releases its hold and never charges.
#[test]
fn review_panel_failure_does_not_charge() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_, run) = begin_with_record(&mut h, &billing, &ledger);
    crate::dfm_review::settle_review_run(&mut h.app, &run, false).expect("the hold is released");
    assert_eq!(billing.commits(), 0);
    assert_eq!(billing.releases(), 1, "the held admission is released exactly once");
}

/// Cancelling before a report releases the hold once.
#[test]
fn review_cancel_without_report_releases() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_, run) = begin_with_record(&mut h, &billing, &ledger);
    h.app.dfm_review.running = true;
    crate::dfm_review::cancel(&mut h.app);
    assert_eq!(billing.releases(), 0, "cancel waits for the exact run's terminal evidence");
    assert!(h.app.dfm_review.cancelling);
    crate::dfm_review::settle_review_run(&mut h.app, &run, false).expect("the cancelled run has no report");
    assert_eq!(billing.releases(), 1);
    // A later attempt to settle the same cancelled check must not release it again.
    crate::dfm_review::release_unstarted(&mut h.app, RefusalCode::UnsafeToStart).expect("already settled");
    assert_eq!(billing.releases(), 1);
}

/// Attaching the same report to the Assistant is a read, not a second Review check. The same run
/// keeps one operation and one charge.
#[test]
fn review_report_attached_to_assistant_is_not_recharged() {
    let mut h = Harness::new();
    h.app.agent_chat.open = true;
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_, run) = begin_with_record(&mut h, &billing, &ledger);
    crate::dfm_review::settle_review_run(&mut h.app, &run, true).expect("the report settles");
    assert_eq!(billing.commits(), 1);

    show(&mut h, card_for(&run));
    let file = crate::dfm_review::attachment(&mut h.app, &run).expect("the report can be attached");
    assert_eq!(file.report_run(), Some(run.as_str()));
    // The settlement seam is still idempotent for the same run.
    crate::dfm_review::settle_review_run(&mut h.app, &run, true).expect("a repeat is a no-op");
    assert_eq!(billing.commits(), 1, "attachment or reload does not recharge");
    assert_eq!(billing.releases(), 0);
}

/// The durable run-to-operation mapping survives a new window: recovery settles the original
/// operation id and never mints or reruns a second check.
#[test]
fn review_run_operation_survives_restart() {
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let mut first = Harness::new();
    let (operation, run) = begin_with_record(&mut first, &billing, &ledger);

    // A new window sees no panel state but the same durable ledger.
    let mut restarted = Harness::new();
    install(&mut restarted.app, &billing, &ledger);
    crate::dfm_review::settle_review_run(&mut restarted.app, &run, true).expect("recovery settles the original operation");
    assert_eq!(billing.commits(), 1);
    assert_eq!(billing.committed_operations(), vec![operation], "recovery keeps the original id");
    assert_eq!(ledger.operation_for(&run), Some(billing.committed_operations()[0].clone()));
}

#[test]
fn review_without_authority_never_starts() {
    let mut h = Harness::new();
    crate::dfm_review::start(&mut h.app);
    let error = h.app.dfm_review.error.clone().expect("the press reports its refusal");
    assert!(error.starts_with("UNSAFE_TO_START:"), "{error}");
    assert!(h.app.dfm_review.operation.is_none(), "no operation is admitted without a verified bound");
}

/// Red for the repair: a Review press may not run without a durable journal and an operation row.
#[test]
fn review_start_requires_a_durable_journal() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Review, EntryCategory::ReviewPanel, Duration::from_secs(30));
    h.app.install_billing(Box::new(billing.clone()), budget);
    crate::dfm_review::start(&mut h.app);
    let error = h.app.dfm_review.error.clone().expect("the press reports its refusal");
    assert!(error.starts_with("AUTH_UNAVAILABLE:"), "{error}");
    assert_eq!(billing.admits(), 0, "no grant is requested without a durable mapping");
}

/// Red for the repair: cancelling after a report exists must not release the hold as no-effect.
#[test]
fn review_cancel_does_not_release_a_finished_report() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_, run) = begin_with_record(&mut h, &billing, &ledger);
    show(&mut h, card_for(&run));
    crate::dfm_review::cancel(&mut h.app);
    assert_eq!(billing.releases(), 0, "a report exists; cancel must keep the hold until it settles");
    crate::dfm_review::settle_review_run(&mut h.app, &run, true).expect("the report settles after cancel");
    assert_eq!(billing.commits(), 1);
}

/// The exact run-to-operation mapping exists before any dispatch, and settlement refuses another
/// run's report: a `latest` or active-run inference never decides this operation.
#[test]
fn review_persists_exact_mapping_before_dispatch() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (operation, run) = begin_with_record(&mut h, &billing, &ledger);
    assert_eq!(h.app.dfm_review.mapped_run.as_deref(), Some(run.as_str()), "the reserved run is the panel's only run");
    assert_eq!(ledger.operation_for(&run), Some(operation.clone()), "the durable mapping exists before dispatch");
    assert_eq!(billing.admits(), 1);
    let error = crate::dfm_review::settle_review_run(&mut h.app, "run-other", true).unwrap_err();
    assert!(error.starts_with("UNSAFE_TO_START:"), "{error}");
    assert_eq!(billing.commits(), 0, "another run's report cannot charge this operation");
}

/// A terminal ledger row survives restart and is never charged again.
#[test]
fn review_terminal_recovery_does_not_charge_twice() {
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let mut first = Harness::new();
    let (_operation, run) = begin_with_record(&mut first, &billing, &ledger);
    crate::dfm_review::settle_review_run(&mut first.app, &run, true).expect("the first settlement commits");
    assert_eq!(billing.commits(), 1);

    let mut restarted = Harness::new();
    install(&mut restarted.app, &billing, &ledger);
    crate::dfm_review::settle_review_run(&mut restarted.app, &run, true).expect("a terminal row is already settled");
    assert_eq!(billing.commits(), 1, "restart does not charge twice");
}

/// An idle service with no terminal evidence for the exact run keeps the hold and surfaces
/// recovery; it never releases from `last_run`/`active_run` or an empty result.
#[test]
fn review_uncertain_run_is_held_for_recovery() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_operation, run) = begin_with_record(&mut h, &billing, &ledger);
    h.app.dfm_review.running = true;
    let ctx = egui::Context::default();
    for _ in 0..5 {
        h.app.dfm_review.polled = -1.0;
        crate::dfm_review::poll(&mut h.app, &ctx);
    }
    assert!(h.app.dfm_review.error.as_deref().is_some_and(|error| error.starts_with("AUTH_UNAVAILABLE:")), "{:?}", h.app.dfm_review.error);
    assert_eq!(billing.releases(), 0, "no terminal evidence means no release");
    assert_eq!(billing.commits(), 0);
    assert_eq!(h.app.dfm_review.mapped_run.as_deref(), Some(run.as_str()));
}

/// The real start path persists the mapping, then a dispatch failure with no project is proven
/// no-effect and releases once.
#[test]
fn review_start_failure_releases_one_hold() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    install(&mut h.app, &billing, &ledger);
    let operation = crate::control::mint_operation().expect("host entropy");
    ledger.insert_record(operation.clone());
    let error = crate::dfm_review::start_with_operation(&mut h.app, operation.clone()).unwrap_err();
    assert!(error.starts_with("UNSAFE_TO_START:"), "{error}");
    assert_eq!(billing.admits(), 1);
    assert_eq!(billing.commits(), 0);
    assert_eq!(billing.releases(), 1, "no work ran, so the single hold is released");
    let run = h.app.dfm_review.mapped_run.clone().expect("the reserved run was bound before dispatch");
    assert_eq!(ledger.operation_for(&run), Some(operation.clone()));
    assert_eq!(ledger.intent_for(&operation), Some(SettlementIntent::Release(RefusalCode::UnsafeToStart)));
    assert_eq!(ledger.status_for(&operation), Some(solidcraft_engine::entitlement::OperationStatus::Released));
}

struct ProjectScratch(std::path::PathBuf);

impl ProjectScratch {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("intcad-review-{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for ProjectScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn project_session(h: &mut Harness, scratch: &ProjectScratch) -> std::path::PathBuf {
    let created = solidcraft_engine::project::create(&scratch.0, "review-project", "part").expect("an empty project");
    h.app.session.path = Some(created.design.to_string_lossy().into_owned());
    created.project.root().to_path_buf()
}

fn stopped_run(root: &std::path::Path, run: &str, outcome: RunOutcome) {
    let store = RunStore::new(root);
    let meta = RunMeta::new(
        run,
        "review-project",
        "sha256:candidate",
        "sha256:config",
        "model",
        "2026-11-21T00:00:00.000Z",
        "2026-11-21T00:00:01.000Z",
        outcome,
        Some("the inert fixture stopped before a report".into()),
    );
    store.write_stopped(&meta).expect("an inert stopped record");
}

fn complete_findings(root: &std::path::Path, run: &str) {
    let store = RunStore::new(root);
    let dir = store.run_dir(run).expect("the run folder");
    let findings = json!({
        "schemaVersion": 2,
        "runId": run,
        "candidateDigest": "sha256:candidate",
        "profileDigest": "sha256:profile",
        "configDigest": "sha256:config",
        "modelVersion": "model",
        "projectId": "review-project",
        "source": {"kind": "native"},
        "findings": [],
        "rules": [],
        "notChecked": "",
    });
    std::fs::write(dir.join("findings.json"), findings.to_string()).unwrap();
}

/// A cancelled stopped run has `run.json` but no `findings.json`; the actual poll path reads its
/// terminal metadata independently of a findings report and releases once.
#[test]
fn review_poll_settles_a_stopped_cancelled_run() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_operation, run) = begin_with_record(&mut h, &billing, &ledger);
    let scratch = ProjectScratch::new("cancelled");
    let root = project_session(&mut h, &scratch);
    stopped_run(&root, &run, RunOutcome::Cancelled);
    h.app.dfm_review.running = true;
    h.app.dfm_review.cancelling = true;
    let ctx = egui::Context::default();
    h.app.dfm_review.polled = -1.0;
    crate::dfm_review::poll(&mut h.app, &ctx);
    assert_eq!(billing.commits(), 0);
    assert_eq!(billing.releases(), 1, "a terminal cancelled run with no report releases once");
    assert!(!h.app.dfm_review.running);
    assert!(!h.app.dfm_review.cancelling, "cancelling must not loop forever");
}

/// A superseded run that completed its findings is durable proof of a produced report and is
/// charged once, even though a newer run owns `latest`.
#[test]
fn review_poll_charges_a_superseded_complete_run() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_operation, run) = begin_with_record(&mut h, &billing, &ledger);
    let scratch = ProjectScratch::new("superseded");
    let root = project_session(&mut h, &scratch);
    stopped_run(&root, &run, RunOutcome::Superseded);
    complete_findings(&root, &run);
    h.app.dfm_review.running = true;
    let ctx = egui::Context::default();
    h.app.dfm_review.polled = -1.0;
    crate::dfm_review::poll(&mut h.app, &ctx);
    assert_eq!(billing.commits(), 1, "superseded with durable findings is still a produced report");
    assert_eq!(billing.releases(), 0);
}

/// A superseded run without findings is ambiguous: keep the hold and surface recovery.
#[test]
fn review_poll_holds_a_superseded_run_without_findings() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_operation, run) = begin_with_record(&mut h, &billing, &ledger);
    let scratch = ProjectScratch::new("superseded-ambiguous");
    let root = project_session(&mut h, &scratch);
    stopped_run(&root, &run, RunOutcome::Superseded);
    h.app.dfm_review.running = true;
    let ctx = egui::Context::default();
    for _ in 0..5 {
        h.app.dfm_review.polled = -1.0;
        crate::dfm_review::poll(&mut h.app, &ctx);
    }
    assert_eq!(billing.commits(), 0);
    assert_eq!(billing.releases(), 0, "ambiguous completion never releases");
    assert!(h.app.dfm_review.error.as_deref().is_some_and(|error| error.starts_with("AUTH_UNAVAILABLE:")), "{:?}", h.app.dfm_review.error);
}

/// A commit whose terminal ledger write fails still persists the settlement intent. A restart
/// must not repeat the wire call blindly; it surfaces recovery and keeps the one charge.
#[test]
fn review_terminal_write_failure_never_repeats_settlement() {
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let mut first = Harness::new();
    let (_operation, run) = begin_with_record(&mut first, &billing, &ledger);
    ledger.fail_next_terminal(true);
    let error = crate::dfm_review::settle_review_run(&mut first.app, &run, true).unwrap_err();
    assert!(error.starts_with("AUTH_UNAVAILABLE:"), "{error}");
    assert_eq!(billing.commits(), 1);

    ledger.fail_next_terminal(false);
    let mut restarted = Harness::new();
    install(&mut restarted.app, &billing, &ledger);
    let recovery = crate::dfm_review::settle_review_run(&mut restarted.app, &run, true).unwrap_err();
    assert!(recovery.starts_with("AUTH_UNAVAILABLE:"), "{recovery}");
    assert_eq!(billing.commits(), 1, "an ambiguous prior settlement is never charged again");
}

/// If the settlement intent cannot be written, no billing wire call is made.
#[test]
fn review_intent_write_failure_does_not_call_billing() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_operation, run) = begin_with_record(&mut h, &billing, &ledger);
    ledger.fail_next_intent(true);
    let error = crate::dfm_review::settle_review_run(&mut h.app, &run, true).unwrap_err();
    assert!(error.starts_with("AUTH_UNAVAILABLE:"), "{error}");
    assert_eq!(billing.commits(), 0);
    assert_eq!(billing.releases(), 0);
    assert!(ledger.intent_for(&h.app.dfm_review.operation.clone().expect("operation")).is_none());
}

/// A restart finds the controlled run from the project's run folders plus the durable store
/// mapping and settles the stopped terminal record without rerunning it.
#[test]
fn review_startup_resumes_a_stopped_cancelled_run() {
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let mut first = Harness::new();
    let (_operation, run) = begin_with_record(&mut first, &billing, &ledger);
    let scratch = ProjectScratch::new("startup-cancelled");
    let root = project_session(&mut first, &scratch);
    stopped_run(&root, &run, RunOutcome::Cancelled);

    let mut restarted = Harness::new();
    install(&mut restarted.app, &billing, &ledger);
    restarted.app.session.path = first.app.session.path.clone();
    crate::dfm_review::resume_review(&mut restarted.app);
    assert_eq!(billing.releases(), 1, "startup settles the one stopped run");
    assert_eq!(billing.commits(), 0);
    assert_eq!(restarted.app.dfm_review.mapped_run.as_deref(), Some(run.as_str()));
    assert!(!restarted.app.dfm_review.running, "startup never reruns the check");
}

#[test]
fn review_startup_charges_a_superseded_complete_run() {
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let mut first = Harness::new();
    let (_operation, run) = begin_with_record(&mut first, &billing, &ledger);
    let scratch = ProjectScratch::new("startup-superseded");
    let root = project_session(&mut first, &scratch);
    stopped_run(&root, &run, RunOutcome::Superseded);
    complete_findings(&root, &run);

    let mut restarted = Harness::new();
    install(&mut restarted.app, &billing, &ledger);
    restarted.app.session.path = first.app.session.path.clone();
    crate::dfm_review::resume_review(&mut restarted.app);
    assert_eq!(billing.commits(), 1, "a durable superseded report is charged once");
    assert_eq!(billing.releases(), 0);
    assert!(!restarted.app.dfm_review.running);
}

/// A mapping-write failure after admission is still a proven-unstarted release, but it must go
/// through the durable original-ID intent and terminal row, never a bare release wire call.
#[test]
fn review_mapping_failure_releases_through_durable_intent() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    install(&mut h.app, &billing, &ledger);
    let operation = crate::control::mint_operation().expect("host entropy");
    ledger.insert_record(operation.clone());
    ledger.fail_next_mapping(true);
    let error = crate::dfm_review::prepare_review_with(&mut h.app, operation.clone()).unwrap_err();
    assert!(error.starts_with("AUTH_UNAVAILABLE:"), "{error}");
    assert_eq!(billing.admits(), 1);
    assert_eq!(billing.releases(), 1);
    assert_eq!(billing.commits(), 0);
    assert_eq!(ledger.intent_for(&operation), Some(SettlementIntent::Release(RefusalCode::UnsafeToStart)));
    assert_eq!(ledger.status_for(&operation), Some(solidcraft_engine::entitlement::OperationStatus::Released));
    assert!(h.app.dfm_review.mapped_run.is_none(), "no mapping survived the failure");
}

/// An existing settlement claim—even with the identical intent—is not rewired.
#[test]
fn review_existing_settlement_claim_is_not_rewired() {
    let mut h = Harness::new();
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let (_operation, run) = begin_with_record(&mut h, &billing, &ledger);
    let operation = h.app.dfm_review.operation.clone().expect("operation");
    // Pretend an earlier process claimed the same intent and stopped before the terminal row.
    ledger.record_settlement_intent_claim(&operation, SettlementIntent::Commit(BillingResult::Succeeded)).expect("claim");
    let error = crate::dfm_review::settle_review_run(&mut h.app, &run, true).unwrap_err();
    assert!(error.starts_with("AUTH_UNAVAILABLE:"), "{error}");
    assert_eq!(billing.commits(), 0, "an existing claim owns the only settlement attempt");
    assert_eq!(billing.releases(), 0);
}

/// A ledger lookup error during startup is visible, not silently treated as "no pending run".
#[test]
fn review_startup_ledger_error_is_visible() {
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let mut first = Harness::new();
    let (_operation, run) = begin_with_record(&mut first, &billing, &ledger);
    let scratch = ProjectScratch::new("startup-error");
    let root = project_session(&mut first, &scratch);
    stopped_run(&root, &run, RunOutcome::Cancelled);

    let mut restarted = Harness::new();
    install(&mut restarted.app, &billing, &ledger);
    restarted.app.session.path = first.app.session.path.clone();
    ledger.fail_next_lookup(true);
    crate::dfm_review::resume_review(&mut restarted.app);
    assert!(
        restarted.app.dfm_review.error.as_deref().is_some_and(|error| error.starts_with("AUTH_UNAVAILABLE:")),
        "{:?}",
        restarted.app.dfm_review.error
    );
    assert_eq!(billing.releases(), 0, "a ledger error never releases or charges");
}

/// Startup skips an already terminal row without reopening it as pending work.
#[test]
fn review_startup_skips_a_terminal_row() {
    let billing = CountingBilling::default();
    let ledger = ReviewLedger::default();
    let mut first = Harness::new();
    let (operation, run) = begin_with_record(&mut first, &billing, &ledger);
    let scratch = ProjectScratch::new("startup-terminal");
    let root = project_session(&mut first, &scratch);
    stopped_run(&root, &run, RunOutcome::Cancelled);
    ledger.set_terminal(&operation, solidcraft_engine::entitlement::OperationStatus::Released);

    let mut restarted = Harness::new();
    install(&mut restarted.app, &billing, &ledger);
    restarted.app.session.path = first.app.session.path.clone();
    crate::dfm_review::resume_review(&mut restarted.app);
    assert!(restarted.app.dfm_review.settled);
    assert_eq!(billing.releases(), 0, "a terminal row is not settled again");
    assert!(restarted.app.dfm_review.error.is_none());
}
