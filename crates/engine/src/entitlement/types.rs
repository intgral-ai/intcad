//! Value types shared by entitlement admission, billing and persistence ports.

use std::time::Duration;

use super::clock::MonotonicInstant;

/// A billed Intcad capability, exactly as the frozen rev3 contract names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    Chat,
    Harness,
    Review,
    Dfm,
    Mcp,
    Control,
    Autocomplete,
    DesignIntent,
}

impl Capability {
    pub const ALL: [Self; 8] = [Self::Chat, Self::Harness, Self::Review, Self::Dfm, Self::Mcp, Self::Control, Self::Autocomplete, Self::DesignIntent];

    /// The wire value from the frozen rev3 OpenAPI document.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Harness => "harness",
            Self::Review => "review",
            Self::Dfm => "dfm",
            Self::Mcp => "mcp",
            Self::Control => "control",
            Self::Autocomplete => "autocomplete",
            Self::DesignIntent => "design_intent",
        }
    }

    /// The capability named by a persisted wire value, for ledger reload only.
    pub fn from_wire(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|capability| capability.wire() == raw)
    }
}

/// Wire error codes. Task 3 owns their serialization; the business meaning is fixed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorCode {
    InvalidRequest,
    AuthRequired,
    AccountDisabled,
    EmailUnverified,
    CapabilityDenied,
    VersionUnsupported,
    ProtocolUnsupported,
    UnsafeToStart,
    CreditsExhausted,
    CreditsHeld,
    OperationConflict,
    OperationFinalized,
    OperationOwnedElsewhere,
    AuthUnavailable,
    InternalError,
}

/// A refusal the host or caller can act on. Transport failures are not business refusals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefusalCode {
    AuthRequired,
    EmailUnverified,
    AccountDisabled,
    CapabilityDenied,
    VersionUnsupported,
    ProtocolUnsupported,
    UnsafeToStart,
    CreditsExhausted,
    CreditsHeld,
    OperationConflict,
    OperationFinalized,
    OperationOwnedElsewhere,
    AuthUnavailable,
}

/// `INVALID_REQUEST` and `INTERNAL_ERROR` describe a broken exchange, not a business refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportError {
    InvalidRequest,
    InternalError,
}

/// A stable refusal code and a human-readable explanation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
}

impl Refusal {
    pub fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    /// Classify a wire error. Transport failures stay outside the business refusal set.
    pub fn from_wire(code: ErrorCode) -> Result<RefusalCode, TransportError> {
        match code {
            ErrorCode::InvalidRequest => Err(TransportError::InvalidRequest),
            ErrorCode::InternalError => Err(TransportError::InternalError),
            ErrorCode::AuthRequired => Ok(RefusalCode::AuthRequired),
            ErrorCode::AccountDisabled => Ok(RefusalCode::AccountDisabled),
            ErrorCode::EmailUnverified => Ok(RefusalCode::EmailUnverified),
            ErrorCode::CapabilityDenied => Ok(RefusalCode::CapabilityDenied),
            ErrorCode::VersionUnsupported => Ok(RefusalCode::VersionUnsupported),
            ErrorCode::ProtocolUnsupported => Ok(RefusalCode::ProtocolUnsupported),
            ErrorCode::UnsafeToStart => Ok(RefusalCode::UnsafeToStart),
            ErrorCode::CreditsExhausted => Ok(RefusalCode::CreditsExhausted),
            ErrorCode::CreditsHeld => Ok(RefusalCode::CreditsHeld),
            ErrorCode::OperationConflict => Ok(RefusalCode::OperationConflict),
            ErrorCode::OperationFinalized => Ok(RefusalCode::OperationFinalized),
            ErrorCode::OperationOwnedElsewhere => Ok(RefusalCode::OperationOwnedElsewhere),
            ErrorCode::AuthUnavailable => Ok(RefusalCode::AuthUnavailable),
        }
    }
}

/// Where a request entered Intcad. Free human entries never reach the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryCategory {
    HumanGuiFree,
    WebGuiFree,
    Mastra,
    ExternalAcpMcp,
    HeadlessMcp,
    GuiBridgeMcp,
    DirectControl,
    CliDfm,
    ReviewPanel,
    ExternalUiAutomation,
    Unknown,
}

