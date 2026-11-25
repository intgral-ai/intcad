//! Native billing for a synchronous, single-operation host. The ledger records identity and
//! intent, never execution authority. One admission pins the exact credential session.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use solidcraft_engine::entitlement::{
    AccountService, Admission, AuthenticatedHostScope, BillingPort, BillingResult, Capability, ConservativeDeadline, EntryCategory, FreshIntent,
    Grant, GrantId, MonotonicClock, MonotonicInstant, OperationId, OperationRecord, OperationStatus, OperationStore, Refusal, RefusalCode,
    RenewalAttempt, RetryDecision, RetryScopeKey, SaveOutcome, SettlementIntent, TransportCorrelation,
};

use crate::http::{refusal, unavailable};
use crate::{
    CredentialHandoff, CredentialSession, FileOperationStore, FixedCredentials, HttpAccountService, Secret, Transport, WireConnection,
    WireCredentials,
};

struct Binding {
    scope: AuthenticatedHostScope,
    generation: u64,
    bearer: Secret,
    installation: Secret,
}

impl Binding {
    fn current(session: &CredentialSession) -> Option<Self> {
        let generation = session.authority_generation();
        if !session.is_persistent() || !matches!(session.logout_pending(), Ok(false)) {
            return None;
        }
        let scope = session.authenticated_scope().filter(AuthenticatedHostScope::is_provable)?;
        let bearer = session.product_session()?;
        let installation = session.installation_credential()?;
        let binding = Self { scope, generation, bearer, installation };
        binding.unchanged(session).then_some(binding)
    }

    fn unchanged(&self, session: &CredentialSession) -> bool {
        session.authority_generation() == self.generation
            && session.authenticated_scope().as_ref() == Some(&self.scope)
            && session.product_session().as_ref() == Some(&self.bearer)
            && session.installation_credential().as_ref() == Some(&self.installation)
            && !session.is_revoked()
            && session.authority_generation() == self.generation
    }

    fn wire(&self) -> FixedCredentials {
        FixedCredentials::none().with_product_session(self.bearer.expose_secret()).with_installation_credential(self.installation.expose_secret())
    }
}

/// Keep the original settlement credential even if the session is withdrawn in flight.
/// Capture the proof in memory only; it is not a settlement credential or restart authority.
#[derive(Clone, Default)]
struct AdmissionSecrets(Rc<RefCell<Secrets>>);

#[derive(Default)]
struct Secrets {
    proof: Option<Secret>,
    settlement: Option<Secret>,
}

impl CredentialHandoff for AdmissionSecrets {
    fn admission_settlement_candidate(&mut self, _: &OperationId, credential: Secret) {
        self.0.borrow_mut().settlement = Some(credential);
    }
    fn initial_lease_proof(&mut self, _: &GrantId, _: Capability, _: Secret) -> Result<(), Refusal> {
        Err(unavailable())
    }
    fn execution_session_proof(&mut self, _: Capability, _: &OperationId, proof: Secret) -> Result<(), Refusal> {
        self.0.borrow_mut().proof = Some(proof);
        Ok(())
    }
    fn settlement_credential(&mut self, _: &OperationId, credential: Secret) -> Result<(), Refusal> {
        self.0.borrow_mut().settlement = Some(credential);
        Ok(())
    }
    fn installation_credential(&mut self, _: &str, _: Secret) -> Result<(), Refusal> {
        Err(unavailable())
    }
    fn first_installation_credential(&mut self, _: &Secret, _: &str, _: Secret) -> Result<(), Refusal> {
        Err(unavailable())
    }
}

struct Active {
    operation: OperationId,
    binding: Binding,
    grant: Grant,
    deadline: ConservativeDeadline,
    renewal: Option<MonotonicInstant>,
    settlement: Secret,
    proof: Secret,
    dispatched: bool,
    checked_commit: bool,
    withdrawn: Option<Refusal>,
    attempted: Cell<Option<BillingResult>>,
}

struct Cleanup {
    operation: OperationId,
    credential: Secret,
    attempted: Cell<Option<BillingResult>>,
}

#[derive(Clone)]
struct Clock(Rc<dyn MonotonicClock>);

