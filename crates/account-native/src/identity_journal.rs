//! Crash-safe, non-secret identity journal.
//!
//! This journal records identity refresh attempts and logout revocation as plain checksummed
//! files under `<root>/identity/`. It is deliberately separate from the OS keyring: it must stay
//! readable when the keyring is locked, and it never stores a token, secret or keyring value.
//!
//! Each record lives in one of two slots (`attempt.0`/`attempt.1`, `revoked.0`/`revoked.1`).
//! A slot is `intcad-identity-journal/1\n<sequence>\n<sha256-hex>\n<json>`. A write goes to the
//! slot with the lower sequence and is synced whole; readers accept only a matching magic,
//! checksum and JSON, and choose the highest sequence. A torn or partial write therefore leaves
//! the previous valid record readable in the other slot. Existing files with no valid slot are
//! [`JournalError::Corrupt`]; no files at all is `None`.
//!
//! Attempt ids are nonzero. [`IdentityJournal::clear_attempt`] writes a tombstone record with
//! `attempt_id = 0` as the newest sequence instead of deleting files, and [`read_attempt`] maps
//! it to `None`. A crash during clear leaves either the old attempt or the tombstone, never a
//! resurrected cleared attempt.
//!
//! [`read_attempt`]: IdentityJournal::read_attempt

// The renewal flow wires this module in later INT-1527 tasks; today only the tests exercise it.
#![cfg_attr(not(test), allow(dead_code))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAGIC: &str = "intcad-identity-journal/1";
const IDENTITY_DIR: &str = "identity";
const RECORD_VERSION: u32 = 1;
/// Slots hold small typed records; refuse oversized files before reading them.
const MAX_SLOT_BYTES: u64 = 64 * 1024;

/// One identity refresh attempt. Non-secret scheduling and backoff state only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttemptRecord {
    pub version: u32,
    pub domain: String,
    pub login_generation: u64,
    pub token_generation: u64,
    pub attempt_id: u64,
    pub first_possible_send_unix: u64,
    pub sends: u8,
    pub cooldown_until_unix: u64,
    pub cooldown_step: u8,
    pub diagnostic_used: bool,
}

/// A durable logout revocation marker. Non-secret; readable without any keyring.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RevocationRecord {
    pub version: u32,
    pub domain: String,
    pub login_generation: u64,
    pub revoked_at_unix: u64,
}

/// The journal could not be used. Messages are static and never quote file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JournalError {
    /// The directory or a slot file could not be created, read or written.
    Unavailable,
    /// A slot exists but no valid record could be recovered, or a record is invalid.
    Corrupt,
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("identity journal unavailable"),
            Self::Corrupt => formatter.write_str("identity journal corrupt"),
        }
    }
}

impl std::error::Error for JournalError {}

/// A durable identity journal rooted at a caller-supplied absolute directory.
pub(crate) struct IdentityJournal {
    dir: PathBuf,
}