impl EntryCategory {
    pub const CONTROLLED: [Self; 9] = [
        Self::Mastra,
        Self::ExternalAcpMcp,
        Self::HeadlessMcp,
        Self::GuiBridgeMcp,
        Self::DirectControl,
        Self::CliDfm,
        Self::ReviewPanel,
        Self::ExternalUiAutomation,
        Self::Unknown,
    ];

    pub fn is_free(self) -> bool {
        matches!(self, Self::HumanGuiFree | Self::WebGuiFree)
    }

    /// The wire category, or `None` for a free human entry that must not be sent.
    pub fn wire_entry_category(self) -> Option<WireEntryCategory> {
        match self {
            Self::HumanGuiFree | Self::WebGuiFree => None,
            Self::Mastra => Some(WireEntryCategory::Mastra),
            Self::ExternalAcpMcp => Some(WireEntryCategory::ExternalAcpMcp),
            Self::HeadlessMcp => Some(WireEntryCategory::HeadlessMcp),
            Self::GuiBridgeMcp => Some(WireEntryCategory::GuiBridgeMcp),
            Self::DirectControl => Some(WireEntryCategory::DirectControl),
            Self::CliDfm => Some(WireEntryCategory::CliDfm),
            Self::ReviewPanel => Some(WireEntryCategory::ReviewPanel),
            Self::ExternalUiAutomation => Some(WireEntryCategory::ExternalUiAutomation),
            Self::Unknown => Some(WireEntryCategory::Unknown),
        }
    }
}

/// The wire-entry subset of [`EntryCategory`] from the frozen rev3 contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WireEntryCategory {
    Mastra,
    ExternalAcpMcp,
    HeadlessMcp,
    GuiBridgeMcp,
    DirectControl,
    CliDfm,
    ReviewPanel,
    ExternalUiAutomation,
    Unknown,
}

/// A malformed operation id. The host must mint a valid one; callers never get to pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationIdError {
    BadPrefix,
    BadLength,
    BadCharacter,
}

/// A host-minted operation identity in the frozen rev3 grammar `^op_[A-Za-z0-9_-]{8,64}$`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(String);

impl OperationId {
    pub fn parse(raw: &str) -> Result<Self, OperationIdError> {
        let suffix = raw.strip_prefix("op_").ok_or(OperationIdError::BadPrefix)?;
        if !(8..=64).contains(&suffix.len()) {
            return Err(OperationIdError::BadLength);
        }
        if !suffix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err(OperationIdError::BadCharacter);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Mint from host entropy: 16..=32 base62 characters after `op_`.
    pub fn mint_from_entropy(entropy: &[u8]) -> Result<Self, OperationIdError> {
        if entropy.len() < 16 {
            return Err(OperationIdError::BadLength);
        }
        const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let len = entropy.len().min(32);
        let mut id = String::with_capacity(3 + len);
        id.push_str("op_");
        for byte in &entropy[..len] {
            id.push(BASE62[usize::from(*byte % 62)] as char);
        }
        Ok(Self(id))
    }

    /// Deterministic ids for in-memory test doubles only.
    #[cfg(test)]
    pub(crate) fn from_counter(counter: u64) -> Self {
        Self(format!("op_test_{counter:08}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(AccountId);
string_id!(InstallationId);
string_id!(HostSessionBinding);
string_id!(GrantId);
string_id!(TransportCorrelation);
string_id!(RunId);

/// The host's stable authenticated identity. Constructed only by the host authentication path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthenticatedHostScope {
    account: AccountId,
    installation: InstallationId,
    session_binding: HostSessionBinding,
}

impl AuthenticatedHostScope {
    /// Host authentication paths are the only permitted callers.
    pub fn new(account: AccountId, installation: InstallationId, session_binding: HostSessionBinding) -> Self {
        Self { account, installation, session_binding }
    }

    /// Test-only fixture. Never use in a production authentication path.
    #[cfg(test)]
    #[doc(hidden)]
    pub fn test_fixture(account: &str, installation: &str) -> Self {
        Self::new(AccountId::new(account), InstallationId::new(installation), HostSessionBinding::new("test_session_binding"))
    }

    pub fn account(&self) -> &AccountId {
        &self.account
    }

    pub fn installation(&self) -> &InstallationId {
        &self.installation
    }

    pub fn session_binding(&self) -> &HostSessionBinding {
        &self.session_binding
    }

    /// Whether the host can prove a stable account, installation and session binding.
    ///
    /// An empty part means the transport could not establish a stable authenticated session; callers
    /// must fail closed instead of starting a controlled operation.
    pub fn is_provable(&self) -> bool {
        !self.account.as_str().is_empty() && !self.installation.as_str().is_empty() && !self.session_binding.as_str().is_empty()
    }
}

/// Account and capability availability from one snapshot. A zero balance does not block chat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Availability {
    pub online: bool,
    pub version_supported: bool,
    pub model_available: bool,
    pub balance: u64,
    pub held: u64,
    pub reasons: Vec<RefusalCode>,
}

impl Availability {
    pub fn available(&self) -> u64 {
        self.balance.saturating_sub(self.held)
    }
}

/// The measured safe-stop bound for a capability and entry, or a fail-closed default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpperBound {
    Verified(Duration),
    Unverified,
}

/// The result of asking to run one logical operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    Allowed,
    NewlyAdmitted { operation: OperationId, grant: Grant },
    AlreadyExists { status: OperationStatus },
    Refused(Refusal),
}

impl Admission {
    pub fn refusal_code(&self) -> Option<RefusalCode> {
        match self {
            Self::Refused(refusal) => Some(refusal.code),
            Self::Allowed | Self::NewlyAdmitted { .. } | Self::AlreadyExists { .. } => None,
        }
    }
}

/// A server-issued execution grant. Local affordability alone can never create one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    id: GrantId,
    capability: Capability,
    expires_at: MonotonicInstant,
}

