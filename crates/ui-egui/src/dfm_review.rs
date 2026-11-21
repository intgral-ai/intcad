//! The Review panel: run a sheet-metal DFM check by hand, read its report, and hand one report
//! to the Assistant.
//!
//! The check is the host's own manual entry (`dfm.check`), so it runs without the Assistant being
//! open and without a model connection. The report is the project's stored run: this panel reads
//! it with `inspect.dfm` and shows the run id, the outcome, the rule rows and every finding with
//! its measured and required value. Selecting a row locates the finding in the viewport, and a
//! label picked in the viewport selects the same row, because both sides read the one selection
//! the viewport keeps.
//!
//! The card can be dragged onto the Assistant, where it becomes a draft attachment of the next
//! message. The attachment names the run it came from, and the message reads that run at send
//! time, so a newer check does not replace what the person chose to send.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use egui::{Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use serde_json::Value;
use solidcraft_engine::entitlement::{
    BillingResult, OperationId, OperationStatus, OperationStoreError, RefusalCode, RunId, SaveOutcome, SettlementIntent,
};
#[cfg(not(target_arch = "wasm32"))]
use solidcraft_engine::entitlement::{Capability, EntryCategory, ReviewOperation, UnsettledOperation, UpperBound};

use crate::SolveApp;
use crate::dfm_chat::Card;
use crate::i18n::{format as trf, tr};
use crate::theme::Tokens;

/// How often a running check asks for its report, in seconds.
const POLL_SECONDS: f64 = 0.4;
/// How many findings the list shows; the rest are counted under it, so a huge report stays cheap.
const MAX_ROWS: usize = 200;
/// The finding states in the order the list groups them; any other state is listed last as not
/// covered, so a finding no rule could judge is never dropped from the report.
const GROUPS: [&str; 4] = ["failed", "advisory", "checked", "passed"];

/// What the Review panel keeps between frames.
#[derive(Default)]
pub struct State {
    /// The report on show, if a check has finished for this project.
    pub card: Option<Card>,
    /// The run the card shows.
    pub run: Option<String>,
    /// A check this panel started is still running or being stopped.
    pub running: bool,
    /// The person asked to stop the exact run; its report still decides commit versus release.
    pub cancelling: bool,
    /// Why the check could not run or did not finish.
    pub error: Option<String>,
    /// When the panel last asked for the report, in the frame clock's seconds.
    polled: f64,
    /// The run id of the card being dragged towards the Assistant.
    pub dragging: Option<String>,
    /// Where the pointer last was while the report was dragged: the release frame may not carry a
    /// position when the drop lands on the Assistant's web view.
    drag_at: Option<egui::Pos2>,
    /// The design whose report the panel has looked for, so opening it again does not.
    document: Option<String>,
    /// Polls in a row where the service was idle and no new run had appeared yet: the worker may
    /// not have picked the job up, and a check that never records is told as a failure later.
    quiet: u8,
    /// The "what was not checked" list is unfolded.
    pub not_checked_open: bool,
    /// The billed logical operation for this panel's check, once one was admitted.
    pub(crate) operation: Option<OperationId>,
    /// The run this operation is durably mapped to, when one has been recorded.
    pub(crate) mapped_run: Option<String>,
    /// Whether this operation has already committed or released. A settled operation is never
    /// settled again, even if the same report is attached to the Assistant.
    pub(crate) settled: bool,
    /// A recovered row whose original scope could not be proven is kept for the original session.
    /// The current session may neither settle nor release it.
    pub(crate) settlement_blocked: bool,
    /// The controlled operation journal could not be verified (inventory or fallback read
    /// failure). This is independent of whether an operation id is known and blocks fresh
    /// admission and settlement. It is conservatively retained for this process; no automatic
    /// revalidation clears it.
    pub(crate) recovery_unverified: bool,
}

impl State {
    /// Forget the report: a new design, a new project or a closed panel has nothing to show.
    /// Billing identity is kept: a check that started here still needs its one settlement even
    /// when the report is replaced by another document's.
    pub fn clear(&mut self) {
        let operation = self.operation.take();
        let mapped_run = self.mapped_run.take();
        let settled = self.settled;
        let cancelling = self.cancelling;
        let settlement_blocked = self.settlement_blocked;
        let recovery_unverified = self.recovery_unverified;
        *self = Self { operation, mapped_run, settled, cancelling, settlement_blocked, recovery_unverified, ..Self::default() };
    }
}

/// Whether this panel can run a check here: the desktop build has the background worker, the
/// browser build has neither a project store nor a worker.
pub fn available() -> bool {
    !cfg!(target_arch = "wasm32")
}

fn refusal_name(code: RefusalCode) -> &'static str {
    match code {
        RefusalCode::AuthRequired => "AUTH_REQUIRED",
        RefusalCode::EmailUnverified => "EMAIL_UNVERIFIED",
        RefusalCode::AccountDisabled => "ACCOUNT_DISABLED",
        RefusalCode::CapabilityDenied => "CAPABILITY_DENIED",
        RefusalCode::VersionUnsupported => "VERSION_UNSUPPORTED",
        RefusalCode::ProtocolUnsupported => "PROTOCOL_UNSUPPORTED",
        RefusalCode::UnsafeToStart => "UNSAFE_TO_START",
        RefusalCode::CreditsExhausted => "CREDITS_EXHAUSTED",
        RefusalCode::CreditsHeld => "CREDITS_HELD",
        RefusalCode::OperationConflict => "OPERATION_CONFLICT",
        RefusalCode::OperationFinalized => "OPERATION_FINALIZED",
        RefusalCode::OperationOwnedElsewhere => "OPERATION_OWNED_ELSEWHERE",
        RefusalCode::AuthUnavailable => "AUTH_UNAVAILABLE",
    }
}

