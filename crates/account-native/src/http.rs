//! Native HTTPS account adapter for the frozen `intcad-auth/1.0` contract.
//!
//! Wire shapes come only from the pinned rev4 OpenAPI document (`contracts/pin.json`). This module
//! maps them onto the engine's [`AccountService`]; it does not define a second contract.
//!
//! Safety rules:
//! - Credentials travel only in the contract's authentication headers. They never enter a URL,
//!   `Debug`, `Display`, an error message, or a response excerpt.
//! - Production is HTTPS-only. Redirects are disabled so a credential cannot follow a redirect.
//! - Responses are bounded in bytes and time. A transport, integrity, or ambiguous failure is a
//!   fail-closed [`RefusalCode::AuthUnavailable`]; it never grants execution or settlement.
//! - HTTP status is transport metadata only. Business meaning comes from the frozen `ErrorCode`.

#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    AccountService, Admission, AuthenticatedHostScope, Availability, BillingResult, Capability, ConservativeDeadline, EntryCategory, ErrorCode,
    Grant, GrantId, MonotonicClock, MonotonicInstant, OperationId, OperationStatus, Refusal, RefusalCode, RenewalAttempt,
};

use crate::credentials::{CredentialSession, ProductExchangeError};
use crate::session_verification::{valid_account, verified_account};

fn renewal_interval(value: Option<&Value>) -> Option<Duration> {
    value.and_then(Value::as_u64).filter(|seconds| (1..=3600).contains(seconds)).map(Duration::from_secs)
}

/// Frozen protocol version from the pinned manifest.
pub const PROTOCOL_VERSION: &str = "intcad-auth/1.0";
/// Hard response-size cap. The largest frozen artifact is 176,487 bytes; responses are far smaller.
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
/// Track the lease fixed by the pinned contract: rev4 OpenAPI `POST /v1/authorizations` sets
/// 300 seconds. Clamp the wall-clock conversion so it can never exceed that lease.
const MAX_ABSOLUTE_GRANT_LEASE: Duration = Duration::from_secs(300);

/// A credential value whose `Debug` and `Display` never reveal the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Only the credential store or an authentication header builder may call this.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// Native sink for credentials the server issues. Task 4 implements OS-protected storage.
///
/// This trait is the explicit native handoff: implementations must never log, serialize, put
/// credentials in a JSON ledger, or store them in plaintext. A handoff failure must fail the
/// exchange closed; execution or settlement must not continue without the credential.
pub trait CredentialHandoff {
    /// A bound response can provide cleanup credentials even when its execution grant is bad.
    /// The default sink does not use this early handoff, and it never authorizes execution.
    fn admission_settlement_candidate(&mut self, _operation: &OperationId, _credential: Secret) {}

    /// A wire-level renewal refusal retires only the original operation, not the account.
    fn renewal_unauthorized(&mut self, _scope: &AuthenticatedHostScope, _operation: &OperationId) {}

    /// Initial authorization-lease proof, keyed by the server-issued authorization id.
    fn initial_lease_proof(&mut self, authorization: &GrantId, capability: Capability, proof: Secret) -> Result<(), Refusal>;
    /// The execution grant proof for one admitted operation.
    fn execution_session_proof(&mut self, capability: Capability, operation: &OperationId, proof: Secret) -> Result<(), Refusal>;
    /// A newly issued or reissued settlement credential, scoped to one operation.
    fn settlement_credential(&mut self, operation: &OperationId, credential: Secret) -> Result<(), Refusal>;
    /// The one-time installation credential.
    fn installation_credential(&mut self, installation_id: &str, credential: Secret) -> Result<(), Refusal>;

    /// Publish the first installation identity and its credential after a provider-verified
    /// product session. `expected_product_session` is the exact bearer sent in the bootstrap
    /// request; an implementation must refuse if the active session changed meanwhile. A failed
    /// write must leave no readable half-binding.
    fn first_installation_credential(&mut self, expected_product_session: &Secret, installation_id: &str, credential: Secret) -> Result<(), Refusal>;
}

/// The host-owned credentials for one account session.
pub trait WireCredentials {
    fn product_session(&self) -> Option<Secret>;
    fn installation_credential(&self) -> Option<Secret>;
    fn settlement_credential(&self, operation: &OperationId) -> Option<Secret>;
    fn execution_session_proof(&self, operation: &OperationId) -> Option<Secret>;
    /// Initial lease proof, keyed by the server-issued authorization id.
    fn initial_lease_proof(&self, authorization: &GrantId) -> Option<Secret>;

    /// Whether this session already has a durable installation identity. A store with a
    /// partially written pair must answer `true` from the identity alone, so a caller never
    /// posts a replacement bootstrap while the previous identity is still recorded.
    fn installation_bound(&self) -> bool {
        self.installation_credential().is_some()
    }
}

/// A fixed credential set. Task 4 replaces this with the OS-protected store.
#[derive(Clone, Debug, Default)]
pub struct FixedCredentials {
    product_session: Option<Secret>,
    installation_credential: Option<Secret>,
    settlement_credential: Option<Secret>,
    execution_session_proof: Option<Secret>,
    initial_lease_proof: Option<Secret>,
}

impl FixedCredentials {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_product_session(mut self, secret: impl Into<String>) -> Self {
        self.product_session = Some(Secret::new(secret));
        self
    }

    pub fn with_installation_credential(mut self, secret: impl Into<String>) -> Self {
        self.installation_credential = Some(Secret::new(secret));
        self
    }

    pub fn with_settlement_credential(mut self, secret: impl Into<String>) -> Self {
        self.settlement_credential = Some(Secret::new(secret));
        self
    }

    pub fn with_execution_session_proof(mut self, secret: impl Into<String>) -> Self {
        self.execution_session_proof = Some(Secret::new(secret));
        self
    }

    pub fn with_initial_lease_proof(mut self, secret: impl Into<String>) -> Self {
        self.initial_lease_proof = Some(Secret::new(secret));
        self
    }
}

impl WireCredentials for FixedCredentials {
    fn product_session(&self) -> Option<Secret> {
        self.product_session.clone()
    }

    fn installation_credential(&self) -> Option<Secret> {
        self.installation_credential.clone()
    }

    fn settlement_credential(&self, _operation: &OperationId) -> Option<Secret> {
        self.settlement_credential.clone()
    }

    fn execution_session_proof(&self, _operation: &OperationId) -> Option<Secret> {
        self.execution_session_proof.clone()
    }

    fn initial_lease_proof(&self, _authorization: &GrantId) -> Option<Secret> {
        self.initial_lease_proof.clone()
    }
}

/// One HTTP request. Authentication header values are deliberately omitted from `Debug`.
#[derive(Clone)]
pub struct WireRequest {
    method: &'static str,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

impl WireRequest {
    /// Build one request from the host composition. New call sites must take the URL from a
    /// validated [`WireConnection`] and must not put a credential in the URL.
    pub(crate) fn new(method: &'static str, url: String, headers: Vec<(String, String)>, body: Option<Vec<u8>>) -> Self {
        Self { method, url, headers, body }
    }

    pub fn method(&self) -> &'static str {
        self.method
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    pub fn header_names(&self) -> impl Iterator<Item = &str> {
        self.headers.iter().map(|(key, _)| key.as_str())
    }

    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }
}

impl fmt::Debug for WireRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WireRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &self.header_names().collect::<Vec<_>>())
            .field("body_bytes", &self.body.as_ref().map_or(0, Vec::len))
            .finish()
    }
}

