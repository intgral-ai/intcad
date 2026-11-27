//! Storage shared by the native desktop and headless account compositions.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Per-user root for the account lock and durable operation ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountPaths(PathBuf);

impl AccountPaths {
    /// The desktop and CLI use the same OS identity and suffix, which makes their shared lock
    /// meaningful. `HOME`, `XDG_DATA_HOME`, `APPDATA`, and app settings overrides cannot move
    /// the lock, ledger, or account/keyring storage. An unavailable, relative or non-directory
    /// OS answer keeps the paid account path closed; there is no environment fallback.
    pub fn for_current_user() -> Option<Self> {
        Self::from_os_base(os_user_directory())
    }

    pub(crate) fn from_os_base(base: Option<PathBuf>) -> Option<Self> {
        let base = base.filter(|base| base.is_absolute() && base.is_dir())?;
        let suffix = if cfg!(target_os = "linux") {
            ".local/share/intcad"
        } else if cfg!(target_os = "macos") {
            "Library/Application Support/Intcad"
        } else if cfg!(windows) {
            "Intcad/data"
        } else {
            return None;
        };
        Some(Self(base.join(suffix)))
    }

    pub fn root(&self) -> &Path {
        &self.0
    }

    /// The headless host can only borrow the existing desktop layout, never provision it.
    ///
    /// Production callers must pass the root returned by [`Self::for_current_user`].
    pub fn lock_existing(root: &Path) -> Option<(PathBuf, File)> {
        if !root.is_absolute() {
            return None;
        }
        let root = root.canonicalize().ok()?;
        let journal = root.join("account");
        let lock_path = root.join("account-keyring.lock");
        if !std::fs::symlink_metadata(&lock_path).ok()?.is_file() || !std::fs::symlink_metadata(&journal).ok()?.is_dir() {
            return None;
        }
        let lock = OpenOptions::new().read(true).write(true).open(lock_path).ok()?;
        lock.try_lock().ok()?;
        Some((journal, lock))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn os_user_directory() -> Option<PathBuf> {
    use nix::unistd::{Uid, User};

    // Safe, reentrant getpwuid_r(getuid()) wrapper; returns an owned path from the OS record.
    User::from_uid(Uid::current()).ok().flatten().map(|user| user.dir)
}

#[cfg(windows)]
fn os_user_directory() -> Option<PathBuf> {
    // Safe SHGetKnownFolderPath(FOLDERID_RoamingAppData) wrapper; copies and frees the OS buffer.
    dirs_sys::known_folder_roaming_app_data()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn os_user_directory() -> Option<PathBuf> {
    None
}