fn coded(code: RefusalCode, message: impl std::fmt::Display) -> String {
    format!("{}: {message}", refusal_name(code))
}

#[cfg(not(target_arch = "wasm32"))]
fn verified_review_bound(app: &SolveApp) -> Result<Duration, String> {
    match app.budget.bound(Capability::Review, EntryCategory::ReviewPanel) {
        UpperBound::Verified(bound) => Ok(bound),
        UpperBound::Unverified => Err(coded(RefusalCode::UnsafeToStart, "this Review path has no verified safe-stop bound")),
    }
}

/// Why a Review press cannot start. This is checked before any operation is admitted, so a
/// missing authority or durable journal never reaches an execution path.
#[cfg(not(target_arch = "wasm32"))]
fn authority_error(app: &SolveApp) -> Option<String> {
    if let Err(error) = verified_review_bound(app) {
        return Some(error);
    }
    if app.billing.is_none() {
        return Some(coded(RefusalCode::AuthUnavailable, "no host billing authority is installed for the Review panel"));
    }
    if app.review_store.is_none() {
        return Some(coded(RefusalCode::AuthUnavailable, "no durable Review journal is installed; the run cannot be bound before dispatch"));
    }
    None
}

/// A recovery read that failed leaves the journal's disposition unverified. This is checked
/// before admission and before settlement, independently of whether an operation id is known.
fn unverified_recovery_refusal(app: &SolveApp) -> Option<String> {
    app.dfm_review.recovery_unverified.then(|| {
        coded(
            RefusalCode::AuthUnavailable,
            "the controlled operation journal could not be verified in this process; no fresh Review admission or settlement is allowed until the app restarts or a reviewed revalidation succeeds",
        )
    })
}

/// The host's exact Review identity: a free-looking latest run or the service's active run is
/// never used to infer this operation.
#[cfg(not(target_arch = "wasm32"))]
fn prepare_review_with(app: &mut SolveApp, operation: OperationId) -> Result<(String, u64), String> {
    if let Some(error) = unverified_recovery_refusal(app) {
        return Err(error);
    }
    if app.dfm_review.settlement_blocked
        && let Some(previous) = app.dfm_review.operation.as_ref()
    {
        return Err(coded(
            RefusalCode::AuthUnavailable,
            format!(
                "Review operation {} belongs to another or unprovable account scope; it is kept for the original session and may not be settled or replaced here",
                previous.as_str()
            ),
        ));
    }
    if !app.dfm_review.settled
        && let Some(previous) = app.dfm_review.operation.as_ref()
    {
        return Err(coded(RefusalCode::UnsafeToStart, format!("the previous Review operation {} has no confirmed settlement", previous.as_str())));
    }
    if let Some(error) = authority_error(app) {
        return Err(error);
    }
    let bound = verified_review_bound(app)?;
    let store = app
        .review_store
        .as_ref()
        .ok_or_else(|| coded(RefusalCode::AuthUnavailable, "no durable Review journal is installed; the run cannot be bound before dispatch"))?;
    match store.load(&operation) {
        Ok(Some(record)) if !record.status.is_terminal() => {}
        Ok(Some(_)) => return Err(coded(RefusalCode::OperationFinalized, "the Review operation is already terminal")),
        Ok(None) => {
            return Err(coded(
                RefusalCode::AuthUnavailable,
                format!("the Review operation {} has no durable row; it cannot be bound before dispatch", operation.as_str()),
            ));
        }
        Err(error) => return Err(coded(RefusalCode::AuthUnavailable, format!("the Review ledger is unavailable: {error:?}"))),
    }
    let grant = app
        .billing
        .as_mut()
        .ok_or_else(|| coded(RefusalCode::AuthUnavailable, "no host billing authority is installed for the Review panel"))?
        .admit(&operation, Capability::Review, EntryCategory::ReviewPanel)
        .map_err(|error| coded(error.code, error.message))?;
    drop(grant); // Execution authority, never a value the panel stores or logs.
    let now = solidcraft_engine::validation::now_ms();
    let stop_at_ms = now.saturating_add(u64::try_from(bound.as_millis()).unwrap_or(u64::MAX));
    let run = solidcraft_engine::validation::generate_run_id(now);
    let recorded = match app.review_store.as_mut() {
        Some(store) => store.record_review(&ReviewOperation::new(RunId::new(&run), operation.clone())),
        None => Ok(()),
    };
    if let Err(error) = recorded {
        // The run was never dispatched, so the one hold is proven unstarted. Route that release
        // through the same durable original-ID intent/terminal path as every other settlement;
        // never send a bare release wire call that a restart could repeat.
        let release = apply_settlement(app, &operation, SettlementIntent::Release(RefusalCode::UnsafeToStart));
        let recovery = match release {
            Ok(()) => "the hold was released durably under the original operation".to_string(),
            Err(release_error) => format!("the release is not confirmed; keep operation {} for recovery: {release_error}", operation.as_str()),
        };
        return Err(coded(RefusalCode::AuthUnavailable, format!("the Review mapping could not be persisted before dispatch: {error:?}; {recovery}")));
    }
    app.dfm_review.operation = Some(operation);
    app.dfm_review.mapped_run = Some(run.clone());
    app.dfm_review.settled = false;
    app.dfm_review.cancelling = false;
    app.dfm_review.settlement_blocked = false;
    Ok((run, stop_at_ms))
}