impl Grant {
    /// Account adapters call this only with a grant received from the server.
    pub fn from_server(id: GrantId, capability: Capability, expires_at: MonotonicInstant) -> Self {
        Self { id, capability, expires_at }
    }

    /// Test-only grant. Never treat this as production authority.
    #[cfg(test)]
    #[doc(hidden)]
    pub fn test_fixture(id: GrantId, capability: Capability) -> Self {
        Self { id, capability, expires_at: MonotonicInstant::from_millis(60_000) }
    }

    pub fn capability(&self) -> Capability {
        self.capability
    }
}

/// Ledger state names from the frozen rev3 contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationStatus {
    Held,
    Pending,
    Charged,
    Released,
    Expired,
}

impl OperationStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Charged | Self::Released | Self::Expired)
    }
}

/// Billing outcome names from the frozen rev3 contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BillingResult {
    Succeeded,
    PartialRetained,
    FailedNoEffect,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    Existing,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationStoreError {
    UnknownOperation,
    TerminalState,
    InvalidState,
    /// The host could not read or write its durable ledger. Callers must fail closed.
    Unavailable,
}

/// Why a credential adapter cannot return, store or delete a secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialErrorCode {
    NoPersistentCredential,
    Unavailable,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialError {
    pub code: CredentialErrorCode,
    pub message: String,
}

/// One durable host-owned ledger row.
///
/// Fields are readable snapshots, not execution authority. A row never carries a credential; a
/// server-issued [`GrantId`] is recorded only after the server admitted the operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationRecord {
    pub operation: OperationId,
    pub account: AccountId,
    pub capability: Capability,
    pub entry: EntryCategory,
    pub status: OperationStatus,
    pub started_at_ms: u64,
    pub deadline_ms: Option<u64>,
    pub grant_id: Option<GrantId>,
    pub review_run_id: Option<RunId>,
}

impl OperationRecord {
    /// A not-yet-started row. Only the host's mint path supplies the operation id and account.
    pub fn held(operation: OperationId, account: AccountId, capability: Capability, entry: EntryCategory, started_at_ms: u64) -> Self {
        Self {
            operation,
            account,
            capability,
            entry,
            status: OperationStatus::Held,
            started_at_ms,
            deadline_ms: None,
            grant_id: None,
            review_run_id: None,
        }
    }

