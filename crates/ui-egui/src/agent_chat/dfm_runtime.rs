//! The host side of automatic DFM checks (PIL-305).
//!
//! An AI round's edits trigger at most one run, after the edits are actually committed. Direct
//! multi-step modelling commits during the round; PIL-300's preview accepts after the reply ended.
//! Both arrive here as one commit event keyed by `(document, AI turn, committed model revision)`,
//! so a duplicate accept notification runs nothing twice, and nothing is scheduled from the end of
//! a reply alone or from every tool call.
//!
//! A part ticket the Assistant works and has said finished is one more such commit event: its check
//! is decided at the end of that turn and held, the same way, until the design is quiet, so the
//! check sees the model the Assistant left and no edit of that turn cancels it.
//!
//! This module only decides *when* to submit and returns a [`Trigger`]; the project-backed capture
//! and validation live in `crate::dfm::service`. A new edit cancels the running check there, and a
//! late result cannot overwrite a newer one: the run record's own submission identity decides
//! that, not this module.

use std::collections::VecDeque;

/// Why a check was submitted.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Trigger {
    /// A person pressed Run DFM.
    Manual,
    /// An AI round's committed edits.
    Edits { document_id: String, turn: String, revision: String },
    /// A PIL-300 preview the user accepted and whose apply succeeded.
    AcceptedPreview { document_id: String, turn: String, revision: String },
    /// The Assistant said the part ticket it works is finished, and that turn has ended.
    TicketFinished { document_id: String, turn: String, revision: String, ticket: String },
}

impl Trigger {
    /// The same trigger for the newest model of its round.
    fn at(&mut self, now: u64) {
        if let Trigger::Edits { revision, .. } | Trigger::TicketFinished { revision, .. } = self {
            *revision = now.to_string();
        }
    }
}

/// How many committed round keys are remembered for duplicate suppression.
const REMEMBERED_COMMITS: usize = 64;

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
#[allow(dead_code)] // installed by the project host when a project opens
#[derive(Default)]
pub(crate) struct HostDfm {
    /// The project's `ci.toml` switch. Off means no automatic runs; manual runs still work.
    auto_enabled: bool,
    /// Recently fired `(document, turn, revision)` keys, oldest first.
    committed: VecDeque<String>,
    settling: Option<Settling>,
    last_seen: Option<(String, u64)>,
    manual_serial: u64,
    /// Tests count decided triggers; the host submits them through the engine service.
    #[cfg(test)]
    pub(crate) fired: usize,
}

#[allow(dead_code)] // the host entry points are used once the project settings page lands
impl HostDfm {
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

    /// A person pressed Run DFM.
    pub(crate) fn manual(&mut self, document_id: &str, revision: u64) -> Option<Trigger> {
        self.manual_serial = self.manual_serial.wrapping_add(1);
        let key = format!("manual|{document_id}|{revision}|{}", self.manual_serial);
        self.fire(key, Trigger::Manual)
    }

