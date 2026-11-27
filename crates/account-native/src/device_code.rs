//! The explicit WorkOS CLI Auth device-code lifecycle.
//!
//! This follows the published WorkOS CLI Auth endpoints (RFC 8628). It never opens a browser and
//! is only started by an explicit caller command; ordinary program paths return `AUTH_REQUIRED`.
//! The device code and issued tokens are redacted from `Debug`, `Display`, and error messages.
//!
//! [`DeviceCodeFlow::complete`] stores only the provider login. It never activates an Intcad
//! product session: the caller must separately obtain a typed product session from a documented
//! [`crate::credentials::ProductSessionExchange`]. The refresh token and access-token expiry are
//! captured in memory on the authorized provider session for the renewal lifecycle; they are
//! not persisted.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use solidcraft_engine::entitlement::{CredentialError, CredentialErrorCode, Refusal, RefusalCode};

use crate::credentials::{
    CredentialSession, MAX_ACCOUNT_BYTES, ProviderLogin, ProviderSession, auth_unavailable, has_control, safe_token, unavailable,
};
use crate::form::form_encode;
use crate::http::Secret;
use crate::token_expiry::access_token_expiry_unix;

/// WorkOS CLI Auth device authorization endpoint (`POST`).
pub const WORKOS_DEVICE_AUTHORIZATION_URL: &str = "https://api.workos.com/user_management/authorize/device";
/// WorkOS CLI Auth device-token endpoint (`POST`).
pub const WORKOS_TOKEN_URL: &str = "https://api.workos.com/user_management/authenticate";
/// The only grant type the WorkOS CLI Auth token endpoint documents.
pub const WORKOS_DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

const MAX_DEVICE_RESPONSE_BYTES: usize = 64 * 1024;

fn auth_required(message: &'static str) -> Refusal {
    Refusal::new(RefusalCode::AuthRequired, message)
}

fn safe_uri(value: &str) -> bool {
    value.starts_with("https://")
        && value.len() > "https://".len()
        && value.len() <= 2048
        && !has_control(value)
        && !value.bytes().any(|byte| byte.is_ascii_whitespace() || byte == b'\\' || byte == b'@')
}

/// One public-client device authorization request returned by WorkOS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCodePrompt {
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: Duration,
    interval: Duration,
}

impl DeviceCodePrompt {
    pub fn user_code(&self) -> &str {
        &self.user_code
    }

    pub fn verification_uri(&self) -> &str {
        &self.verification_uri
    }

    pub fn verification_uri_complete(&self) -> Option<&str> {
        self.verification_uri_complete.as_deref()
    }

    pub fn expires_in(&self) -> Duration {
        self.expires_in
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }
}

impl fmt::Display for DeviceCodePrompt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "To sign in, open {} on any device and enter the code {}.", self.verification_uri, self.user_code)?;
        if let Some(complete) = self.verification_uri_complete.as_deref() {
            write!(formatter, " You can also open {complete}.")?;
        }
        formatter.write_str(" This program never opens a browser for you.")
    }
}

/// A stable WorkOS device-token error that callers can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceCodeError {
    InvalidRequest,
    InvalidClient,
    InvalidGrant,
    UnsupportedGrantType,
}

/// One poll result. Tokens are kept inside the flow until [`DeviceCodeFlow::complete`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceCodePoll {
    Pending { interval: Duration },
    SlowDown { interval: Duration },
    Authorized,
    Denied,
    Expired,
    Failed(DeviceCodeError),
}

/// Whether the caller explicitly asked for an interactive sign-in. A normal program call is never
/// allowed to start device authorization or open a browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceCodeMode {
    Explicit,
    Programmatic,
}

/// The HTTP seam for WorkOS CLI Auth. Tests script this; configuration never supplies a URL.
pub trait DeviceCodeTransport {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError>;
}

impl<T: DeviceCodeTransport + ?Sized> DeviceCodeTransport for Box<T> {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError> {
        (**self).post(url, content_type, body)
    }
}