impl MonotonicClock for Clock {
    fn now(&self) -> MonotonicInstant {
        self.0.now()
    }

    fn wall_now(&self) -> std::time::SystemTime {
        self.0.wall_now()
    }
}

/// Credential-backed CLI authority. It accepts neither a synthetic scope nor a grant from its
/// caller. The entry supplies its independently measured safe-stop bound at dispatch.
pub struct SessionBilling {
    session: CredentialSession,
    original: Option<Binding>,
    connection: WireConnection,
    transport: Box<dyn Transport>,
    journal: PathBuf,
    clock: Clock,
    active: Option<Active>,
    publish: Option<(OperationId, ConservativeDeadline)>,
    cleanup: Option<Cleanup>,
}

impl SessionBilling {
    pub fn new(
        session: CredentialSession,
        connection: WireConnection,
        transport: Box<dyn Transport>,
        journal: PathBuf,
        clock: impl MonotonicClock + 'static,
    ) -> Self {
        let original = Binding::current(&session);
        Self { session, original, connection, transport, journal, clock: Clock(Rc::new(clock)), active: None, publish: None, cleanup: None }
    }

    pub fn scope(&self) -> Option<AuthenticatedHostScope> {
        self.original.as_ref().filter(|binding| binding.unchanged(&self.session)).map(|binding| binding.scope.clone())
    }

    fn ledger(&self) -> Result<FileOperationStore, Refusal> {
        FileOperationStore::open(&self.journal).map_err(|_| unavailable())
    }

    fn service(&self, credentials: FixedCredentials, handoff: AdmissionSecrets) -> HttpAccountService<&dyn Transport> {
        HttpAccountService::with_transport(
            &*self.transport,
            self.connection.clone(),
            Box::new(credentials),
            Box::new(handoff),
            Box::new(self.clock.clone()),
        )
    }

    fn check(&self, active: &mut Active, bound: Duration) -> Result<(), Refusal> {
        if let Some(error) = &active.withdrawn {
            return Err(error.clone());
        }
        let check = (|| {
            let unsafe_start = || refusal(RefusalCode::UnsafeToStart);
            if !active.binding.unchanged(&self.session) {
                return Err(unavailable());
            }
            active.deadline.remaining(self.clock.now()).ok_or_else(unsafe_start)?;
            let mut service =
                self.service(active.binding.wire().with_execution_session_proof(active.proof.expose_secret()), AdmissionSecrets::default());
            service.controlled_status(&active.binding.scope, active.grant.capability())?;
            let now = self.clock.now();
            active.deadline.remaining(now).ok_or_else(unsafe_start)?;
            if !active.binding.unchanged(&self.session) {
                return Err(unavailable());
            }
            if active.renewal.is_some_and(|due| now >= due) {
                let (grant, sent_at, interval) = service.exchange_renewal(
                    &active.binding.scope,
                    &active.operation,
                    active.grant.id(),
                    active.grant.capability(),
                    Some(&active.deadline),
                )?;
                let received_at = self.clock.now();
                let attempt = RenewalAttempt { sent_at, received_at, old_deadline: active.deadline.clone(), still_valid: true };
                let duration = grant.expires_at().as_duration().saturating_sub(sent_at.as_duration());
                active.deadline = active.deadline.renewed(&attempt, duration).map_err(|_| unsafe_start())?;
                active.renewal = interval.map(|interval| sent_at.saturating_add(interval));
            }
            if !active.binding.unchanged(&self.session) {
                return Err(unavailable());
            }
            let remaining = active.deadline.remaining(self.clock.now()).ok_or_else(unsafe_start)?;
            if remaining < bound {
                return Err(unsafe_start());
            }
            Ok(())
        })();
        if let Err(error) = &check {
            active.withdrawn = Some(error.clone());
        }
        check
    }

    fn check_before_settlement(&mut self, operation: &OperationId) -> Result<(), Refusal> {
        let active = self.active.as_ref().ok_or_else(unavailable)?;
        if !active.checked_commit {
            return self.check_commit(operation);
        }
        if !active.binding.unchanged(&self.session) {
            return Err(unavailable());
        }
        active.deadline.remaining(self.clock.now()).map(|_| ()).ok_or_else(|| refusal(RefusalCode::UnsafeToStart))
    }