    /// Whether two rows name the same logical operation identity. Other fields are settlement
    /// state, not identity, so a retry cannot overwrite them.
    pub fn common_identity_matches(&self, other: &Self) -> bool {
        self.account == other.account && self.capability == other.capability && self.entry == other.entry
    }
}

/// A persisted run-to-operation identity. Task 7/10 reuse this one type; they must not define a
/// second mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewOperation {
    pub run_id: RunId,
    pub operation_id: OperationId,
}

impl ReviewOperation {
    pub fn new(run_id: RunId, operation_id: OperationId) -> Self {
        Self { run_id, operation_id }
    }
}

/// The persisted reverse mapping for one retried transport request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryMapping {
    pub scope: RetryScopeKey,
    pub operation: OperationId,
    pub capability: Capability,
    pub entry: EntryCategory,
    pub created_at_ms: u64,
}

/// The answer to a retry lookup. Only [`RetryDecision::Original`] names the original operation;
/// no variant grants execution, renewal or submission authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// The same authenticated scope and correlation map to a non-terminal original row.
    Original(OperationRecord),
    /// The mapping exists but names a different capability, entry or identity. Fail closed.
    Conflict,
    /// The host cannot prove that the retry belongs to the same authenticated session. Fail closed.
    Unprovable,
    /// The mapping is present but its row is missing, unreadable or already terminal. Fail closed.
    Unknown,
}

/// A brand-new logical operation the host explicitly decided to mint.
///
/// It is never inferred from an unprovable retry and it has no `Deserialize` implementation:
/// external request ids, tool-call ids, ACP message ids and run ids are data, not authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshIntent {
    host_minted_operation: OperationId,
    scope: AuthenticatedHostScope,
    transport_correlation: TransportCorrelation,
}

impl FreshIntent {
    /// The host composition root calls this only after it decided to start a new operation.
    pub fn new(host_minted_operation: OperationId, scope: AuthenticatedHostScope, transport_correlation: TransportCorrelation) -> Self {
        Self { host_minted_operation, scope, transport_correlation }
    }

    pub fn host_minted_operation(&self) -> &OperationId {
        &self.host_minted_operation
    }

    pub fn scope(&self) -> &AuthenticatedHostScope {
        &self.scope
    }

    pub fn transport_correlation(&self) -> &TransportCorrelation {
        &self.transport_correlation
    }

    pub fn is_provable(&self) -> bool {
        self.scope.is_provable()
    }
}

/// The reverse-lookup key for a retried transport request. Correlation is never authority.
///
/// It is not deserializable: external JSON can never construct one.
///
/// ```compile_fail
/// let _key: solidcraft_engine::entitlement::RetryScopeKey = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RetryScopeKey {
    pub host_scope: AuthenticatedHostScope,
    pub transport_correlation: TransportCorrelation,
}

/// Host-minted authority for one logical operation. It has no `Deserialize` implementation.
///
/// Only the host's authenticated path can create it:
///
/// ```compile_fail
/// let _ctx = solidcraft_engine::entitlement::TrustedContext {
///     scope: todo!(),
///     operation: todo!(),
///     capability: todo!(),
///     entry: todo!(),
/// };
/// ```
///
/// External JSON cannot construct it either:
///
/// ```compile_fail
/// let _ctx: solidcraft_engine::entitlement::TrustedContext = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedContext {
    scope: AuthenticatedHostScope,
    operation: OperationId,
    capability: Capability,
    entry: EntryCategory,
}

impl TrustedContext {
    /// Mint one logical operation from an authenticated host scope.
    ///
    /// `operation` must come from the host's authenticated mint/store path, and `scope` only from
    /// host authentication. Never build either from external wire data: request ids, tool-call ids,
    /// ACP message ids and caller-supplied account/session fields are data, not authority.
    pub fn mint(scope: &AuthenticatedHostScope, operation: &str, capability: Capability, entry: EntryCategory) -> Result<Self, OperationIdError> {
        Ok(Self { scope: scope.clone(), operation: OperationId::parse(operation)?, capability, entry })
    }

    pub fn operation(&self) -> &OperationId {
        &self.operation
    }
}