/// Persist the settlement intent, then perform exactly one wire action for this operation.
/// A terminal row finishes without another call; an existing intent without a terminal row is an
/// ambiguous earlier attempt and is surfaced, never repeated.
fn apply_settlement(app: &mut SolveApp, operation: &OperationId, intent: SettlementIntent) -> Result<(), String> {
    if let Some(error) = unverified_recovery_refusal(app) {
        return Err(error);
    }
    if app.dfm_review.settlement_blocked {
        return Err(coded(
            RefusalCode::AuthUnavailable,
            format!(
                "Review operation {} is kept under another or unprovable account scope; this session may not settle or release it",
                operation.as_str()
            ),
        ));
    }
    if let Some(store) = app.review_store.as_ref() {
        match store.load(operation) {
            Ok(Some(record)) if record.status.is_terminal() => {
                app.dfm_review.settled = true;
                return Ok(());
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(coded(
                    RefusalCode::AuthUnavailable,
                    format!("the Review operation {} has no durable row; it cannot settle safely", operation.as_str()),
                ));
            }
            Err(error) => {
                return Err(coded(RefusalCode::AuthUnavailable, format!("the Review ledger is unavailable: {error:?}")));
            }
        }
    }
    if let Some(store) = app.review_store.as_mut() {
        match store.record_settlement_intent(operation, intent) {
            Ok(SaveOutcome::Saved) => {}
            Ok(SaveOutcome::Existing) => {
                return Err(coded(
                    RefusalCode::AuthUnavailable,
                    format!(
                        "operation {} already has a settlement claim, even for the same intent; an earlier wire call may have completed. Query/resume under the same id and never repeat it blindly",
                        operation.as_str()
                    ),
                ));
            }
            Ok(SaveOutcome::Conflict) => {
                return Err(coded(
                    RefusalCode::AuthUnavailable,
                    format!("operation {} could not claim its settlement intent; refuse rather than wire an ambiguous action", operation.as_str()),
                ));
            }
            Err(OperationStoreError::TerminalState) => {
                app.dfm_review.settled = true;
                return Ok(());
            }
            Err(error) => {
                return Err(coded(
                    RefusalCode::AuthUnavailable,
                    format!("operation {} could not persist its settlement intent; no wire call was made: {error:?}", operation.as_str()),
                ));
            }
        }
    }
    let Some(billing) = app.billing.as_mut() else {
        return Err(coded(RefusalCode::AuthUnavailable, "no host billing authority is installed for the Review panel"));
    };
    let outcome = match intent {
        SettlementIntent::Commit(result) => billing.commit(operation, result).map(|_| ()),
        SettlementIntent::Release(reason) => billing.release(operation, reason),
    };
    match outcome {
        Ok(()) => {
            let terminal = match intent {
                SettlementIntent::Commit(_) => OperationStatus::Charged,
                SettlementIntent::Release(_) => OperationStatus::Released,
            };
            if let Some(store) = app.review_store.as_mut()
                && let Err(error) = store.mark_terminal(operation, terminal)
                && error != OperationStoreError::TerminalState
            {
                app.dfm_review.settled = true;
                return Err(coded(
                    RefusalCode::AuthUnavailable,
                    format!(
                        "operation {} settled but its terminal ledger row was not written; do not settle it again: {error:?}",
                        operation.as_str()
                    ),
                ));
            }
            app.dfm_review.settled = true;
            Ok(())
        }
        Err(error) => Err(coded(
            error.code,
            format!("the Review result was handled but settlement of operation {} was not confirmed; do not run it again", operation.as_str()),
        )),
    }
}

/// Settle one exact Review run once. `produced` must come from that run's own report, never from
/// the newest run or an active-run inference. A terminal ledger row is never settled again.
pub(crate) fn settle_review_run(app: &mut SolveApp, run: &str, produced: bool) -> Result<(), String> {
    if app.dfm_review.settled {
        return Ok(());
    }
    if app.dfm_review.mapped_run.as_deref() != Some(run) && app.dfm_review.operation.is_some() {
        return Err(coded(RefusalCode::UnsafeToStart, "the Review settlement names another run"));
    }
    let operation = match app.dfm_review.operation.clone() {
        Some(operation) => operation,
        None => {
            let Some(store) = app.review_store.as_ref() else { return Ok(()) };
            match store.review_operation(&RunId::new(run)) {
                Ok(Some(operation)) => {
                    app.dfm_review.mapped_run = Some(run.to_string());
                    operation
                }
                Ok(None) => return Ok(()),
                Err(error) => {
                    return Err(coded(RefusalCode::AuthUnavailable, format!("the Review ledger could not read this run: {error:?}")));
                }
            }
        }
    };
    let intent = if produced { SettlementIntent::Commit(BillingResult::Succeeded) } else { SettlementIntent::Release(RefusalCode::UnsafeToStart) };
    apply_settlement(app, &operation, intent)
}

