//! The host side of a DFM check in the Assistant: one `dfm.check` run, its frozen identities and
//! the findings the card lists.
//!
//! The project's stored factory rows never go to the model or the page. The page gets the report
//! fields it draws; a model summary, when one is built, is whitelisted from the same findings and
//! carries no requirements table.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::SolveApp;

/// Which findings the viewport marks: all from the run, the selected one, or none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Display {
    #[default]
    All,
    Selected,
    Off,
}

impl Display {
    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Selected => "selected",
            Self::Off => "off",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "all" => Some(Self::All),
            "selected" => Some(Self::Selected),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// What the Assistant keeps for the check card and the viewport marks.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub card: Option<Card>,
    pub selected: Option<(String, String)>,
    pub display: Display,
    pub next: u64,
}

/// One run's result, as `dfm.check` returns it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Card {
    pub run_id: String,
    pub candidate_digest: String,
    pub config_digest: String,
    pub model_version: String,
    pub project_id: String,
    pub source: String,
    pub freshness: String,
    pub findings: Vec<Finding>,
    pub rules: Vec<RuleRow>,
    pub not_checked: String,
    /// The card's own state: `running`, `cancelled`, `done` or `error`.
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Finding {
    pub id: String,
    pub sheet: usize,
    pub body: String,
    pub from: String,
    pub rules: Vec<String>,
    pub state: String,
    pub status: String,
    pub code: Option<String>,
    pub measured: Option<Measure>,
    pub requirement: Option<Requirement>,
    pub feature: Option<Feature>,
    pub anchor: Option<[f64; 3]>,
    pub flat: Option<[f64; 2]>,
    pub faces: Vec<String>,
    pub outline: Vec<[f64; 3]>,
    /// Why the rule could not cover this finding ("not covered: …").
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Measure {
    pub value: f64,
    pub unit: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Requirement {
    pub value: f64,
    pub unit: String,
    pub layer: String,
    pub confirmed: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Feature {
    pub kind: String,
    pub number: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuleRow {
    pub rule_id: String,
    pub state: String,
    pub count: usize,
}

impl Card {
    pub fn from_value(value: &Value) -> Self {
        let mut card: Card = serde_json::from_value(value.clone()).unwrap_or_default();
        if card.status.is_empty() {
            card.status = "done".into();
        }
        card
    }

    pub fn error(reason: String) -> Self {
        Self { status: "error".into(), error: Some(reason), ..Self::default() }
    }

    pub fn counts(&self) -> Value {
        let mut passed = 0;
        let mut failed = 0;
        let mut advisory = 0;
        let mut checked = 0;
        let mut unsupported = 0;
        for rule in &self.rules {
            match rule.state.as_str() {
                "passed" => passed += 1,
                "failed" => failed += 1,
                "advisory" => advisory += 1,
                "unsupported" => unsupported += 1,
                _ => checked += 1,
            }
        }
        json!({"passed": passed, "failed": failed, "advisory": advisory, "checked": checked, "unsupported": unsupported})
    }
}

/// The fields the page draws: the report and its card state, never a project data table.
pub fn page(card: &Card) -> Value {
    let mut value = serde_json::to_value(card).unwrap_or(Value::Null);
    value["counts"] = card.counts();
    value
}

/// The fields a model may be told: findings and their own requirement, no requirements table and
/// no stored factory rows.
pub fn model_context(card: &Card) -> Value {
    json!({
        "runId": card.run_id,
        "projectId": card.project_id,
        "candidateDigest": card.candidate_digest,
        "configDigest": card.config_digest,
        "modelVersion": card.model_version,
        "freshness": card.freshness,
        "notChecked": card.not_checked,
        "findings": card.findings.iter().map(|finding| json!({
            "id": finding.id, "sheet": finding.sheet, "body": finding.body, "from": finding.from,
            "rules": finding.rules, "state": finding.state, "status": finding.status, "code": finding.code,
            "measured": finding.measured, "requirement": finding.requirement, "feature": finding.feature,
            "anchor": finding.anchor, "flat": finding.flat, "faces": finding.faces, "outline": finding.outline,
        })).collect::<Vec<_>>(),
    })
}

/// Run the same `dfm.check` entry the button runs. A missing command or a refused run is an error
/// card, never a made-up result.
pub fn run(app: &mut SolveApp) -> Card {
    match app.run("dfm.check", json!({})) {
        Ok(reply) => Card::from_value(&reply),
        Err(error) => Card::error(error),
    }
}

/// A message that asks for the check in words, so chat and the button share one entry.
pub fn is_check_request(text: &str) -> bool {
    let text = text.trim().trim_end_matches(['.', '。', '!', '！']).to_lowercase();
    matches!(text.as_str(), "检查一下当前设计" | "检查当前设计" | "check this design" | "check the current design")
}

/// Select one `(run_id, finding_id)` from the card.
pub fn select(app: &mut SolveApp, run_id: &str, finding_id: &str) -> Result<(), String> {
    let card = app.dfm_chat.card.as_ref().ok_or("finding: there is no check result")?;
    if card.run_id != run_id {
        return Err("finding: that finding belongs to another run".into());
    }
    if !card.findings.iter().any(|finding| finding.id == finding_id) {
        return Err("finding: the result has no such finding".into());
    }
    app.dfm_chat.selected = Some((run_id.to_string(), finding_id.to_string()));
    // B's overlay reads this selection from `app.dfm_chat.selected` and locates the same key.
    Ok(())
}