impl<T: DeviceCodeTransport + ?Sized> DeviceCodeTransport for Arc<T> {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError> {
        (**self).post(url, content_type, body)
    }
}

/// The documented WorkOS public-client flow. HTTPS-only, no redirects, bounded response.
pub struct UreqDeviceCodeTransport {
    agent: ureq::Agent,
}

impl UreqDeviceCodeTransport {
    pub fn new() -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .timeout_connect(Some(Duration::from_secs(10)))
            .user_agent("intcad-account-device")
            .build()
            .into();
        Self { agent }
    }
}

impl Default for UreqDeviceCodeTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceCodeTransport for UreqDeviceCodeTransport {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError> {
        let _trace = crate::http::SuppressWireTrace::new();
        let mut response =
            self.agent.post(url).header("Accept", "application/json").header("Content-Type", content_type).send(body).map_err(|_| unavailable())?;
        let status = response.status().as_u16();
        let body = response.body_mut().with_config().limit(MAX_DEVICE_RESPONSE_BYTES as u64).read_to_vec().map_err(|_| unavailable())?;
        Ok((status, body))
    }
}

/// A WorkOS public-client id. Empty or malformed configuration is rejected before any request.
#[derive(Clone, Debug)]
pub struct WorkOsPublicClient {
    client_id: String,
}

impl WorkOsPublicClient {
    pub fn new(client_id: impl Into<String>) -> Option<Self> {
        let client_id = client_id.into();
        if client_id.is_empty() || client_id.len() > 256 || has_control(&client_id) || client_id.bytes().any(|byte| byte.is_ascii_whitespace()) {
            None
        } else {
            Some(Self { client_id })
        }
    }
}

struct PendingDeviceCode {
    device_code: Secret,
    interval: Duration,
}

struct DeviceCodeAuthorization {
    account_id: String,
    access_token: Secret,
    refresh_token: Secret,
    expires_at_unix_seconds: u64,
}

/// Explicit device-code lifecycle over one shared credential session.
pub struct DeviceCodeFlow<T: DeviceCodeTransport> {
    client: WorkOsPublicClient,
    transport: T,
    session: CredentialSession,
    mode: DeviceCodeMode,
    pending: Option<PendingDeviceCode>,
    authorized: Option<DeviceCodeAuthorization>,
}

impl<T: DeviceCodeTransport> fmt::Debug for DeviceCodeFlow<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceCodeFlow")
            .field("mode", &self.mode)
            .field("pending", &self.pending.is_some())
            .field("authorized", &self.authorized.is_some())
            .field("credentials", &"<redacted>")
            .finish()
    }
}

impl<T: DeviceCodeTransport> DeviceCodeFlow<T> {
    pub fn new(client: WorkOsPublicClient, transport: T, session: CredentialSession, mode: DeviceCodeMode) -> Self {
        Self { client, transport, session, mode, pending: None, authorized: None }
    }

    pub fn session(&self) -> &CredentialSession {
        &self.session
    }

    /// Ask WorkOS for a code. Only an explicit caller command may do this.
    pub fn start(&mut self) -> Result<DeviceCodePrompt, Refusal> {
        if self.mode != DeviceCodeMode::Explicit {
            return Err(auth_required("an explicit sign-in command is required"));
        }
        if self.pending.is_some() || self.authorized.is_some() {
            return Err(auth_unavailable("a device sign-in is already in progress"));
        }
        let body = format!("client_id={}", form_encode(&self.client.client_id));
        let (status, body) = self
            .transport
            .post(WORKOS_DEVICE_AUTHORIZATION_URL, "application/x-www-form-urlencoded", body.as_bytes())
            .map_err(|_| auth_unavailable("the sign-in service is unavailable"))?;
        if !(200..300).contains(&status) {
            return Err(auth_unavailable("the sign-in service rejected the request"));
        }
        let value: Value = serde_json::from_slice(&body).map_err(|_| auth_unavailable("the sign-in service returned an invalid response"))?;
        let device_code = required_secret(&value, "device_code")?;
        let user_code = required_text(&value, "user_code", 64)?;
        let verification_uri = required_uri(&value, "verification_uri")?;
        let verification_uri_complete = optional_uri(&value, "verification_uri_complete")?;
        let expires_in = required_seconds(&value, "expires_in")?;
        let interval = required_seconds(&value, "interval")?;
        self.pending = Some(PendingDeviceCode { device_code, interval });
        Ok(DeviceCodePrompt { user_code, verification_uri, verification_uri_complete, expires_in, interval })
    }

