//! Built-in skills. A skill is a declarative descriptor: no scripts, no second tool catalog.
//! `text-to-cad` uses the native MCP tools; `anatomy` is a descriptor for Phase 2 only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contract::{Budget, ValidationProfileRef};

pub const TEXT_TO_CAD_ID: &str = "text-to-cad";
pub const ANATOMY_ID: &str = "anatomy";

/// How the host must sequence a turn. Model text cannot relax it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StagePolicy {
    pub require_inspect_before_write: bool,
    pub require_snapshot_before_apply: bool,
    pub require_check_after_apply: bool,
    pub require_validation_after_apply: bool,
}

/// What one tool call may do under a skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
    Forbidden,
}

/// A declarative built-in skill descriptor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Skill {
    pub id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub system_note: String,
    pub read_tools: Vec<String>,
    pub write_tools: Vec<String>,
    pub stage_policy: StagePolicy,
    pub validation_profile: ValidationProfileRef,
    pub required_checks: Vec<String>,
    pub budget: Budget,
    pub acceptance: String,
}

/// Commands that only read. Mirrors the control gate's read set for the skill's purposes.
fn reads_command(id: &str) -> bool {
    id.starts_with("inspect.")
        || id.starts_with("model.")
        || id.ends_with(".inspect")
        || id.ends_with(".list")
        || matches!(id, "document.inspect" | "document.revision" | "engine.commands" | "engine.describe" | "edit.checkpoint" | "render.status")
}

/// CAD command areas a text-to-cad turn may write. Files, documents, apps, preferences,
/// diagnostics and scripts are never the skill's.
fn writes_command(id: &str) -> bool {
    if id.starts_with("diagnostics.") || id.starts_with("app.") || id.starts_with("prefs.") {
        return false;
    }
    if id.starts_with("file.") || id.starts_with("doc.") || id.starts_with("library.") {
        return false;
    }
    matches!(
        id.split('.').next().unwrap_or_default(),
        "appearance"
            | "assembly"
            | "body"
            | "component"
            | "config"
            | "construct"
            | "contact"
            | "explode"
            | "expr"
            | "face"
            | "joint"
            | "material"
            | "mesh"
            | "occurrence"
            | "parameters"
            | "plastic"
            | "sheet"
            | "sketch"
            | "solid"
            | "surface"
            | "timeline"
    )
}

fn command_id(arguments: &Value) -> Option<&str> {
    arguments.get("command").and_then(Value::as_str).or_else(|| arguments.get("params").and_then(|p| p.get("command")).and_then(Value::as_str))
}

impl Skill {
    /// Classify one host tool call. `tool` may be an MCP tool name or a control method; the
    /// arguments are the same JSON the gate sees.
    pub fn classify(&self, tool: &str, arguments: &Value) -> ToolKind {
        if let Some(id) = command_id(arguments) {
            let id = id.trim();
            if reads_command(id) {
                return ToolKind::Read;
            }
            return if writes_command(id) { ToolKind::Write } else { ToolKind::Forbidden };
        }
        if self.read_tools.iter().any(|t| t == tool) {
            return ToolKind::Read;
        }
        if self.write_tools.iter().any(|t| t == tool) {
            return ToolKind::Write;
        }
        ToolKind::Forbidden
    }

