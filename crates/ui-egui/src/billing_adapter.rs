//! The desktop billing port, expressed as calls over the installed commit authority.
//!
//! `BillingPort` is the host-facing port that controlled entries use
//! (`admit`/`commit`/`release`). The desktop already installs a verified
//! `AccountService` plus its `AuthenticatedHostScope` and `MonotonicClock` as
//! the commit authority; this module maps the port onto those calls instead of
//! introducing a second network stack or a second source of authority.
//!
//! Why free functions rather than a stored adapter: `SolveApp` owns
//! `commit_account` and `billing` in separate fields, so anything holding the
//! account could not also be stored as the port. `control.rs` already borrows
//! the pair at the call site (`app.commit_scope.as_ref()`,
//! `app.commit_account.as_deref()`); these functions take that same borrow.
//!
//! Contract, matching the server's frozen settlement semantics:
//!
//! - `admit` maps to `begin`. Only `NewlyAdmitted` carries execution
//!   authority. `AlreadyExists` is status only and is refused here, because a
//!   completed start must never become a second grant.
//! - `commit` maps to `finalize`. The result is confirmed only by a terminal
//!   status; anything else leaves the settlement unresolved for the caller's
//!   durable retry.
//! - `release` maps to `finalize(FailedNoEffect)`, which the server books as
//!   `RELEASED` and which also revokes the operation's execution grants.

use solidcraft_engine::entitlement::{
    AccountService, Admission, AuthenticatedHostScope, BillingPort, BillingResult, Capability, EntryCategory, Grant, MonotonicClock, OperationId,
    OperationStatus, Refusal, RefusalCode,
};

/// Admit one controlled operation, returning the server's execution grant.
///
/// `AlreadyExists` is deliberately a refusal: the start already happened, and
/// reusing its status as authority would run a second time on one hold.
pub(crate) fn admit_for(
    scope: &AuthenticatedHostScope,
    _clock: &dyn MonotonicClock,
    service: &mut dyn AccountService,
    operation: &OperationId,
    capability: Capability,
    entry: EntryCategory,
) -> Result<Grant, Refusal> {
    match service.begin(scope, operation, capability, entry)? {
        Admission::NewlyAdmitted { grant, .. } => Ok(grant),
        Admission::Allowed => Err(Refusal::new(RefusalCode::AuthUnavailable, "the billing authority admitted without a server execution grant")),
        Admission::AlreadyExists { .. } => {
            Err(Refusal::new(RefusalCode::UnsafeToStart, "this operation already started; its status is not execution authority"))
        }
        Admission::Refused(refusal) => Err(refusal),
    }
}

/// Settle one controlled operation with a charge.
///
/// Only a terminal status confirms the charge. A non-terminal status is an
/// error so the caller keeps the settlement pending rather than reporting a
/// charge that has not happened.
pub(crate) fn commit_for(
    scope: &AuthenticatedHostScope,
    _clock: &dyn MonotonicClock,
    service: &mut dyn AccountService,
    operation: &OperationId,
    result: BillingResult,
) -> Result<OperationStatus, Refusal> {
    let status = service.finalize(scope, operation, result)?;
    if status.is_terminal() {
        Ok(status)
    } else {
        Err(Refusal::new(RefusalCode::UnsafeToStart, "the settlement did not reach a terminal state; keep it pending for the durable retry"))
    }
}

/// Release one controlled operation's hold.
///
/// `FailedNoEffect` is the server's release result; it is never a `commit` with
/// a different code, because the ledger must record `RELEASED` rather than a
/// charge. A failure here is returned, not swallowed: a lost release keeps the
/// hold, and the caller's durable settlement path must retry it under the same
/// operation id.
pub(crate) fn release_for(
    scope: &AuthenticatedHostScope,
    _clock: &dyn MonotonicClock,
    service: &mut dyn AccountService,
    operation: &OperationId,
    _reason: RefusalCode,
) -> Result<OperationStatus, Refusal> {
    service.finalize(scope, operation, BillingResult::FailedNoEffect)
}

#[cfg(test)]
#[path = "billing_adapter_tests.rs"]
mod tests;

/// `BillingPort` over the installed commit authority.
///
/// The app stores the account, scope and clock as separate fields, so this is
/// built at the call site from those borrows rather than stored. It exists so
/// the existing `admit_control_request` signature (which takes `&mut dyn
/// BillingPort`) can be used unchanged.
pub(crate) struct CommitAuthorityBilling<'a> {
    scope: &'a AuthenticatedHostScope,
    clock: &'a dyn MonotonicClock,
    service: &'a mut dyn AccountService,
}

impl<'a> CommitAuthorityBilling<'a> {
    pub(crate) fn new(scope: &'a AuthenticatedHostScope, clock: &'a dyn MonotonicClock, service: &'a mut dyn AccountService) -> Self {
        Self { scope, clock, service }
    }
}

impl BillingPort for CommitAuthorityBilling<'_> {
    fn admit(&mut self, operation: &OperationId, capability: Capability, entry: EntryCategory) -> Result<Grant, Refusal> {
        admit_for(self.scope, self.clock, self.service, operation, capability, entry)
    }

    fn commit(&mut self, operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        commit_for(self.scope, self.clock, self.service, operation, result).map(|_| result)
    }

    fn release(&mut self, operation: &OperationId, reason: RefusalCode) -> Result<(), Refusal> {
        release_for(self.scope, self.clock, self.service, operation, reason).map(|_| ())
    }
}

/// The billing authority a controlled entry should use, in preference order.
pub(crate) enum BillingSource<'a> {
    Installed(&'a mut (dyn BillingPort + Send)),
    Authority(CommitAuthorityBilling<'a>),
}

impl BillingSource<'_> {
    pub(crate) fn port(&mut self) -> &mut dyn BillingPort {
        match self {
            Self::Installed(billing) => *billing,
            Self::Authority(authority) => authority,
        }
    }
}
