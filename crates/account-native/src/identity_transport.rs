//! The WorkOS refresh-token transport for the identity lifecycle.
//!
//! [`UreqIdentityTransport`] posts the public-client form (`grant_type=refresh_token`,
//! `client_id`, `refresh_token`) to the fixed WorkOS HTTPS token URL and classifies the reply:
//! a confirmed `400 invalid_grant` is [`RefreshTransportError::Terminal`]; `408`, `429`, `5xx`
//! and transport failures are `Transient` (credentials are kept); every other status and every
//! malformed body is `Protocol`. Only a `200` with an access token, a refresh token and a user id
//! is a success. Tokens are redacted from `Debug` and never appear in a URL or error message.
//!
//! [`RefreshHttp`] is the test seam: tests script replies without a network. [`UreqRefreshHttp`]
//! is the only network adapter; it reuses the device transport's TLS/root-certificate and
//! no-redirect configuration with a 5-second total/connect timeout and a 64 KiB body cap.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::credentials::{MAX_ACCOUNT_BYTES, has_control, safe_token};
use crate::device_code::WORKOS_TOKEN_URL;
use crate::form::form_encode;
use crate::http::Secret;

const MAX_REFRESH_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_CLIENT_ID_BYTES: usize = 256;
const MAX_RETRY_AFTER_SECONDS: u64 = 3600;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// One successful WorkOS refresh. The tokens are secrets; `Debug` never reveals them.
pub(crate) struct RefreshResponse {
    pub access_token: Secret,
    pub refresh_token: Secret,
    pub provider_account_id: String,
    pub expires_in: Option<Duration>,
}

impl fmt::Debug for RefreshResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefreshResponse")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("provider_account_id", &self.provider_account_id)
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// The stable refresh failure classification the lifecycle acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefreshTransportError {
    /// Recoverable: keep the credentials and retry within the budget.
    Transient { retry_after: Option<Duration> },
    /// A confirmed `invalid_grant`: the refresh token is retired.
    Terminal,
    /// An unclassifiable status or malformed response: grant nothing, keep the credentials.
    Protocol,
}

/// The refresh seam. The lifecycle depends only on this trait.
pub(crate) trait IdentityTransport: Send + Sync {
    fn refresh(&self, client_id: &str, refresh_token: &Secret) -> Result<RefreshResponse, RefreshTransportError>;
}

/// One scripted or real HTTP reply. `Debug` prints the body length, never the body.
#[derive(Clone)]
pub(crate) struct RefreshHttpReply {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: Vec<u8>,
}

impl fmt::Debug for RefreshHttpReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefreshHttpReply")
            .field("status", &self.status)
            .field("retry_after", &self.retry_after)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// The HTTP seam for the refresh exchange. Tests script this; configuration never supplies a URL.
