//! Pure entitlement, budget and billing interfaces.
//!
//! The module is wasm safe: it owns no network, OS credential or wall-clock calls. Hosts supply
//! time, identity and billing through the ports in [`ports`]. Controlled paths fail closed until
//! a safe-stop bound is measured; ordinary CAD and free human entries do not depend on the network.

mod budget;
mod clock;
mod ports;
mod types;

#[cfg(test)]
mod tests;

pub use budget::BudgetTable;
pub use clock::{ConservativeDeadline, DeadlineError, FakeMonotonicClock, MonotonicClock, MonotonicInstant, RenewalAttempt};
pub use ports::{AccountService, BillingPort, CountingBilling, CredentialStore, FakeAccountService, InMemoryOperationStore, OperationStore};
pub use types::*;

use std::time::Duration;

/// Pure chat is not a billed operation. Zero credits do not block it.
///
/// It still needs an online authorized session, a supported version and an available model.
pub fn admit_chat(availability: &Availability, _budget: &BudgetTable) -> Admission {
    if let Some(code) = availability.reasons.first() {
        return Admission::Refused(Refusal::new(*code, "the account snapshot reports a refusal"));
    }
    if !availability.online {
        return Admission::Refused(Refusal::new(RefusalCode::AuthUnavailable, "chat needs an online session"));
    }
    if !availability.version_supported {
        return Admission::Refused(Refusal::new(RefusalCode::VersionUnsupported, "chat needs a supported client version"));
    }
    if !availability.model_available {
        return Admission::Refused(Refusal::new(RefusalCode::CapabilityDenied, "the selected model is unavailable"));
    }
    Admission::Allowed
}

/// Local preflight for a billed operation. It never turns affordability into execution authority.
///
/// A positive local balance still needs a server-issued grant, so this function refuses with
/// [`RefusalCode::AuthUnavailable`] until an account adapter has admitted the operation.
pub fn admit_billing(
    capability: Capability,
    entry: EntryCategory,
    availability: &Availability,
    budget: &BudgetTable,
    _store: &mut dyn OperationStore,
) -> Admission {
    if entry.is_free() {
        return Admission::Allowed;
    }
    if let Err(refusal) = budget.require(capability, entry, availability, Duration::MAX) {
        return Admission::Refused(refusal);
    }
    if availability.available() == 0 {
        let code = if availability.held > 0 { RefusalCode::CreditsHeld } else { RefusalCode::CreditsExhausted };
        return Admission::Refused(Refusal::new(code, "no credits are available for a new billed operation"));
    }
    Admission::Refused(Refusal::new(
        RefusalCode::AuthUnavailable,
        "local affordability cannot mint an execution grant; a server admission is required",
    ))
}
