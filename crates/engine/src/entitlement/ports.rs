//! Ports the host implements, plus deterministic test doubles.

use std::collections::{BTreeMap, VecDeque};

use super::types::{
    AuthenticatedHostScope, Availability, BillingResult, Capability, CredentialError, EntryCategory, Grant, GrantId, OperationId, OperationStatus,
    OperationStoreError, Refusal, RefusalCode, RetryScopeKey, SaveOutcome, TransportCorrelation,
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

pub trait OperationStore {
    fn load_status(&self, operation: &OperationId) -> Option<OperationStatus>;
    fn save_new(&mut self, operation: &OperationId, capability: Capability, entry: EntryCategory) -> SaveOutcome;
    fn mark_started(&mut self, operation: &OperationId) -> Result<(), OperationStoreError>;
    fn mark_terminal(&mut self, operation: &OperationId, status: OperationStatus) -> Result<(), OperationStoreError>;
    fn persist_intent(
        &mut self,
        scope: &AuthenticatedHostScope,
        correlation: &TransportCorrelation,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<OperationId, SaveOutcome>;
    fn lookup_original_retry(&self, key: &RetryScopeKey) -> Option<OperationId>;
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

#[derive(Debug, Default)]
pub struct FakeAccountService {
    availability: Availability,
    grants: VecDeque<Grant>,
}

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
#[derive(Debug, Default)]
pub struct InMemoryOperationStore {
    records: BTreeMap<OperationId, (Capability, EntryCategory, OperationStatus)>,
    mappings: BTreeMap<RetryScopeKey, OperationId>,
    next: u64,
    pub admissions: usize,
    pub commits: usize,
}

impl OperationStore for InMemoryOperationStore {
    fn load_status(&self, operation: &OperationId) -> Option<OperationStatus> {
        self.records.get(operation).map(|record| record.2)
    }

    fn save_new(&mut self, operation: &OperationId, capability: Capability, entry: EntryCategory) -> SaveOutcome {
        if let Some(existing) = self.records.get(operation) {
            return if existing.0 == capability && existing.1 == entry { SaveOutcome::Existing } else { SaveOutcome::Conflict };
        }
        self.records.insert(operation.clone(), (capability, entry, OperationStatus::Held));
        self.admissions += 1;
        SaveOutcome::Saved
    }

    fn mark_started(&mut self, operation: &OperationId) -> Result<(), OperationStoreError> {
        let record = self.records.get_mut(operation).ok_or(OperationStoreError::UnknownOperation)?;
        if record.2.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        record.2 = OperationStatus::Pending;
        Ok(())
    }

    fn mark_terminal(&mut self, operation: &OperationId, status: OperationStatus) -> Result<(), OperationStoreError> {
        if !status.is_terminal() {
            return Err(OperationStoreError::InvalidState);
        }
        let record = self.records.get_mut(operation).ok_or(OperationStoreError::UnknownOperation)?;
        if record.2.is_terminal() {
            return Err(OperationStoreError::TerminalState);
        }
        record.2 = status;
        if status == OperationStatus::Charged {
            self.commits += 1;
        }
        Ok(())
    }

    fn persist_intent(
        &mut self,
        scope: &AuthenticatedHostScope,
        correlation: &TransportCorrelation,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<OperationId, SaveOutcome> {
        let key = RetryScopeKey { host_scope: scope.clone(), transport_correlation: correlation.clone() };
        if let Some(operation) = self.mappings.get(&key) {
            let existing = self.records.get(operation).ok_or(SaveOutcome::Conflict)?;
            return if existing.0 == capability && existing.1 == entry { Ok(operation.clone()) } else { Err(SaveOutcome::Conflict) };
        }
        self.next = self.next.saturating_add(1);
        let operation = OperationId::from_counter(self.next);
        self.records.insert(operation.clone(), (capability, entry, OperationStatus::Held));
        self.mappings.insert(key, operation.clone());
        self.admissions += 1;
        Ok(operation)
    }

    fn lookup_original_retry(&self, key: &RetryScopeKey) -> Option<OperationId> {
        self.mappings.get(key).cloned()
    }
}

/// A counting billing port for tests. Grants are fake fixtures, never production authority.
#[derive(Debug, Default)]
pub struct CountingBilling {
    pub admits: usize,
    pub commits: usize,
    pub releases: usize,
}

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