/// After a restart, recover the durable disposition of the one controlled Review operation.
///
/// A row whose original scope cannot be proven is bound to its original id and blocks a fresh
/// check; this session neither settles nor releases it. An owned row with an exact run settles
/// through the ordinary Review path once and is never rerun. Several unresolved rows are never
/// collapsed into a guessed target.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn resume_review(app: &mut SolveApp) {
    if !available() || app.dfm_review.running || app.dfm_review.operation.is_some() {
        return;
    }
    let Some(store) = app.review_store.as_ref() else { return };
    let rows = match store.unsettled_operations(app.commit_scope.as_ref()) {
        Ok(rows) => rows,
        Err(error) => {
            app.dfm_review.recovery_unverified = true;
            app.dfm_review.error =
                Some(coded(RefusalCode::AuthUnavailable, format!("the Review ledger could not list unresolved operations: {error:?}")));
            return;
        }
    };
    let review_rows: Vec<UnsettledOperation> = rows
        .into_iter()
        .filter(|row| {
            let record = row.record();
            record.capability == Capability::Review && record.entry == EntryCategory::ReviewPanel
        })
        .collect();
    if let Some(first) = review_rows.first() {
        bind_recovered(app, first);
        if review_rows.len() > 1 {
            app.dfm_review.settlement_blocked = true;
            let operation = first.record().operation.as_str().to_string();
            app.dfm_review.error = Some(coded(
                RefusalCode::AuthUnavailable,
                format!(
                    "{} unresolved Review operations are kept; refusing to choose between them and keeping {} under its original id",
                    review_rows.len(),
                    operation
                ),
            ));
            return;
        }
        match first {
            UnsettledOperation::Unproven(record) => {
                app.dfm_review.error = Some(coded(
                    RefusalCode::AuthUnavailable,
                    format!(
                        "Review operation {} has no durable account-scope binding; it is kept unreleased and may not be settled or rerun by this session",
                        record.operation.as_str()
                    ),
                ));
            }
            UnsettledOperation::Foreign(record) => {
                app.dfm_review.error = Some(coded(
                    RefusalCode::AuthUnavailable,
                    format!(
                        "Review operation {} belongs to another account, installation or session; it is kept for the original session and may not be settled or rerun here",
                        record.operation.as_str()
                    ),
                ));
            }
            UnsettledOperation::Owned(record) => match record.review_run_id.as_ref() {
                Some(run) => settle_recovered_run(app, run.as_str()),
                None => app.dfm_review.error = Some(surface_uncertain(app)),
            },
        }
        return;
    }

    // No non-terminal Review row remains. The project's durable run records can still show an
    // already terminal operation; it is skipped, never reopened.
    let Some(path) = app.session.path.as_deref() else { return };
    let Ok(Some(project)) = solidcraft_engine::project::Project::of(std::path::Path::new(path)) else { return };
    let run_store = solidcraft_engine::dfm::run::RunStore::new(project.root());
    let mut candidates = Vec::new();
    if let Ok(Some(submitted)) = run_store.submitted() {
        candidates.push(submitted.run_id);
    }
    if let Ok(entries) = std::fs::read_dir(run_store.runs_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("run-") && !candidates.iter().any(|candidate| candidate == &name) {
                candidates.push(name);
            }
        }
    }
    // Bounded current-project resumption. Scanning multiple pending controlled runs across
    // projects and choosing the right one is an explicit Task 11 lifecycle gate.
    let mut lookup_error = None;
    let mapped = candidates.into_iter().find_map(|run| match store.review_operation(&RunId::new(&run)) {
        Ok(Some(operation)) => Some((run, operation)),
        Ok(None) => None,
        Err(error) => {
            lookup_error = Some(error);
            None
        }
    });
    if let Some(error) = lookup_error {
        app.dfm_review.recovery_unverified = true;
        app.dfm_review.error = Some(coded(RefusalCode::AuthUnavailable, format!("the Review ledger could not scan mapped runs: {error:?}")));
        return;
    }
    let Some((run, operation)) = mapped else { return };
    match store.load(&operation) {
        Ok(Some(record)) if record.status.is_terminal() => {
            app.dfm_review.settled = true;
        }
        Ok(Some(_)) => {
            // The inventory above should have classified this row. Keep it fail-closed rather than
            // settle a row whose scope was never checked.
            app.dfm_review.operation = Some(operation.clone());
            app.dfm_review.mapped_run = Some(run.clone());
            app.dfm_review.settled = false;
            app.dfm_review.settlement_blocked = true;
            app.dfm_review.recovery_unverified = true;
            app.dfm_review.error = Some(coded(
                RefusalCode::AuthUnavailable,
                format!("Review operation {} remains unresolved but was not classified by the durable inventory; it is kept", operation.as_str()),
            ));
        }
        Ok(None) => {
            app.dfm_review.recovery_unverified = true;
            app.dfm_review.error = Some(coded(RefusalCode::AuthUnavailable, format!("Review operation {} has no durable row", operation.as_str())));
        }
        Err(error) => {
            app.dfm_review.recovery_unverified = true;
            app.dfm_review.error = Some(coded(RefusalCode::AuthUnavailable, format!("the Review ledger is unavailable: {error:?}")));
        }
    }
}

/// Bind a recovered row to its durable original identity. The panel keeps it visible so no code
/// path can mint a replacement operation while the disposition is unresolved.
#[cfg(not(target_arch = "wasm32"))]
fn bind_recovered(app: &mut SolveApp, row: &UnsettledOperation) {
    let record = row.record();
    app.dfm_review.operation = Some(record.operation.clone());
    app.dfm_review.mapped_run = record.review_run_id.as_ref().map(|run| run.as_str().to_owned());
    app.dfm_review.settled = false;
    app.dfm_review.settlement_blocked = !matches!(row, UnsettledOperation::Owned(_));
}

/// Settle an owned recovered run with the same evidence rules as a live poll. Missing terminal
/// evidence surfaces the original operation for later recovery; it is never released from an
/// uncertain outcome.
#[cfg(not(target_arch = "wasm32"))]
fn settle_recovered_run(app: &mut SolveApp, run: &str) {
    let card = crate::dfm_chat::refresh_run(app, Some(run));
    let produced = card.report_produced;
    let settle = if matches!(card.outcome.as_deref(), Some("cancelled" | "failed")) && !produced {
        settle_review_run(app, run, false)
    } else if produced {
        settle_review_run(app, run, true)
    } else {
        app.dfm_review.error = Some(surface_uncertain(app));
        return;
    };
    app.dfm_review.error = settle.err();
}

