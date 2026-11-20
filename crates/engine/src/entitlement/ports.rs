//! Ports the host implements, plus deterministic test doubles.

#[cfg(test)]
use std::collections::{BTreeMap, VecDeque};

#[cfg(test)]
use super::types::GrantId;
#[cfg(test)]
use super::types::RetryMapping;
use super::types::{
    AuthenticatedHostScope, Availability, BillingResult, Capability, CredentialError, EntryCategory, FreshIntent, Grant, OperationId,
    OperationRecord, OperationStatus, OperationStoreError, Refusal, RefusalCode, RetryDecision, RetryScopeKey, ReviewOperation, RunId, SaveOutcome,
    TransportCorrelation,
};

pub trait AccountService {
    fn status(&self) -> Result<Availability, Refusal>;
    fn authorize(&mut self, scope: &AuthenticatedHostScope, capability: Capability) -> Result<Grant, Refusal>;
    fn begin(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<Grant, Refusal>;
    fn finalize(&mut self, scope: &AuthenticatedHostScope, operation: &OperationId, result: BillingResult) -> Result<OperationStatus, Refusal>;
    fn operation(&self, scope: &AuthenticatedHostScope, operation: &OperationId) -> Result<OperationStatus, Refusal>;
}

/// The durable operation journal. It records identity and settlement state; it never grants
/// execution authority.
pub trait OperationStore {
    /// Create one row. A row is immutable: the same operation with a different account,
    /// capability or entry is a conflict, never an overwrite or merge.
    fn save_new(&mut self, record: &OperationRecord) -> Result<SaveOutcome, OperationStoreError>;
    /// The persisted row, or `None`. Never synthesised from an in-memory hint.
    fn load(&self, operation: &OperationId) -> Result<Option<OperationRecord>, OperationStoreError>;
    fn mark_started(&mut self, operation: &OperationId, started_at_ms: u64) -> Result<(), OperationStoreError>;
    fn mark_terminal(&mut self, operation: &OperationId, status: OperationStatus) -> Result<(), OperationStoreError>;
    /// Persist the retry mapping and the original row before the caller's first wire attempt.
    fn persist_intent(
        &mut self,
        scope: &AuthenticatedHostScope,
        correlation: &TransportCorrelation,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<OperationRecord, SaveOutcome>;
    /// Create a brand-new operation only from an explicit host [`FreshIntent`].
    ///
    /// An unprovable or unknown retry must never create a new operation implicitly; the host mints
    /// the id and passes it here after deciding that this is a fresh logical operation.
    fn persist_fresh_intent(&mut self, intent: &FreshIntent, capability: Capability, entry: EntryCategory) -> Result<OperationRecord, SaveOutcome>;
    /// Reverse lookup from the authenticated host scope plus the transport correlation. The caller
    /// does not supply an operation id; external ids alone can never recover or elevate a row.
    fn lookup_original_retry(&self, key: &RetryScopeKey) -> RetryDecision;
    fn record_review(&mut self, review: &ReviewOperation) -> Result<(), OperationStoreError>;
    fn review_operation(&self, run_id: &RunId) -> Result<Option<OperationId>, OperationStoreError>;
}

pub trait CredentialStore {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError>;
    fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError>;
    fn delete(&mut self, key: &str) -> Result<(), CredentialError>;
    fn persistent(&self) -> bool;
}

pub trait BillingPort {
    fn admit(&mut self, operation: &OperationId, capability: Capability, entry: EntryCategory) -> Result<Grant, Refusal>;
    fn commit(&mut self, operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal>;
    fn release(&mut self, operation: &OperationId, reason: RefusalCode) -> Result<(), Refusal>;
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct FakeAccountService {
    availability: Availability,
    grants: VecDeque<Grant>,
}

#[cfg(test)]
impl FakeAccountService {
    pub fn with_availability(availability: Availability) -> Self {
        Self { availability, grants: VecDeque::new() }
    }

    pub fn push_grant(&mut self, grant: Grant) {
        self.grants.push_back(grant);
    }

    fn pop_grant(&mut self, capability: Capability) -> Result<Grant, Refusal> {
        match self.grants.pop_front() {
            Some(grant) if grant.capability() == capability => Ok(grant),
            _ => Err(Refusal::new(RefusalCode::AuthUnavailable, "the scripted grant is absent")),
        }
    }
}

#[cfg(test)]
impl AccountService for FakeAccountService {
    fn status(&self) -> Result<Availability, Refusal> {
        Ok(self.availability.clone())
    }

    fn authorize(&mut self, _scope: &AuthenticatedHostScope, capability: Capability) -> Result<Grant, Refusal> {
        self.pop_grant(capability)
    }

    fn begin(
        &mut self,
        _scope: &AuthenticatedHostScope,
        _operation: &OperationId,
        capability: Capability,
        _entry: EntryCategory,
    ) -> Result<Grant, Refusal> {
        self.pop_grant(capability)
    }

    fn finalize(&mut self, _scope: &AuthenticatedHostScope, _operation: &OperationId, result: BillingResult) -> Result<OperationStatus, Refusal> {
        Ok(match result {
            BillingResult::Succeeded | BillingResult::PartialRetained => OperationStatus::Charged,
            BillingResult::FailedNoEffect => OperationStatus::Released,
            BillingResult::Unknown => OperationStatus::Pending,
        })
    }

    fn operation(&self, _scope: &AuthenticatedHostScope, _operation: &OperationId) -> Result<OperationStatus, Refusal> {
        Ok(OperationStatus::Held)
    }
}

/// In-memory identity and status ledger for tests. It never yields execution authority.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct InMemoryOperationStore {
    records: BTreeMap<OperationId, OperationRecord>,
    mappings: BTreeMap<RetryScopeKey, RetryMapping>,
    reviews: BTreeMap<RunId, OperationId>,
    next: u64,
    pub admissions: usize,
    pub commits: usize,
}

#[cfg(test)]
impl InMemoryOperationStore {
    fn mint(&mut self) -> OperationId {
        self.next = self.next.saturating_add(1);
        OperationId::from_counter(self.next)
    }
}

#[cfg(test)]
impl OperationStore for InMemoryOperationStore {
    fn save_new(&mut self, record: &OperationRecord) -> Result<SaveOutcome, OperationStoreError> {
        if record.status != OperationStatus::Held || record.grant_id.is_some() {
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
        if !scope.is_provable() {
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
        let operation = self.mint();
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
        self.commits = self.commits.saturating_add(1);
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
            let same = existing.account == *intent.scope().account() && existing.capability == capability && existing.entry == entry;
            return if same { Ok(existing.clone()) } else { Err(SaveOutcome::Conflict) };
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
        Ok(record)
    }

    fn lookup_original_retry(&self, key: &RetryScopeKey) -> RetryDecision {
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
        self.reviews.insert(review.run_id.clone(), review.operation_id.clone());
        record.review_run_id = Some(review.run_id.clone());
        Ok(())
    }

    fn review_operation(&self, run_id: &RunId) -> Result<Option<OperationId>, OperationStoreError> {
        Ok(self.reviews.get(run_id).cloned())
    }
}

/// A counting billing port for tests. Grants are fake fixtures, never production authority.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct CountingBilling {
    pub admits: usize,
    pub commits: usize,
    pub releases: usize,
}

#[cfg(test)]
impl BillingPort for CountingBilling {
    fn admit(&mut self, operation: &OperationId, capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
        self.admits += 1;
        Ok(Grant::test_fixture(GrantId::new(format!("grant_{}", operation.as_str())), capability))
    }

    fn commit(&mut self, _operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        self.commits += 1;
        Ok(result)
    }

    fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
        self.releases += 1;
        Ok(())
    }
}