    /// Poll the token endpoint once. The caller decides when to call again; the returned interval
    /// is the server's, increased after `slow_down` as RFC 8628 requires. A 200 response must
    /// carry a non-empty refresh token and a usable access-token lifetime (`expires_in` or the
    /// JWT `exp` claim); a response without both is refused.
    pub fn poll(&mut self) -> Result<DeviceCodePoll, Refusal> {
        let (device_code, interval) = {
            let pending = self.pending.as_ref().ok_or_else(|| auth_required("start a device sign-in first"))?;
            (pending.device_code.clone(), pending.interval)
        };
        let form =
            [("grant_type", WORKOS_DEVICE_GRANT_TYPE), ("device_code", device_code.expose_secret()), ("client_id", self.client.client_id.as_str())]
                .iter()
                .map(|(name, value)| format!("{}={}", form_encode(name), form_encode(value)))
                .collect::<Vec<_>>()
                .join("&");
        let (status, body) = self
            .transport
            .post(WORKOS_TOKEN_URL, "application/x-www-form-urlencoded", form.as_bytes())
            .map_err(|_| auth_unavailable("the sign-in service is unavailable"))?;
        if (200..300).contains(&status) {
            let value: Value = serde_json::from_slice(&body).map_err(|_| auth_unavailable("the sign-in service returned an invalid response"))?;
            let access_token = required_secret(&value, "access_token")?;
            let refresh_token = required_secret(&value, "refresh_token")?;
            let account_id = value
                .pointer("/user/id")
                .and_then(Value::as_str)
                .filter(|id| safe_token(id, MAX_ACCOUNT_BYTES))
                .ok_or_else(|| auth_unavailable("the sign-in service returned an invalid account"))?
                .to_owned();
            let expires_at_unix_seconds = token_expiry_seconds(&value, access_token.expose_secret())?;
            self.pending = None;
            self.authorized = Some(DeviceCodeAuthorization { account_id, access_token, refresh_token, expires_at_unix_seconds });
            return Ok(DeviceCodePoll::Authorized);
        }
        if status != 400 {
            return Err(auth_unavailable("the sign-in service is unavailable"));
        }
        let value: Value = serde_json::from_slice(&body).map_err(|_| auth_unavailable("the sign-in service returned an invalid response"))?;
        let error = value.get("error").and_then(Value::as_str).ok_or_else(|| auth_unavailable("the sign-in service returned an invalid error"))?;
        match error {
            "authorization_pending" => Ok(DeviceCodePoll::Pending { interval }),
            "slow_down" => {
                let interval = interval.saturating_add(Duration::from_secs(5)).min(Duration::from_secs(3600));
                if let Some(pending) = self.pending.as_mut() {
                    pending.interval = interval;
                }
                Ok(DeviceCodePoll::SlowDown { interval })
            }
            "access_denied" => {
                self.pending = None;
                Ok(DeviceCodePoll::Denied)
            }
            "expired_token" => {
                self.pending = None;
                Ok(DeviceCodePoll::Expired)
            }
            "invalid_request" => {
                self.pending = None;
                Ok(DeviceCodePoll::Failed(DeviceCodeError::InvalidRequest))
            }
            "invalid_client" => {
                self.pending = None;
                Ok(DeviceCodePoll::Failed(DeviceCodeError::InvalidClient))
            }
            "invalid_grant" => {
                self.pending = None;
                Ok(DeviceCodePoll::Failed(DeviceCodeError::InvalidGrant))
            }
            "unsupported_grant_type" => {
                self.pending = None;
                Ok(DeviceCodePoll::Failed(DeviceCodeError::UnsupportedGrantType))
            }
            _ => Err(auth_unavailable("the sign-in service returned an unknown error")),
        }
    }

