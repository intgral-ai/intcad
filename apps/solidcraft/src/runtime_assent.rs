//! Startup agreement for the Microsoft runtime a packaged copy carries.
//!
//! A copy that ships `mastra-agent\vc-runtime-receipt.json` is used only after the person agreed
//! to the shipped terms. The agreement is recorded for the exact runtime build (the receipt's
//! first file SHA-256) under the app's own data folder. It is never inferred from how the app was
//! started: a `--control` run reaches this gate like any other, and with nobody to answer it
//! stops instead of continuing.
//!
//! The MSI records the installer's own assent in `vc-runtime-accepted.json` beside the program:
//! its licence page, or the explicit `INTCAD_RUNTIME_TERMS=1` for a silent install, already
//! required agreement. A portable copy has no such record and asks once.

use solidcraft_ui_egui::runtime_terms::{LaunchMode, Startup, record_acceptance, startup_terms, terms_path};
use std::path::Path;

/// What this start may do about the runtime terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Assent {
    /// Nothing to ask: no packaged runtime, an installer record, or an earlier acceptance.
    Continue,
    /// The person did not agree, could not be asked, or the acceptance could not be recorded.
    Refuse(String),
}

/// Asking a person. Separate from the window so a start can be tested without one.
pub(crate) trait Prompt {
    /// Whether the person agreed, or `None` when this run cannot ask anyone.
    fn agree(&self, terms: &Path, summary: &str) -> Option<bool>;
}

/// The desktop's native dialog (`rfd`), also what a portable copy shows on first start. A dialog
/// this run cannot show (no session, no display) is not an agreement: it counts as a refusal.
pub(crate) struct NativePrompt;

impl Prompt for NativePrompt {
    fn agree(&self, terms: &Path, summary: &str) -> Option<bool> {
        let details = format!("{summary}\n\n{}", terms.display());
        // `YesNo` is the dialog the Windows build shows without the non-default
        // `common-controls-v6` feature; the text states exactly what Yes and No do.
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title("Intcad — Microsoft Visual C++ Runtime")
            .set_description(details)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        Some(matches!(answer, rfd::MessageDialogResult::Yes))
    }
}

/// The agreement text shown before assent. It is not the full terms: those are the shipped file,
/// and the dialog names it. Only the obligations a person must know to agree are here.
pub(crate) const SUMMARY: &str = "\
This copy of Intcad includes unmodified Microsoft Visual C++ Runtime files so the built-in \
Assistant works without a separate runtime installation.

They are licensed by Microsoft Corporation. Use them only as part of this copy of Intcad, \
without modification, and do not distribute them separately or remove their notices. If you \
pass this copy to someone else, pass these terms with it. Microsoft is a third-party \
beneficiary of those terms and may enforce them.

The full terms are in VC-RUNTIME-TERMS.txt in this installation folder's mastra-agent folder.

Choose Yes to accept these terms for this copy. Choose No to close Intcad without using it.";

