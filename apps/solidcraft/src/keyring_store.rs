//! Desktop account credentials in the operating-system store.
//!
//! The account keyring is separate from agent chat and isolated by product HTTPS origin.
//! No credential is written to a file or the ledger.

use std::fs::{File, OpenOptions};
use std::path::Path;

use solidcraft_account_native::{ACCOUNT_KEYRING_SERVICE, WireConnection};

/// Normalize the product origin once for both the transport and the keyring. Apply the wire
/// adapter's strict origin-only check before URL parsing can normalize a path away.
pub fn product_connection(origin: &str, client_version: &str) -> Option<WireConnection> {
    if origin.contains(['\\', '%']) {
        return None;
    }
    let validated = WireConnection::new(origin, client_version).ok()?;
    let parsed = url::Url::parse(validated.base_url()).ok()?;
    if parsed.port() == Some(0) {
        return None;
    }
    WireConnection::new(parsed.origin().ascii_serialization(), client_version).ok()
}

/// Stable across desktop aliases and client versions. The legacy unscoped service has no
/// trustworthy origin, so never read or migrate its entries (including logout tombstones).
pub fn account_keyring_service(product: &WireConnection) -> String {
    format!("{ACCOUNT_KEYRING_SERVICE} [{}]", product.base_url())
}

/// Provision the per-user lock also acquired by the CLI. Keep this independent of the product
/// origin, the desktop alias and settings overrides.
/// The caller retains the handle until its credential workers have stopped.
pub fn try_lock_account_namespace(directory: &Path) -> std::io::Result<Option<File>> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("account-keyring.lock");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() => return Err(std::io::ErrorKind::InvalidInput.into()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{account_keyring_service, product_connection, try_lock_account_namespace};

    #[test]
    fn product_namespace_is_stable_for_the_same_https_origin_across_versions() {
        let a = product_connection("https://Product.Example:00443/", "1").unwrap();
        let b = product_connection("https://product.example", "2").unwrap();
        assert_eq!(a.base_url(), b.base_url());
        assert_eq!(account_keyring_service(&a), account_keyring_service(&b));
        assert_ne!(account_keyring_service(&a), solidcraft_account_native::ACCOUNT_KEYRING_SERVICE);
        let different_port = product_connection("https://product.example:8443", "1").unwrap();
        assert_ne!(account_keyring_service(&a), account_keyring_service(&different_port));
    }

    #[test]
    fn product_connection_rejects_non_origin_and_malformed_configuration() {
        for origin in [
            "http://example.test",
            "https://example.test/.",
            "https://example.test/#",
            "https://example.test/?",
            "https://user@example.test",
            "https://example.test\\",
            "https://exa%6dple.test",
            "https://example.test:0",
            "https://example.test:65536",
        ] {
            assert!(product_connection(origin, "1").is_none(), "{origin}");
        }
    }

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