    fn settle(
        &self,
        operation: &OperationId,
        scope: &AuthenticatedHostScope,
        credential: &Secret,
        intent: SettlementIntent,
        attempted: Option<&Cell<Option<BillingResult>>>,
    ) -> Result<BillingResult, Refusal> {
        let mut ledger = self.ledger()?;
        let result = match intent {
            SettlementIntent::Commit(result) => result,
            SettlementIntent::Release(_) => BillingResult::FailedNoEffect,
        };
        let matches_intent = |recorded| match (recorded, intent) {
            (Some(SettlementIntent::Release(_)), SettlementIntent::Release(_)) => true,
            (recorded, intent) => recorded == Some(intent),
        };
        let inspect = |ledger: &FileOperationStore| {
            let row = ledger.load(operation).map_err(|_| unavailable())?.ok_or_else(unavailable)?;
            if row.account != *scope.account() {
                return Err(unavailable());
            }
            let existing = ledger.settlement_intent(operation).map_err(|_| unavailable())?;
            if (existing.is_some() || row.status.is_terminal()) && !matches_intent(existing) {
                return Err(refusal(RefusalCode::OperationFinalized));
            }
            if row.status.is_terminal()
                && !matches!(
                    (row.status, result),
                    (OperationStatus::Charged, BillingResult::Succeeded | BillingResult::PartialRetained)
                        | (OperationStatus::Released, BillingResult::FailedNoEffect)
                )
            {
                return Err(refusal(RefusalCode::OperationFinalized));
            }
            Ok((row.status, existing))
        };
        // An immutable terminal receipt is safe to return even if another process
        // currently holds the retry lock. Re-read nonterminal rows after taking it.
        if inspect(&ledger)?.0.is_terminal() {
            return Ok(result);
        }
        let _settlement_lock = ledger.try_lock_settlement_recovery(scope).map_err(|_| unavailable())?.ok_or_else(unavailable)?;
        let (status, existing) = inspect(&ledger)?;
        if status.is_terminal() {
            return Ok(result);
        }
        let mut service = self.service(FixedCredentials::none().with_settlement_credential(credential.expose_secret()), AdmissionSecrets::default());
        let status = if existing.is_some() {
            // An explicit same-id retry checks the original receipt first. Only a positive
            // live, nonterminal state with no recorded result permits resending the immutable
            // intent. A missing/expired state never triggers another POST.
            let (status, received) = service.operation_receipt(operation)?;
            match (status, received) {
                (_, Some(received)) if received == result => status,
                (OperationStatus::Held | OperationStatus::Pending, None) => {
                    if let Some(attempted) = attempted {
                        attempted.set(Some(result));
                    }
                    service.finalize(scope, operation, result)?
                }
                _ => return Err(refusal(RefusalCode::OperationFinalized)),
            }
        } else {
            if ledger.record_settlement_intent(operation, intent).map_err(|_| unavailable())? != SaveOutcome::Saved {
                return Err(unavailable());
            }
            if let Some(attempted) = attempted {
                attempted.set(Some(result));
            }
            service.finalize(scope, operation, result)?
        };
        if status.is_terminal() {
            match ledger.mark_terminal(operation, status) {
                Ok(()) | Err(solidcraft_engine::entitlement::OperationStoreError::TerminalState) => {}
                Err(_) => return Err(unavailable()),
            }
        }
        Ok(result)
    }

    fn settle_active(&mut self, operation: &OperationId, intent: SettlementIntent) -> Result<BillingResult, Refusal> {
        if let Some(active) = self.active.as_ref().filter(|active| &active.operation == operation) {
            let result = self.settle(operation, &active.binding.scope, &active.settlement, intent, Some(&active.attempted));
            if let Ok(received) = result {
                if active.dispatched {
                    self.publish = Some((operation.clone(), active.deadline.clone()));
                }
                self.active = None;
                return Ok(received);
            }
            // Do not drop the original credential on a local journal or wire failure.
            return result;
        }
        // Recovery can only read settlement credentials belonging to the current account and
        // installation. No retry path reloads a grant or dispatches work.
        let scope = self.scope().ok_or_else(unavailable)?;
        let credential = self.session.settlement_credential(operation).ok_or_else(unavailable)?;
        self.settle(operation, &scope, &credential, intent, None)
    }

