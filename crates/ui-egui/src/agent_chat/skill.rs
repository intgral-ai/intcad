//! Built-in Chat skills.
//!
//! A skill is a host-side mode of the existing Chat turn. It is not a second agent loop, session,
//! provider or tool registry: selecting one freezes a descriptor and the current document,
//! selection and connection on the turn, then adds a short host note to the same native prompt.
//! The existing gate, approvals, Stop and per-turn undo stay in charge of every CAD write.

use serde_json::Value;

use crate::i18n::{format as trf, tr};

/// The Text-to-CAD skill id.
pub const TEXT_TO_CAD_ID: &str = "text-to-cad";
/// The Anatomy skill id.
pub const ANATOMY_ID: &str = "anatomy";
/// The descriptor version both Beta built-ins share.
pub const VERSION: &str = "0.1.0";

/// What the selected backend can prove for a skill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkillSupport {
    /// The host tested this skill on this backend path.
    Supported,
    /// The turn may run, but the UI must not claim verification.
    #[default]
    Unverified,
    /// This backend cannot run the skill; the host refuses before a model call.
    Unsupported,
}

/// A built-in skill, or ordinary Chat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Skill {
    /// No skill: the default, unchanged Chat.
    #[default]
    Plain,
    TextToCad,
    Anatomy,
}

impl Skill {
    pub const ALL: [Skill; 3] = [Skill::Plain, Skill::TextToCad, Skill::Anatomy];

    pub fn id(self) -> &'static str {
        match self {
            Skill::Plain => "plain",
            Skill::TextToCad => TEXT_TO_CAD_ID,
            Skill::Anatomy => ANATOMY_ID,
        }
    }

    /// The stable descriptor version; ordinary Chat has none.
    pub fn version(self) -> &'static str {
        match self {
            Skill::Plain => "",
            Skill::TextToCad | Skill::Anatomy => VERSION,
        }
    }

    pub fn title(self) -> &'static str {
        tr(match self {
            Skill::Plain => "ui.agent_chat_skill_plain",
            Skill::TextToCad => "ui.agent_chat_skill_text_to_cad",
            Skill::Anatomy => "ui.agent_chat_skill_anatomy",
        })
    }

    pub fn description(self) -> &'static str {
        tr(match self {
            Skill::Plain => "ui.agent_chat_skill_plain_tip",
            Skill::TextToCad => "ui.agent_chat_skill_text_to_cad_tip",
            Skill::Anatomy => "ui.agent_chat_skill_anatomy_tip",
        })
    }

    /// Anatomy is read-only for the whole turn. Text-to-CAD writes only through the existing gate.
    pub fn read_only(self) -> bool {
        self == Skill::Anatomy
    }

    /// The host note prepended to the same native prompt. It is a policy statement, not a second
    /// system prompt or tool catalog.
    pub fn prompt_note(self) -> Option<String> {
        match self {
            Skill::Plain => None,
            Skill::TextToCad => Some(format!(
                "[Host skill {TEXT_TO_CAD_ID} v{VERSION}] Inspect the current design before the first write. \
                 Use the native CAD tools; keep features and parameters editable. The host, not your prose, \
                 decides success: a write is only successful after the host checks and validates the exact \
                 captured snapshot. Do not claim success from your own message."
            )),
            Skill::Anatomy => Some(format!(
                "[Host skill {ANATOMY_ID} v{VERSION}] This turn is read-only. Explain the current design or \
                 selection using the host facts and exact measurements supplied. Do not change geometry, \
                 parameters, feature history or undo. Only object names the host supplies are clickable; \
                 never invent a clickable target. State missing feature history or assembly information honestly."
            )),
        }
    }

    /// The immutable descriptor a UI or receipt can show.
    pub fn descriptor(self) -> Value {
        serde_json::json!({
            "id": self.id(),
            "version": self.version(),
            "title": self.title(),
            "readOnly": self.read_only(),
            "builtin": true,
        })
    }
}

/// The turn's frozen skill/document/selection/connection binding. A binding is never rewritten by
/// a model or by a later document switch; the UI compares it with the live document instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub skill: Skill,
    pub version: String,
    pub document_id: String,
    pub revision: u64,
    pub model: String,
    pub connection: String,
    pub selection: Vec<String>,
    pub support: SkillSupport,
}

impl Binding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        skill: Skill,
        document_id: impl Into<String>,
        revision: u64,
        model: impl Into<String>,
        connection: impl Into<String>,
        selection: Vec<String>,
        support: SkillSupport,
    ) -> Self {
        Self {
            skill,
            version: skill.version().into(),
            document_id: document_id.into(),
            revision,
            model: model.into(),
            connection: connection.into(),
            selection,
            support,
        }
    }

    /// The binding no longer names the live document revision.
    pub fn is_stale(&self, document_id: &str, revision: u64) -> bool {
        self.document_id != document_id || self.revision != revision
    }

    /// One line for the panel. It always names the exact document revision the turn was bound to.
    pub fn summary(&self) -> String {
        let mut text = trf(
            "ui.agent_chat_skill_binding",
            &[
                ("skill", self.skill.title()),
                ("version", &self.version),
                ("document", &self.document_id),
                ("revision", &self.revision.to_string()),
                ("model", &self.model),
            ],
        );
        if !self.selection.is_empty() {
            text.push_str(&trf("ui.agent_chat_skill_selection", &[("n", &self.selection.len().to_string())]));
        }
        text
    }

    /// The support label to show beside a binding.
    pub fn support_label(&self) -> Option<&'static str> {
        match self.support {
            SkillSupport::Supported => None,
            SkillSupport::Unverified => Some(tr("ui.agent_chat_skill_unverified")),
            SkillSupport::Unsupported => Some(tr("ui.agent_chat_skill_unsupported")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_chat_is_the_unchanged_default() {
        assert_eq!(Skill::default(), Skill::Plain);
        assert_eq!(Skill::Plain.prompt_note(), None);
        assert!(!Skill::Plain.read_only());
        assert_eq!(Skill::Plain.version(), "");
    }

    #[test]
    fn the_two_beta_descriptors_are_fixed_and_distinct() {
        assert_eq!(Skill::TextToCad.id(), TEXT_TO_CAD_ID);
        assert_eq!(Skill::Anatomy.id(), ANATOMY_ID);
        assert_eq!(Skill::TextToCad.version(), VERSION);
        assert_eq!(Skill::Anatomy.version(), VERSION);
        assert!(Skill::Anatomy.read_only());
        assert!(!Skill::TextToCad.read_only());
        assert!(Skill::TextToCad.prompt_note().is_some_and(|note| note.contains("native CAD tools")));
        assert!(Skill::Anatomy.prompt_note().is_some_and(|note| note.contains("read-only")));
    }

    #[test]
    fn a_binding_is_frozen_to_the_document_revision_and_selection() {
        let binding =
            Binding::new(Skill::Anatomy, "doc-1", 8, "model", "connection", vec!["Bracket".into(), "Cover".into()], SkillSupport::Unverified);
        assert!(!binding.is_stale("doc-1", 8));
        assert!(binding.is_stale("doc-1", 9), "a later revision is not the bound turn");
        assert!(binding.is_stale("doc-2", 8), "a same-revision other document is not the bound turn");
        assert!(binding.summary().contains("doc-1") && binding.summary().contains("8"));
        assert_eq!(binding.support_label(), Some(tr("ui.agent_chat_skill_unverified")));
    }
}