/// Surface a held Review operation whose exact run has no terminal evidence. It is never released
/// from an uncertain outcome; the host must resume settlement under the original id.
pub(crate) fn surface_uncertain(app: &mut SolveApp) -> String {
    let operation = app.dfm_review.operation.as_ref().map(|operation| operation.as_str().to_string()).unwrap_or_default();
    coded(
        RefusalCode::AuthUnavailable,
        format!("the Review run has no terminal report; keep operation {operation} for settlement recovery and do not rerun it"),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn release_unstarted(app: &mut SolveApp, reason: RefusalCode) -> Result<(), String> {
    if app.dfm_review.settled {
        return Ok(());
    }
    let Some(operation) = app.dfm_review.operation.clone() else { return Ok(()) };
    apply_settlement(app, &operation, SettlementIntent::Release(reason))
}

/// Start one check from the panel and keep it on show while it runs. Nothing of the Assistant is
/// needed: the entry is the host's own, and a refusal is the panel's error, not a chat notice.
pub fn start(app: &mut SolveApp) {
    app.dfm_review.error = None;
    app.dfm_review.dragging = None;
    if !available() {
        app.dfm_review.error = Some(tr("ui.dfm_review_unavailable").to_string());
        return;
    }
    if app.dfm_review.running {
        // One press is one logical operation; a second click while it runs starts nothing.
        return;
    }
    let operation = match crate::control::mint_operation() {
        Some(operation) => operation,
        None => {
            app.dfm_review.error = Some(coded(RefusalCode::AuthUnavailable, "the host could not mint an operation for the Review check"));
            return;
        }
    };
    if let Err(error) = start_with_operation(app, operation) {
        app.dfm_review.error = Some(error);
    }
}

/// Prepare the durable mapping and then dispatch exactly the reserved run. A service failure
/// before dispatch is proven no-effect: its one hold is released and no second operation is minted.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn start_with_operation(app: &mut SolveApp, operation: OperationId) -> Result<(), String> {
    let (run, stop_at_ms) = prepare_review_with(app, operation.clone())?;
    let reservation = solidcraft_engine::dfm::run::Reservation { run_id: run, operation_id: operation.as_str().to_string(), stop_at_ms };
    match solidcraft_engine::dfm::service::start_reserved(&mut app.session, reservation) {
        Ok(_) => {
            app.dfm_review.running = true;
            app.dfm_review.cancelling = false;
            app.dfm_review.polled = 0.0;
            app.dfm_review.quiet = 0;
            Ok(())
        }
        Err(error) => {
            release_unstarted(app, RefusalCode::UnsafeToStart)?;
            Err(coded(RefusalCode::UnsafeToStart, format!("the Review check did not start: {error}")))
        }
    }
}

/// The browser has no background DFM worker or project store. Keep this entry fail-closed even
/// if a future caller reaches it without the panel's `available()` check.
#[cfg(target_arch = "wasm32")]
pub(crate) fn start_with_operation(_app: &mut SolveApp, _operation: OperationId) -> Result<(), String> {
    Err(coded(RefusalCode::AuthUnavailable, "Review checks are unavailable in the browser"))
}

/// Stop the live check. The hold is not released here: a worker that already finished may have
/// produced a report, so the exact run's terminal evidence decides commit versus release.
pub fn cancel(app: &mut SolveApp) {
    if available() {
        crate::agent_chat::cancel_dfm(app);
    }
    if app.dfm_review.running {
        app.dfm_review.cancelling = true;
    }
}

/// Read the project's newest report into the panel, without running anything. A project with no
/// finished check leaves the panel empty, ready for its Run button.
pub fn reload(app: &mut SolveApp) {
    if !available() {
        return;
    }
    let card = crate::dfm_chat::refresh_run(app, None);
    if card.available {
        adopt(app, card);
    }
}

/// While a check this panel started runs, read its report until it finishes. A frame that finds
/// it finished shows the new run, or the reason it failed. The panel asks every [`POLL_SECONDS`],
/// not every frame: the worker writes its record once, and each ask is a project read.
pub fn poll(app: &mut SolveApp, ctx: &egui::Context) {
    if !app.dfm_review.running || !available() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let waited = now - app.dfm_review.polled;
    if (0.0..POLL_SECONDS).contains(&waited) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(POLL_SECONDS - waited));
        return;
    }
    app.dfm_review.polled = now;
    ctx.request_repaint();
    let Some(run) = app.dfm_review.mapped_run.clone() else {
        app.dfm_review.running = false;
        app.dfm_review.error = Some(coded(RefusalCode::AuthUnavailable, "the Review run identity is missing; no result can be settled"));
        return;
    };
    // Read only the exact reserved run. The service's `last_run` and `active_run` are diagnostics,
    // never this operation's identity.
    let card = crate::dfm_chat::refresh_run(app, Some(&run));
    match card.outcome.as_deref() {
        Some("current") if card.report_produced => {
            let settle = settle_review_run(app, &run, true);
            app.dfm_review.running = false;
            app.dfm_review.cancelling = false;
            app.dfm_review.error = settle.err();
            adopt(app, card);
        }
        Some("superseded") if card.report_produced => {
            // A superseded run completed its report before a newer submission; its own findings
            // are durable evidence, so it is charged once like a current run.
            let settle = settle_review_run(app, &run, true);
            app.dfm_review.running = false;
            app.dfm_review.cancelling = false;
            app.dfm_review.error = settle.err();
            adopt(app, card);
        }
        Some("cancelled" | "failed") if !card.report_produced => {
            let settle = settle_review_run(app, &run, false);
            app.dfm_review.running = false;
            app.dfm_review.cancelling = false;
            app.dfm_review.error = settle.err().or_else(|| Some(tr("ui.dfm_review_did_not_finish").to_string()));
        }
        _ if card.report_produced => {
            let settle = settle_review_run(app, &run, true);
            app.dfm_review.running = false;
            app.dfm_review.cancelling = false;
            app.dfm_review.error = settle.err();
            adopt(app, card);
        }
        _ => {
            // No terminal evidence yet. While the service runs or the person has asked to stop,
            // keep waiting for that exact run. Never release a hold from an idle service alone.
            let status = crate::agent_chat::dfm_state();
            if status.running || app.dfm_review.cancelling {
                app.dfm_review.quiet = 0;
                return;
            }
            app.dfm_review.quiet = app.dfm_review.quiet.saturating_add(1);
            if app.dfm_review.quiet < 5 {
                return;
            }
            app.dfm_review.running = false;
            app.dfm_review.cancelling = false;
            app.dfm_review.error = Some(surface_uncertain(app));
        }
    }
}