    /// A release has no publication effect. If the final intent cannot be written after a
    /// fresh admission, its pre-wire cleanup record still authorizes recovery. Attempt the
    /// original release now with the captured credential rather than strand the hold.
    fn emergency_release(&self, operation: &OperationId, scope: &AuthenticatedHostScope, credential: &Secret) -> Result<(), Refusal> {
        let mut service = self.service(FixedCredentials::none().with_settlement_credential(credential.expose_secret()), AdmissionSecrets::default());
        let result = service.finalize(scope, operation, BillingResult::FailedNoEffect)?;
        if let Ok(mut ledger) = self.ledger()
            && (ledger.record_settlement_intent(operation, SettlementIntent::Release(RefusalCode::AuthUnavailable)).is_ok()
                || ledger.settlement_intent(operation).ok().flatten().is_some_and(|intent| matches!(intent, SettlementIntent::Release(_))))
        {
            let _ = ledger.mark_terminal(operation, result);
        }
        Ok(())
    }

    fn recover_release(&mut self, operation: &OperationId, reason: RefusalCode) -> Result<(), Refusal> {
        let binding = self.original.as_ref().ok_or_else(unavailable)?;
        let scope = &binding.scope;
        if let Some(cleanup) = self.cleanup.as_ref().filter(|cleanup| &cleanup.operation == operation) {
            let result = self.settle(operation, scope, &cleanup.credential, SettlementIntent::Release(reason), Some(&cleanup.attempted));
            if let Err(error) = result {
                if error.code != RefusalCode::AuthUnavailable || cleanup.attempted.get().is_some() {
                    return Err(error);
                }
                cleanup.attempted.set(Some(BillingResult::FailedNoEffect));
                self.emergency_release(operation, scope, &cleanup.credential)?;
            }
            self.cleanup = None;
            return Ok(());
        }
        let mut ledger = self.ledger()?;
        if !ledger.has_admission_cleanup(operation) || !binding.unchanged(&self.session) {
            return Err(unavailable());
        }
        if let Some(credential) = self.session.settlement_credential(operation) {
            return self.settle(operation, scope, &credential, SettlementIntent::Release(reason), None).map(|_| ());
        }
        let retry = RetryScopeKey { host_scope: scope.clone(), transport_correlation: TransportCorrelation::new(operation.as_str()) };
        let RetryDecision::Original(row) = ledger.lookup_original_retry(&retry) else {
            return Err(unavailable());
        };
        if &row.operation != operation {
            return Err(unavailable());
        }
        let lock = ledger.try_lock_settlement_recovery(scope).map_err(|_| unavailable())?.ok_or_else(unavailable)?;
        match ledger.settlement_intent(operation).map_err(|_| unavailable())? {
            None => {
                if ledger.record_settlement_intent(operation, SettlementIntent::Release(reason)).map_err(|_| unavailable())? != SaveOutcome::Saved {
                    return Err(unavailable());
                }
            }
            Some(SettlementIntent::Release(_)) => {}
            _ => return Err(refusal(RefusalCode::OperationFinalized)),
        }
        // No credential survived the first reply. Use the frozen same-id begin to recover
        // only a settlement credential. Even a first successful begin here will never run.
        let handoff = AdmissionSecrets::default();
        let response = self.service(binding.wire(), handoff.clone()).begin(scope, operation, row.capability, row.entry);
        if let Ok(Admission::AlreadyExists { status: status @ (OperationStatus::Released | OperationStatus::Expired) }) = response {
            // The same-installation server response already proves there is no live hold.
            ledger.mark_terminal(operation, status).map_err(|_| unavailable())?;
            return Ok(());
        }
        let credential = handoff.0.borrow_mut().settlement.take().ok_or_else(unavailable)?;
        let _ = self.session.persist_bound_settlement(scope, binding.generation, operation, &credential);
        self.cleanup = Some(Cleanup { operation: operation.clone(), credential, attempted: Cell::new(None) });
        drop(lock);
        self.recover_release(operation, reason)
    }
}