/// Run the gate before the account runtime and the window exist. `data_dir` is where the person's
/// acceptance is recorded.
pub(crate) fn begin(exe_dir: &Path, data_dir: Option<&Path>, mode: LaunchMode, prompt: &dyn Prompt) -> Assent {
    match startup_terms(exe_dir, data_dir, mode) {
        Startup::Unpackaged | Startup::Accepted => Assent::Continue,
        Startup::MissingTerms { .. } => Assent::Refuse(
            "the installed copy is missing VC-RUNTIME-TERMS.txt beside its Microsoft runtime files, so the terms cannot be shown or accepted".into(),
        ),
        Startup::NeedsAcceptance { key } => {
            let Some(data_dir) = data_dir else {
                return Assent::Refuse("the Microsoft runtime terms cannot be recorded on this system".into());
            };
            let Some(terms) = terms_path(exe_dir) else {
                return Assent::Refuse("the installed copy is missing VC-RUNTIME-TERMS.txt beside its Microsoft runtime files".into());
            };
            match prompt.agree(&terms, SUMMARY) {
                Some(true) if record_acceptance(data_dir, &key) => Assent::Continue,
                Some(true) => Assent::Refuse("the Microsoft runtime terms could not be recorded".into()),
                Some(false) => Assent::Refuse("the Microsoft runtime terms were not accepted".into()),
                None => Assent::Refuse("the Microsoft runtime terms must be accepted before this copy is used".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_ui_egui::runtime_terms::accepted_in;
    use std::path::PathBuf;

    struct Scripted {
        answer: Option<bool>,
    }

    impl Prompt for Scripted {
        fn agree(&self, terms: &Path, summary: &str) -> Option<bool> {
            assert!(terms.is_file(), "the prompt is shown with the shipped terms: {}", terms.display());
            assert!(summary.contains("VC-RUNTIME-TERMS.txt"), "the summary names the shipped file");
            self.answer
        }
    }

    fn packaged_copy(name: &str, key: &str, terms: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("intcad-assent-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let agent = dir.join("mastra-agent");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::write(
            agent.join("vc-runtime-receipt.json"),
            serde_json::json!({ "schema": 1, "files": [{ "name": "VCRUNTIME140.dll", "sha256": key }] }).to_string(),
        )
        .unwrap();
        if terms {
            std::fs::write(agent.join("VC-RUNTIME-TERMS.txt"), "full terms\n").unwrap();
        }
        std::fs::create_dir_all(dir.join("data")).unwrap();
        dir
    }

    #[test]
    fn a_normal_start_and_a_control_start_ask_the_same_question() {
        let dir = packaged_copy("both", &"a".repeat(64), true);
        let data = dir.join("data");
        for mode in [LaunchMode::Normal, LaunchMode::Control] {
            assert_eq!(begin(&dir, Some(&data), mode, &Scripted { answer: Some(true) }), Assent::Continue, "{mode:?}");
            std::fs::remove_file(data.join("runtime-terms.json")).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_control_start_without_an_answer_stops_and_records_nothing() {
        let dir = packaged_copy("control-none", &"b".repeat(64), true);
        let data = dir.join("data");
        let outcome = begin(&dir, Some(&data), LaunchMode::Control, &Scripted { answer: None });
        assert!(matches!(outcome, Assent::Refuse(_)), "{outcome:?}");
        assert!(!accepted_in(&data, &"b".repeat(64), "runtime-terms.json"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn declining_stops_the_start_and_records_nothing() {
        let dir = packaged_copy("decline", &"c".repeat(64), true);
        let data = dir.join("data");
        let outcome = begin(&dir, Some(&data), LaunchMode::Normal, &Scripted { answer: Some(false) });
        assert!(matches!(outcome, Assent::Refuse(_)), "{outcome:?}");
        assert!(!accepted_in(&data, &"c".repeat(64), "runtime-terms.json"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_earlier_acceptance_or_an_installer_record_is_not_asked_again() {
        let dir = packaged_copy("accepted", &"d".repeat(64), true);
        let data = dir.join("data");
        assert!(record_acceptance(&data, &"d".repeat(64)));
        assert_eq!(begin(&dir, Some(&data), LaunchMode::Control, &Scripted { answer: None }), Assent::Continue);
        std::fs::remove_file(data.join("runtime-terms.json")).unwrap();
        std::fs::write(dir.join("vc-runtime-accepted.json"), serde_json::json!({ "receipt": "d".repeat(64), "channel": "msi" }).to_string()).unwrap();
        assert_eq!(begin(&dir, Some(&data), LaunchMode::Control, &Scripted { answer: None }), Assent::Continue);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_without_its_shipped_terms_stops_before_asking() {
        let dir = packaged_copy("terms-missing", &"e".repeat(64), false);
        let data = dir.join("data");
        let outcome = begin(&dir, Some(&data), LaunchMode::Normal, &Scripted { answer: Some(true) });
        assert!(matches!(outcome, Assent::Refuse(ref why) if why.contains("VC-RUNTIME-TERMS.txt")), "{outcome:?}");
        assert!(!accepted_in(&data, &"e".repeat(64), "runtime-terms.json"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_without_the_runtime_asks_nothing() {
        let dir = std::env::temp_dir().join(format!("intcad-assent-{}-plain", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mastra-agent")).unwrap();
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        assert_eq!(begin(&dir, Some(&data), LaunchMode::Control, &Scripted { answer: None }), Assent::Continue);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