    pub fn allows(&self, tool: &str, arguments: &Value) -> bool {
        self.classify(tool, arguments) != ToolKind::Forbidden
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillError(pub String);

impl std::fmt::Display for SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SkillError {}

/// The built-in registry. Phase 0 registers built-ins only; no installable scripts.
#[derive(Clone, Debug, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, Skill>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a skill. An id/version is immutable once registered.
    pub fn register(&mut self, skill: Skill) -> Result<(), SkillError> {
        if skill.id.trim().is_empty() || skill.id.len() > 128 {
            return Err(SkillError("a skill id is 1..=128 bytes".into()));
        }
        if skill.version.trim().is_empty() || skill.version.len() > 64 {
            return Err(SkillError("a skill version is 1..=64 bytes".into()));
        }
        if skill.read_tools.iter().any(|tool| skill.write_tools.iter().any(|write| write == tool)) {
            return Err(SkillError(format!("skill {} lists a tool as both read and write", skill.id)));
        }
        if let Some(existing) = self.skills.get(&skill.id) {
            if existing.version != skill.version {
                return Err(SkillError(format!("skill {} is already registered at version {}", skill.id, existing.version)));
            }
            if existing != &skill {
                return Err(SkillError(format!("skill {} version {} is already registered with another descriptor", skill.id, skill.version)));
            }
            return Ok(());
        }
        self.skills.insert(skill.id.clone(), skill);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Skill> {
        self.skills.get(id)
    }

    pub fn ids(&self) -> Vec<&str> {
        self.skills.keys().map(String::as_str).collect()
    }
}

/// The Phase 1 built-in skill. The host supplies the real versioned profile identity.
pub fn text_to_cad(profile: ValidationProfileRef, budget: Budget) -> Skill {
    Skill {
        id: TEXT_TO_CAD_ID.into(),
        version: "0.1.0".into(),
        title: "Text to CAD".into(),
        description: "Create or modify a parametric design from natural language, then let the host validate it.".into(),
        system_note: "You are the Text to CAD skill. Inspect the design before editing. Use the native CAD tools only. \
                      A change is proposed as a tool call; the host, not your prose, decides success. \
                      Keep parameters editable and state the plan before the first write."
            .into(),
        read_tools: vec![
            "list_commands".into(),
            "describe_command".into(),
            "inspect_design".into(),
            "measure".into(),
            "body_topology".into(),
            "check_parameters".into(),
            "engine.commands".into(),
            "engine.describe".into(),
            "document.inspect".into(),
            "document.revision".into(),
            "inspect.measure".into(),
            "inspect.clearance".into(),
            "inspect.interference".into(),
        ],
        write_tools: vec!["execute".into(), "batch".into(), "set_parameter".into(), "engine.execute".into(), "engine.batch".into()],
        stage_policy: StagePolicy {
            require_inspect_before_write: true,
            require_snapshot_before_apply: true,
            require_check_after_apply: true,
            require_validation_after_apply: true,
        },
        validation_profile: profile,
        required_checks: vec!["input.integrity".into(), "native.rebuild".into()],
        budget,
        acceptance: "phase1-text-to-cad-e2e".into(),
    }
}

/// Phase 2 descriptor only: no anatomy checker is implemented in Phase 0.
pub fn anatomy(profile: ValidationProfileRef, budget: Budget) -> Skill {
    Skill {
        id: ANATOMY_ID.into(),
        version: "0.1.0".into(),
        title: "Anatomy".into(),
        description: "Declarative semantic checks over a finished design (Phase 2).".into(),
        system_note: "You are the Anatomy skill. State the supported scope; never run scripts.".into(),
        read_tools: vec!["inspect_design".into(), "measure".into(), "body_topology".into()],
        write_tools: Vec::new(),
        stage_policy: StagePolicy {
            require_inspect_before_write: true,
            require_snapshot_before_apply: true,
            require_check_after_apply: true,
            require_validation_after_apply: true,
        },
        validation_profile: profile,
        required_checks: vec!["input.integrity".into(), "native.rebuild".into()],
        budget,
        acceptance: "phase2-anatomy-e2e".into(),
    }
}

/// Register the built-ins a Phase 0 host offers.
pub fn builtins(profile: ValidationProfileRef, budget: Budget) -> Result<SkillRegistry, SkillError> {
    let mut registry = SkillRegistry::new();
    registry.register(text_to_cad(profile.clone(), budget.clone()))?;
    registry.register(anatomy(profile, budget))?;
    Ok(registry)
}
