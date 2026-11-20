//! Native HTTPS account adapter for the frozen `intcad-auth/1.0` contract.
//!
//! Wire shapes come only from the pinned rev3 OpenAPI document (`contracts/pin.json`). This module
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
    AccountService, Admission, AuthenticatedHostScope, Availability, BillingResult, Capability, EntryCategory, ErrorCode, Grant, GrantId,
    MonotonicClock, MonotonicInstant, OperationId, OperationStatus, Refusal, RefusalCode,
};

/// Frozen protocol version from the rev3 manifest.
pub const PROTOCOL_VERSION: &str = "intcad-auth/1.0";
/// Hard response-size cap. The largest frozen artifact is 176,410 bytes; responses are far smaller.
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
/// The rev3 contract fixes the default lease at 60 seconds. A wall-clock conversion is clamped to
/// this value so a slow or skewed wall clock cannot extend execution authority.
const MAX_ABSOLUTE_GRANT_LEASE: Duration = Duration::from_secs(60);

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
    /// Initial authorization proof, or the execution grant proof for one operation.
    fn execution_session_proof(&mut self, capability: Capability, operation: Option<&OperationId>, proof: Secret) -> Result<(), Refusal>;
    /// A newly issued or reissued settlement credential, scoped to one operation.
    fn settlement_credential(&mut self, operation: &OperationId, credential: Secret) -> Result<(), Refusal>;
    /// The one-time installation credential.
    fn installation_credential(&mut self, installation_id: &str, credential: Secret) -> Result<(), Refusal>;
}

/// The host-owned credentials for one account session.
pub trait WireCredentials {
    fn product_session(&self) -> Option<Secret>;
    fn installation_credential(&self) -> Option<Secret>;
    fn settlement_credential(&self, operation: &OperationId) -> Option<Secret>;
    fn execution_session_proof(&self, operation: &OperationId) -> Option<Secret>;
}

/// A fixed credential set. Task 4 replaces this with the OS-protected store.
#[derive(Clone, Debug, Default)]
pub struct FixedCredentials {
    product_session: Option<Secret>,
    installation_credential: Option<Secret>,
    settlement_credential: Option<Secret>,
    execution_session_proof: Option<Secret>,
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

/// HTTPS transport built on `ureq`, with OS certificates, no redirects, and hard bounds.
#[derive(Clone, Debug)]
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .timeout_connect(Some(Duration::from_secs(10)))
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

    fn url(&self, path: &str) -> String {
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
    /// the frozen 60-second default lease. A wall rollback can shorten but never extend authority.
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
        let installation_id = required_str(&value, "/installation_id")?;
        let credential = Secret::new(required_str(&value, "/installation_credential")?);
        let created_at = required_str(&value, "/created_at")?;
        if installation_id.is_empty() || created_at.is_empty() || credential.is_empty() {
            return Err(unavailable());
        }
        self.handoff.installation_credential(&installation_id, credential.clone()).map_err(|_| unavailable())?;
        Ok(InstallationBootstrap { installation_id, created_at, credential })
    }

    /// POST `/v1/authorizations` for an initial lease. Renewal uses [`Self::renew`].
    pub fn renew(
        &mut self,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
        authorization_id: &GrantId,
        capability: Capability,
    ) -> Result<Grant, Refusal> {
        require_provable(scope)?;
        let sent_at = self.clock.now();
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
        let (status, value) = self.exchange(request)?;
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
        Ok(self.grant(sent_at, id, capability, remaining))
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
                    || value.get("settlement_deadline") != Some(&Value::Null)
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
                let settlement =
                    value.get("settlement_credential").and_then(Value::as_str).filter(|credential| !credential.is_empty()).ok_or_else(unavailable)?;
                let grant = self.grant_from_absolute(sent_at, wall_sent, grant_id, capability, authorization_expires_at, grant_expires_at)?;
                self.handoff.execution_session_proof(capability, Some(operation), Secret::new(proof)).map_err(|_| unavailable())?;
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

impl<T: Transport> AccountService for HttpAccountService<T> {
    fn status(&self) -> Result<Availability, Refusal> {
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
        let account_status = value.pointer("/account/status").and_then(Value::as_str).ok_or_else(unavailable)?;
        match account_status {
            "active" => {}
            "disabled" => return Err(refusal(RefusalCode::AccountDisabled)),
            "deleted" => return Err(refusal(RefusalCode::AuthRequired)),
            _ => return Err(unavailable()),
        }
        let balance = required_u64(&value, "/balance")?;
        let held = required_u64(&value, "/held")?;
        let available = required_u64(&value, "/available")?;
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
        Ok(Availability { online: true, version_supported, model_available, balance, held, reasons })
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
        let grant = self.grant(sent_at, id, capability, remaining);
        self.handoff.execution_session_proof(capability, None, Secret::new(proof)).map_err(|_| unavailable())?;
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
        Ok(state)
    }
}

fn require_provable(scope: &AuthenticatedHostScope) -> Result<(), Refusal> {
    if scope.is_provable() { Ok(()) } else { Err(refusal(RefusalCode::AuthRequired)) }
}

fn refusal(code: RefusalCode) -> Refusal {
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

fn unavailable() -> Refusal {
    refusal(RefusalCode::AuthUnavailable)
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

fn required_str(value: &Value, pointer: &str) -> Result<String, Refusal> {
    value.pointer(pointer).and_then(Value::as_str).map(str::to_owned).ok_or_else(unavailable)
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, Refusal> {
    value.pointer(pointer).and_then(Value::as_u64).ok_or_else(unavailable)
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
    pub(crate) proofs: Vec<(Capability, Option<OperationId>, Secret)>,
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
    fn execution_session_proof(&mut self, capability: Capability, operation: Option<&OperationId>, proof: Secret) -> Result<(), Refusal> {
        if self.fail {
            return Err(unavailable());
        }
        self.log.borrow_mut().proofs.push((capability, operation.cloned(), proof));
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
}
