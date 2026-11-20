//! The commit boundary: recheck authorization before writing a result, and settle only with
//! positive evidence.
//!
//! A result arriving after `D` (the server's revocation time) must not be written, even though
//! the server's own answer is a generic `401`/no-state response. The client can only stop writes
//! after it has observed the revocation; it never claims to have prevented results already
//! committed before that observation. A known local deadline at or after expiry maps to no
//! charge; an unrelated `401` does not.
//!
//! This module is pure: it owns no network, clock or durable store. Hosts supply the current
//! account snapshot, the local authorization bound and the operation status.

use std::time::Duration;

use super::clock::{ConservativeDeadline, MonotonicInstant};
use super::types::{
    AuthenticatedHostScope, Availability, BillingResult, Capability, EntryCategory, OperationId, OperationStatus, Refusal, RefusalCode,
    TrustedContext, UpperBound,
};

/// Whether a result may be written and the operation scheduled to settle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitDecision {
    Allow,
    Drop { code: RefusalCode },
}

impl CommitDecision {
    pub fn allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// The host's local authorization bound for one operation.
///
/// The conservative [`ConservativeDeadline`] is preferred when the host measured a request
/// window. A server-issued grant expiry is the fallback; it is anchored when the grant arrived,
/// so it is never extended by a lost response or a network error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationBound<'a> {
    Deadline(&'a ConservativeDeadline),
    ExpiresAt(MonotonicInstant),
}

impl AuthorizationBound<'_> {
    /// Remaining time before the half-open bound, or `None` when the bound has passed or the
    /// clock reading is anomalous.
    pub fn remaining(&self, now: MonotonicInstant) -> Option<Duration> {
        match self {
            Self::Deadline(deadline) => deadline.remaining(now),
            Self::ExpiresAt(expires_at) => (now < *expires_at).then(|| expires_at.as_duration().saturating_sub(now.as_duration())),
        }
    }

    /// The half-open bound instant.
    pub fn expires_at(&self) -> MonotonicInstant {
        match self {
            Self::Deadline(deadline) => deadline.expires_at(),
            Self::ExpiresAt(expires_at) => *expires_at,
        }
    }

    /// Whether a request at `at` is at or after the known bound.
    pub fn expired_at(&self, at: MonotonicInstant) -> bool {
        match self {
            Self::Deadline(deadline) => deadline.expired_at(at),
            Self::ExpiresAt(expires_at) => at >= *expires_at,
        }
    }
}

/// One require-before-commit check.
///
/// Order matters. Identity and the current authenticated scope are checked first: a different
/// account, installation or session can never commit another operation's result. The current
/// account snapshot follows, so an observed disable/revocation stops the write. A terminal or
/// already committed ledger state is rejected before the deadline, because no second settle may
/// charge the same logical operation twice. Expiry is last: it only applies to an operation that
/// still has identity and commit authority.
#[derive(Clone, Debug)]
pub struct CommitGate<'a> {
    operation: &'a OperationId,
    capability: Capability,
    entry: EntryCategory,
    trusted: Option<&'a TrustedContext>,
    current_scope: Option<&'a AuthenticatedHostScope>,
    availability: Option<&'a Availability>,
    status: Option<OperationStatus>,
    bound: Option<AuthorizationBound<'a>>,
    now: MonotonicInstant,
}

impl<'a> CommitGate<'a> {
    /// A gate for a controlled operation. Free entries are always allowed; Task 9's gate only
    /// guards billed work. `now` must come from the host's monotonic clock reading at the commit
    /// point, not from the request's creation time.
    pub fn new(operation: &'a OperationId, capability: Capability, entry: EntryCategory) -> Self {
        Self {
            operation,
            capability,
            entry,
            trusted: None,
            current_scope: None,
            availability: None,
            status: None,
            bound: None,
            now: MonotonicInstant::from_millis(0),
        }
    }

    pub fn with_trusted(mut self, trusted: &'a TrustedContext) -> Self {
        self.trusted = Some(trusted);
        self
    }

    /// The account, installation and session the host can prove right now.
    pub fn with_current_scope(mut self, scope: &'a AuthenticatedHostScope) -> Self {
        self.current_scope = Some(scope);
        self
    }

    /// The current account snapshot. A controlled commit without one is refused.
    pub fn with_availability(mut self, availability: &'a Availability) -> Self {
        self.availability = Some(availability);
        self
    }

