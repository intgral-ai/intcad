//! Desktop account credentials in the operating-system store.
//!
//! This is the thin desktop adapter: the account service (`"Intcad account"`) is separate from
//! the agent chat's `"SolidCraft agent chat"` service, and Task 11 owns the account runtime that
//! injects this session into the HTTP adapter. No credential is written to a file or the ledger.
#![allow(dead_code)]

use solidcraft_account_native::{CredentialSession, KeyringStore, ACCOUNT_KEYRING_SERVICE};

/// Keychain (macOS), Credential Manager (Windows), or Secret Service (Linux).
pub fn account_credential_session() -> CredentialSession {
    CredentialSession::new(Box::new(KeyringStore::new(ACCOUNT_KEYRING_SERVICE)))
}
