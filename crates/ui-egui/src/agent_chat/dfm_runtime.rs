//! The host side of automatic DFM checks (PIL-305).
//!
//! An AI round's edits trigger at most one run, after the edits are actually committed. Direct
//! multi-step modelling commits during the round; PIL-300's preview accepts after the reply ended.
//! Both arrive here as one commit event keyed by `(document, AI turn, committed model revision)`,
//! so a duplicate accept notification runs nothing twice, and nothing is scheduled from the end of
//! a reply alone or from every tool call.
//!
//! The actual submission (capture, freeze, `ValidationPort`) is [`RunHost`]. The desktop host
//! installs the project-backed one; tests install a recorder. A new edit cancels the running check
//! and a late result can never overwrite a newer one: the run record's own submission identity
//! decides that, not this module.

use std::collections::VecDeque;

/// Why a check was submitted.
#[allow(dead_code)] // manual and accepted-preview triggers are wired when the project host lands
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// A person pressed Run DFM.
    Manual,
    /// An AI round's committed edits.
    Edits { document_id: String, turn: String, revision: String },
    /// A PIL-300 preview the user accepted and whose apply succeeded.
    AcceptedPreview { document_id: String, turn: String, revision: String },
}

/// The host that captures the snapshot, freezes the project and submits the check.
///
/// `submit` must not block the window thread on the check itself: it captures and queues, and the
/// run completes on a worker. `Ok(None)` means there is nothing to check (no project, no model
/// change, an empty design); it is not an error.
pub(crate) trait RunHost {
    fn submit(&mut self, trigger: &Trigger) -> Result<Option<String>, String>;
    fn cancel(&mut self, run_id: &str);
}

/// How many committed round keys are remembered for duplicate suppression.
const REMEMBERED_COMMITS: usize = 64;

#[allow(dead_code)] // the host adapter and Run DFM entry land with the project module
#[derive(Clone, Debug)]
struct ActiveRun {
    run_id: String,
    document_id: String,
    revision: u64,
}

/// A round whose commit is being held until the design stops changing.
#[derive(Clone, Debug)]
struct Settling {
    document_id: String,
    turn: String,
    trigger: Trigger,
    /// The last revision seen; the run fires once one frame sees it unchanged.
    revision: u64,
    /// A design change was still being applied when the reply ended, so a later revision belongs
    /// to this round. When false, any further revision is a person's edit and runs nothing.
    deferred: bool,
}

/// Automatic-check state for one conversation and its document.
#[allow(dead_code)] // install/manual/active are the host seam waiting for the project module
#[derive(Default)]
pub(crate) struct HostDfm {
    host: Option<Box<dyn RunHost>>,
    /// The project's `ci.toml` switch. Off means no automatic runs; manual runs still work.
    auto_enabled: bool,
    /// Recently fired `(document, turn, revision)` keys, oldest first.
    committed: VecDeque<String>,
    active: Option<ActiveRun>,
    settling: Option<Settling>,
    last_seen: Option<(String, u64)>,
    manual_serial: u64,
    last_error: Option<String>,
}

#[allow(dead_code)] // the host seam's entry points are used once the project module lands
impl HostDfm {
    /// Install the project-backed host. Replacing it drops the previous host; its running check is
    /// cancelled first so the worker does not outlive the project it belongs to.
    pub(crate) fn install(&mut self, mut host: Box<dyn RunHost>) {
        if let Some(active) = self.active.take() {
            host.cancel(&active.run_id);
        }
        self.host = Some(host);
        self.settling = None;
    }

    pub(crate) fn is_installed(&self) -> bool {
        self.host.is_some()
    }

    /// Read the project's automatic-check switch.
    pub(crate) fn set_auto_enabled(&mut self, enabled: bool) {
        self.auto_enabled = enabled;
        if !enabled {
            self.settling = None;
        }
    }

    pub(crate) fn auto_enabled(&self) -> bool {
        self.auto_enabled
    }

    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// The run the host is waiting for, if one is live.
    pub(crate) fn active(&self) -> Option<(&str, &str)> {
        self.active.as_ref().map(|active| (active.run_id.as_str(), active.document_id.as_str()))
    }

    /// A person pressed Run DFM.
    pub(crate) fn manual(&mut self, document_id: &str, revision: u64) -> bool {
        self.manual_serial = self.manual_serial.wrapping_add(1);
        let key = format!("manual|{document_id}|{revision}|{}", self.manual_serial);
        self.fire(key, Trigger::Manual, document_id, revision)
    }