/// A bounded HTTP response. The body is omitted from `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct WireResponse {
    status: u16,
    body: Vec<u8>,
}

impl WireResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self { status, body: body.into() }
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Debug for WireResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("WireResponse").field("status", &self.status).field("body_bytes", &self.body.len()).finish()
    }
}

/// Why a request could not be completed. Details are never surfaced to callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportFailure {
    Timeout,
    Network,
    Tls,
    Redirected,
    BodyTooLarge,
}

/// The injectable HTTP boundary. Scripted transports implement the same trait in tests.
pub trait Transport {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
        (**self).send(request)
    }
}

impl<T: Transport + ?Sized> Transport for std::sync::Arc<T> {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
        (**self).send(request)
    }
}

impl<T: Transport + ?Sized> Transport for &T {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
        (**self).send(request)
    }
}

/// HTTPS transport built on `ureq`, with OS certificates, no redirects, and hard bounds.
#[derive(Clone, Debug)]
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        Self::with_limits(Duration::from_secs(30), Duration::from_secs(10))
    }

    /// A transport for one bounded startup check. The caller supplies both the total and connect
    /// limits; TLS verification, HTTPS-only and zero redirects are unchanged.
    pub fn with_limits(total: Duration, connect: Duration) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(total))
            .timeout_connect(Some(connect))
            .user_agent("intcad-account")
            .build()
            .into();
        Self { agent }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for UreqTransport {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
        let mut response = match request.method {
            "GET" => {
                let mut outgoing = self.agent.get(&request.url);
                for (name, value) in &request.headers {
                    outgoing = outgoing.header(name, value);
                }
                outgoing.call()
            }
            "POST" => {
                let body = request.body.as_deref().ok_or(TransportFailure::Network)?;
                let mut outgoing = self.agent.post(&request.url);
                for (name, value) in &request.headers {
                    outgoing = outgoing.header(name, value);
                }
                outgoing.send(body)
            }
            _ => return Err(TransportFailure::Network),
        }
        .map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let body = response.body_mut().with_config().limit(MAX_RESPONSE_BYTES as u64).read_to_vec().map_err(|_| TransportFailure::BodyTooLarge)?;
        Ok(WireResponse { status, body })
    }
}

fn map_ureq_error(error: ureq::Error) -> TransportFailure {
    match error {
        ureq::Error::Timeout(_) => TransportFailure::Timeout,
        ureq::Error::Tls(_) => TransportFailure::Tls,
        ureq::Error::RedirectFailed | ureq::Error::TooManyRedirects => TransportFailure::Redirected,
        ureq::Error::BodyExceedsLimit(_) => TransportFailure::BodyTooLarge,
        ureq::Error::StatusCode(_)
        | ureq::Error::Http(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::InvalidProxyUrl
        | ureq::Error::Protocol(_)
        | ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed => TransportFailure::Network,
        // `ureq::Error` is non-exhaustive; new variants fail closed as a network failure.
        _ => TransportFailure::Network,
    }
}

/// A validated base URL and version headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireConnection {
    base_url: String,
    client_version: String,
}

impl WireConnection {
    /// Normalize a configured product origin to the keyring namespace shared with desktop.
    pub fn product_origin(origin: &str, client_version: &str) -> Result<Self, HttpConfigError> {
        let validated = Self::new(origin, client_version)?;
        if origin.contains(['\\', '%']) {
            return Err(HttpConfigError::MalformedBaseUrl);
        }
        let uri: ureq::http::Uri = validated.base_url().parse().map_err(|_| HttpConfigError::MalformedBaseUrl)?;
        let authority = uri.authority().ok_or(HttpConfigError::MalformedBaseUrl)?;
        let host = authority.host().to_ascii_lowercase();
        if host.is_empty() {
            return Err(HttpConfigError::MalformedBaseUrl);
        }
        let port = authority.as_str().strip_prefix(authority.host()).ok_or(HttpConfigError::MalformedBaseUrl)?;
        let suffix = if port.is_empty() {
            String::new()
        } else {
            match port.strip_prefix(':').and_then(|value| value.parse::<u16>().ok()) {
                Some(443) => String::new(),
                Some(port) if port != 0 => format!(":{port}"),
                _ => return Err(HttpConfigError::MalformedBaseUrl),
            }
        };
        Self::new(format!("https://{host}{suffix}"), client_version)
    }

    /// Production connections are HTTPS-only. The base URL may contain only a host and an optional
    /// `/` path: no credentials, query, fragment, path prefix, or plain HTTP.
    pub fn new(base_url: impl Into<String>, client_version: impl Into<String>) -> Result<Self, HttpConfigError> {
        let base_url = base_url.into();
        let client_version = client_version.into();
        if client_version.trim().is_empty() {
            return Err(HttpConfigError::EmptyClientVersion);
        }
        let rest = base_url.strip_prefix("https://").ok_or(HttpConfigError::NotHttps)?;
        if rest.is_empty() || rest.starts_with('/') {
            return Err(HttpConfigError::MalformedBaseUrl);
        }
        if rest.contains('@') || rest.contains('?') || rest.contains('#') || rest.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(HttpConfigError::MalformedBaseUrl);
        }
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let path = &rest[authority_end..];
        if authority.is_empty() || !(path.is_empty() || path == "/") {
            return Err(HttpConfigError::MalformedBaseUrl);
        }
        Ok(Self { base_url: format!("https://{authority}"), client_version })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn client_version(&self) -> &str {
        &self.client_version
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpConfigError {
    NotHttps,
    MalformedBaseUrl,
    EmptyClientVersion,
}

impl fmt::Display for HttpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotHttps => "the account service URL must use HTTPS",
            Self::MalformedBaseUrl => "the account service URL is malformed",
            Self::EmptyClientVersion => "the client version must not be empty",
        })
    }
}

impl std::error::Error for HttpConfigError {}

/// Installation platforms from the frozen `InstallationBootstrapRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootstrapPlatform {
    Windows,
    Macos,
    Linux,
    Unknown,
}

impl BootstrapPlatform {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Unknown => "unknown",
        }
    }
}

/// A one-time installation credential. Debug and Display redact it.
pub struct InstallationBootstrap {
    installation_id: String,
    created_at: String,
    credential: Secret,
}

impl InstallationBootstrap {
    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }

    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    /// Move the raw credential into the OS-protected store. Never log it.
    pub fn into_credential(self) -> Secret {
        self.credential
    }
}

impl fmt::Debug for InstallationBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstallationBootstrap")
            .field("installation_id", &self.installation_id)
            .field("created_at", &self.created_at)
            .field("credential", &"<redacted>")
            .finish()
    }
}

/// The native account adapter for `AccountService`. The transport is injectable so tests never
/// touch a network.
pub struct HttpAccountService<T: Transport> {
    transport: T,
    connection: WireConnection,
    credentials: Box<dyn WireCredentials>,
    handoff: Box<dyn CredentialHandoff>,
    clock: Box<dyn MonotonicClock>,
}

/// A serial renewal driver for one already admitted desktop control operation.
/// Drive it only on a worker with bounded transport and a suspend-aware clock shared
/// with the dispatch/commit gates. Constructing it does not install authority.
pub struct OperationRenewal<T: Transport> {
    service: HttpAccountService<T>,
    session: CredentialSession,
    scope: AuthenticatedHostScope,
    operation: OperationId,
    authorization: GrantId,
    bearer: Secret,
    proof: Secret,
    generation: u64,
    deadline: ConservativeDeadline,
    next: Option<MonotonicInstant>,
}