    /// The typed provider session after [`DeviceCodePoll::Authorized`], without writing it. A live
    /// desktop worker offers it to the configured [`crate::credentials::ProductSessionExchange`]
    /// first and persists both only after verification. The provider token is never a product
    /// bearer and this call does not activate anything.
    pub fn authorized_provider(&self) -> Result<ProviderSession, Refusal> {
        let authorization = self.authorized.as_ref().ok_or_else(|| auth_required("poll until the sign-in is authorized first"))?;
        Ok(ProviderSession::from_authorized_refreshable(
            authorization.account_id.clone(),
            authorization.access_token.clone(),
            authorization.refresh_token.clone(),
            authorization.expires_at_unix_seconds,
        ))
    }

    /// Persist the authorized provider login into the shared credential store. This does not
    /// activate an Intcad product session: a trusted [`crate::credentials::ProductSessionExchange`]
    /// must run explicitly afterwards. The caller must have polled until
    /// [`DeviceCodePoll::Authorized`].
    pub fn complete(&mut self) -> Result<ProviderLogin, Refusal> {
        let authorization = self.authorized.take().ok_or_else(|| auth_required("poll until the sign-in is authorized first"))?;
        self.session.complete_provider_login(&authorization.account_id, authorization.access_token.expose_secret()).map_err(|error| {
            match error.code {
                CredentialErrorCode::NoPersistentCredential => auth_required("no secure credential store is available"),
                _ => auth_unavailable("the credential store could not be written"),
            }
        })
    }
}

fn required_secret(value: &Value, field: &str) -> Result<Secret, Refusal> {
    let text = value.get(field).and_then(Value::as_str).filter(|text| !text.is_empty() && !has_control(text));
    text.map(Secret::new).ok_or_else(|| auth_unavailable("the sign-in service returned an invalid credential"))
}

fn required_text(value: &Value, field: &str, max: usize) -> Result<String, Refusal> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= max && !has_control(text) && !text.bytes().any(|byte| byte.is_ascii_whitespace()))
        .map(str::to_owned)
        .ok_or_else(|| auth_unavailable("the sign-in service returned an invalid code"))
}

fn required_uri(value: &Value, field: &str) -> Result<String, Refusal> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|uri| safe_uri(uri))
        .map(str::to_owned)
        .ok_or_else(|| auth_unavailable("the sign-in service returned an insecure verification address"))
}

fn optional_uri(value: &Value, field: &str) -> Result<Option<String>, Refusal> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(uri)) if safe_uri(uri) => Ok(Some(uri.clone())),
        _ => Err(auth_unavailable("the sign-in service returned an insecure verification address")),
    }
}

/// The access-token expiry in Unix seconds: the server's `expires_in` when it is a valid
/// 1..=3600 seconds lifetime, or the access token's JWT `exp` claim if the
/// field is absent. A present but malformed lifetime is a protocol error.
fn token_expiry_seconds(value: &Value, access_token: &str) -> Result<u64, Refusal> {
    if value.get("expires_in").is_some() {
        let seconds = required_seconds(value, "expires_in")?.as_secs();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
        return Ok(now.saturating_add(seconds));
    }
    access_token_expiry_unix(access_token).ok_or_else(|| auth_unavailable("the sign-in service returned a token without a usable lifetime"))
}

fn required_seconds(value: &Value, field: &str) -> Result<Duration, Refusal> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .filter(|seconds| (1..=3600).contains(seconds))
        .map(Duration::from_secs)
        .ok_or_else(|| auth_unavailable("the sign-in service returned an invalid lifetime"))
}