    /// The AI's reply ended. `changed` is true only when this round actually committed a design
    /// change; a pure answer, a rejected edit or a preview that is still only a preview is false.
    /// `pending` says that the window still has a design change being applied (off the window
    /// thread), so a later revision belongs to this round; a person's edit after a settled reply
    /// is not that.
    pub(crate) fn on_turn_end(&mut self, document_id: &str, turn: &str, revision: u64, changed: bool, pending: bool) {
        if !changed || !self.auto_enabled || self.host.is_none() {
            return;
        }
        // The commit may still be being applied off the window thread. Hold the round until one
        // frame sees the same revision and no work in flight.
        self.settling = Some(Settling {
            document_id: document_id.to_string(),
            turn: turn.to_string(),
            trigger: Trigger::Edits { document_id: document_id.to_string(), turn: turn.to_string(), revision: revision.to_string() },
            revision,
            deferred: pending,
        });
    }

    /// A PIL-300 preview was accepted and applying it succeeded. The reply has already ended, so
    /// this is a separate commit event for the same turn.
    pub(crate) fn on_preview_accepted(&mut self, document_id: &str, turn: &str, revision: u64) {
        if !self.auto_enabled || self.host.is_none() {
            return;
        }
        let key = commit_key(document_id, turn, revision);
        let trigger = Trigger::AcceptedPreview { document_id: document_id.to_string(), turn: turn.to_string(), revision: revision.to_string() };
        self.fire(key, trigger, document_id, revision);
    }

    /// A rejected preview, a failed accept or an accept refused because the user edited the design
    /// meanwhile. It commits nothing, so it runs nothing.
    pub(crate) fn on_preview_not_committed(&mut self, _document_id: &str, _turn: &str) {}

    /// Called once per UI frame. Cancels a check whose model is no longer the live one, and fires a
    /// held round once the design is quiet.
    pub(crate) fn observe(&mut self, document_id: &str, revision: u64, idle: bool) {
        let changed_document = self.last_seen.as_ref().is_some_and(|(document, _)| document != document_id);
        let changed_revision = self.last_seen.as_ref().is_some_and(|(_, seen)| *seen != revision);
        self.last_seen = Some((document_id.to_string(), revision));
        if changed_document {
            self.cancel_active();
            self.settling = None;
            return;
        }
        if changed_revision || changed_document {
            // A new edit makes the old report stale and stops its check; a late result cannot take
            // `latest` back because the run record rejects a superseded submission. The check
            // started for this very revision is not stale.
            let stale = self.active.as_ref().is_some_and(|active| active.revision != revision || active.document_id != document_id);
            if stale {
                self.cancel_active();
            }
        }
        let Some(settling) = self.settling.clone() else { return };
        if settling.document_id != document_id {
            self.settling = None;
            return;
        }
        if settling.revision != revision {
            if !settling.deferred {
                // The reply had already committed everything it changed; a further revision is a
                // person's edit or undo, which only makes the old report stale.
                self.settling = None;
                return;
            }
            // More edits landed after the reply: hold the round for the newest model.
            if let Some(settling) = self.settling.as_mut() {
                settling.revision = revision;
                settling.trigger =
                    Trigger::Edits { document_id: document_id.to_string(), turn: settling.turn.clone(), revision: revision.to_string() };
            }
            return;
        }
        if !idle {
            return;
        }
        self.settling = None;
        let key = commit_key(document_id, &settling.turn, revision);
        self.fire(key, settling.trigger, document_id, revision);
    }

    /// Stop the live check without starting another (project closed, chat disconnected).
    pub(crate) fn cancel_active(&mut self) {
        let Some(active) = self.active.take() else { return };
        if let Some(host) = self.host.as_mut() {
            host.cancel(&active.run_id);
        }
    }

    fn fire(&mut self, key: String, trigger: Trigger, document_id: &str, revision: u64) -> bool {
        if !self.remember(&key) || self.host.is_none() {
            return false;
        }
        self.cancel_active();
        let Some(host) = self.host.as_mut() else { return false };
        match host.submit(&trigger) {
            Ok(Some(run_id)) => {
                self.last_error = None;
                self.active = Some(ActiveRun { run_id, document_id: document_id.to_string(), revision });
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.last_error = Some(error);
                false
            }
        }
    }

