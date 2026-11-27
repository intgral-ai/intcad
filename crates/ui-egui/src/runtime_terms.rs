//! The Microsoft Visual C++ runtime terms a packaged copy carries.
//!
//! A package that includes the runtime beside the built-in Agent ships `vc-runtime-receipt.json` and
//! the terms in `VC-RUNTIME-TERMS.txt`. The receipt's first file SHA-256 names that exact runtime
//! build, so an acceptance covers one build: an update that carries newer runtime files asks again.
//!
//! The terms are agreed to before the copy is used, never inferred from how it was started. A
//! `--control` start reaches the same gate as a normal one. Agreement is recorded in the app's
//! own data folder, except for an installed copy: the MSI's licence page or the explicit
//! `INTCAD_RUNTIME_TERMS=1` silent property is that assent, and the installer leaves
//! `vc-runtime-accepted.json` beside the program for the build it installed.

use std::path::{Path, PathBuf};

const RECEIPT: &str = "vc-runtime-receipt.json";
const STATE: &str = "runtime-terms.json";
const TERMS: &str = "VC-RUNTIME-TERMS.txt";
const INSTALLER_RECORD: &str = "vc-runtime-accepted.json";

/// How this start was asked for. It decides nothing about assent: both values must meet the
/// same gate, and a control run with nobody to answer stops instead of continuing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    Normal,
    Control,
}

impl LaunchMode {
    fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Control => "control",
        }
    }
}

/// What a start must do about the packaged runtime's terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Startup {
    /// No packaged runtime: nothing to present or record.
    Unpackaged,
    /// This build was accepted on this machine (or by the installer that placed it).
    Accepted,
    /// This copy carries a build nobody has accepted yet.
    NeedsAcceptance { key: String },
    /// This copy carries a build whose shipped terms file is missing: there is nothing to agree to.
    MissingTerms { key: String },
}

/// The runtime build this copy carries, from its receipt. `None`: no packaged runtime.
fn carried_runtime(agent_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(agent_dir.join(RECEIPT)).ok()?;
    let receipt: serde_json::Value = serde_json::from_str(&text).ok()?;
    let digest = receipt["files"].as_array()?.first()?["sha256"].as_str()?;
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then(|| digest.to_ascii_lowercase())
}

/// Whether a record file names this runtime build as accepted.
fn record_names(data_dir: &Path, key: &str, file: &str) -> bool {
    let path = data_dir.join(file);
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|value| value["receipt"].as_str().map(str::to_owned)).as_deref() == Some(key)
}

/// Whether this machine accepted a runtime build. The installer's record and a person's agreement
/// are both kept as `{ "receipt": <the receipt's first file hash> }`.
pub fn accepted_in(data_dir: &Path, key: &str, file: &str) -> bool {
    record_names(data_dir, key, file)
}

/// Record a person's agreement for one runtime build. `false`: it could not be written, and the
/// start must then be refused rather than run unrecorded.
pub fn record_acceptance(data_dir: &Path, key: &str) -> bool {
    if std::fs::create_dir_all(data_dir).is_err() {
        return false;
    }
    let text = serde_json::to_string(&serde_json::json!({
        "receipt": key,
        "acceptedAt": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    }));
    match text {
        Ok(text) => std::fs::write(data_dir.join(STATE), text).is_ok(),
        Err(_) => false,
    }
}

/// What this start must do about the runtime terms carried beside `exe_dir`.
///
/// `data_dir` is the app's own data folder; without one nothing can be recorded, so a packaged
/// copy still asks and its caller must refuse if the answer cannot be kept.
pub fn startup_terms(exe_dir: &Path, data_dir: Option<&Path>, mode: LaunchMode) -> Startup {
    let agent_dir = exe_dir.join("mastra-agent");
    let Some(key) = carried_runtime(&agent_dir) else { return Startup::Unpackaged };
    if !agent_dir.join(TERMS).is_file() {
        solidcraft_engine::note!("runtime terms: the copy carries a Microsoft runtime but not its terms file; a {} start refuses", mode.label());
        return Startup::MissingTerms { key };
    }
    if record_names(exe_dir, &key, INSTALLER_RECORD) {
        return Startup::Accepted;
    }
    if data_dir.is_some_and(|dir| record_names(dir, &key, STATE)) {
        return Startup::Accepted;
    }
    solidcraft_engine::note!("runtime terms: a {} start must accept the Microsoft runtime terms for this copy", mode.label());
    Startup::NeedsAcceptance { key }
}