    /// The durable ledger state of the original operation, when the host has one. `None` means
    /// the host could not supply a durable row; the caller must say so in its release evidence.
    pub fn with_status(mut self, status: OperationStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_bound(mut self, bound: AuthorizationBound<'a>) -> Self {
        self.bound = Some(bound);
        self
    }

    /// The host's monotonic reading at the actual commit point.
    pub fn with_now(mut self, now: MonotonicInstant) -> Self {
        self.now = now;
        self
    }

    pub fn check(&self) -> CommitDecision {
        if self.entry.is_free() {
            return CommitDecision::Allow;
        }
        if let Some(trusted) = self.trusted {
            if !trusted.is_provable() {
                return CommitDecision::Drop { code: RefusalCode::CapabilityDenied };
            }
            if trusted.operation() != self.operation || trusted.capability() != self.capability || trusted.entry() != self.entry {
                return CommitDecision::Drop { code: RefusalCode::CapabilityDenied };
            }
            match self.current_scope {
                Some(scope) if scope != trusted.scope() => return CommitDecision::Drop { code: RefusalCode::AuthRequired },
                Some(_) => {}
                None => return CommitDecision::Drop { code: RefusalCode::AuthUnavailable },
            }
        }
        let Some(availability) = self.availability else {
            return CommitDecision::Drop { code: RefusalCode::AuthUnavailable };
        };
        if let Some(code) = availability.reasons.first() {
            return CommitDecision::Drop { code: *code };
        }
        if !availability.online {
            return CommitDecision::Drop { code: RefusalCode::AuthUnavailable };
        }
        if matches!(self.status, Some(OperationStatus::Charged | OperationStatus::Released | OperationStatus::Expired)) {
            return CommitDecision::Drop { code: RefusalCode::OperationFinalized };
        }
        let Some(bound) = self.bound else {
            return CommitDecision::Drop { code: RefusalCode::UnsafeToStart };
        };
        if bound.remaining(self.now).is_none() {
            return CommitDecision::Drop { code: RefusalCode::UnsafeToStart };
        }
        CommitDecision::Allow
    }
}

/// What a finalize or lookup reports after a known local deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupDisposition {
    /// The host knew the operation's local deadline and the request is at or after it. The
    /// contract maps this to no charge and a generic no-state answer.
    ExpiredNoState,
    /// No local deadline, a foreign operation, a credential problem or an early request. A `401`
    /// alone is never proof of no charge.
    Unknown,
}

/// The only mapping from a finalize/lookup failure to "no charge".
///
/// A known local bound that the request is at or after is sufficient. Anything else — including
/// every bare `401` — stays [`LookupDisposition::Unknown`] so the caller cannot invent a release.
pub fn finalize_lookup_disposition(bound: Option<AuthorizationBound<'_>>, at: MonotonicInstant) -> LookupDisposition {
    match bound {
        Some(bound) if bound.expired_at(at) => LookupDisposition::ExpiredNoState,
        _ => LookupDisposition::Unknown,
    }
}

/// What the host can prove about the result it is about to settle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultEvidence {
    /// The host refused before dispatch: a stale document precondition, a pre-send parameter
    /// error or a local cancellation. Only this may release a hold.
    NoEffect,
    /// The operation completed and its effect is retained.
    Effect,
    /// The host cannot prove whether an effect occurred. It is finalized, never rerun.
    Unknown,
}

/// The settlement action for proven evidence. There is deliberately no retry variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettlementAction {
    Release(RefusalCode),
    Commit(BillingResult),
}

/// Classify one result. An unknown outcome is only finalized under the original id; it never
/// authorizes a second engineering operation.
pub fn settlement_for(evidence: ResultEvidence) -> SettlementAction {
    match evidence {
        ResultEvidence::NoEffect => SettlementAction::Release(RefusalCode::UnsafeToStart),
        ResultEvidence::Effect => SettlementAction::Commit(BillingResult::Succeeded),
        ResultEvidence::Unknown => SettlementAction::Commit(BillingResult::Unknown),
    }
}

/// The latest point a batch of measured steps may run and still stop safely.
///
/// `next_safe_stop` is the end of the worst case for every planned step; started work may stop at
/// the next step boundary before it. `remaining` is the host's measured authorization at plan
/// time and `upper_bound` is the conservative sum of the steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SafeStopPlan {
    pub next_safe_stop: MonotonicInstant,
    pub remaining: Duration,
    pub upper_bound: Duration,
}