pub(crate) trait RefreshHttp: Send + Sync {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<RefreshHttpReply, RefreshHttpError>;
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RefreshHttpError {
    Transport,
    BodyTooLarge,
}

/// The documented WorkOS public-client refresh over one [`RefreshHttp`] adapter.
pub(crate) struct UreqIdentityTransport<H: RefreshHttp> {
    http: H,
}

impl<H: RefreshHttp> UreqIdentityTransport<H> {
    pub(crate) fn new(http: H) -> Self {
        Self { http }
    }
}

impl<H: RefreshHttp> IdentityTransport for UreqIdentityTransport<H> {
    fn refresh(&self, client_id: &str, refresh_token: &Secret) -> Result<RefreshResponse, RefreshTransportError> {
        if client_id.is_empty()
            || client_id.len() > MAX_CLIENT_ID_BYTES
            || has_control(client_id)
            || client_id.bytes().any(|byte| byte.is_ascii_whitespace())
        {
            return Err(RefreshTransportError::Protocol);
        }
        let token = refresh_token.expose_secret();
        if token.is_empty() || has_control(token) {
            return Err(RefreshTransportError::Protocol);
        }
        let form = [("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", token)]
            .iter()
            .map(|(name, value)| format!("{}={}", form_encode(name), form_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let reply = self.http.post(WORKOS_TOKEN_URL, "application/x-www-form-urlencoded", form.as_bytes()).map_err(|error| match error {
            RefreshHttpError::Transport => RefreshTransportError::Transient { retry_after: None },
            RefreshHttpError::BodyTooLarge => RefreshTransportError::Protocol,
        })?;
        if reply.body.len() > MAX_REFRESH_RESPONSE_BYTES {
            return Err(RefreshTransportError::Protocol);
        }
        match reply.status {
            200 => parse_success(&reply.body),
            400 => {
                let value: Value = serde_json::from_slice(&reply.body).map_err(|_| RefreshTransportError::Protocol)?;
                if value.get("error").and_then(Value::as_str) == Some("invalid_grant") {
                    Err(RefreshTransportError::Terminal)
                } else {
                    Err(RefreshTransportError::Protocol)
                }
            }
            408 | 429 | 500..=599 => {
                Err(RefreshTransportError::Transient { retry_after: reply.retry_after.filter(|delay| delay.as_secs() <= MAX_RETRY_AFTER_SECONDS) })
            }
            _ => Err(RefreshTransportError::Protocol),
        }
    }
}

fn parse_success(body: &[u8]) -> Result<RefreshResponse, RefreshTransportError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| RefreshTransportError::Protocol)?;
    let access_token = required_secret(&value, "access_token")?;
    let refresh_token = required_secret(&value, "refresh_token")?;
    let provider_account_id = value
        .pointer("/user/id")
        .and_then(Value::as_str)
        .filter(|id| safe_token(id, MAX_ACCOUNT_BYTES))
        .ok_or(RefreshTransportError::Protocol)?
        .to_owned();
    let expires_in = value.get("expires_in").and_then(Value::as_u64).filter(|seconds| (1..=3600).contains(seconds)).map(Duration::from_secs);
    Ok(RefreshResponse { access_token, refresh_token, provider_account_id, expires_in })
}

fn required_secret(value: &Value, field: &str) -> Result<Secret, RefreshTransportError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && !has_control(text))
        .map(Secret::new)
        .ok_or(RefreshTransportError::Protocol)
}

/// The only network adapter. HTTPS-only, platform root certificates, no redirects, bounded time
/// and body. The refresh token is sent only to the fixed WorkOS token URL.
pub(crate) struct UreqRefreshHttp {
    agent: ureq::Agent,
}

impl UreqRefreshHttp {
    pub(crate) fn new() -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .timeout_connect(Some(REQUEST_TIMEOUT))
            .user_agent("intcad-account-refresh")
            .build()
            .into();
        Self { agent }
    }
}

impl RefreshHttp for UreqRefreshHttp {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<RefreshHttpReply, RefreshHttpError> {
        if url != WORKOS_TOKEN_URL {
            return Err(RefreshHttpError::Transport);
        }
        let _trace = crate::http::SuppressWireTrace::new();
        let mut response = self
            .agent
            .post(url)
            .header("Accept", "application/json")
            .header("Content-Type", content_type)
            .send(body)
            .map_err(map_refresh_http_error)?;
        let status = response.status().as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|value| value.to_str().ok()).and_then(parse_retry_after);
        let body = response.body_mut().with_config().limit(MAX_REFRESH_RESPONSE_BYTES as u64).read_to_vec().map_err(map_refresh_http_error)?;
        Ok(RefreshHttpReply { status, retry_after, body })
    }
}

pub(crate) fn map_refresh_http_error(error: ureq::Error) -> RefreshHttpError {
    match error {
        ureq::Error::BodyExceedsLimit(_) => RefreshHttpError::BodyTooLarge,
        _ => RefreshHttpError::Transport,
    }
}

/// Only an integer `Retry-After` in `0..=3600` seconds is usable; HTTP-date forms are ignored.
pub(crate) fn parse_retry_after(value: &str) -> Option<Duration> {
    let seconds: u64 = value.trim().parse().ok()?;
    (seconds <= MAX_RETRY_AFTER_SECONDS).then(|| Duration::from_secs(seconds))
}