/// Show `card` and let the viewport draw its marks, so a row and a label address one finding.
fn adopt(app: &mut SolveApp, card: Card) {
    app.dfm_review.run = Some(card.run_id.clone()).filter(|run| !run.is_empty());
    crate::dfm_chat::show_marks(app, &card);
    app.dfm_chat.card = Some(card.clone());
    app.dfm_review.card = Some(card);
}

/// The report card's run as the Assistant's attachment: the run to read again when the message is
/// sent, and what the chip says.
pub fn attachment(app: &mut SolveApp, run: &str) -> Result<crate::agent_chat::attach::Attachment, String> {
    // The card on show is the report itself; only a run the panel does not show is read back.
    let card = match app.dfm_review.card.clone().filter(|card| card.run_id == run) {
        Some(card) => card,
        None => crate::dfm_chat::refresh_run(app, Some(run)),
    };
    if card.status == "error" {
        return Err(card.error.unwrap_or_else(|| tr("ui.dfm_review_unreadable").to_string()));
    }
    if card.run_id.is_empty() || card.run_id != run {
        return Err(trf("ui.dfm_review_no_such_run", &[("run", run)]));
    }
    Ok(crate::agent_chat::attach::Attachment::dfm_report(&card))
}

/// The Assistant's panel takes a dropped report: say so while the card is dragged over it.
pub fn drag_hint(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(_run) = app.dfm_review.dragging.as_deref() else { return };
    // While the pointer is over the Assistant's web view the app may not be told a position on
    // the frame the button comes up, so the drag remembers the last one it was told.
    if let Some(at) = ctx.input(|i| i.pointer.latest_pos()) {
        app.dfm_review.drag_at = Some(at);
    }
    let Some(rect) = app.agent_chat.drop_rect else { return };
    let t = Tokens::get();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("sc_dfm_review_drop_hint")));
    let zone = rect.shrink(6.0);
    painter.rect(zone, t.radius_xl, t.accent.gamma_multiply(0.16), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(t.font_heading);
    let text = painter.layout(tr("ui.dfm_review_drop_hint").to_string(), font, t.text, (zone.width() - 24.0).max(40.0));
    painter.galley(zone.center() - text.size() / 2.0, text, t.text);
    crate::scenario::publish_rect("dfm_review:drop_hint", zone);
}

/// The pointer was released after dragging a report: over the Assistant it becomes an attachment,
/// anywhere else the drag is dropped.
pub fn release(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(run) = app.dfm_review.dragging.clone() else { return };
    let (released, at) = ctx.input(|i| (i.pointer.any_released(), i.pointer.latest_pos()));
    if !released {
        return;
    }
    app.dfm_review.dragging = None;
    let at = at.or(app.dfm_review.drag_at.take());
    if let (Some(at), Some(rect)) = (at, app.agent_chat.drop_rect)
        && rect.contains(at)
    {
        attach_to_assistant(app, &run);
    }
    ctx.request_repaint();
}

/// Put the report on the Assistant's next message, or say why it cannot be.
pub fn attach_to_assistant(app: &mut SolveApp, run: &str) {
    match attachment(app, run) {
        Ok(file) => {
            if let Err(reason) = crate::agent_chat::attach_ready(app, file) {
                crate::agent_chat::notify(app, reason, true);
            }
        }
        Err(reason) => crate::agent_chat::notify(app, reason, true),
    }
}

