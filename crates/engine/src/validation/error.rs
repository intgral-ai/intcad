//! Contract-shaped errors for the validation port (Mastra contract §3.2).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The error codes the validation port can return. Unknown future codes never become success.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidRequest,
    InvalidCandidate,
    DependencyChanged,
    ProfileUnsupported,
    ReportMismatch,
    NotReady,
    NotFound,
    IdempotencyConflict,
    RequestExpired,
    ResourceExhausted,
    Timeout,
    Cancelled,
    OutcomeUnknown,
    InternalError,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidRequest => "INVALID_REQUEST",
            ErrorCode::InvalidCandidate => "INVALID_CANDIDATE",
            ErrorCode::DependencyChanged => "DEPENDENCY_CHANGED",
            ErrorCode::ProfileUnsupported => "PROFILE_UNSUPPORTED",
            ErrorCode::ReportMismatch => "REPORT_MISMATCH",
            ErrorCode::NotReady => "NOT_READY",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::IdempotencyConflict => "IDEMPOTENCY_CONFLICT",
            ErrorCode::RequestExpired => "REQUEST_EXPIRED",
            ErrorCode::ResourceExhausted => "RESOURCE_EXHAUSTED",
            ErrorCode::Timeout => "TIMEOUT",
            ErrorCode::Cancelled => "CANCELLED",
            ErrorCode::OutcomeUnknown => "OUTCOME_UNKNOWN",
            ErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }

    fn retry(self) -> &'static str {
        match self {
            ErrorCode::ResourceExhausted | ErrorCode::NotReady | ErrorCode::Timeout | ErrorCode::Cancelled | ErrorCode::OutcomeUnknown => {
                "same_submission"
            }
            ErrorCode::InvalidRequest
            | ErrorCode::InvalidCandidate
            | ErrorCode::DependencyChanged
            | ErrorCode::ProfileUnsupported
            | ErrorCode::ReportMismatch
            | ErrorCode::NotFound
            | ErrorCode::IdempotencyConflict
            | ErrorCode::RequestExpired
            | ErrorCode::InternalError => "never",
        }
    }
}

/// A validation-port error: a contract code plus redacted human-readable text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Value,
}

impl ValidationError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ValidationError { code, message: cap_message(message.into()), details: json!({}) }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    pub fn invalid_candidate(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidCandidate, message)
    }

    pub fn dependency_changed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DependencyChanged, message)
    }

    pub fn profile_unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ProfileUnsupported, message)
    }

    pub fn report_mismatch(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ReportMismatch, message)
    }

    pub fn not_ready(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotReady, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn idempotency_conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::IdempotencyConflict, message)
    }

    pub fn request_expired(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::RequestExpired, message)
    }

    pub fn resource_exhausted(message: impl Into<String>, resource: &str, limit: u64) -> Self {
        Self::new(ErrorCode::ResourceExhausted, message).with_details(json!({"resource": resource, "limit": limit}))
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Timeout, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Cancelled, message)
    }

    pub fn outcome_unknown(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::OutcomeUnknown, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }

    /// The report's `error` field (contract §3.2 `Error`).
    pub fn wire(&self) -> WireError {
        WireError {
            code: self.code.as_str().to_string(),
            message: self.message.clone(),
            retry: self.code.retry().to_string(),
            details: self.details.clone(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

/// The contract caps an error message at 2,048 UTF-8 bytes.
fn cap_message(mut message: String) -> String {
    const MAX: usize = 2_048;
    if message.len() <= MAX {
        return message;
    }
    let mut end = MAX;
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message
}

impl std::error::Error for ValidationError {}

/// The serialized error shape embedded in a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireError {
    pub code: String,
    pub message: String,
    pub retry: String,
    pub details: Value,
}
