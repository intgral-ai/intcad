#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::identity_journal::{AttemptRecord, CurrentLoginRecord, IdentityJournal, JournalError, RevocationRecord};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A unique temp directory per test: process id plus an atomic counter, removed on drop.
struct TempRoot {
    root: PathBuf,
}

impl TempRoot {
    fn new(tag: &str) -> Self {
        let serial = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("solidcraft-identity-journal-{tag}-{}-{serial}", std::process::id()));
        Self { root }
    }

    fn journal(&self) -> IdentityJournal {
        IdentityJournal::new(self.root.clone(), "fixture").expect("open identity journal")
    }

    fn slot(&self, stem: &str, index: u8) -> PathBuf {
        self.root.join("identity/fixture").join(format!("{stem}.{index}"))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn attempt(attempt_id: u64) -> AttemptRecord {
    AttemptRecord {
        version: 1,
        domain: "https://account.example.test".to_owned(),
        login_generation: 7,
        token_generation: 3,
        attempt_id,
        first_possible_send_unix: 1_700_000_000,
        sends: 2,
        cooldown_until_unix: 1_700_000_060,
        cooldown_step: 1,
        diagnostic_used: false,
    }
}

fn revocation(revoked_at_unix: u64) -> RevocationRecord {
    RevocationRecord { version: 1, domain: "https://account.example.test".to_owned(), login_generation: 7, revoked_at_unix }
}

fn current(login_generation: u64) -> CurrentLoginRecord {
    CurrentLoginRecord { version: 1, domain: "https://account.example.test".to_owned(), login_generation }
}

#[test]
fn current_login_round_trips_across_reopen() {
    let temp = TempRoot::new("current-round-trip");
    let journal = temp.journal();
    assert_eq!(journal.read_current_login().unwrap(), None);
    journal.write_current_login(&current(7)).unwrap();
    assert_eq!(temp.journal().read_current_login().unwrap(), Some(current(7)));
    let slot = std::fs::read_to_string(temp.slot("current", 0)).unwrap();
    let fields: serde_json::Value = serde_json::from_str(slot.rsplit('\n').next().unwrap()).unwrap();
    assert_eq!(fields, serde_json::json!({"version":1,"domain":"https://account.example.test","login_generation":7}));
}

#[test]
fn the_newest_current_login_slot_wins() {
    let temp = TempRoot::new("current-newest");
    let journal = temp.journal();
    for generation in [7, 8, 9] {
        journal.write_current_login(&current(generation)).unwrap();
        assert_eq!(temp.journal().read_current_login().unwrap(), Some(current(generation)));
    }
}

#[test]
fn a_torn_current_login_slot_falls_back_to_the_previous_slot() {
    let temp = TempRoot::new("current-torn");
    let journal = temp.journal();
    journal.write_current_login(&current(7)).unwrap();
    journal.write_current_login(&current(8)).unwrap();
    std::fs::write(temp.slot("current", 1), b"intcad-identity-journal/1\n2\n").unwrap();
    assert_eq!(temp.journal().read_current_login().unwrap(), Some(current(7)));
    journal.write_current_login(&current(9)).unwrap();
    assert_eq!(temp.journal().read_current_login().unwrap(), Some(current(9)));
}

#[test]
fn attempt_and_revocation_records_round_trip() {
    let temp = TempRoot::new("round-trip");
    let journal = temp.journal();
    assert_eq!(journal.read_attempt().unwrap(), None);
    assert_eq!(journal.read_revocation().unwrap(), None);

    let attempt = attempt(42);
    journal.write_attempt(&attempt).unwrap();
    assert_eq!(journal.read_attempt().unwrap(), Some(attempt.clone()));

    let revocation = revocation(1_700_000_120);
    journal.write_revocation(&revocation).unwrap();
    assert_eq!(journal.read_revocation().unwrap(), Some(revocation.clone()));

    // A fresh instance over the same root reads the same records.
    let reopened = temp.journal();
    assert_eq!(reopened.read_attempt().unwrap(), Some(attempt));
    assert_eq!(reopened.read_revocation().unwrap(), Some(revocation));

    // The stored payload carries exactly the non-secret record fields, nothing else.
    let text = std::fs::read_to_string(temp.slot("attempt", 0)).unwrap();
    let payload = text.rsplit('\n').next().unwrap();
    let value: serde_json::Value = serde_json::from_str(payload).unwrap();
    let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "attempt_id",
            "cooldown_step",
            "cooldown_until_unix",
            "diagnostic_used",
            "domain",
            "first_possible_send_unix",
            "login_generation",
            "sends",
            "token_generation",
            "version",
        ]
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(temp.root.join("identity/fixture")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}

#[test]
fn the_newest_valid_sequence_wins() {
    let temp = TempRoot::new("newest-wins");
    let journal = temp.journal();
    journal.write_attempt(&attempt(1)).unwrap();
    journal.write_attempt(&attempt(2)).unwrap();
    assert_eq!(journal.read_attempt().unwrap().unwrap().attempt_id, 2);
    // The third write reuses the lower-sequence slot and must still win.
    journal.write_attempt(&attempt(3)).unwrap();
    assert_eq!(journal.read_attempt().unwrap().unwrap().attempt_id, 3);
    assert_eq!(temp.journal().read_attempt().unwrap().unwrap().attempt_id, 3);

    journal.write_revocation(&revocation(10)).unwrap();
    journal.write_revocation(&revocation(20)).unwrap();
    assert_eq!(journal.read_revocation().unwrap().unwrap().revoked_at_unix, 20);
}

#[test]
fn a_truncated_newest_slot_falls_back_to_the_previous_slot() {
    let temp = TempRoot::new("truncated-newest");
    let journal = temp.journal();
    journal.write_attempt(&attempt(11)).unwrap();
    journal.write_attempt(&attempt(12)).unwrap();

    // Simulate a torn write: the newest slot keeps only a prefix of its bytes.
    let newest = temp.slot("attempt", 1);
    let bytes = std::fs::read(&newest).unwrap();
    std::fs::write(&newest, &bytes[..bytes.len() / 2]).unwrap();
    assert_eq!(journal.read_attempt().unwrap().unwrap().attempt_id, 11);

    // A later write repairs the pair: it targets the torn slot and becomes the newest.
    journal.write_attempt(&attempt(13)).unwrap();
    assert_eq!(journal.read_attempt().unwrap().unwrap().attempt_id, 13);
}

#[test]
fn two_corrupt_slots_are_refused() {
    let temp = TempRoot::new("two-corrupt");
    let journal = temp.journal();
    std::fs::write(temp.slot("attempt", 0), b"not a journal slot").unwrap();
    std::fs::write(temp.slot("attempt", 1), b"").unwrap();
    assert!(matches!(journal.read_attempt(), Err(JournalError::Corrupt)));

    // One corrupt file with no valid sibling is refused too, not reported as absent.
    let single = TempRoot::new("one-corrupt");
    let journal = single.journal();
    std::fs::write(single.slot("attempt", 0), b"intcad-identity-journal/1\n9\n").unwrap();
    assert!(matches!(journal.read_attempt(), Err(JournalError::Corrupt)));

    // A payload that no longer matches its checksum is corrupt as well.
    let tampered = TempRoot::new("bad-checksum");
    let journal = tampered.journal();
    journal.write_attempt(&attempt(5)).unwrap();
    let path = tampered.slot("attempt", 0);
    let text = std::fs::read_to_string(&path).unwrap();
    let edited = text.replace("\"sends\":2", "\"sends\":3");
    assert_ne!(edited, text);
    std::fs::write(&path, edited).unwrap();
    assert!(matches!(journal.read_attempt(), Err(JournalError::Corrupt)));

    // No files at all remains None.
    let absent = TempRoot::new("absent");
    assert_eq!(absent.journal().read_attempt().unwrap(), None);
}

#[test]
fn a_cleared_attempt_stays_cleared_after_a_truncated_slot() {
    let temp = TempRoot::new("cleared-stays");
    let journal = temp.journal();
    journal.write_attempt(&attempt(21)).unwrap();
    journal.clear_attempt().unwrap();
    assert_eq!(journal.read_attempt().unwrap(), None);

    // Truncate the slot holding the pre-clear attempt: the tombstone must still win,
    // so a cleared attempt cannot be resurrected by a torn older slot.
    let old = temp.slot("attempt", 0);
    let bytes = std::fs::read(&old).unwrap();
    std::fs::write(&old, &bytes[..8]).unwrap();
    assert_eq!(journal.read_attempt().unwrap(), None);
    assert_eq!(temp.journal().read_attempt().unwrap(), None);

    // A new attempt after a clear is visible again; the tombstone is not sticky.
    journal.write_attempt(&attempt(22)).unwrap();
    assert_eq!(journal.read_attempt().unwrap().unwrap().attempt_id, 22);

    // Clearing an empty journal is a durable no-attempt state too.
    let empty = TempRoot::new("clear-empty");
    empty.journal().clear_attempt().unwrap();
    assert_eq!(empty.journal().read_attempt().unwrap(), None);
}

#[test]
fn a_revocation_record_is_readable_without_any_keyring() {
    let temp = TempRoot::new("no-keyring");
    let journal = temp.journal();
    journal.write_revocation(&revocation(1_700_000_999)).unwrap();

    // The journal is plain checksummed files: a fresh instance reads it with no
    // credential store, keyring handle or unlock involved.
    let reopened = temp.journal();
    assert_eq!(reopened.read_revocation().unwrap(), Some(revocation(1_700_000_999)));

    // Revocation is independent of the attempt slots and survives attempt clears.
    journal.write_attempt(&attempt(31)).unwrap();
    journal.clear_attempt().unwrap();
    assert_eq!(journal.read_revocation().unwrap(), Some(revocation(1_700_000_999)));

    // The newest-valid-sequence fallback applies to revocation slots as well.
    journal.write_revocation(&revocation(2_000_000_000)).unwrap();
    let newest = temp.slot("revoked", 1);
    let bytes = std::fs::read(&newest).unwrap();
    std::fs::write(&newest, &bytes[..4]).unwrap();
    assert_eq!(journal.read_revocation().unwrap(), Some(revocation(1_700_000_999)));
}

#[test]
fn a_torn_clear_write_leaves_the_previous_attempt() {
    let temp = TempRoot::new("torn-clear");
    let journal = temp.journal();
    journal.write_attempt(&attempt(41)).unwrap();
    // Simulate a crash mid-clear: the tombstone slot holds only a header prefix.
    // The visible state is the old attempt or the tombstone, never a half record.
    std::fs::write(temp.slot("attempt", 1), b"intcad-identity-journal/1\n2\n").unwrap();
    assert_eq!(journal.read_attempt().unwrap().unwrap().attempt_id, 41);
    // Completing the clear afterwards makes the tombstone the newest record.
    journal.clear_attempt().unwrap();
    assert_eq!(journal.read_attempt().unwrap(), None);
}

#[test]
fn a_relative_root_is_refused() {
    assert!(matches!(IdentityJournal::new(PathBuf::from("relative-identity-root"), "fixture"), Err(JournalError::Unavailable)));
}

#[test]
fn write_attempt_refuses_a_zero_attempt_id() {
    let temp = TempRoot::new("zero-id");
    let journal = temp.journal();
    assert!(matches!(journal.write_attempt(&attempt(0)), Err(JournalError::Corrupt)));
    assert_eq!(journal.read_attempt().unwrap(), None);
}
#[test]
fn oversized_slot_and_sequence_overflow_are_rejected() {
    let temp = TempRoot::new("oversized-overflow");
    let journal = temp.journal();
    let slot = temp.slot("current", 0);
    journal.write_current_login(&current(7)).unwrap();
    let original = std::fs::read_to_string(&slot).unwrap();
    std::fs::write(&slot, original.replacen("\n1\n", &format!("\n{}\n", u64::MAX), 1)).unwrap();
    assert_eq!(journal.read_current_login().unwrap(), Some(current(7)));
    assert_eq!(journal.write_current_login(&current(8)), Err(JournalError::Corrupt));
    std::fs::write(&slot, vec![0; 65_537]).unwrap();
    assert_eq!(journal.read_current_login(), Err(JournalError::Corrupt));
}

#[cfg(unix)]
#[test]
fn public_and_symlink_slots_are_rejected_and_replacement_does_not_follow_the_link() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = TempRoot::new("hostile");
    let journal = temp.journal();
    let slot = temp.slot("current", 0);
    journal.write_current_login(&current(7)).unwrap();
    std::fs::set_permissions(&slot, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(journal.read_current_login(), Err(JournalError::Corrupt));
    std::fs::set_permissions(&slot, std::fs::Permissions::from_mode(0o600)).unwrap();
    let outside = temp.root.join("outside");
    std::fs::rename(&slot, &outside).unwrap();
    let original = std::fs::read(&outside).unwrap();
    symlink(&outside, &slot).unwrap();
    assert_eq!(journal.read_current_login(), Err(JournalError::Corrupt));
    journal.write_current_login(&current(8)).unwrap();
    assert_eq!(journal.read_current_login().unwrap(), Some(current(8)));
    assert_eq!(std::fs::read(&outside).unwrap(), original);
}