/// The shipped terms text beside a packaged copy. The startup gate shows where it is; `None`
/// means the copy is packaged without it, which is refused rather than accepted blind.
pub fn terms_path(exe_dir: &Path) -> Option<PathBuf> {
    let path = exe_dir.join("mastra-agent").join(TERMS);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_receipt_names_the_runtime_build_it_carries() {
        let dir = std::env::temp_dir().join(format!("intcad-runtime-terms-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let digest = "a".repeat(64);
        std::fs::write(
            dir.join(RECEIPT),
            serde_json::json!({ "schema": 1, "files": [{ "name": "VCRUNTIME140.dll", "sha256": digest }] }).to_string(),
        )
        .unwrap();
        assert_eq!(carried_runtime(&dir).as_deref(), Some(digest.as_str()));
        std::fs::write(dir.join(RECEIPT), "{}").unwrap();
        assert_eq!(carried_runtime(&dir), None, "a receipt without files asks for nothing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_acceptance_covers_only_the_runtime_build_it_names() {
        // The record names the accepted build; another build asks again.
        let data = std::env::temp_dir().join(format!("intcad-runtime-accepted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let key = "b".repeat(64);
        let other = "c".repeat(64);
        assert!(record_acceptance(&data, &key), "the acceptance is written");
        assert!(accepted_in(&data, &key, STATE));
        assert!(!accepted_in(&data, &other, STATE));
        std::fs::remove_dir_all(&data).unwrap();
    }

    /// A fresh install folder: `mastra-agent` with the receipt, and the terms file unless asked
    /// to leave it out. Each test gets its own folder.
    fn packaged_copy(name: &str, key: &str, terms: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("intcad-terms-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let agent = dir.join("mastra-agent");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::write(agent.join(RECEIPT), serde_json::json!({ "schema": 1, "files": [{ "name": "VCRUNTIME140.dll", "sha256": key }] }).to_string())
            .unwrap();
        if terms {
            std::fs::write(agent.join(TERMS), "the shipped terms\n").unwrap();
        }
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        dir
    }

    fn fresh_data(dir: &Path) -> PathBuf {
        let data = dir.with_file_name(format!("{}-data-accepted", dir.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        data
    }

    #[test]
    fn an_unaccepted_packaged_copy_needs_acceptance_before_a_normal_or_a_control_start() {
        // A portable copy carries the runtime and nobody has agreed to its terms yet. Both ways of
        // starting reach the same gate: a control run is not a record of anyone's agreement.
        let dir = packaged_copy("both-modes", &"d".repeat(64), true);
        let data = fresh_data(&dir);
        for mode in [LaunchMode::Normal, LaunchMode::Control] {
            assert_eq!(startup_terms(&dir, Some(&data), mode), Startup::NeedsAcceptance { key: "d".repeat(64) }, "{mode:?} start must ask");
        }
        assert!(!data.join(STATE).exists(), "asking is not accepting");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn starting_through_the_control_channel_never_records_assent() {
        let dir = packaged_copy("control-record", &"e".repeat(64), true);
        let data = fresh_data(&dir);
        assert_eq!(startup_terms(&dir, Some(&data), LaunchMode::Control), Startup::NeedsAcceptance { key: "e".repeat(64) });
        assert!(!accepted_in(&data, &"e".repeat(64), STATE), "a control start wrote an acceptance");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn explicit_acceptance_covers_only_the_runtime_build_it_names() {
        let dir = packaged_copy("explicit", &"f".repeat(64), true);
        let data = fresh_data(&dir);
        assert_eq!(startup_terms(&dir, Some(&data), LaunchMode::Normal), Startup::NeedsAcceptance { key: "f".repeat(64) });
        assert!(record_acceptance(&data, &"f".repeat(64)));
        assert_eq!(startup_terms(&dir, Some(&data), LaunchMode::Normal), Startup::Accepted);
        assert_eq!(startup_terms(&dir, Some(&data), LaunchMode::Control), Startup::Accepted);
        // A new release carrying a newer runtime asks again.
        let newer = packaged_copy("explicit-newer", &"0".repeat(64), true);
        assert_eq!(startup_terms(&newer, Some(&data), LaunchMode::Normal), Startup::NeedsAcceptance { key: "0".repeat(64) });
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&newer).unwrap();
    }

    #[test]
    fn the_installer_records_assent_for_the_runtime_build_it_installed() {
        // The MSI's licence page or its explicit silent property is the assent; the installed
        // copy carries that record, so a per-machine install does not ask each user again.
        let dir = packaged_copy("installer", &"1".repeat(64), true);
        let data = fresh_data(&dir);
        std::fs::write(dir.join(INSTALLER_RECORD), serde_json::json!({ "receipt": "1".repeat(64), "channel": "msi" }).to_string()).unwrap();
        assert_eq!(startup_terms(&dir, Some(&data), LaunchMode::Control), Startup::Accepted);
        // The record names the build it was given for; another build's files ask again.
        std::fs::write(dir.join(INSTALLER_RECORD), serde_json::json!({ "receipt": "2".repeat(64), "channel": "msi" }).to_string()).unwrap();
        assert_eq!(startup_terms(&dir, Some(&data), LaunchMode::Normal), Startup::NeedsAcceptance { key: "1".repeat(64) });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_packaged_copy_without_the_shipped_terms_refuses_instead_of_asking() {
        // The agreement is the shipped text: without it there is nothing to present or accept.
        let dir = packaged_copy("terms-missing", &"3".repeat(64), false);
        let data = fresh_data(&dir);
        for mode in [LaunchMode::Normal, LaunchMode::Control] {
            assert_eq!(startup_terms(&dir, Some(&data), mode), Startup::MissingTerms { key: "3".repeat(64) }, "{mode:?}");
        }
        assert_eq!(terms_path(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_without_a_runtime_receipt_asks_nothing() {
        let dir = std::env::temp_dir().join(format!("intcad-terms-{}-plain", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mastra-agent")).unwrap();
        let data = fresh_data(&dir);
        assert_eq!(startup_terms(&dir, Some(&data), LaunchMode::Control), Startup::Unpackaged);
        assert_eq!(startup_terms(&dir, None, LaunchMode::Normal), Startup::Unpackaged);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