    /// The AI's reply ended. `changed` is true only when this round actually committed a design
    /// change; a pure answer, a rejected edit or a preview that is still only a preview is false.
    /// `pending` says that the window still has a design change being applied (off the window
    /// thread), so a later revision belongs to this round; a person's edit after a settled reply
    /// is not that.
    pub(crate) fn on_turn_end(&mut self, document_id: &str, turn: &str, revision: u64, changed: bool, pending: bool) {
        if !changed || !self.auto_enabled {
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

    /// The turn in which the Assistant said its part ticket `ticket` is finished has ended: hold its
    /// check, as a round's edits are held, until the design is quiet. It needs no change in this
    /// turn (the edits may have come in earlier ones), and it takes the place of the turn's edits.
    pub(crate) fn on_ticket_finished(&mut self, document_id: &str, turn: &str, revision: u64, pending: bool, ticket: &str) {
        if !self.auto_enabled {
            return;
        }
        let trigger = Trigger::TicketFinished {
            document_id: document_id.to_string(),
            turn: turn.to_string(),
            revision: revision.to_string(),
            ticket: ticket.to_string(),
        };
        self.settling =
            Some(Settling { document_id: document_id.to_string(), turn: format!("{turn}|{ticket}"), trigger, revision, deferred: pending });
    }

    /// A PIL-300 preview was accepted and applying it succeeded. The reply has already ended, so
    /// this is a separate commit event for the same turn.
    pub(crate) fn on_preview_accepted(&mut self, document_id: &str, turn: &str, revision: u64) -> Option<Trigger> {
        if !self.auto_enabled {
            return None;
        }
        let key = commit_key(document_id, turn, revision);
        let trigger = Trigger::AcceptedPreview { document_id: document_id.to_string(), turn: turn.to_string(), revision: revision.to_string() };
        self.fire(key, trigger)
    }

    /// A rejected preview, a failed accept or an accept refused because the user edited the design
    /// meanwhile. It commits nothing, so it runs nothing.
    pub(crate) fn on_preview_not_committed(&mut self, _document_id: &str, _turn: &str) {}

    /// Called once per UI frame. Fires a held round once the design is quiet; a person's edit
    /// while a round settles drops that round (the service cancels any live check separately).
    pub(crate) fn observe(&mut self, document_id: &str, revision: u64, idle: bool) -> Option<Trigger> {
        let changed_document = self.last_seen.as_ref().is_some_and(|(document, _)| document != document_id);
        self.last_seen = Some((document_id.to_string(), revision));
        if changed_document {
            self.settling = None;
            return None;
        }
        let settling = self.settling.clone()?;
        if settling.document_id != document_id {
            self.settling = None;
            return None;
        }
        if settling.revision != revision {
            if !settling.deferred {
                // The reply had already committed everything it changed; a further revision is a
                // person's edit or undo, which only makes the old report stale.
                self.settling = None;
                return None;
            }
            // More edits landed after the reply: hold the round for the newest model.
            if let Some(settling) = self.settling.as_mut() {
                settling.revision = revision;
                settling.trigger.at(revision);
            }
            return None;
        }
        if !idle {
            return None;
        }
        self.settling = None;
        let key = commit_key(document_id, &settling.turn, revision);
        self.fire(key, settling.trigger)
    }

    fn fire(&mut self, key: String, trigger: Trigger) -> Option<Trigger> {
        let first = self.remember(&key);
        #[cfg(test)]
        if first {
            self.fired += 1;
        }
        first.then_some(trigger)
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

    fn runtime() -> HostDfm {
        let mut dfm = HostDfm::default();
        dfm.set_auto_enabled(true);
        dfm
    }

    /// Direct multi-step modelling: three tool commits in one round are one run.
    #[test]
    fn a_direct_multi_step_round_runs_once_after_its_commits() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 10, true);
        // The last edit of the round lands, then the reply ends with a changed design.
        dfm.observe("doc-1", 13, true);
        dfm.on_turn_end("doc-1", "turn-1", 13, true, false);
        assert!(dfm.observe("doc-1", 13, false).is_none(), "no run while work is in flight");
        let trigger = dfm.observe("doc-1", 13, true).expect("one run after the round");
        assert_eq!(trigger, Trigger::Edits { document_id: "doc-1".into(), turn: "turn-1".into(), revision: "13".into() });
        assert!(dfm.observe("doc-1", 13, true).is_none(), "once per round");
    }

    /// A preview is a copy until the user accepts it: the reply ending runs nothing, the accept
    /// runs exactly one check.
    #[test]
    fn a_preview_runs_nothing_until_it_is_accepted() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 20, true);
        dfm.on_turn_end("doc-1", "turn-2", 20, false, false);
        assert!(dfm.observe("doc-1", 20, true).is_none(), "preview only: zero runs");
        let trigger = dfm.on_preview_accepted("doc-1", "turn-2", 21).expect("one run on accept");
        assert!(matches!(trigger, Trigger::AcceptedPreview { .. }));
        // The committed revision seen next frame does not create a second run.
        assert!(dfm.observe("doc-1", 21, true).is_none());
    }

    /// The page may report the same accept twice; the commit key makes it one run.
    #[test]
    fn a_duplicate_accept_notification_runs_once() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 30, true);
        dfm.on_turn_end("doc-1", "turn-3", 30, false, false);
        assert!(dfm.on_preview_accepted("doc-1", "turn-3", 31).is_some());
        assert!(dfm.on_preview_accepted("doc-1", "turn-3", 31).is_none());
    }

    /// Reject, a stale preview, an empty turn and the switch being off all run nothing.
    #[test]
    fn reject_stale_preview_empty_turn_and_switch_off_run_nothing() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 40, true);
        dfm.on_preview_not_committed("doc-1", "turn-4");
        dfm.on_turn_end("doc-1", "turn-4", 40, false, false);
        assert!(dfm.observe("doc-1", 40, true).is_none());
        let mut off = runtime();
        off.set_auto_enabled(false);
        off.observe("doc-1", 41, true);
        off.on_turn_end("doc-1", "turn-5", 42, true, false);
        assert!(off.observe("doc-1", 42, true).is_none(), "the project's switch is off");
        assert!(off.on_preview_accepted("doc-1", "turn-5", 42).is_none());
    }

    /// Two accepted rounds are two triggers with different revisions; the service cancels the
    /// first and the run store keeps only the later result as latest.
    #[test]
    fn two_accepted_rounds_emit_the_later_check() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 50, true);
        dfm.on_turn_end("doc-1", "turn-6", 50, false, false);
        assert!(dfm.on_preview_accepted("doc-1", "turn-6", 51).is_some());
        let later = dfm.on_preview_accepted("doc-1", "turn-7", 52).expect("the later accepted round");
        assert!(matches!(later, Trigger::AcceptedPreview { revision, .. } if revision == "52"));
        // An older accept notification does not fire again for the later model.
        assert!(dfm.on_preview_accepted("doc-1", "turn-6", 51).is_none());
    }

    /// A manual run works with the switch off and never comes from a reply end.
    #[test]
    fn a_manual_run_needs_no_ai_round() {
        let mut dfm = runtime();
        dfm.set_auto_enabled(false);
        assert!(dfm.manual("doc-1", 60).is_some());
        assert!(dfm.manual("doc-1", 60).is_some(), "pressing Run again is a new run");
    }

    /// A person's edit while a settled round waits drops that round.
    #[test]
    fn a_later_person_edit_runs_nothing() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 70, true);
        dfm.on_turn_end("doc-1", "turn-8", 70, true, false);
        assert!(dfm.observe("doc-1", 71, true).is_none(), "a person's edit does not start an automatic run");
    }

    /// A finished ticket's turn holds its check until the design is quiet, needs no change in that
    /// turn, fires once, and fires nothing with the project's switch off.
    #[test]
    fn a_finished_tickets_turn_holds_its_check_until_the_design_is_quiet() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 80, true);
        dfm.on_ticket_finished("doc-1", "turn-9", 80, true, "T-2");
        assert!(dfm.observe("doc-1", 80, false).is_none(), "no check while work is in flight");
        assert!(dfm.observe("doc-1", 81, true).is_none(), "a late edit of the turn: wait for a stable revision");
        let trigger = dfm.observe("doc-1", 81, true).expect("one check for the finished ticket");
        assert_eq!(trigger, Trigger::TicketFinished { document_id: "doc-1".into(), turn: "turn-9".into(), revision: "81".into(), ticket: "T-2".into() });
        assert!(dfm.observe("doc-1", 81, true).is_none(), "once per turn");
        let mut off = runtime();
        off.set_auto_enabled(false);
        off.on_ticket_finished("doc-1", "turn-9", 80, false, "T-2");
        assert!(off.observe("doc-1", 80, true).is_none(), "the project's switch is off");
    }

    /// A change still being applied when the reply ended belongs to the round.
    #[test]
    fn a_deferred_commit_is_waited_for() {
        let mut dfm = runtime();
        dfm.observe("doc-1", 5, true);
        dfm.on_turn_end("doc-1", "turn-a", 5, true, true);
        assert!(dfm.observe("doc-1", 5, false).is_none());
        assert!(dfm.observe("doc-1", 6, true).is_none(), "wait for a stable revision");
        let trigger = dfm.observe("doc-1", 6, true).expect("the deferred commit is the round's own model");
        assert!(matches!(trigger, Trigger::Edits { revision, .. } if revision == "6"));
    }
}
