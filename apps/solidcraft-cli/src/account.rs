//! Headless account composition.
//!
//! A normal program call never starts a device sign-in: it returns the stable `AUTH_REQUIRED`
//! refusal and no request is sent. Only an explicit command may call
//! [`begin_explicit_device_login`], and even then the caller displays the URL and code; this
//! program never opens a browser. The fallback session is process-local and is never persisted.
#![allow(dead_code)]

use solidcraft_account_native::{
    CredentialSession, DeviceCodeFlow, DeviceCodeMode, DeviceCodePrompt, MemorySession, NoPersistentStore, UreqDeviceCodeTransport,
    WorkOsPublicClient,
};
use solidcraft_engine::entitlement::{Refusal, RefusalCode};

/// A session that refuses every durable write: for a headless process with no secure store.
pub fn no_persistent_session() -> CredentialSession {
    CredentialSession::new(Box::new(NoPersistentStore))
}

/// The explicit short session: process memory only, gone when the process exits.
pub fn process_memory_session() -> CredentialSession {
    CredentialSession::new(Box::new(MemorySession::default()))
}

/// Ordinary program paths must use this and surface the refusal; they must not begin a flow.
pub fn programmatic_login() -> Result<DeviceCodePrompt, Refusal> {
    Err(Refusal::new(RefusalCode::AuthRequired, "an explicit sign-in command is required"))
}

/// Start an explicitly requested device-code sign-in. The caller polls and completes the returned
/// flow; the prompt contains the verification URL and user code, never the device code.
pub fn begin_explicit_device_login(client_id: &str) -> Result<(DeviceCodeFlow<UreqDeviceCodeTransport>, DeviceCodePrompt), Refusal> {
    let client = WorkOsPublicClient::new(client_id).ok_or_else(|| Refusal::new(RefusalCode::AuthRequired, "no WorkOS client id is configured"))?;
    let mut flow = DeviceCodeFlow::new(client, UreqDeviceCodeTransport::new(), process_memory_session(), DeviceCodeMode::Explicit);
    let prompt = flow.start()?;
    Ok((flow, prompt))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn programmatic_login_never_starts_a_flow() {
        assert_eq!(programmatic_login().unwrap_err().code, RefusalCode::AuthRequired);
    }

    #[test]
    fn explicit_login_without_configuration_is_refused_before_any_request() {
        assert_eq!(begin_explicit_device_login("").unwrap_err().code, RefusalCode::AuthRequired);
    }
}