/// Plan a controlled batch before its first step.
///
/// Every step must have a verified upper bound, and the measured remaining authorization must
/// cover their sum. Otherwise the whole batch is refused with `UnsafeToStart` before any step
/// runs. A free entry bypasses this planner; the ordinary CAD path is unchanged.
pub fn plan_safe_stop(now: MonotonicInstant, bound: Option<AuthorizationBound<'_>>, step_bounds: &[UpperBound]) -> Result<SafeStopPlan, Refusal> {
    let mut upper_bound = Duration::ZERO;
    for step in step_bounds {
        match step {
            UpperBound::Verified(bound) => upper_bound = upper_bound.saturating_add(*bound),
            UpperBound::Unverified => {
                return Err(Refusal::new(RefusalCode::UnsafeToStart, "a batch step has no verified safe-stop upper bound"));
            }
        }
    }
    let Some(bound) = bound else {
        return Err(Refusal::new(RefusalCode::UnsafeToStart, "the batch has no measured authorization bound"));
    };
    let Some(remaining) = bound.remaining(now) else {
        return Err(Refusal::new(RefusalCode::UnsafeToStart, "the batch authorization has already expired"));
    };
    if remaining < upper_bound {
        return Err(Refusal::new(RefusalCode::UnsafeToStart, "the remaining authorization is shorter than the batch's safe-stop bound"));
    }
    Ok(SafeStopPlan { next_safe_stop: now.saturating_add(upper_bound), remaining, upper_bound })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlement::types::{AccountId, HostSessionBinding, InstallationId};

    fn online() -> Availability {
        Availability { online: true, version_supported: true, model_available: true, balance: 10, held: 0, reasons: Vec::new() }
    }

    fn revoked() -> Availability {
        Availability {
            online: true,
            version_supported: true,
            model_available: true,
            balance: 10,
            held: 0,
            reasons: vec![RefusalCode::AccountDisabled],
        }
    }

    fn scope(account: &str) -> AuthenticatedHostScope {
        AuthenticatedHostScope::new(AccountId::new(account), InstallationId::new("install-test"), HostSessionBinding::new("session-test"))
    }

    fn operation() -> OperationId {
        OperationId::parse("op_gate_commit_0001").unwrap()
    }

    fn trusted(scope: &AuthenticatedHostScope) -> TrustedContext {
        TrustedContext::mint(scope, operation().as_str(), Capability::Control, EntryCategory::DirectControl).unwrap()
    }

    fn deadline(expires_ms: u64) -> ConservativeDeadline {
        ConservativeDeadline::new(MonotonicInstant::from_millis(0), Duration::from_millis(expires_ms))
    }

    fn gate<'a>(
        operation: &'a OperationId,
        availability: &'a Availability,
        trusted: &'a TrustedContext,
        scope: &'a AuthenticatedHostScope,
        deadline: &'a ConservativeDeadline,
        now_ms: u64,
    ) -> CommitGate<'a> {
        CommitGate::new(operation, Capability::Control, EntryCategory::DirectControl)
            .with_trusted(trusted)
            .with_current_scope(scope)
            .with_availability(availability)
            .with_bound(AuthorizationBound::Deadline(deadline))
            .with_now(MonotonicInstant::from_millis(now_ms))
    }

    #[test]
    fn result_arriving_after_revoke_is_not_applied() {
        let scope = scope("acct-revoked");
        let trusted = trusted(&scope);
        let revoked = revoked();
        let ask_again = gate(&operation(), &revoked, &trusted, &scope, &deadline(60_000), 1_000).check();
        assert_eq!(ask_again, CommitDecision::Drop { code: RefusalCode::AccountDisabled });
    }

    #[test]
    fn committed_work_survives_revoke() {
        let scope = scope("acct-commit");
        let trusted = trusted(&scope);
        let online = online();
        let before = gate(&operation(), &online, &trusted, &scope, &deadline(60_000), 1_000).check();
        assert_eq!(before, CommitDecision::Allow, "a result committed before revocation stays");
        let revoke_observed = revoked();
        let after = gate(&operation(), &revoke_observed, &trusted, &scope, &deadline(60_000), 2_000).check();
        assert_eq!(after, CommitDecision::Drop { code: RefusalCode::AccountDisabled }, "only post-observation writes stop");
    }

    #[test]
    fn expired_operation_cannot_start_new_step() {
        let scope = scope("acct-expired");
        let trusted = trusted(&scope);
        let online = online();
        let deadline = deadline(10_000);
        let at_before = gate(&operation(), &online, &trusted, &scope, &deadline, 9_999).check();
        assert_eq!(at_before, CommitDecision::Allow);
        let at_deadline = gate(&operation(), &online, &trusted, &scope, &deadline, 10_000).check();
        assert_eq!(at_deadline, CommitDecision::Drop { code: RefusalCode::UnsafeToStart });
    }

    #[test]
    fn a_different_account_or_session_cannot_commit() {
        let trusted_scope = scope("acct-original");
        let trusted = trusted(&trusted_scope);
        let switched = scope("acct-other");
        let online = online();
        let decision = gate(&operation(), &online, &trusted, &switched, &deadline(60_000), 1_000).check();
        assert_eq!(decision, CommitDecision::Drop { code: RefusalCode::AuthRequired });
    }

    #[test]
    fn a_terminal_operation_is_never_settled_twice() {
        let scope = scope("acct-terminal");
        let trusted = trusted(&scope);
        let online = online();
        for status in [OperationStatus::Charged, OperationStatus::Released, OperationStatus::Expired] {
            let decision = gate(&operation(), &online, &trusted, &scope, &deadline(60_000), 1_000).with_status(status).check();
            assert_eq!(decision, CommitDecision::Drop { code: RefusalCode::OperationFinalized }, "{status:?}");
        }
    }

    #[test]
    fn a_controlled_commit_without_a_current_account_snapshot_is_refused() {
        let operation = operation();
        let decision = CommitGate::new(&operation, Capability::Control, EntryCategory::DirectControl).check();
        assert_eq!(decision, CommitDecision::Drop { code: RefusalCode::AuthUnavailable });
    }

    #[test]
    fn a_free_entry_is_never_blocked_by_the_commit_gate() {
        let operation = operation();
        let decision = CommitGate::new(&operation, Capability::Control, EntryCategory::HumanGuiFree).check();
        assert_eq!(decision, CommitDecision::Allow);
    }

    #[test]
    fn known_deadline_expiry_maps_to_no_charge() {
        let deadline = deadline(10_000);
        let at = MonotonicInstant::from_millis(10_000);
        assert_eq!(finalize_lookup_disposition(Some(AuthorizationBound::Deadline(&deadline)), at), LookupDisposition::ExpiredNoState);
        let grant_expiry = AuthorizationBound::ExpiresAt(MonotonicInstant::from_millis(5_000));
        assert_eq!(finalize_lookup_disposition(Some(grant_expiry), at), LookupDisposition::ExpiredNoState);
    }

    #[test]
    fn an_early_request_at_a_known_deadline_is_not_expired() {
        let deadline = deadline(10_000);
        let at = MonotonicInstant::from_millis(9_999);
        assert_eq!(finalize_lookup_disposition(Some(AuthorizationBound::Deadline(&deadline)), at), LookupDisposition::Unknown);
    }

    #[test]
    fn unrelated_401_is_not_automatically_no_charge() {
        // A 401 with no local bound, a foreign operation, or a credential error is unknown.
        assert_eq!(finalize_lookup_disposition(None, MonotonicInstant::from_millis(60_000)), LookupDisposition::Unknown);
        // A backwards clock is an anomaly, not proof that the known deadline passed.
        let deadline = deadline(10_000);
        assert_eq!(
            finalize_lookup_disposition(Some(AuthorizationBound::Deadline(&deadline)), MonotonicInstant::from_millis(4_000)),
            LookupDisposition::Unknown
        );
    }

    #[test]
    fn unknown_result_is_finalized_not_rerun() {
        assert_eq!(settlement_for(ResultEvidence::Unknown), SettlementAction::Commit(BillingResult::Unknown));
        assert_eq!(settlement_for(ResultEvidence::Effect), SettlementAction::Commit(BillingResult::Succeeded));
        assert_eq!(settlement_for(ResultEvidence::NoEffect), SettlementAction::Release(RefusalCode::UnsafeToStart));
    }

    #[test]
    fn batch_without_verified_upper_bound_is_refused_before_first_step() {
        let deadline = deadline(60_000);
        let steps = [UpperBound::Verified(Duration::from_secs(1)), UpperBound::Unverified];
        let refusal = plan_safe_stop(MonotonicInstant::from_millis(0), Some(AuthorizationBound::Deadline(&deadline)), &steps).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::UnsafeToStart);
        assert!(refusal.message.contains("verified safe-stop upper bound"), "{}", refusal.message);
    }

    #[test]
    fn remaining_shorter_than_upper_bound_is_unsafe_to_start() {
        let deadline = deadline(10_000);
        let steps = [UpperBound::Verified(Duration::from_secs(6)), UpperBound::Verified(Duration::from_secs(6))];
        let refusal = plan_safe_stop(MonotonicInstant::from_millis(0), Some(AuthorizationBound::Deadline(&deadline)), &steps).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::UnsafeToStart);
    }

    #[test]
    fn a_batch_that_fits_is_planned_to_stop_before_its_bound() {
        let deadline = deadline(60_000);
        let steps = [UpperBound::Verified(Duration::from_secs(10)), UpperBound::Verified(Duration::from_secs(5))];
        let plan = plan_safe_stop(MonotonicInstant::from_millis(1_000), Some(AuthorizationBound::Deadline(&deadline)), &steps).unwrap();
        assert_eq!(plan.upper_bound, Duration::from_secs(15));
        assert_eq!(plan.next_safe_stop, MonotonicInstant::from_millis(16_000));
        assert!(!AuthorizationBound::Deadline(&deadline).expired_at(plan.next_safe_stop));
    }

    #[test]
    fn a_batch_without_a_measured_bound_is_refused() {
        let steps = [UpperBound::Verified(Duration::from_secs(1))];
        let refusal = plan_safe_stop(MonotonicInstant::from_millis(0), None, &steps).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::UnsafeToStart);
    }
}
