//! The Microsoft Visual C++ runtime terms a packaged copy carries.
//!
//! A package that includes the runtime beside the built-in Agent ships `vc-runtime-receipt.json`.
//! Its first file's SHA-256 names that exact runtime build, so accepting the terms once covers
//! that build only: an Intcad update that carries newer runtime files asks again. The acceptance
//! is recorded under the app's own data folder, never in the design or the shared preferences.
//!
//! A machine that installed the MSI accepted the same terms in the installer; the dialog is the
//! portable and per-machine first-run record for the person using this copy.

use crate::{SolveApp, ui_kit};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const RECEIPT: &str = "vc-runtime-receipt.json";

/// A run driven through `--control` is automation: its operator answers for the machine and the
/// install-time terms already covered an installed copy, so the first-run window is not drawn.
static AUTOMATED: AtomicBool = AtomicBool::new(false);

pub fn set_automated(automated: bool) {
    AUTOMATED.store(automated, Ordering::Relaxed);
}
const STATE: &str = "runtime-terms.json";

/// The runtime build this copy carries, from its receipt. `None`: no packaged runtime.
fn carried_runtime(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(RECEIPT)).ok()?;
    let receipt: serde_json::Value = serde_json::from_str(&text).ok()?;
    let digest = receipt["files"].as_array()?.first()?["sha256"].as_str()?;
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then(|| digest.to_ascii_lowercase())
}

fn state_path() -> Option<PathBuf> {
    solidcraft_engine::recovery::data_dir().map(|dir| dir.join(STATE))
}

fn accepted(key: &str) -> bool {
    let Some(path) = state_path() else { return false };
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|value| value["receipt"].as_str().map(str::to_owned)).as_deref() == Some(key)
}

fn remember(key: &str) -> bool {
    let Some(path) = state_path() else { return false };
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    serde_json::to_string(&serde_json::json!({ "receipt": key, "acceptedAt": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) }))
        .ok()
        .and_then(|text| std::fs::write(&path, text).ok())
        .is_some()
}

/// Whether this copy must ask the person before it is used: it carries the runtime and no
/// acceptance for that exact build is recorded on this machine.
pub(crate) fn required(app: &SolveApp) -> bool {
    let _ = app;
    if AUTOMATED.load(Ordering::Relaxed) {
        return false;
    }
    let Some(exe) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) else { return false };
    carried_runtime(&exe.join("mastra-agent")).is_some_and(|key| !accepted(&key))
}

/// The first-run terms of the packaged Microsoft runtime. Agreeing records this build; closing
/// the window quits before the app is used.
pub(crate) fn show(app: &mut SolveApp, ctx: &egui::Context) {
    if !required(app) {
        return;
    }
    let Some(exe) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) else { return };
    let Some(key) = carried_runtime(&exe.join("mastra-agent")) else { return };
    let (closed, action) = ui_kit::Confirm::new(egui::Id::new("sc_runtime_terms"), "help", crate::i18n::tr_label("Microsoft Visual C++ Runtime"))
        .actions(crate::i18n::tr_label("I agree"), crate::i18n::tr_label("Quit"), None)
        .show(ctx, true, |ui| {
            for line in [
                "This copy of Intcad includes unmodified Microsoft Visual C++ Runtime files so the built-in Assistant works without a separate runtime installation.",
                "They are licensed by Microsoft Corporation under the terms in VC-RUNTIME-TERMS.txt in the installation folder. You may use them only with this copy of Intcad, without modification or separate distribution. Microsoft is a third-party beneficiary of those terms.",
                "By choosing I agree you accept those terms for this copy. Choosing Quit closes Intcad without using it.",
            ] {
                ui_kit::text(ui, crate::i18n::tr_label(line), ui_kit::Text::Note(line));
            }
        });
    if closed {
        if action == ui_kit::Action::Ok {
            let _ = remember(&key);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
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
        // The state file names the accepted build; another build asks again.
        let key = "b".repeat(64);
        let other = "c".repeat(64);
        assert!(remember(&key) || state_path().is_none(), "the acceptance is written when there is a data folder");
        if state_path().is_some() {
            assert!(accepted(&key));
            assert!(!accepted(&other));
        }
    }
}