impl BillingPort for SessionBilling {
    fn preflight(&self) -> Result<(), Refusal> {
        self.scope().map(|_| ()).ok_or_else(unavailable)
    }

    fn checks_live_window(&self) -> bool {
        true
    }

    fn admit(&mut self, operation: &OperationId, capability: Capability, entry: EntryCategory) -> Result<Grant, Refusal> {
        if !matches!((capability, entry), (Capability::Mcp, EntryCategory::HeadlessMcp) | (Capability::Dfm, EntryCategory::CliDfm)) {
            return Err(refusal(RefusalCode::CapabilityDenied));
        }
        self.publish = None;
        if self.active.is_some() || self.cleanup.is_some() {
            return Err(refusal(RefusalCode::OperationConflict));
        }
        let binding = Binding::current(&self.session).ok_or_else(unavailable)?;
        if self.original.as_ref().is_none_or(|original| !original.unchanged(&self.session) || original.scope != binding.scope) {
            return Err(unavailable());
        }
        let handoff = AdmissionSecrets::default();
        let mut service = self.service(binding.wire(), handoff.clone());
        // A restart can restore local labels, but only the configured product service can
        // confirm that the bearer is currently eligible. Do this before writing an intent.
        service.controlled_status(&binding.scope, capability)?;
        if !binding.unchanged(&self.session) {
            return Err(unavailable());
        }
        let mut ledger = self.ledger()?;
        let row =
            OperationRecord::held(operation.clone(), binding.scope.account().clone(), capability, entry, solidcraft_engine::validation::now_ms());
        let saved = ledger.save_new(&row).map_err(|_| unavailable())?;
        if saved != SaveOutcome::Saved {
            if saved == SaveOutcome::Existing
                && ledger.has_admission_cleanup(operation)
                && ledger.load(operation).ok().flatten().is_some_and(|row| !row.status.is_terminal())
                && matches!(ledger.settlement_intent(operation), Ok(None | Some(SettlementIntent::Release(_))))
            {
                let _ = self.recover_release(operation, RefusalCode::AuthUnavailable);
            }
            return Err(refusal(RefusalCode::OperationConflict));
        }
        let fresh = FreshIntent::new(operation.clone(), binding.scope.clone(), TransportCorrelation::new(operation.as_str()));
        ledger.persist_fresh_intent(&fresh, capability, entry).map_err(|_| unavailable())?;
        ledger.record_admission_cleanup(operation).map_err(|_| unavailable())?;
        if !binding.unchanged(&self.session) {
            return Err(unavailable());
        }
        let sent_at = self.clock.now();
        let admission = service.begin(&binding.scope, operation, capability, entry);
        let Secrets { proof, settlement } = std::mem::take(&mut *handoff.0.borrow_mut());
        let grant = match admission {
            Ok(Admission::NewlyAdmitted { operation: original, grant }) if &original == operation => grant,
            Ok(Admission::Refused(error)) => return Err(error),
            Ok(_) => return Err(refusal(RefusalCode::OperationConflict)),
            Err(error) => {
                if let Some(credential) = settlement {
                    let _ = self.session.persist_bound_settlement(&binding.scope, binding.generation, operation, &credential);
                    self.cleanup = Some(Cleanup { operation: operation.clone(), credential, attempted: Cell::new(None) });
                }
                let _ = self.recover_release(operation, error.code);
                return Err(error);
            }
        };
        let settlement = settlement.ok_or_else(unavailable)?;
        let proof = proof.ok_or_else(unavailable)?;
        let (deadline, renewal) = match grant.renewal_schedule(&binding.scope, operation) {
            Some((deadline, due)) => (deadline.clone(), Some(due)),
            None => (ConservativeDeadline::new(sent_at, grant.expires_at().as_duration().saturating_sub(sent_at.as_duration())), None),
        };
        let usable = self.session.persist_bound_settlement(&binding.scope, binding.generation, operation, &settlement).and_then(|()| {
            if !binding.unchanged(&self.session) {
                return Err(unavailable());
            }
            deadline.remaining(self.clock.now()).map(|_| ()).ok_or_else(|| refusal(RefusalCode::UnsafeToStart))
        });
        let active = Active {
            operation: operation.clone(),
            binding,
            deadline,
            grant: grant.clone(),
            renewal,
            settlement,
            proof,
            dispatched: false,
            checked_commit: false,
            withdrawn: None,
            attempted: Cell::new(None),
        };
        self.active = Some(active);
        if let Err(error) = usable {
            self.release(operation, error.code)?;
            return Err(error);
        }
        Ok(grant)
    }