impl<T: Transport> OperationRenewal<T> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transport: T,
        connection: WireConnection,
        session: CredentialSession,
        clock: Box<dyn MonotonicClock>,
        scope: AuthenticatedHostScope,
        operation: OperationId,
        grant: Grant,
        deadline: ConservativeDeadline,
        interval: Duration,
    ) -> Result<Self, Refusal> {
        let now = clock.now();
        if !(Duration::from_secs(1)..=Duration::from_secs(3600)).contains(&interval) {
            return Err(refusal(RefusalCode::UnsafeToStart));
        }
        Self::at_due(transport, connection, session, clock, scope, operation, grant, deadline, now.saturating_add(interval))
    }

    pub fn from_admission(
        transport: T,
        connection: WireConnection,
        session: CredentialSession,
        clock: Box<dyn MonotonicClock>,
        scope: AuthenticatedHostScope,
        operation: OperationId,
        grant: Grant,
    ) -> Result<Self, Refusal> {
        let (deadline, due) = grant.renewal_schedule(&scope, &operation).ok_or_else(|| refusal(RefusalCode::UnsafeToStart))?;
        let deadline = deadline.clone();
        Self::at_due(transport, connection, session, clock, scope, operation, grant, deadline, due)
    }

    #[allow(clippy::too_many_arguments)]
    fn at_due(
        transport: T,
        connection: WireConnection,
        session: CredentialSession,
        clock: Box<dyn MonotonicClock>,
        scope: AuthenticatedHostScope,
        operation: OperationId,
        grant: Grant,
        deadline: ConservativeDeadline,
        due: MonotonicInstant,
    ) -> Result<Self, Refusal> {
        if grant.capability() != Capability::Control || deadline.expires_at() > grant.expires_at() || deadline.remaining(clock.now()).is_none() {
            return Err(refusal(RefusalCode::UnsafeToStart));
        }
        let generation = session.authority_generation();
        let bearer = WireCredentials::product_session(&session).ok_or_else(|| refusal(RefusalCode::AuthRequired))?;
        let proof = WireCredentials::execution_session_proof(&session, &operation).ok_or_else(|| refusal(RefusalCode::AuthRequired))?;
        let fixed = FixedCredentials::none().with_product_session(bearer.expose_secret()).with_execution_session_proof(proof.expose_secret());
        let service = HttpAccountService::with_transport(transport, connection, Box::new(fixed), Box::new(session.clone()), clock);
        let next = Some(due);
        let driver = Self { service, session, scope, operation, authorization: grant.id().clone(), bearer, proof, generation, deadline, next };
        driver.check_binding()?;
        Ok(driver)
    }

    fn check_binding(&self) -> Result<(), Refusal> {
        if let Some(code) = self.session.renewal_refusal(&self.scope, &self.operation) {
            return Err(refusal(code));
        }
        if self.session.authenticated_scope().as_ref() != Some(&self.scope)
            || WireCredentials::product_session(&self.session).as_ref() != Some(&self.bearer)
            || WireCredentials::execution_session_proof(&self.session, &self.operation).as_ref() != Some(&self.proof)
            || self.session.authority_generation() != self.generation
        {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        Ok(())
    }

    /// Attempt a renewal only when due. A failed exchange permanently retires this
    /// driver and leaves the old deadline intact; no retries or replacement operations.
    pub fn tick(&mut self) -> Result<bool, Refusal> {
        let Some(next) = self.next else { return Ok(false) };
        let now = self.service.clock.now();
        if self.deadline.remaining(now).is_none() {
            self.next = None;
            return Err(refusal(RefusalCode::UnsafeToStart));
        }
        if now < next {
            return Ok(false);
        }
        self.next = None;
        self.check_binding()?;
        let (grant, sent_at, interval) =
            self.service.exchange_renewal(&self.scope, &self.operation, &self.authorization, Capability::Control, Some(&self.deadline))?;
        let received_at = self.service.clock.now();
        self.check_binding()?;
        let interval = interval.ok_or_else(unavailable)?;
        let attempt = RenewalAttempt { sent_at, received_at, old_deadline: self.deadline.clone(), still_valid: true };
        let remaining = grant.expires_at().as_duration().saturating_sub(sent_at.as_duration());
        let candidate = self.deadline.renewed(&attempt, remaining).map_err(|_| refusal(RefusalCode::UnsafeToStart))?;
        let applied_at = self.service.clock.now();
        if self.deadline.remaining(applied_at).is_none()
            || candidate.remaining(applied_at).is_none()
            || candidate.expires_at() < self.deadline.expires_at()
        {
            return Err(refusal(RefusalCode::UnsafeToStart));
        }
        self.deadline = candidate;
        self.next = Some(sent_at.saturating_add(interval));
        Ok(true)
    }

    pub fn deadline(&self) -> &ConservativeDeadline {
        &self.deadline
    }

    /// Time until the next attempt or expiry, whichever is earlier.
    pub fn wait_for_next_tick(&self) -> Option<Duration> {
        let now = self.service.clock.now();
        let due = self.next?.min(self.deadline.expires_at());
        Some(due.as_duration().saturating_sub(now.as_duration()))
    }

    #[cfg(test)]
    pub(crate) fn transport(&self) -> &T {
        self.service.transport()
    }
}

impl<T: Transport> fmt::Debug for HttpAccountService<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpAccountService")
            .field("base_url", &self.connection.base_url())
            .field("client_version", &self.connection.client_version())
            .field("credentials", &"<redacted>")
            .field("handoff", &"<redacted>")
            .finish()
    }
}

impl HttpAccountService<UreqTransport> {
    pub fn new(
        connection: WireConnection,
        credentials: Box<dyn WireCredentials>,
        handoff: Box<dyn CredentialHandoff>,
        clock: Box<dyn MonotonicClock>,
    ) -> Self {
        Self::with_transport(UreqTransport::new(), connection, credentials, handoff, clock)
    }
}

impl<T: Transport> HttpAccountService<T> {
    pub fn with_transport(
        transport: T,
        connection: WireConnection,
        credentials: Box<dyn WireCredentials>,
        handoff: Box<dyn CredentialHandoff>,
        clock: Box<dyn MonotonicClock>,
    ) -> Self {
        Self { transport, connection, credentials, handoff, clock }
    }

    #[cfg(test)]
    pub(crate) fn transport(&self) -> &T {
        &self.transport
    }

    fn request(&self, method: &'static str, path: &str, body: Option<Value>) -> WireRequest {
        let mut headers = vec![
            ("Accept".to_owned(), "application/json".to_owned()),
            ("X-Client-Version".to_owned(), self.connection.client_version.clone()),
            ("X-Protocol-Version".to_owned(), PROTOCOL_VERSION.to_owned()),
        ];
        let body = body.map(|value| value.to_string().into_bytes());
        if body.is_some() {
            headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
        }
        WireRequest { method, url: self.connection.url(path), headers, body }
    }

