//! The desktop billing adapter: `BillingPort` over the installed commit authority.
//!
//! Controlled entries (Review/DFM, Control, MCP, Mastra) need a `BillingPort`.
//! Before this adapter the desktop installed none, so every controlled entry
//! refused with `AuthUnavailable`. These tests pin the mapping onto the
//! already-reviewed `AccountService` calls:
//!
//! - `admit`   -> `begin`; only `NewlyAdmitted` carries execution authority
//! - `commit`  -> `finalize`; a terminal status confirms the charge
//! - `release` -> `finalize(FailedNoEffect)`; the server books `RELEASED`

use solidcraft_engine::entitlement::{
    AccountId, AccountService, Admission, AuthenticatedHostScope, Availability, BillingResult, Capability, EntryCategory, Grant, GrantId,
    HostSessionBinding, InstallationId, MonotonicClock, MonotonicInstant, OperationId, OperationStatus, Refusal, RefusalCode,
};
use std::collections::VecDeque;

use crate::billing_adapter::{admit_for, commit_for, release_for};

struct Scripted {
    begins: VecDeque<Result<Admission, Refusal>>,
    finalizes: VecDeque<Result<OperationStatus, Refusal>>,
    seen: Vec<String>,
}

impl Scripted {
    fn new(begins: Vec<Result<Admission, Refusal>>, finalizes: Vec<Result<OperationStatus, Refusal>>) -> Self {
        Self { begins: begins.into(), finalizes: finalizes.into(), seen: Vec::new() }
    }
}

impl AccountService for Scripted {
    fn status(&self) -> Result<Availability, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "unused"))
    }

    fn authorize(&mut self, _: &AuthenticatedHostScope, _: Capability) -> Result<Grant, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "unused"))
    }

    fn begin(&mut self, _: &AuthenticatedHostScope, operation: &OperationId, _: Capability, entry: EntryCategory) -> Result<Admission, Refusal> {
        self.seen.push(format!("begin:{}:{entry:?}", operation.as_str()));
        self.begins.pop_front().unwrap_or_else(|| Err(Refusal::new(RefusalCode::AuthUnavailable, "no scripted begin")))
    }

    fn finalize(&mut self, _: &AuthenticatedHostScope, operation: &OperationId, result: BillingResult) -> Result<OperationStatus, Refusal> {
        self.seen.push(format!("finalize:{}:{result:?}", operation.as_str()));
        self.finalizes.pop_front().unwrap_or_else(|| Err(Refusal::new(RefusalCode::AuthUnavailable, "no scripted finalize")))
    }

    fn operation(&self, _: &AuthenticatedHostScope, _: &OperationId) -> Result<OperationStatus, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "unused"))
    }
}

struct Fixed(bool);

impl MonotonicClock for Fixed {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_millis(20_000)
    }

    fn wall_now(&self) -> std::time::SystemTime {
        if self.0 { std::time::UNIX_EPOCH } else { std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000) }
    }
}

fn scope() -> AuthenticatedHostScope {
    // The engine's `test_fixture` constructor is `#[cfg(test)]` inside the engine
    // crate, so it is not visible from this crate's tests; build it the way the
    // host does instead.
    AuthenticatedHostScope::new(
        AccountId::new("test-account"),
        InstallationId::new("test-installation"),
        HostSessionBinding::new("test-session-binding"),
    )
}

fn operation() -> OperationId {
    OperationId::parse("op_desktop_adapter").expect("a valid operation id")
}

fn grant(capability: Capability) -> Grant {
    Grant::from_server(GrantId::new("grant_desktop_adapter"), capability, MonotonicInstant::from_millis(80_000))
}

#[test]
fn admit_returns_the_server_grant_and_never_invents_one() {
    let mut service = Scripted::new(vec![Ok(Admission::NewlyAdmitted { operation: operation(), grant: grant(Capability::Review) })], vec![]);
    let clock = Fixed(true);
    let admitted = admit_for(&scope(), &clock, &mut service, &operation(), Capability::Review, EntryCategory::ReviewPanel)
        .expect("a newly admitted operation is admitted");
    assert_eq!(admitted.id().as_str(), "grant_desktop_adapter");
    assert_eq!(service.seen, vec!["begin:op_desktop_adapter:ReviewPanel".to_string()]);
}

#[test]
fn admit_keeps_already_exists_as_status_only_never_authority() {
    let mut service = Scripted::new(vec![Ok(Admission::AlreadyExists { status: OperationStatus::Held })], vec![]);
    let clock = Fixed(true);
    let refusal = admit_for(&scope(), &clock, &mut service, &operation(), Capability::Dfm, EntryCategory::CliDfm)
        .expect_err("an existing operation must not become execution authority");
    assert_eq!(refusal.code, RefusalCode::UnsafeToStart);
}

#[test]
fn admit_surfaces_a_business_refusal_unchanged() {
    let mut service = Scripted::new(vec![Ok(Admission::Refused(Refusal::new(RefusalCode::CreditsExhausted, "no credit left")))], vec![]);
    let clock = Fixed(true);
    let refusal = admit_for(&scope(), &clock, &mut service, &operation(), Capability::Control, EntryCategory::DirectControl)
        .expect_err("a business refusal is an error at this seam");
    assert_eq!(refusal.code, RefusalCode::CreditsExhausted);
}

#[test]
fn commit_confirms_only_a_terminal_status() {
    let mut service = Scripted::new(vec![], vec![Ok(OperationStatus::Charged), Ok(OperationStatus::Held)]);
    let clock = Fixed(true);
    assert_eq!(commit_for(&scope(), &clock, &mut service, &operation(), BillingResult::Succeeded).map(OperationStatus::is_terminal), Ok(true));
    assert!(commit_for(&scope(), &clock, &mut service, &operation(), BillingResult::Succeeded).is_err());
    assert_eq!(service.seen, vec!["finalize:op_desktop_adapter:Succeeded".to_string(), "finalize:op_desktop_adapter:Succeeded".to_string()]);
}

#[test]
fn release_books_failed_no_effect() {
    let mut service = Scripted::new(vec![], vec![Ok(OperationStatus::Released)]);
    let clock = Fixed(true);
    release_for(&scope(), &clock, &mut service, &operation(), RefusalCode::UnsafeToStart).expect("release must reach the server as a release");
    assert_eq!(service.seen, vec!["finalize:op_desktop_adapter:FailedNoEffect".to_string()]);
}

#[test]
fn release_reports_a_failed_release_instead_of_swallowing_it() {
    let mut service = Scripted::new(vec![], vec![Err(Refusal::new(RefusalCode::AuthUnavailable, "transport"))]);
    let clock = Fixed(true);
    let refusal = release_for(&scope(), &clock, &mut service, &operation(), RefusalCode::UnsafeToStart)
        .expect_err("a failed release must stay visible so the hold is not lost");
    assert_eq!(refusal.code, RefusalCode::AuthUnavailable);
}