    fn dispatch(&mut self, operation: &OperationId, bound: Duration) -> Result<(), Refusal> {
        let mut active = self.active.take().ok_or_else(unavailable)?;
        let result = (|| {
            if &active.operation != operation || active.dispatched {
                return Err(refusal(RefusalCode::OperationConflict));
            }
            self.ledger()?.mark_started(operation, solidcraft_engine::validation::now_ms()).map_err(|_| unavailable())?;
            self.check(&mut active, bound)?;
            active.dispatched = true;
            Ok(())
        })();
        self.active = Some(active);
        result
    }

    fn check_commit(&mut self, operation: &OperationId) -> Result<(), Refusal> {
        let mut active = self.active.take().ok_or_else(unavailable)?;
        let result = if &active.operation == operation && active.dispatched {
            self.check(&mut active, Duration::ZERO)
        } else {
            Err(refusal(RefusalCode::OperationConflict))
        };
        active.checked_commit = result.is_ok();
        self.active = Some(active);
        result
    }

    fn check_publish(&mut self, operation: &OperationId) -> Result<(), Refusal> {
        let (original, deadline) = self.publish.take().ok_or_else(unavailable)?;
        if &original != operation || self.scope().is_none() {
            return Err(unavailable());
        }
        deadline.remaining(self.clock.now()).map(|_| ()).ok_or_else(|| refusal(RefusalCode::UnsafeToStart))
    }

    fn commit(&mut self, operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        if let Some(active) = self.active.as_ref().filter(|active| &active.operation == operation)
            && let Some(attempted) = active.attempted.get()
        {
            // Recover only the already selected result. This is a receipt retry, not a new
            // authorization decision: check_publish still uses the original deadline.
            return if attempted == result {
                self.settle_active(operation, SettlementIntent::Commit(result))
            } else {
                Err(refusal(RefusalCode::OperationFinalized))
            };
        }
        if self.active.as_ref().is_some_and(|active| &active.operation == operation)
            && let Err(error) = self.check_before_settlement(operation)
        {
            if result == BillingResult::Unknown {
                self.settle_active(operation, SettlementIntent::Commit(BillingResult::Unknown))?;
            } else {
                self.release(operation, error.code)?;
            }
            return Err(error);
        }
        // This also handles a same-id receipt lookup, but never creates a new intent except for
        // the active admission.
        if self.active.as_ref().is_none_or(|active| &active.operation != operation)
            && self.ledger()?.settlement_intent(operation).map_err(|_| unavailable())?.is_none()
        {
            return Err(unavailable());
        }
        let settlement = self.settle_active(operation, SettlementIntent::Commit(result));
        if matches!(&settlement, Err(error) if error.code == RefusalCode::AuthUnavailable)
            && self.active.as_ref().is_some_and(|active| &active.operation == operation && active.attempted.get().is_none())
        {
            let _ = self.release(operation, RefusalCode::AuthUnavailable);
        }
        settlement
    }

    fn release(&mut self, operation: &OperationId, reason: RefusalCode) -> Result<(), Refusal> {
        if self.active.as_ref().is_none_or(|active| &active.operation != operation) {
            return self.recover_release(operation, reason);
        }
        let result = self.settle_active(operation, SettlementIntent::Release(reason));
        if let Err(error) = result {
            let active = self.active.as_ref().ok_or_else(unavailable)?;
            if error.code == RefusalCode::AuthUnavailable && active.attempted.get().is_none() {
                active.attempted.set(Some(BillingResult::FailedNoEffect));
                self.emergency_release(operation, &active.binding.scope, &active.settlement)?;
                self.active = None;
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }
}
