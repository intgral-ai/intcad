//! The capability budget table. It ships unverified and fails closed until a bound is measured.

use std::collections::BTreeMap;
use std::time::Duration;

use super::types::{Availability, Capability, EntryCategory, Refusal, RefusalCode, UpperBound};

/// Measured safe-stop bounds per capability and entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BudgetTable {
    bounds: BTreeMap<(Capability, EntryCategory), UpperBound>,
}

impl BudgetTable {
    /// Every controlled path starts unverified. No host may admit a controlled path until it
    /// replaces a bound with a measured value.
    pub fn unverified_default() -> Self {
        let mut bounds = BTreeMap::new();
        for capability in Capability::ALL {
            for entry in EntryCategory::CONTROLLED {
                bounds.insert((capability, entry), UpperBound::Unverified);
            }
        }
        Self { bounds }
    }

    pub fn set_bound(&mut self, capability: Capability, entry: EntryCategory, bound: UpperBound) {
        self.bounds.insert((capability, entry), bound);
    }

    /// Record a measured safe-stop bound. Callers must have evidence for the value.
    pub fn set_verified(&mut self, capability: Capability, entry: EntryCategory, bound: Duration) {
        self.set_bound(capability, entry, UpperBound::Verified(bound));
    }

    pub fn bound(&self, capability: Capability, entry: EntryCategory) -> UpperBound {
        self.bounds.get(&(capability, entry)).copied().unwrap_or(UpperBound::Unverified)
    }

    pub fn is_verified(&self, capability: Capability, entry: EntryCategory) -> bool {
        entry.is_free() || matches!(self.bound(capability, entry), UpperBound::Verified(_))
    }

    /// The safe-stop bound for one call, or a refusal.
    ///
    /// Free human entries (`HumanGuiFree`, `WebGuiFree`) return `Ok(remaining)` and consult neither
    /// the network nor this table. For a controlled entry, a verified bound is required:
    ///
    /// - `Ok(bound)` only when the bound is `Verified` and `remaining >= bound`;
    /// - `Err(UnsafeToStart)` for `Unknown`, an unverified path or a remaining time shorter than the
    ///   measured bound;
    /// - `Err(availability.reasons[0])`, `Err(AuthUnavailable)` or `Err(VersionUnsupported)` when the
    ///   account snapshot itself blocks the call.
    ///
    /// The caller must pass the actual remaining authorization, not a placeholder. `admit_billing`
    /// is only a local preflight and cannot authorize dispatch or commit.
    pub fn require(
        &self,
        capability: Capability,
        entry: EntryCategory,
        availability: &Availability,
        remaining: Duration,
    ) -> Result<Duration, Refusal> {
        if entry.is_free() {
            return Ok(remaining);
        }
        if entry == EntryCategory::Unknown {
            return Err(unsafe_to_start("the request entry is unclassified"));
        }
        if let Some(code) = availability.reasons.first() {
            return Err(Refusal::new(*code, "the account snapshot reports a refusal"));
        }
        if !availability.online {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the account is not online"));
        }
        if !availability.version_supported {
            return Err(Refusal::new(RefusalCode::VersionUnsupported, "the client version is not supported"));
        }
        match self.bound(capability, entry) {
            UpperBound::Unverified => Err(unsafe_to_start("this controlled path has no verified safe-stop bound")),
            UpperBound::Verified(bound) if remaining < bound => {
                Err(unsafe_to_start("the remaining authorization is shorter than the safe-stop bound"))
            }
            UpperBound::Verified(bound) => Ok(bound),
        }
    }
}

fn unsafe_to_start(message: &'static str) -> Refusal {
    Refusal::new(RefusalCode::UnsafeToStart, message)
}
