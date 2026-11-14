//! The bytes one DFM run uses, read once and hashed.
//!
//! [`Project::freeze`] runs on the UI thread together with the model snapshot. After it returns, a
//! run never reads `.intgral/`, the live project or the app data directory again. A missing or
//! mismatched data copy is a frozen state with a reason, not a silent fallback to another table.

use std::fs;

use serde::{Deserialize, Serialize};
use solidcraft_dfm_rules::{Declared, Override as RuleOverride, Profile};

use super::{DataKind, OverrideLimit, Project};
use crate::factory_data;

/// The project identity and the data one run uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenDataRef {
    pub kind: DataKind,
    pub name: String,
    pub version: String,
    pub sha256: String,
    /// The copy inside the project, relative to it; empty for generic data.
    pub file: String,
    /// Why the selected copy cannot be used. `None`: it was read and hashed.
    #[serde(default)]
    pub unavailable: Option<String>,
}

/// A project as one run sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenProject {
    pub project_id: String,
    pub project_name: String,
    /// `project.toml`, including `thickness_mm` and `material` as written (no fallback here).
    pub declared: Declared,
    /// The generic rows exactly as the rule layer parses them.
    pub generic: Profile,
    /// The selected data rows exactly as parsed by the rule layer, when the copy is usable.
    pub shop: Option<Profile>,
    /// Name, version, sha256, and the project-relative path of the copy in `.intgral/`.
    pub data: FrozenDataRef,
    pub overrides: Vec<RuleOverride>,
    /// SHA-256 over the canonical JSON of everything above, model excluded.
    pub config_digest: String,
}

impl Project {
    /// Read the project's own copy once and hash it. The result carries everything a run needs.
    pub fn freeze(&self) -> Result<FrozenProject, super::ProjectError> {
        let manifest = &self.manifest;
        let ci = &self.ci;
        let declared = Declared {
            process: non_empty(&manifest.process),
            material: non_empty(&manifest.material),
            thickness_mm: manifest.thickness_mm,
            thickness_from: manifest.thickness_mm.map(|_| "project".to_string()),
        };
        let generic = factory_data::generic_profile();
        let overrides: Vec<RuleOverride> = ci.overrides.iter().map(rule_override).collect();
        let mut data = FrozenDataRef {
            kind: ci.data.kind,
            name: ci.data.name.clone(),
            version: ci.data.version.clone(),
            sha256: ci.data.sha256.clone(),
            file: ci.data.file.clone(),
            unavailable: None,
        };
        let mut shop = None;
        if ci.data.kind == DataKind::Generic {
            data.name = factory_data::GENERIC_NAME.into();
            data.version = factory_data::GENERIC_VERSION.into();
        } else {
            match read_selected(self.root(), &ci.data.file, &ci.data.sha256) {
                Ok(profile) => {
                    data.unavailable = factory_data::applicability(&profile, &declared);
                    shop = Some(profile);
                }
                Err(reason) => data.unavailable = Some(reason),
            }
        }
        if let Some(selected) = &shop {
            let probe = solidcraft_dfm_rules::CheckInput {
                declared: declared.clone(),
                generic: generic.clone(),
                shop: Some(selected.clone()),
                overrides: overrides.clone(),
                facts: Default::default(),
                model: Default::default(),
                critical: Vec::new(),
            };
            if let Err(error) = solidcraft_dfm_rules::validate(&probe) {
                data.unavailable = Some(format!("the selected data is not valid: {error}"));
                shop = None;
            }
        }
        let config_digest = digest(&manifest.id, &manifest.name, &declared, &generic, &shop, &data, &overrides)?;
        Ok(FrozenProject {
            project_id: manifest.id.clone(),
            project_name: manifest.name.clone(),
            declared,
            generic,
            shop,
            data,
            overrides,
            config_digest,
        })
    }
}

fn read_selected(root: &std::path::Path, file: &str, wanted_sha: &str) -> Result<Profile, String> {
    if file.trim().is_empty() {
        return Err("the project selects factory data but keeps no copy of it".into());
    }
    let path = root.join(file);
    let bytes = fs::read(&path).map_err(|error| format!("the project's data copy {} cannot be read: {error}", path.display()))?;
    let sha = factory_data::sha256_hex(&bytes);
    if sha != wanted_sha {
        return Err(format!("the project's data copy has sha256 {sha}, but ci.toml records {wanted_sha}"));
    }
    factory_data::parse(&bytes).map_err(|error| error.to_string())
}

fn rule_override(over: &super::Override) -> RuleOverride {
    RuleOverride {
        rule: over.rule.clone(),
        limit: match over.limit {
            OverrideLimit::Min => solidcraft_dfm_rules::Limit::Min,
            OverrideLimit::Max => solidcraft_dfm_rules::Limit::Max,
        },
        value: over.value.clone(),
        reason: over.reason.clone(),
        by: over.by.clone(),
        date: over.date.clone(),
    }
}

/// `config_digest`: stable across runs for the same bytes. `serde_json::Map` keeps keys sorted.
fn digest(
    project_id: &str,
    project_name: &str,
    declared: &Declared,
    generic: &Profile,
    shop: &Option<Profile>,
    data: &FrozenDataRef,
    overrides: &[RuleOverride],
) -> Result<String, super::ProjectError> {
    let value = serde_json::json!({
        "projectId": project_id,
        "projectName": project_name,
        "declared": declared,
        "generic": generic,
        "shop": shop,
        "data": data,
        "overrides": overrides,
    });
    let text = serde_json::to_string(&value)
        .map_err(|error| super::ProjectError::Invalid { file: "project".into(), message: format!("the configuration digest: {error}") })?;
    Ok(factory_data::sha256_hex(text.as_bytes()))
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
