//! `dfm.override`: a project's named replacement of one rule's number.
//!
//! An override is advisory until the number has its own independent confirmation; the rule layer
//! decides that when the check runs. This command only records the old rule, the new value and
//! who asked for it, and it refuses an entry without a reason and a signature.

use std::path::Path;

use serde_json::Value;
use solidcraft_dfm_rules::RULE_ORDER;

use super::CommandSpec;
use crate::params::bad;
use crate::project::{Override, OverrideLimit, Project};
use crate::schema::TypedContract;
use crate::schema::project::{OverrideParams, OverridesReply};
use crate::{Result, Session};

const OVERRIDE_CONTRACT: TypedContract<OverrideParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new("dfm.override", "Change a DFM Number", crate::checked!("dfm.override", set_override))
    .icon("settings")
    .noundo()
    .contract(&OVERRIDE_CONTRACT)
    .output(crate::schema::output_schema_for::<OverridesReply>)
    .params(
        "rule: the rule id whose number the project replaces; limit?: min | max (default min); \
         value?: {mm: 2.5} or {t: 1.5, r: 1}; reason: why; by: who asked; date?: when; remove?: remove it \
         → overrides: [{rule, limit, value, reason, by, date}]. A missing reason or name is refused",
    )];

fn open_project(s: &Session) -> Result<Project> {
    let path = s.path.as_deref().ok_or_else(|| bad("dfm.override", "the open design is not saved in a project"))?;
    Project::of(Path::new(path))
        .map_err(|error| bad("dfm.override", error.to_string()))?
        .ok_or_else(|| bad("dfm.override", "the open design is not in a project; make or open a project first"))
}

fn set_override(s: &mut Session, p: &Value) -> Result<Value> {
    let params: OverrideParams = serde_json::from_value(p.clone()).map_err(|error| bad("dfm.override", error.to_string()))?;
    let rule = params.rule.trim().to_string();
    let limit = match params.limit.as_deref() {
        Some("max") => OverrideLimit::Max,
        Some("min") | None => OverrideLimit::Min,
        Some(other) => return Err(bad("dfm.override", format!("`{other}` is not a limit; use min or max"))),
    };
    let mut project = open_project(s)?;
    let mut overrides = project.ci().overrides.clone();
    if params.remove {
        overrides.retain(|over| !(over.rule == rule && (params.limit.is_none() || over.limit == limit)));
    } else {
        if !RULE_ORDER.contains(&rule.as_str()) || rule.contains("gate_") {
            return Err(bad("dfm.override", format!("`{rule}` is not a rule this project can override")));
        }
        let value = params
            .value
            .filter(|value| !value.is_empty())
            .ok_or_else(|| bad("dfm.override", "an override needs a number, for example {\"mm\": 2.5}"))?;
        for (name, number) in &value {
            if !matches!(name.as_str(), "mm" | "t" | "r" | "d" | "b") {
                return Err(bad("dfm.override", format!("`{name}` is not a limit input; use mm, t, r, d or b")));
            }
            if !number.is_finite() {
                return Err(bad("dfm.override", "the override value must be a finite number"));
            }
        }
        let reason = params.reason.unwrap_or_default().trim().to_string();
        if reason.is_empty() {
            return Err(bad("dfm.override", "an override needs a reason"));
        }
        let by = params.by.unwrap_or_default().trim().to_string();
        if by.is_empty() {
            return Err(bad("dfm.override", "an override needs the name of who asked for it"));
        }
        overrides.retain(|over| !(over.rule == rule && over.limit == limit));
        overrides.push(Override { rule, limit, value, reason, by, date: params.date.unwrap_or_default() });
    }
    project.set_overrides(overrides).map_err(|error| bad("dfm.override", error.to_string()))?;
    let reply = OverridesReply {
        overrides: project
            .ci()
            .overrides
            .iter()
            .map(|over| crate::schema::project::OverrideSummary {
                rule: over.rule.clone(),
                limit: match over.limit {
                    OverrideLimit::Min => "min".into(),
                    OverrideLimit::Max => "max".into(),
                },
                value: over.value.clone(),
                reason: over.reason.clone(),
                by: over.by.clone(),
                date: over.date.clone(),
            })
            .collect(),
    };
    serde_json::to_value(reply).map_err(|error| bad("dfm.override", format!("the result could not be written: {error}")))
}