/// The panel's contents, below the rail's title strip.
pub fn panel(app: &mut SolveApp, ui: &mut egui::Ui, area: Rect) {
    // A design the panel has not seen yet: show the report the project already has, if any. A
    // report of another design is never shown for this one.
    let document = app.session.document_stamp().document_id;
    if app.dfm_review.document.as_deref() != Some(document.as_str()) {
        app.dfm_review.clear();
        app.dfm_review.document = Some(document);
        reload(app);
        #[cfg(not(target_arch = "wasm32"))]
        resume_review(app);
    }
    let t = Tokens::get();
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(area.shrink2(vec2(t.space_12, t.space_8))).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui = &mut child;
    actions(app, ui);
    ui.add_space(t.space_8);
    if let Some(error) = app.dfm_review.error.clone() {
        crate::ui_kit::text(ui, &error, crate::ui_kit::Text::Error(&error));
        if error.starts_with("AUTH_REQUIRED") {
            crate::account::show_inline_sign_in(app, ui, "account.sign_in_for_review", "review:sign_in");
        }
        ui.add_space(t.space_8);
    }
    let Some(card) = app.dfm_review.card.clone() else {
        if !app.dfm_review.running {
            crate::ui_kit::text(ui, tr("ui.dfm_review_empty"), crate::ui_kit::Text::Note(tr("ui.dfm_review_empty")));
        }
        return;
    };
    card_header(app, ui, &card);
    ui.add_space(t.space_8);
    // The list takes the room left above the footer; "not checked" is a folded line, because the
    // report's own list of what no rule covered is long and is read when someone looks for it.
    let footer = t.space_16 * 2.0;
    let list_height = (ui.available_height() - footer).max(t.control_height * 2.0);
    findings(app, ui, &card, list_height);
    let missing = card.not_checked.lines().filter(|line| !line.trim().is_empty()).count();
    if missing > 0 {
        ui.add_space(t.space_4);
        let label = trf("ui.dfm_review_not_checked", &[("n", &missing.to_string())]);
        let open = app.dfm_review.not_checked_open;
        let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), t.control_height * 0.8), Sense::click());
        let icon = Rect::from_min_size(rect.min, vec2(rect.height(), rect.height())).shrink(t.form_icon_inset);
        let fold = crate::ui_kit::disclosure_button_at(ui, icon, open, &label);
        let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(t.font_caption), t.text_dim);
        ui.painter().galley(pos2(icon.right() + t.space_4, rect.center().y - galley.size().y / 2.0), galley, t.text_dim);
        if row.clicked() || fold.clicked() {
            app.dfm_review.not_checked_open = !open;
        }
        if open {
            ui.label(RichText::new(&card.not_checked).font(egui::FontId::proportional(t.font_caption)).color(t.text_dim));
        }
    }
}

/// Run, cancel and the running state.
fn actions(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    if app.dfm_review.running {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(tr("ui.dfm_review_running")).font(egui::FontId::proportional(t.font_small)).color(t.text_dim));
        });
        if crate::ui_kit::button(ui, tr("ui.dfm_review_cancel"), crate::ui_kit::Button::Normal).clicked() {
            cancel(app);
        }
        return;
    }
    let label = if app.dfm_review.card.is_some() { tr("ui.dfm_review_run_again") } else { tr("ui.dfm_review_run") };
    let response = crate::ui_kit::button(ui, label, crate::ui_kit::Button::Primary);
    crate::scenario::publish_target("review:run", response.rect);
    if response.clicked() {
        start(app);
    }
}

/// The card's top: its run, outcome and counts, and the grip that drags it to the Assistant.
fn card_header(app: &mut SolveApp, ui: &mut egui::Ui, card: &Card) {
    let t = Tokens::get();
    let run = card.run_id.clone();
    let outcome = card.status.clone();
    let freshness = card.freshness.clone();
    let counts = card.counts();
    let state = match outcome.as_str() {
        "running" => tr("ui.dfm_review_running"),
        "cancelled" => tr("ui.dfm_review_cancelled"),
        _ => tr("ui.dfm_review_done"),
    };
    let summary = trf(
        "ui.dfm_review_counts",
        &[
            ("failed", &count(&counts, "failed")),
            ("advisory", &count(&counts, "advisory")),
            ("checked", &count(&counts, "checked")),
            ("not_covered", &count(&counts, "unsupported")),
        ],
    );
    let drag = format!("{}\n{}", tr("ui.dfm_review_drag"), run);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), t.control_height * 2.0), Sense::click_and_drag());
    let painter = ui.painter();
    let fill = if response.hovered() || response.dragged() { t.hover } else { t.panel };
    painter.rect_filled(rect, t.radius_md, fill);
    painter.rect_stroke(rect, t.radius_md, t.primary_border(t.border), egui::StrokeKind::Inside);
    let ink = t.text;
    let mut top = rect.top() + t.space_6;
    let title = painter.layout_no_wrap(state.to_string(), egui::FontId::proportional(t.font_label), ink);
    painter.galley(pos2(rect.left() + t.space_8, top), title.clone(), ink);
    top += title.size().y + t.space_2;
    let line = painter.layout_no_wrap(summary, egui::FontId::proportional(t.font_caption), t.text_dim);
    painter.galley(pos2(rect.left() + t.space_8, top), line.clone(), t.text_dim);
    top += line.size().y + t.space_2;
    let stale = matches!(freshness.as_str(), "staleModel" | "staleConfig" | "staleBoth");
    let note = if stale { format!("{} · {}", short_run(&run), tr("ui.dfm_review_stale")) } else { short_run(&run) };
    let ink = if stale { t.warning } else { t.text_dim };
    let g = painter.layout_no_wrap(note, egui::FontId::proportional(t.font_caption), ink);
    painter.galley(pos2(rect.left() + t.space_8, top), g, ink);
    response.clone().on_hover_text(drag);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tr("ui.dfm_review_card")));
    crate::scenario::publish_target("review:card", rect);
    if response.drag_started() {
        app.dfm_review.dragging = Some(run.clone());
    }
    if response.dragged()
        && let Some(at) = ui.ctx().pointer_latest_pos()
    {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("sc_dfm_review_ghost")));
        let text = painter.layout_no_wrap(tr("ui.dfm_review_dragging").to_string(), egui::FontId::proportional(t.font_small), t.on_accent);
        let size = text.size() + vec2(t.space_12, t.space_6);
        let at = at + vec2(t.space_12, t.space_12);
        let frame = Rect::from_min_size(at, size);
        painter.rect(frame, t.radius_md, t.form_primary_fill, Stroke::NONE, egui::StrokeKind::Inside);
        painter.galley(at + vec2(t.space_6, t.space_3), text, t.on_accent);
    }
}