    fn push_auth(&self, request: &mut WireRequest, name: &str, secret: Option<Secret>) -> Result<(), Refusal> {
        let secret = secret.filter(|value| !value.is_empty()).ok_or_else(|| refusal(RefusalCode::AuthRequired))?;
        // A credential is host-owned, but it still must not be able to inject a header.
        if secret.expose_secret().bytes().any(|byte| byte == b'\r' || byte == b'\n' || byte == 0) {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        request.headers.push((name.to_owned(), secret.expose_secret().to_owned()));
        Ok(())
    }

    fn exchange(&self, request: WireRequest) -> Result<(u16, Value), Refusal> {
        let response = self.transport.send(&request).map_err(|_| unavailable())?;
        Self::decode_response(response)
    }

    fn decode_response(response: WireResponse) -> Result<(u16, Value), Refusal> {
        if response.body.len() > MAX_RESPONSE_BYTES {
            return Err(unavailable());
        }
        let value = if response.body.is_empty() { Value::Null } else { serde_json::from_slice(&response.body).map_err(|_| unavailable())? };
        Ok((response.status, value))
    }

    fn grant(&self, sent_at: MonotonicInstant, id: &str, capability: Capability, remaining_seconds: u64) -> Grant {
        let remaining = Duration::from_secs(remaining_seconds.min(3600));
        Grant::from_server(GrantId::new(id), capability, sent_at.saturating_add(remaining))
    }

    /// Map an absolute server timestamp to a local monotonic bound anchored at send. The wall clock
    /// is used only to measure the remaining server-issued interval, and the result is clamped to
    /// the lease fixed by the pinned contract. A shorter server interval is not extended.
    fn grant_from_absolute(
        &self,
        sent_at: MonotonicInstant,
        wall_sent: SystemTime,
        id: &str,
        capability: Capability,
        authorization_expires_at: &str,
        grant_expires_at: &str,
    ) -> Result<Grant, Refusal> {
        // Numeric comparison, not lexical: both timestamps are parsed to Unix milliseconds first,
        // and either malformed value fails closed.
        let authorization_ms = parse_rfc3339_utc_millis(authorization_expires_at).ok_or_else(unavailable)?;
        let grant_ms = parse_rfc3339_utc_millis(grant_expires_at).ok_or_else(unavailable)?;
        let expires_ms = authorization_ms.min(grant_ms);
        let wall_ms = wall_sent.duration_since(UNIX_EPOCH).map_err(|_| unavailable())?.as_millis();
        let wall_ms = u64::try_from(wall_ms).map_err(|_| unavailable())?;
        let remaining = expires_ms.saturating_sub(wall_ms);
        let remaining = Duration::from_millis(remaining).min(MAX_ABSOLUTE_GRANT_LEASE);
        Ok(Grant::from_server(GrantId::new(id), capability, sent_at.saturating_add(remaining)))
    }

    /// POST `/v1/installations`: one-time installation proof bootstrap. The raw credential is
    /// handed to the native [`CredentialHandoff`] (Task 4 stores it in the OS-protected store) and
    /// also returned once; this adapter never logs or persists it.
    pub fn bootstrap_installation(
        &mut self,
        scope: &AuthenticatedHostScope,
        platform: BootstrapPlatform,
        label: Option<&str>,
    ) -> Result<InstallationBootstrap, Refusal> {
        require_provable(scope)?;
        if label.is_some_and(|value| value.len() > 128 || value.contains(['\r', '\n'])) {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        let mut body = json!({"platform": platform.wire()});
        if let Some(label) = label {
            body["label"] = Value::String(label.to_owned());
        }
        let mut request = self.request("POST", "/v1/installations", Some(body));
        self.push_auth(
            &mut request,
            "Authorization",
            self.credentials.product_session().map(|secret| Secret::new(format!("Bearer {}", secret.expose_secret()))),
        )?;
        let (status, value) = self.exchange(request)?;
        if status != 201 {
            return Err(error_refusal(status, &value));
        }
        let (installation_id, credential, created_at) = validate_installation_bootstrap(&value)?;
        self.handoff.installation_credential(&installation_id, Secret::new(credential.clone())).map_err(|_| unavailable())?;
        Ok(InstallationBootstrap { installation_id, created_at, credential: Secret::new(credential) })
    }

    /// POST `/v1/installations` for the first installation of a provider-verified product
    /// session. Unlike [`Self::bootstrap_installation`], this path needs no
    /// [`AuthenticatedHostScope`]: the installation identity does not exist yet, so requiring a
    /// scope would be circular. The active product session is the authority, and the response is
    /// validated against the frozen bootstrap schema before the pair is published.
    pub fn bootstrap_first_installation(&mut self, platform: BootstrapPlatform, label: Option<&str>) -> Result<InstallationBootstrap, Refusal> {
        let product_session = self.credentials.product_session().ok_or_else(|| refusal(RefusalCode::AuthRequired))?;
        if self.credentials.installation_bound() {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        let installation = self.request_first_installation(&product_session, platform, label)?;
        self.handoff
            .first_installation_credential(&product_session, &installation.installation_id, installation.credential.clone())
            .map_err(|_| unavailable())?;
        Ok(installation)
    }

    /// Request the checked installation pair for a verified login that has not yet been published.
    /// The login worker commits the resulting pair together with the verified session.
    pub fn request_first_installation(
        &self,
        product_session: &Secret,
        platform: BootstrapPlatform,
        label: Option<&str>,
    ) -> Result<InstallationBootstrap, Refusal> {
        if label.is_some_and(|value| value.len() > 128 || value.contains(['\r', '\n'])) {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        let mut body = json!({"platform": platform.wire()});
        if let Some(label) = label {
            body["label"] = Value::String(label.to_owned());
        }
        let mut request = self.request("POST", "/v1/installations", Some(body));
        // Send the exact token that will be rechecked at publication, not a refreshed read.
        self.push_auth(&mut request, "Authorization", Some(Secret::new(format!("Bearer {}", product_session.expose_secret()))))?;
        let (status, value) = self.exchange(request)?;
        if status != 201 {
            return Err(error_refusal(status, &value));
        }
        let (installation_id, credential, created_at) = validate_installation_bootstrap(&value)?;
        Ok(InstallationBootstrap { installation_id, created_at, credential: Secret::new(credential) })
    }

    /// POST `/v1/authorizations` to renew an existing operation's lease.
    pub fn renew(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        authorization_id: &GrantId,
        capability: Capability,
    ) -> Result<Grant, Refusal> {
        self.exchange_renewal(scope, operation, authorization_id, capability, None).map(|(grant, _, _)| grant)
    }

    pub(crate) fn exchange_renewal(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        authorization_id: &GrantId,
        capability: Capability,
        prior: Option<&ConservativeDeadline>,
    ) -> Result<(Grant, MonotonicInstant, Option<Duration>), Refusal> {
        require_provable(scope)?;
        let mut request = self.request(
            "POST",
            "/v1/authorizations",
            Some(json!({
                "capability": capability.wire(),
                "operation_id": operation.as_str(),
                "renewal": {"authorization_id": authorization_id.as_str()},
            })),
        );
        self.push_auth(
            &mut request,
            "Authorization",
            self.credentials.product_session().map(|secret| Secret::new(format!("Bearer {}", secret.expose_secret()))),
        )?;
        self.push_auth(&mut request, "X-Execution-Session-Proof", self.credentials.execution_session_proof(operation))?;
        let sent_at = self.clock.now();
        if prior.is_some_and(|deadline| deadline.remaining(sent_at).is_none()) {
            return Err(refusal(RefusalCode::UnsafeToStart));
        }
        let response = self.transport.send(&request).map_err(|_| unavailable())?;
        if response.status() == 401 {
            self.handoff.renewal_unauthorized(scope, operation);
            return Err(refusal(RefusalCode::AuthRequired));
        }
        let (status, value) = Self::decode_response(response)?;
        if status != 200 {
            return Err(error_refusal(status, &value));
        }
        if value.get("renewed") != Some(&Value::Bool(true))
            || value.get("operation_id").and_then(Value::as_str) != Some(operation.as_str())
            || value.get("authorization_id").and_then(Value::as_str) != Some(authorization_id.as_str())
            || value.get("execution_session_proof") != Some(&Value::Null)
            || value.get("capability").and_then(Value::as_str) != Some(capability.wire())
        {
            return Err(unavailable());
        }
        let remaining = value.get("remaining_seconds").and_then(Value::as_u64).ok_or_else(unavailable)?;
        let id = value.get("authorization_id").and_then(Value::as_str).ok_or_else(unavailable)?;
        let interval = renewal_interval(value.get("renew_after_seconds"));
        Ok((self.grant(sent_at, id, capability, remaining), sent_at, interval))
    }

    /// Start with an optional trusted authorization lease.
    ///
    /// The frozen trait has no lease argument, so [`AccountService::begin`] passes `None`. A host
    /// that already holds a trusted lease may pass it here: the optional `authorization_id` is then
    /// forwarded exactly. Full lease plumbing belongs to Tasks 6/9 and is deliberately not invented
    /// from untrusted caller data.
    pub fn begin_with_authorization(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        capability: Capability,
        entry: EntryCategory,
        authorization_id: Option<&GrantId>,
    ) -> Result<Admission, Refusal> {
        require_provable(scope)?;
        let Some(entry_category) = entry.wire_entry_category() else {
            return Ok(Admission::Refused(refusal(RefusalCode::UnsafeToStart)));
        };
        let sent_at = self.clock.now();
        let wall_sent = self.clock.wall_now();
        let mut body = json!({
            "operation_id": operation.as_str(),
            "capability": capability.wire(),
            "entry_category": entry_category.wire(),
        });
        if let Some(authorization_id) = authorization_id {
            body["authorization_id"] = Value::String(authorization_id.as_str().to_owned());
        }
        let mut request = self.request("POST", "/v1/operations", Some(body));
        self.push_auth(
            &mut request,
            "Authorization",
            self.credentials.product_session().map(|secret| Secret::new(format!("Bearer {}", secret.expose_secret()))),
        )?;
        self.push_auth(&mut request, "X-Installation-Credential", self.credentials.installation_credential())?;
        let (status, value) = self.exchange(request)?;
        if status != 200 {
            return Ok(Admission::Refused(error_refusal(status, &value)));
        }
        match value.get("outcome").and_then(Value::as_str).ok_or_else(unavailable)? {
            "NEWLY_ADMITTED" => {
                if value.get("operation_id").and_then(Value::as_str) != Some(operation.as_str())
                    || value.get("installation_id").and_then(Value::as_str) != Some(scope.installation().as_str())
                    || value.get("state").and_then(Value::as_str) != Some("HELD")
                {
                    return Err(unavailable());
                }
                let settlement =
                    value.get("settlement_credential").and_then(Value::as_str).filter(|credential| !credential.is_empty()).ok_or_else(unavailable)?;
                self.handoff.admission_settlement_candidate(operation, Secret::new(settlement));
                if value.get("settlement_deadline") != Some(&Value::Null)
                    || value.pointer("/execution_grant/capability").and_then(Value::as_str) != Some(capability.wire())
                {
                    return Err(unavailable());
                }
                let grant_id = value.pointer("/execution_grant/grant_id").and_then(Value::as_str).ok_or_else(unavailable)?;
                let grant_expires_at = value.pointer("/execution_grant/expires_at").and_then(Value::as_str).ok_or_else(unavailable)?;
                let authorization_expires_at = value.get("authorization_expires_at").and_then(Value::as_str).ok_or_else(unavailable)?;
                let proof = value
                    .pointer("/execution_grant/session_proof")
                    .and_then(Value::as_str)
                    .filter(|proof| !proof.is_empty())
                    .ok_or_else(unavailable)?;
                let mut grant = self.grant_from_absolute(sent_at, wall_sent, grant_id, capability, authorization_expires_at, grant_expires_at)?;
                if let Some(interval) = renewal_interval(value.pointer("/execution_grant/renew_after_seconds")) {
                    grant = grant.with_admission_renewal(scope.clone(), operation.clone(), sent_at, interval);
                }
                self.handoff.execution_session_proof(capability, operation, Secret::new(proof)).map_err(|_| unavailable())?;
                self.handoff.settlement_credential(operation, Secret::new(settlement)).map_err(|_| unavailable())?;
                Ok(Admission::NewlyAdmitted { operation: operation.clone(), grant })
            }
            "ALREADY_EXISTS" => {
                if value.get("operation_id").and_then(Value::as_str) != Some(operation.as_str())
                    || value.get("installation_id").and_then(Value::as_str) != Some(scope.installation().as_str())
                {
                    return Err(unavailable());
                }
                let state = parse_operation_status(value.get("state").and_then(Value::as_str).ok_or_else(unavailable)?)?;
                let terminal = value.get("terminal").and_then(Value::as_bool).ok_or_else(unavailable)?;
                if terminal != state.is_terminal() || value.get("execution_grant").is_some() {
                    return Err(unavailable());
                }
                // ALREADY_EXISTS is authoritative status only. It may hand off settlement authority
                // for a nonterminal operation, but it never carries or mints execution authority.
                match value.get("settlement_credential") {
                    Some(Value::Null) => {}
                    Some(Value::String(credential)) if !credential.is_empty() => {
                        if terminal {
                            return Err(unavailable());
                        }
                        self.handoff.settlement_credential(operation, Secret::new(credential.clone())).map_err(|_| unavailable())?;
                    }
                    _ => return Err(unavailable()),
                }
                Ok(Admission::AlreadyExists { status: state })
            }
            "OPERATION_OWNED_ELSEWHERE" | "OPERATION_CONFLICT" => {
                let code = value.pointer("/error/code").and_then(Value::as_str).ok_or_else(unavailable)?;
                let expected = if value.get("outcome") == Some(&Value::String("OPERATION_OWNED_ELSEWHERE".into())) {
                    "OPERATION_OWNED_ELSEWHERE"
                } else {
                    "OPERATION_CONFLICT"
                };
                if code != expected {
                    return Err(unavailable());
                }
                Ok(Admission::Refused(error_refusal(status, &value)))
            }
            _ => Err(unavailable()),
        }
    }
}

impl<T: Transport> HttpAccountService<T> {
    /// Read one authenticated `/v1/status` value. The bearer is read from the host credential
    /// session and sent as the only credential; a missing or unusable bearer is refused before any
    /// request.
    fn status_value(&self) -> Result<Value, Refusal> {
        let mut request = self.request("GET", "/v1/status", None);
        self.push_auth(
            &mut request,
            "Authorization",
            self.credentials.product_session().map(|secret| Secret::new(format!("Bearer {}", secret.expose_secret()))),
        )?;
        let (status, value) = self.exchange(request)?;
        if status != 200 {
            return Err(error_refusal(status, &value));
        }
        Ok(value)
    }

    /// Decode the permissive status snapshot the generic [`AccountService::status`] contract uses.
    /// This deliberately still accepts an active-but-ineligible or unsupported account; activation
    /// and restart checks apply the stricter consumed predicate separately.
    fn parse_status_snapshot(&self, value: &Value) -> Result<(String, Availability), Refusal> {
        let account = value.pointer("/account/id").and_then(Value::as_str).filter(|account| valid_account(account)).ok_or_else(unavailable)?;
        let account_status = value.pointer("/account/status").and_then(Value::as_str).ok_or_else(unavailable)?;
        match account_status {
            "active" => {}
            "disabled" => return Err(refusal(RefusalCode::AccountDisabled)),
            "deleted" => return Err(refusal(RefusalCode::AuthRequired)),
            _ => return Err(unavailable()),
        }
        let balance = required_u64(value, "/balance")?;
        let held = required_u64(value, "/held")?;
        let available = required_u64(value, "/available")?;
        if held > balance || available != balance.saturating_sub(held) {
            return Err(unavailable());
        }
        let eligible = value.get("eligible").and_then(Value::as_bool).ok_or_else(unavailable)?;
        let version_supported = value.pointer("/client/supported").and_then(Value::as_bool).ok_or_else(unavailable)?;
        let protocol_supported = value.pointer("/protocol/supported").and_then(Value::as_bool).ok_or_else(unavailable)?;
        let email_verified = value.pointer("/account/email_verified").and_then(Value::as_bool).ok_or_else(unavailable)?;
        let model_available = value.pointer("/capabilities/chat").and_then(Value::as_bool).ok_or_else(unavailable)?;
        let mut reasons = Vec::new();
        for reason in value.get("reasons").and_then(Value::as_array).ok_or_else(unavailable)? {
            let code = reason.as_str().and_then(parse_error_code).ok_or_else(unavailable)?;
            match Refusal::from_wire(code) {
                Ok(code) => reasons.push(code),
                Err(_) => return Err(unavailable()),
            }
        }
        // The flags and reasons are one server snapshot. An inconsistent snapshot is not a grant.
        if (!eligible && reasons.is_empty())
            || (eligible && !reasons.is_empty())
            || (!version_supported && !reasons.contains(&RefusalCode::VersionUnsupported))
            || (!protocol_supported && !reasons.contains(&RefusalCode::ProtocolUnsupported))
            || (!email_verified && !reasons.contains(&RefusalCode::EmailUnverified))
        {
            return Err(unavailable());
        }
        Ok((account.to_owned(), Availability { online: true, version_supported, model_available, balance, held, reasons }))
    }

    /// Read one authenticated `/v1/status` snapshot and return the exact server account id with it.
    fn status_snapshot(&self) -> Result<(String, Availability), Refusal> {
        self.parse_status_snapshot(&self.status_value()?)
    }

    /// Prove that the configured product service accepts the host's exact bearer for the expected
    /// account under the same consumed activation conditions as a fresh login.
    ///
    /// This is a restart presentation check only: it never bootstraps an installation, issues a
    /// grant, or claims that the installation credential is still server-valid.
    pub fn restart_status(&self, expected_account: &str) -> Result<Availability, Refusal> {
        let value = self.status_value()?;
        let (_, availability) = self.parse_status_snapshot(&value)?;
        let account = verified_account(&value).map_err(activation_refusal)?;
        if account != expected_account {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        Ok(availability)
    }

    pub(crate) fn controlled_status(&self, scope: &AuthenticatedHostScope, capability: Capability) -> Result<(), Refusal> {
        let value = self.status_value()?;
        let (account, snapshot) = self.parse_status_snapshot(&value)?;
        if account != scope.account().as_str() {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        if let Some(code) = snapshot.reasons.first() {
            return Err(refusal(*code));
        }
        if verified_account(&value).map_err(activation_refusal)? != scope.account().as_str() {
            return Err(refusal(RefusalCode::AuthRequired));
        }
        match value.get("capabilities").and_then(|caps| caps.get(capability.wire())).and_then(Value::as_bool) {
            Some(true) => Ok(()),
            Some(false) => Err(refusal(RefusalCode::CapabilityDenied)),
            None => Err(unavailable()),
        }
    }

    pub(crate) fn operation_receipt(&self, operation: &OperationId) -> Result<(OperationStatus, Option<BillingResult>), Refusal> {
        let mut request = self.request("GET", &format!("/v1/operations/{}", operation.as_str()), None);
        self.push_auth(&mut request, "X-Settlement-Credential", self.credentials.settlement_credential(operation))?;
        let (status, value) = self.exchange(request)?;
        if status != 200 {
            return Err(error_refusal(status, &value));
        }
        if value.get("operation_id").and_then(Value::as_str) != Some(operation.as_str()) {
            return Err(unavailable());
        }
        let state = parse_operation_status(value.get("state").and_then(Value::as_str).ok_or_else(unavailable)?)?;
        let terminal = value.get("terminal").and_then(Value::as_bool).ok_or_else(unavailable)?;
        let result = match value.get("result").ok_or_else(unavailable)? {
            Value::Null => None,
            Value::String(raw) => Some(parse_billing_result(raw)?),
            _ => return Err(unavailable()),
        };
        validate_operation_status(state, result, terminal)?;
        validate_optional_timestamp(&value, "authorization_expires_at")?;
        validate_optional_timestamp(&value, "settlement_deadline")?;
        Ok((state, result))
    }
}

impl<T: Transport> AccountService for HttpAccountService<T> {
    fn status(&self) -> Result<Availability, Refusal> {
        self.status_snapshot().map(|(_, availability)| availability)
    }

    fn authorize(&mut self, scope: &AuthenticatedHostScope, capability: Capability) -> Result<Grant, Refusal> {
        require_provable(scope)?;
        let sent_at = self.clock.now();
        let mut request = self.request("POST", "/v1/authorizations", Some(json!({"capability": capability.wire()})));
        self.push_auth(
            &mut request,
            "Authorization",
            self.credentials.product_session().map(|secret| Secret::new(format!("Bearer {}", secret.expose_secret()))),
        )?;
        let (status, value) = self.exchange(request)?;
        if status != 200 {
            return Err(error_refusal(status, &value));
        }
        if value.get("capability").and_then(Value::as_str) != Some(capability.wire())
            || value.get("renewed") != Some(&Value::Bool(false))
            || value.get("operation_id") != Some(&Value::Null)
        {
            return Err(unavailable());
        }
        // Initial issuance must carry a non-empty proof; renewal is the only branch without one.
        let proof = value.get("execution_session_proof").and_then(Value::as_str).filter(|proof| !proof.is_empty()).ok_or_else(unavailable)?;
        let remaining = value.get("remaining_seconds").and_then(Value::as_u64).ok_or_else(unavailable)?;
        let id = value.get("authorization_id").and_then(Value::as_str).ok_or_else(unavailable)?;
        let authorization = GrantId::new(id);
        let grant = self.grant(sent_at, id, capability, remaining);
        self.handoff.initial_lease_proof(&authorization, capability, Secret::new(proof)).map_err(|_| unavailable())?;
        Ok(grant)
    }

    fn begin(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        capability: Capability,
        entry: EntryCategory,
    ) -> Result<Admission, Refusal> {
        self.begin_with_authorization(scope, operation, capability, entry, None)
    }

    fn finalize(&mut self, _scope: &AuthenticatedHostScope, operation: &OperationId, result: BillingResult) -> Result<OperationStatus, Refusal> {
        let mut request = self.request("POST", &format!("/v1/operations/{}/finalize", operation.as_str()), Some(json!({"result": result.wire()})));
        self.push_auth(&mut request, "X-Settlement-Credential", self.credentials.settlement_credential(operation))?;
        let (status, value) = self.exchange(request)?;
        if status != 200 {
            return Err(error_refusal(status, &value));
        }
        if value.get("operation_id").and_then(Value::as_str) != Some(operation.as_str()) {
            return Err(unavailable());
        }
        let state = parse_operation_status(value.get("state").and_then(Value::as_str).ok_or_else(unavailable)?)?;
        let terminal = value.get("terminal").and_then(Value::as_bool).ok_or_else(unavailable)?;
        let reported = value.get("result").and_then(Value::as_str).map(parse_billing_result).transpose()?.ok_or_else(unavailable)?;
        if reported != result {
            return Err(unavailable());
        }
        validate_finalize_receipt(state, result, terminal, &value)?;
        Ok(state)
    }

    fn operation(&self, _scope: &AuthenticatedHostScope, operation: &OperationId) -> Result<OperationStatus, Refusal> {
        self.operation_receipt(operation).map(|(status, _)| status)
    }
}

fn require_provable(scope: &AuthenticatedHostScope) -> Result<(), Refusal> {
    if scope.is_provable() { Ok(()) } else { Err(refusal(RefusalCode::AuthRequired)) }
}

pub(crate) fn refusal(code: RefusalCode) -> Refusal {
    Refusal::new(code, message(code))
}

fn message(code: RefusalCode) -> &'static str {
    match code {
        RefusalCode::AuthRequired => "authentication is required",
        RefusalCode::EmailUnverified => "the account email is not verified",
        RefusalCode::AccountDisabled => "the account is disabled",
        RefusalCode::CapabilityDenied => "the capability is not authorized",
        RefusalCode::VersionUnsupported => "the client version is not supported",
        RefusalCode::ProtocolUnsupported => "the protocol version is not supported",
        RefusalCode::UnsafeToStart => "no verified safe stop bound is available",
        RefusalCode::CreditsExhausted => "no credits are available",
        RefusalCode::CreditsHeld => "all credits are held",
        RefusalCode::OperationConflict => "the operation conflicts with its original identity",
        RefusalCode::OperationFinalized => "the operation already has a different terminal result",
        RefusalCode::OperationOwnedElsewhere => "the operation belongs to another installation",
        RefusalCode::AuthUnavailable => "the authorization service is unavailable",
    }
}

pub(crate) fn unavailable() -> Refusal {
    refusal(RefusalCode::AuthUnavailable)
}

/// Map the shared activation predicate's result to the HTTP adapter's refusal codes. The predicate
/// is reused by fresh login and restart; its unavailable and rejected outcomes stay fail-closed.
fn activation_refusal(error: ProductExchangeError) -> Refusal {
    match error {
        ProductExchangeError::Rejected => refusal(RefusalCode::AuthRequired),
        ProductExchangeError::NotConfigured | ProductExchangeError::Unavailable => unavailable(),
    }
}

fn validate_finalize_receipt(state: OperationStatus, result: BillingResult, terminal: bool, value: &Value) -> Result<(), Refusal> {
    let finalized_at = value.get("finalized_at").ok_or_else(unavailable)?;
    value.get("settlement_deadline").and_then(Value::as_str).filter(|value| !value.is_empty()).ok_or_else(unavailable)?;
    match (state, result, terminal) {
        (OperationStatus::Charged, BillingResult::Succeeded | BillingResult::PartialRetained, true)
        | (OperationStatus::Released, BillingResult::FailedNoEffect, true) => require_timestamp(finalized_at),
        (OperationStatus::Pending, BillingResult::Unknown, false) if finalized_at.is_null() => Ok(()),
        _ => Err(unavailable()),
    }
}

fn validate_operation_status(state: OperationStatus, result: Option<BillingResult>, terminal: bool) -> Result<(), Refusal> {
    let valid = match state {
        OperationStatus::Held => result.is_none() && !terminal,
        OperationStatus::Pending => !terminal && (result.is_none() || result == Some(BillingResult::Unknown)),
        OperationStatus::Charged => terminal && matches!(result, Some(BillingResult::Succeeded | BillingResult::PartialRetained)),
        OperationStatus::Released => terminal && result == Some(BillingResult::FailedNoEffect),
        OperationStatus::Expired => terminal && result.is_none(),
    };
    if valid { Ok(()) } else { Err(unavailable()) }
}

fn validate_optional_timestamp(value: &Value, field: &str) -> Result<(), Refusal> {
    match value.get(field) {
        Some(Value::Null) => Ok(()),
        Some(Value::String(text)) if !text.is_empty() => Ok(()),
        _ => Err(unavailable()),
    }
}

fn require_timestamp(value: &Value) -> Result<(), Refusal> {
    value.as_str().filter(|text| !text.is_empty()).map(|_| ()).ok_or_else(unavailable)
}

fn error_refusal(status: u16, value: &Value) -> Refusal {
    let Some(raw) = value.pointer("/error/code").and_then(Value::as_str) else {
        return generic_status_refusal(status);
    };
    let Some(code) = parse_error_code(raw) else {
        return generic_status_refusal(status);
    };
    match Refusal::from_wire(code) {
        Ok(code) => refusal(code),
        Err(_) => generic_status_refusal(status),
    }
}

/// Only the contract's generic 401 may be inferred from the transport status. Every other status
/// needs an explicit frozen `ErrorCode`; otherwise the exchange fails closed.
fn generic_status_refusal(status: u16) -> Refusal {
    if status == 401 { refusal(RefusalCode::AuthRequired) } else { unavailable() }
}

fn parse_error_code(raw: &str) -> Option<ErrorCode> {
    Some(match raw {
        "INVALID_REQUEST" => ErrorCode::InvalidRequest,
        "AUTH_REQUIRED" => ErrorCode::AuthRequired,
        "ACCOUNT_DISABLED" => ErrorCode::AccountDisabled,
        "EMAIL_UNVERIFIED" => ErrorCode::EmailUnverified,
        "CAPABILITY_DENIED" => ErrorCode::CapabilityDenied,
        "VERSION_UNSUPPORTED" => ErrorCode::VersionUnsupported,
        "PROTOCOL_UNSUPPORTED" => ErrorCode::ProtocolUnsupported,
        "UNSAFE_TO_START" => ErrorCode::UnsafeToStart,
        "CREDITS_EXHAUSTED" => ErrorCode::CreditsExhausted,
        "CREDITS_HELD" => ErrorCode::CreditsHeld,
        "OPERATION_CONFLICT" => ErrorCode::OperationConflict,
        "OPERATION_FINALIZED" => ErrorCode::OperationFinalized,
        "OPERATION_OWNED_ELSEWHERE" => ErrorCode::OperationOwnedElsewhere,
        "AUTH_UNAVAILABLE" => ErrorCode::AuthUnavailable,
        "INTERNAL_ERROR" => ErrorCode::InternalError,
        _ => return None,
    })
}

fn parse_operation_status(raw: &str) -> Result<OperationStatus, Refusal> {
    match raw {
        "HELD" => Ok(OperationStatus::Held),
        "PENDING" => Ok(OperationStatus::Pending),
        "CHARGED" => Ok(OperationStatus::Charged),
        "RELEASED" => Ok(OperationStatus::Released),
        "EXPIRED" => Ok(OperationStatus::Expired),
        _ => Err(unavailable()),
    }
}

fn parse_billing_result(raw: &str) -> Result<BillingResult, Refusal> {
    match raw {
        "SUCCEEDED" => Ok(BillingResult::Succeeded),
        "PARTIAL_RETAINED" => Ok(BillingResult::PartialRetained),
        "FAILED_NO_EFFECT" => Ok(BillingResult::FailedNoEffect),
        "UNKNOWN" => Ok(BillingResult::Unknown),
        _ => Err(unavailable()),
    }
}

trait BillingResultWire {
    fn wire(self) -> &'static str;
}

impl BillingResultWire for BillingResult {
    fn wire(self) -> &'static str {
        match self {
            Self::Succeeded => "SUCCEEDED",
            Self::PartialRetained => "PARTIAL_RETAINED",
            Self::FailedNoEffect => "FAILED_NO_EFFECT",
            Self::Unknown => "UNKNOWN",
        }
    }
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, Refusal> {
    value.pointer(pointer).and_then(Value::as_u64).ok_or_else(unavailable)
}

/// Validate the frozen `InstallationBootstrapResponse` before any credential is published.
/// `additionalProperties: false` is enforced by requiring exactly the three schema fields, so a
/// response cannot smuggle a balance, settlement credential or another binding.
fn validate_installation_bootstrap(value: &Value) -> Result<(String, String, String), Refusal> {
    let object = value.as_object().ok_or_else(unavailable)?;
    if object.len() != 3
        || !object.contains_key("installation_id")
        || !object.contains_key("installation_credential")
        || !object.contains_key("created_at")
    {
        return Err(unavailable());
    }
    let installation_id = object.get("installation_id").and_then(Value::as_str).ok_or_else(unavailable)?;
    if !valid_installation_id(installation_id) {
        return Err(unavailable());
    }
    let credential = object.get("installation_credential").and_then(Value::as_str).ok_or_else(unavailable)?;
    if !valid_opaque_credential(credential) {
        return Err(unavailable());
    }
    let created_at = object.get("created_at").and_then(Value::as_str).ok_or_else(unavailable)?;
    if parse_rfc3339_utc_millis(created_at).is_none() {
        return Err(unavailable());
    }
    Ok((installation_id.to_owned(), credential.to_owned(), created_at.to_owned()))
}

/// The frozen `InstallationId` grammar: `^inst_[A-Za-z0-9_-]{8,64}$`.
fn valid_installation_id(value: &str) -> bool {
    value.strip_prefix("inst_").is_some_and(|suffix| {
        (8..=64).contains(&suffix.len()) && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}

/// The frozen `OpaqueCredential` grammar: 32–4096 bytes from `[A-Za-z0-9._~+/=-]`.
fn valid_opaque_credential(value: &str) -> bool {
    (32..=4096).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-'))
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.fff]Z` into Unix milliseconds. Only UTC and the exact frozen shape
/// are accepted. The parser works on bytes and never slices the `&str` at an untrusted index, so
/// arbitrary UTF-8 input is total and returns `None` instead of panicking.
fn parse_rfc3339_utc_millis(raw: &str) -> Option<u64> {
    let bytes = raw.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year = parse_digits(bytes.get(0..4)?)?;
    let month = parse_digits(bytes.get(5..7)?)?;
    let day = parse_digits(bytes.get(8..10)?)?;
    let hour = parse_digits(bytes.get(11..13)?)?;
    let minute = parse_digits(bytes.get(14..16)?)?;
    let second = parse_digits(bytes.get(17..19)?)?;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let (fraction, suffix) = if bytes.get(19) == Some(&b'.') {
        let mut end = 20;
        while end < bytes.len() && bytes[end] != b'Z' {
            if !bytes[end].is_ascii_digit() {
                return None;
            }
            end += 1;
        }
        (parse_millis(bytes.get(20..end)?)?, end)
    } else {
        (0, 19)
    };
    if bytes.get(suffix) != Some(&b'Z') || bytes.len() != suffix + 1 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days.checked_mul(86_400)?.checked_add(hour * 3_600 + minute * 60 + second)?;
    seconds.checked_mul(1_000)?.checked_add(fraction)
}

fn parse_digits(raw: &[u8]) -> Option<u64> {
    if raw.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for byte in raw {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
    }
    Some(value)
}

fn parse_millis(raw: &[u8]) -> Option<u64> {
    if raw.is_empty() || raw.len() > 9 || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value = 0u64;
    for index in 0..3 {
        value = value * 10 + u64::from(raw.get(index).map_or(0, |byte| byte - b'0'));
    }
    Some(value)
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Howard Hinnant's days-from-civil algorithm, restricted to valid dates.
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let year = if month <= 2 { year.saturating_sub(1) } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ScriptedTransport {
    responses: RefCell<VecDeque<Result<WireResponse, TransportFailure>>>,
    pub(crate) requests: RefCell<Vec<WireRequest>>,
}

#[cfg(test)]
impl ScriptedTransport {
    pub(crate) fn new(responses: Vec<Result<WireResponse, TransportFailure>>) -> Self {
        Self { responses: RefCell::new(responses.into()), requests: RefCell::new(Vec::new()) }
    }

    pub(crate) fn take_requests(&self) -> Vec<WireRequest> {
        self.requests.borrow_mut().drain(..).collect()
    }
}

#[cfg(test)]
impl Transport for ScriptedTransport {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
        self.requests.borrow_mut().push(request.clone());
        self.responses.borrow_mut().pop_front().unwrap_or(Err(TransportFailure::Network))
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct HandoffLog {
    pub(crate) initial_proofs: Vec<(GrantId, Capability, Secret)>,
    pub(crate) proofs: Vec<(Capability, OperationId, Secret)>,
    pub(crate) settlements: Vec<(OperationId, Secret)>,
    pub(crate) installations: Vec<(String, Secret)>,
}

#[cfg(test)]
pub(crate) struct RecordingHandoff {
    log: std::rc::Rc<RefCell<HandoffLog>>,
    fail: bool,
}

#[cfg(test)]
impl RecordingHandoff {
    pub(crate) fn new() -> (Self, std::rc::Rc<RefCell<HandoffLog>>) {
        let log = std::rc::Rc::new(RefCell::new(HandoffLog::default()));
        (Self { log: log.clone(), fail: false }, log)
    }

    pub(crate) fn failing() -> (Self, std::rc::Rc<RefCell<HandoffLog>>) {
        let log = std::rc::Rc::new(RefCell::new(HandoffLog::default()));
        (Self { log: log.clone(), fail: true }, log)
    }
}

#[cfg(test)]
impl CredentialHandoff for RecordingHandoff {
    fn initial_lease_proof(&mut self, authorization: &GrantId, capability: Capability, proof: Secret) -> Result<(), Refusal> {
        if self.fail {
            return Err(unavailable());
        }
        self.log.borrow_mut().initial_proofs.push((authorization.clone(), capability, proof));
        Ok(())
    }

    fn execution_session_proof(&mut self, capability: Capability, operation: &OperationId, proof: Secret) -> Result<(), Refusal> {
        if self.fail {
            return Err(unavailable());
        }
        self.log.borrow_mut().proofs.push((capability, operation.clone(), proof));
        Ok(())
    }

    fn settlement_credential(&mut self, operation: &OperationId, credential: Secret) -> Result<(), Refusal> {
        if self.fail {
            return Err(unavailable());
        }
        self.log.borrow_mut().settlements.push((operation.clone(), credential));
        Ok(())
    }

    fn installation_credential(&mut self, installation_id: &str, credential: Secret) -> Result<(), Refusal> {
        if self.fail {
            return Err(unavailable());
        }
        self.log.borrow_mut().installations.push((installation_id.to_owned(), credential));
        Ok(())
    }

    fn first_installation_credential(
        &mut self,
        _expected_product_session: &Secret,
        installation_id: &str,
        credential: Secret,
    ) -> Result<(), Refusal> {
        self.installation_credential(installation_id, credential)
    }
}
