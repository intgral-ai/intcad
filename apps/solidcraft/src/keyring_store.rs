//! Desktop account credentials in the operating-system store.
//!
//! This is the thin desktop adapter: the account service (`"Intcad account"`) is separate from
//! the agent chat's `"SolidCraft agent chat"` service, and Task 11 owns the account runtime that
//! injects this session into the HTTP adapter. No credential is written to a file or the ledger.
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::path::Path;

use solidcraft_account_native::{ACCOUNT_KEYRING_SERVICE, CredentialSession, KeyringStore};

/// Serialize access by desktop processes that share the per-user keyring namespace.
/// The caller retains the handle until its credential workers have stopped.
pub fn try_lock_account_namespace(directory: &Path) -> std::io::Result<Option<File>> {
    std::fs::create_dir_all(directory)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(directory.join("account-keyring.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

/// Keychain (macOS), Credential Manager (Windows), or Secret Service (Linux).
pub fn account_credential_session() -> CredentialSession {
    CredentialSession::new(Box::new(KeyringStore::new(ACCOUNT_KEYRING_SERVICE)))
}

#[cfg(test)]
mod tests {
    use super::try_lock_account_namespace;

    #[test]
    fn account_namespace_is_exclusive_until_its_owner_releases_it() {
        let directory = std::env::temp_dir().join(format!("intcad-account-keyring-{}", std::process::id()));
        let first = try_lock_account_namespace(&directory).unwrap().unwrap();
        assert!(try_lock_account_namespace(&directory).unwrap().is_none());
        drop(first);
        let second = try_lock_account_namespace(&directory).unwrap().unwrap();
        drop(second);
        std::fs::remove_file(directory.join("account-keyring.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