fn count(counts: &Value, key: &str) -> String {
    counts.get(key).and_then(Value::as_u64).unwrap_or(0).to_string()
}

fn short_run(run: &str) -> String {
    run.chars().take(24).collect()
}

/// The rule rows, then the findings grouped by what they say: failures first, then advice, then
/// what was checked, and last what passed. A row selects its finding in the viewport.
fn findings(app: &mut SolveApp, ui: &mut egui::Ui, card: &Card, height: f32) {
    let t = Tokens::get();
    let selected = app.viewport.dfm.selected_key().map(|(run, id)| (run.to_string(), id.to_string())).or_else(|| app.dfm_chat.selected.clone());
    egui::ScrollArea::vertical().id_salt("sc_dfm_review_findings").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let mut shown = 0;
        let mut states: Vec<&str> = GROUPS.to_vec();
        for finding in &card.findings {
            if !states.contains(&finding.state.as_str()) {
                states.push(finding.state.as_str());
            }
        }
        for state in states {
            let rows: Vec<&crate::dfm_chat::Finding> = card.findings.iter().filter(|finding| finding.state == state).collect();
            if rows.is_empty() {
                continue;
            }
            ui.add_space(t.space_6);
            ui.label(
                RichText::new(format!("{} {}", rows.len(), state_word(state))).font(egui::FontId::proportional(t.font_caption)).color(t.text_dim),
            );
            for finding in rows {
                if shown >= MAX_ROWS {
                    return;
                }
                shown += 1;
                let is_selected = selected.as_ref().is_some_and(|(run, id)| run == &card.run_id && id == &finding.id);
                if row(ui, finding, is_selected) && available() {
                    // The row is this card's own: it never depends on which run the Assistant
                    // happens to show, so a check started from Chat does not stop these rows.
                    app.dfm_chat.selected = Some((card.run_id.clone(), finding.id.clone()));
                    app.viewport.select_dfm_finding(Some(&finding.id));
                }
            }
        }
        if shown == 0 {
            crate::ui_kit::text(ui, tr("ui.dfm_review_no_findings"), crate::ui_kit::Text::Note(tr("ui.dfm_review_no_findings")));
        } else if card.findings.len() > shown {
            let left = (card.findings.len() - shown).to_string();
            let more = trf("ui.dfm_review_more", &[("n", &left)]);
            crate::ui_kit::text(ui, &more, crate::ui_kit::Text::Note(&more));
        }
    });
}

fn state_word(state: &str) -> &'static str {
    match state {
        "failed" => tr("ui.dfm_review_failed"),
        "advisory" => tr("ui.dfm_review_advisory"),
        "checked" => tr("ui.dfm_review_checked"),
        "passed" => tr("ui.dfm_review_passed"),
        _ => tr("ui.dfm_review_not_covered"),
    }
}

fn state_ink(state: &str) -> Color32 {
    let t = Tokens::get();
    match state {
        "failed" => t.error,
        "advisory" => t.warning,
        "passed" | "checked" => t.text_dim,
        _ => t.warning,
    }
}

/// One finding: its state, its rule and what it applies to, and the measured and required values.
fn row(ui: &mut egui::Ui, finding: &crate::dfm_chat::Finding, selected: bool) -> bool {
    let t = Tokens::get();
    let width = ui.available_width();
    let lines: Vec<(String, Color32)> = {
        let mut lines = Vec::new();
        let mut head = format!("{} · {}", finding.rules.join(", "), finding.body);
        if let Some(feature) = &finding.feature
            && !feature.kind.is_empty()
        {
            head.push_str(&format!(" · {}{}", feature.kind, feature.number.map(|n| format!(" {n}")).unwrap_or_default()));
        }
        lines.push((head, t.text));
        if let Some(measured) = &finding.measured {
            let value = format!("{} {}", measured.value, measured.unit);
            let line = match &finding.requirement {
                Some(req) => trf("ui.dfm_review_value", &[("measured", &value), ("required", &format!("{} {}", req.value, req.unit))]),
                None => trf("ui.dfm_review_value_no_requirement", &[("measured", &value)]),
            };
            lines.push((line, t.text_dim));
        } else if let Some(reason) = &finding.reason {
            lines.push((reason.clone(), t.text_dim));
        }
        lines
    };
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(t.font_caption)));
    let height = t.space_4 * 2.0 + line_height * lines.len() as f32;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, t.radius_sm, t.selected_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, t.radius_sm, t.hover);
    }
    let ink = if selected { t.selected_ink } else { t.text };
    let mut y = rect.top() + t.space_4;
    for (at, (text, tone)) in lines.iter().enumerate() {
        let colour = if selected {
            t.selected_ink
        } else if at == 0 {
            unit_ink(&finding.state, ink)
        } else {
            *tone
        };
        let galley = painter.layout(text.clone(), egui::FontId::proportional(t.font_caption), colour, (rect.width() - t.space_8 * 2.0).max(20.0));
        painter.galley(pos2(rect.left() + t.space_8, y), galley, colour);
        y += line_height;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &finding.id));
    crate::scenario::publish_target(&format!("review:finding:{}", finding.id), rect);
    response.clicked()
}

fn unit_ink(state: &str, ink: Color32) -> Color32 {
    match state {
        "failed" | "advisory" => state_ink(state),
        _ => ink,
    }
}

#[cfg(test)]
#[path = "dfm_review_tests.rs"]
mod tests;