impl IdentityJournal {
    /// Create `<root>/identity` (mode `0700` on Unix) and open the journal.
    ///
    /// The root must be absolute; the caller owns making it private to the current user.
    /// Any I/O failure is [`JournalError::Unavailable`].
    pub(crate) fn new(root: PathBuf) -> Result<Self, JournalError> {
        if !root.is_absolute() {
            return Err(JournalError::Unavailable);
        }
        let dir = root.join(IDENTITY_DIR);
        fs::create_dir_all(&dir).map_err(|_| JournalError::Unavailable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|_| JournalError::Unavailable)?;
        }
        Ok(Self { dir })
    }

    /// The newest valid attempt, or `None` when no attempt exists or the newest record is the
    /// clear tombstone (`attempt_id = 0`).
    pub(crate) fn read_attempt(&self) -> Result<Option<AttemptRecord>, JournalError> {
        match self.read_newest::<AttemptRecord>("attempt")? {
            Some(record) if record.attempt_id == 0 => Ok(None),
            record => Ok(record),
        }
    }

    /// Persist an attempt as the newest sequence. Attempt ids are nonzero; zero is reserved
    /// for the clear tombstone and is refused.
    pub(crate) fn write_attempt(&self, record: &AttemptRecord) -> Result<(), JournalError> {
        if record.attempt_id == 0 {
            return Err(JournalError::Corrupt);
        }
        self.write_newest("attempt", record)
    }

    /// Durably clear the attempt by writing a tombstone (`attempt_id = 0`) as the newest
    /// sequence. Never deletes slot files, so a crash cannot resurrect a cleared attempt.
    pub(crate) fn clear_attempt(&self) -> Result<(), JournalError> {
        let tombstone = AttemptRecord {
            version: RECORD_VERSION,
            domain: String::new(),
            login_generation: 0,
            token_generation: 0,
            attempt_id: 0,
            first_possible_send_unix: 0,
            sends: 0,
            cooldown_until_unix: 0,
            cooldown_step: 0,
            diagnostic_used: false,
        };
        self.write_newest("attempt", &tombstone)
    }

    /// The newest valid revocation marker, or `None` when none was ever written.
    pub(crate) fn read_revocation(&self) -> Result<Option<RevocationRecord>, JournalError> {
        self.read_newest("revoked")
    }

    /// Persist a revocation marker as the newest sequence.
    pub(crate) fn write_revocation(&self, record: &RevocationRecord) -> Result<(), JournalError> {
        self.write_newest("revoked", record)
    }

    fn slot_path(&self, stem: &str, index: usize) -> PathBuf {
        self.dir.join(format!("{stem}.{index}"))
    }

    /// Choose the highest-sequence valid slot. Existing files with no valid slot are corrupt;
    /// no files at all is `None`.
    fn read_newest<T: DeserializeOwned>(&self, stem: &str) -> Result<Option<T>, JournalError> {
        let mut best: Option<(u64, T)> = None;
        let mut any_file = false;
        for index in 0..2 {
            match read_slot::<T>(&self.slot_path(stem, index))? {
                Slot::Missing => {}
                Slot::Invalid => any_file = true,
                Slot::Valid { sequence, record } => {
                    any_file = true;
                    if best.as_ref().is_none_or(|(current, _)| sequence > *current) {
                        best = Some((sequence, record));
                    }
                }
            }
        }
        match best {
            Some((_, record)) => Ok(Some(record)),
            None if any_file => Err(JournalError::Corrupt),
            None => Ok(None),
        }
    }

    /// Write the record to the slot with the lower sequence (missing or invalid slots count as
    /// lower than any valid one), then sync. A crash mid-write leaves a torn slot that readers
    /// ignore in favour of the other slot's previous valid record.
    fn write_newest<T: Serialize>(&self, stem: &str, record: &T) -> Result<(), JournalError> {
        let json = serde_json::to_string(record).map_err(|_| JournalError::Unavailable)?;
        let mut sequences = [None, None];
        for (index, slot_sequence) in sequences.iter_mut().enumerate() {
            if let Slot::Valid { sequence, .. } = read_slot::<serde_json::Value>(&self.slot_path(stem, index))? {
                *slot_sequence = Some(sequence);
            }
        }
        let newest = sequences.iter().flatten().copied().max().unwrap_or(0);
        let target = match (sequences[0], sequences[1]) {
            (Some(first), Some(second)) if first > second => 1,
            (Some(_), None) => 1,
            _ => 0,
        };
        let payload = format!("{MAGIC}\n{}\n{}\n{json}", newest.saturating_add(1), sha256_hex(json.as_bytes()));
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(self.slot_path(stem, target)).map_err(|_| JournalError::Unavailable)?;
        file.write_all(payload.as_bytes()).and_then(|()| file.sync_all()).map_err(|_| JournalError::Unavailable)
    }
}

enum Slot<T> {
    Missing,
    Invalid,
    Valid { sequence: u64, record: T },
}

fn read_slot<T: DeserializeOwned>(path: &Path) -> Result<Slot<T>, JournalError> {
    // symlink_metadata: a symlink is never a valid slot, and a missing file is not an error.
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Slot::Missing),
        Err(_) => return Err(JournalError::Unavailable),
    };
    if !metadata.is_file() || metadata.len() > MAX_SLOT_BYTES {
        return Ok(Slot::Invalid);
    }
    let bytes = fs::read(path).map_err(|_| JournalError::Unavailable)?;
    Ok(match parse_slot(&bytes) {
        Some((sequence, record)) => Slot::Valid { sequence, record },
        None => Slot::Invalid,
    })
}

/// Parse `magic\nsequence\nsha256-hex\njson`; the checksum covers exactly the JSON bytes.
fn parse_slot<T: DeserializeOwned>(bytes: &[u8]) -> Option<(u64, T)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.splitn(4, '\n');
    if lines.next()? != MAGIC {
        return None;
    }
    let sequence: u64 = lines.next()?.parse().ok()?;
    let checksum = lines.next()?;
    let json = lines.next()?;
    if checksum != sha256_hex(json.as_bytes()) {
        return None;
    }
    let record = serde_json::from_str(json).ok()?;
    Some((sequence, record))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