    /// `true` the first time this round/revision is fired.
    fn remember(&mut self, key: &str) -> bool {
        if self.committed.iter().any(|seen| seen == key) {
            return false;
        }
        self.committed.push_back(key.to_string());
        while self.committed.len() > REMEMBERED_COMMITS {
            self.committed.pop_front();
        }
        true
    }
}

fn commit_key(document_id: &str, turn: &str, revision: u64) -> String {
    format!("{document_id}|{turn}|{revision}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Recorder {
        submits: Arc<Mutex<Vec<Trigger>>>,
        cancels: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    impl RunHost for Recorder {
        fn submit(&mut self, trigger: &Trigger) -> Result<Option<String>, String> {
            if self.fail {
                return Err("no project is open".into());
            }
            let mut submits = self.submits.lock().unwrap();
            submits.push(trigger.clone());
            Ok(Some(format!("run-{:016}-{:016}", submits.len(), submits.len())))
        }

        fn cancel(&mut self, run_id: &str) {
            self.cancels.lock().unwrap().push(run_id.to_string());
        }
    }

    fn runtime() -> (HostDfm, Recorder) {
        let recorder = Recorder::default();
        let mut dfm = HostDfm::default();
        dfm.install(Box::new(recorder.clone()));
        dfm.set_auto_enabled(true);
        (dfm, recorder)
    }

    fn submits(recorder: &Recorder) -> Vec<Trigger> {
        recorder.submits.lock().unwrap().clone()
    }

    /// Direct multi-step modelling: three tool commits in one round are one run.
    #[test]
    fn a_direct_multi_step_round_runs_once_after_its_commits() {
        let (mut dfm, recorder) = runtime();
        dfm.observe("doc-1", 10, true);
        // The last edit of the round lands, then the reply ends with a changed design.
        dfm.observe("doc-1", 13, true);
        dfm.on_turn_end("doc-1", "turn-1", 13, true, false);
        dfm.observe("doc-1", 13, false);
        assert!(submits(&recorder).is_empty(), "no run while work is in flight");
        dfm.observe("doc-1", 13, true);
        assert_eq!(submits(&recorder), [Trigger::Edits { document_id: "doc-1".into(), turn: "turn-1".into(), revision: "13".into() }]);
        assert_eq!(submits(&recorder).len(), 1);
    }

    /// A preview is a copy until the user accepts it: the reply ending runs nothing, the accept
    /// runs exactly one check.
    #[test]
    fn a_preview_runs_nothing_until_it_is_accepted() {
        let (mut dfm, recorder) = runtime();
        dfm.observe("doc-1", 20, true);
        dfm.on_turn_end("doc-1", "turn-2", 20, false, false);
        dfm.observe("doc-1", 20, true);
        assert!(submits(&recorder).is_empty(), "preview only: zero runs");
        dfm.on_preview_accepted("doc-1", "turn-2", 21);
        assert_eq!(submits(&recorder).len(), 1);
        assert!(matches!(submits(&recorder)[0], Trigger::AcceptedPreview { .. }));
        // The next frame sees the committed revision: the check started for it is not cancelled.
        dfm.observe("doc-1", 21, true);
        assert!(dfm.active().is_some());
        assert!(recorder.cancels.lock().unwrap().is_empty());
    }

    /// The page may report the same accept twice; the commit key makes it one run.
    #[test]
    fn a_duplicate_accept_notification_runs_once() {
        let (mut dfm, recorder) = runtime();
        dfm.observe("doc-1", 30, true);
        dfm.on_turn_end("doc-1", "turn-3", 30, false, false);
        dfm.on_preview_accepted("doc-1", "turn-3", 31);
        dfm.on_preview_accepted("doc-1", "turn-3", 31);
        assert_eq!(submits(&recorder).len(), 1);
    }

    /// Reject, a stale preview, an empty turn and the switch being off all run nothing.
    #[test]
    fn reject_stale_preview_empty_turn_and_switch_off_run_nothing() {
        let (mut dfm, recorder) = runtime();
        dfm.observe("doc-1", 40, true);
        // Reject or failed accept: the host never calls the accepted hook.
        dfm.on_preview_not_committed("doc-1", "turn-4");
        // A reply with no model change.
        dfm.on_turn_end("doc-1", "turn-4", 40, false, false);
        dfm.observe("doc-1", 40, true);
        assert!(submits(&recorder).is_empty());
        // Two accepted rounds with no auto checks enabled.
        let (mut off, recorder) = runtime();
        off.set_auto_enabled(false);
        off.observe("doc-1", 41, true);
        off.on_turn_end("doc-1", "turn-5", 42, true, false);
        off.observe("doc-1", 42, true);
        off.on_preview_accepted("doc-1", "turn-5", 42);
        assert!(submits(&recorder).is_empty(), "the project's switch is off");
    }

    /// Two accepted rounds: the second becomes the live check and the first is cancelled; a late
    /// first result cannot replace the second because the run record keys on its submission.
    #[test]
    fn two_accepted_rounds_keep_only_the_later_check() {
        let (mut dfm, recorder) = runtime();
        dfm.observe("doc-1", 50, true);
        dfm.on_turn_end("doc-1", "turn-6", 50, false, false);
        dfm.on_preview_accepted("doc-1", "turn-6", 51);
        let (first, first_document) = dfm.active().expect("the first check");
        assert_eq!((first.to_string(), first_document.to_string()), ("run-0000000000000001-0000000000000001".into(), "doc-1".into()));
        dfm.on_preview_accepted("doc-1", "turn-7", 52);
        assert_eq!(submits(&recorder).len(), 2);
        assert_eq!(dfm.active().unwrap().0, "run-0000000000000002-0000000000000002");
        assert_eq!(recorder.cancels.lock().unwrap().as_slice(), ["run-0000000000000001-0000000000000001"]);
    }

    /// A manual run works with the switch off and never comes from a reply end.
    #[test]
    fn a_manual_run_needs_no_ai_round() {
        let (mut dfm, recorder) = runtime();
        dfm.set_auto_enabled(false);
        assert!(dfm.manual("doc-1", 60));
        assert_eq!(submits(&recorder), [Trigger::Manual]);
        assert!(dfm.manual("doc-1", 60), "pressing Run again is a new run");
        assert_eq!(submits(&recorder).len(), 2);
    }

    /// A manual edit or undo between rounds cancels the old check and starts nothing.
    #[test]
    fn a_newer_model_cancels_the_old_check_and_runs_nothing_by_itself() {
        let (mut dfm, recorder) = runtime();
        dfm.observe("doc-1", 70, true);
        dfm.on_preview_accepted("doc-1", "turn-8", 71);
        assert_eq!(submits(&recorder).len(), 1);
        dfm.observe("doc-1", 72, true);
        assert_eq!(submits(&recorder).len(), 1, "a person's edit does not start an automatic run");
        assert_eq!(recorder.cancels.lock().unwrap().len(), 1);
        assert!(dfm.active().is_none());
    }

    /// A change still being applied when the reply ended belongs to the round; a person's edit
    /// after a settled reply does not.
    #[test]
    fn a_deferred_commit_is_waited_for_but_a_later_person_edit_runs_nothing() {
        let (mut dfm, recorder) = runtime();
        dfm.observe("doc-1", 5, true);
        dfm.on_turn_end("doc-1", "turn-a", 5, true, true);
        dfm.observe("doc-1", 5, false);
        assert!(submits(&recorder).is_empty());
        dfm.observe("doc-1", 6, true);
        dfm.observe("doc-1", 6, true);
        assert_eq!(submits(&recorder).len(), 1, "the deferred commit is the round's own model");
        assert!(matches!(&submits(&recorder)[0], Trigger::Edits { revision, .. } if revision == "6"));

        // The next reply committed nothing new and settled; the user's edit arrives before the
        // next frame. It only makes the report stale.
        dfm.on_turn_end("doc-1", "turn-b", 6, true, false);
        dfm.observe("doc-1", 7, true);
        assert_eq!(submits(&recorder).len(), 1, "a person's edit after the reply starts no check");
    }

    #[test]
    fn a_host_that_cannot_submit_records_why_and_runs_nothing() {
        let recorder = Recorder { fail: true, ..Recorder::default() };
        let mut dfm = HostDfm::default();
        dfm.install(Box::new(recorder));
        dfm.set_auto_enabled(true);
        dfm.observe("doc-1", 80, true);
        dfm.on_turn_end("doc-1", "turn-9", 80, true, false);
        dfm.observe("doc-1", 80, true);
        assert!(!dfm.manual("doc-1", 80));
        assert_eq!(dfm.last_error(), Some("no project is open"));
    }
}
