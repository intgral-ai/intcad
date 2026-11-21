//! The host side of a DFM check in the Assistant: one `dfm.check` run, its frozen identities and
//! the findings the card lists.
//!
//! `dfm.check` queues the run; `inspect.dfm` is the report. The project's stored factory rows never
//! go to the model or the page: the page gets the report fields it draws, and a model summary is
//! whitelisted from the same findings and carries no requirements table.

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

/// One run's result, from `inspect.dfm` (a report) or `dfm.check` (a queued run).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Card {
    pub run_id: String,
    pub candidate_digest: String,
    pub config_digest: String,
    pub model_version: String,
    pub project_id: String,
    pub source: Value,
    pub freshness: String,
    pub findings: Vec<Finding>,
    pub rules: Vec<RuleRow>,
    pub not_checked: String,
    /// The card's own state: `running`, `cancelled`, `done` or `error`.
    pub status: String,
    pub outcome: Option<String>,
    pub reason: Option<String>,
    pub error: Option<String>,
    /// Whether a saved report exists (`inspect.dfm.available`).
    pub available: bool,
    /// Whether `inspect.dfm` carried a parsed findings report for this exact run. A stopped or
    /// failed run has terminal metadata but no findings; that is not a produced report.
    pub report_produced: bool,
}

impl Card {
    /// Parse the `inspect.dfm` reply: the run's state plus the CONTRACT §2 `findings` object.
    pub fn from_inspect(value: &Value) -> Self {
        let running = value["running"].as_bool().unwrap_or(false);
        let outcome = value["outcome"].as_str().map(str::to_string);
        let reason = value["reason"].as_str().map(str::to_string);
        let available = value["available"].as_bool().unwrap_or(false);
        let report: Option<RunFindings> = value.get("findings").and_then(|findings| serde_json::from_value(findings.clone()).ok());
        let mut card = Self {
            run_id: text(value, "runId"),
            candidate_digest: text(value, "candidateDigest"),
            config_digest: text(value, "configDigest"),
            model_version: text(value, "modelVersion"),
            source: Value::Null,
            freshness: text(value, "freshness"),
            status: "done".into(),
            outcome: outcome.clone(),
            reason: reason.clone(),
            available,
            ..Self::default()
        };
        if let Some(report) = report {
            card.run_id = report.run_id;
            card.candidate_digest = report.candidate_digest;
            card.config_digest = report.config_digest;
            card.model_version = report.model_version;
            card.project_id = report.project_id;
            card.source = serde_json::to_value(report.source).unwrap_or(Value::Null);
            card.findings = report.findings;
            card.rules = report.rules;
            card.not_checked = report.not_checked;
            card.report_produced = true;
        }
        card.status = match (running, outcome.as_deref(), available) {
            (true, _, _) => "running",
            (false, Some("cancelled" | "superseded"), _) => "cancelled",
            (false, Some("failed"), _) => "error",
            (false, Some("current"), _) => "done",
            (false, None, true) => "done",
            _ => "running",
        }
        .into();
        if card.status == "error" && card.error.is_none() {
            card.error = reason.or_else(|| Some("the check failed".into()));
        }
        card
    }

    /// Parse the `dfm.check` reply: the run was queued and is running.
    pub fn from_check(value: &Value) -> Self {
        Self {
            run_id: String::new(),
            candidate_digest: text(value, "candidateDigest"),
            config_digest: text(value, "configDigest"),
            model_version: String::new(),
            project_id: text(value, "projectId"),
            source: Value::Null,
            freshness: "unknown".into(),
            status: "running".into(),
            available: false,
            ..Self::default()
        }
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

/// The CONTRACT §2 `RunFindings` object nested under `inspect.dfm.findings`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RunFindings {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    run_id: String,
    candidate_digest: String,
    profile_digest: String,
    config_digest: String,
    model_version: String,
    project_id: String,
    source: Value,
    findings: Vec<Finding>,
    rules: Vec<RuleRow>,
    not_checked: String,
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

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().map(str::to_string).unwrap_or_default()
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

/// Queue the same `dfm.check` entry the button runs. The card starts running; [`refresh`] asks
/// `inspect.dfm` for the report until the run finishes.
pub fn run(app: &mut SolveApp) -> Card {
    match app.run("dfm.check", json!({})) {
        Ok(reply) => Card::from_check(&reply),
        Err(error) => Card::error(error),
    }
}

/// Read the live run's report. A refusal is an error card, never a made-up result.
pub fn refresh(app: &mut SolveApp) -> Card {
    refresh_run(app, None)
}

/// Read one run's report: `run` names it, and `None` asks for the newest complete run. A report
/// that cannot be read is an error card, never the newest report standing in for it.
pub fn refresh_run(app: &mut SolveApp, run: Option<&str>) -> Card {
    let params = match run {
        Some(run) => json!({ "run": run }),
        None => json!({}),
    };
    match app.run("inspect.dfm", params) {
        Ok(reply) => Card::from_inspect(&reply),
        Err(error) => Card::error(error),
    }
}

/// Give the viewport the same report a card shows, so a row and a mark address one finding.
pub fn show_marks(app: &mut SolveApp, card: &Card) {
    let value = json!({
        "runId": card.run_id,
        "candidateDigest": card.candidate_digest,
        "findings": card.findings,
    });
    let Ok(run) = serde_json::from_value::<crate::dfm_overlay::RunFindings>(value) else { return };
    let freshness = match card.freshness.as_str() {
        "current" => crate::dfm_overlay::Freshness::Current,
        "staleModel" => crate::dfm_overlay::Freshness::StaleModel,
        "staleConfig" => crate::dfm_overlay::Freshness::StaleConfig,
        "staleBoth" => crate::dfm_overlay::Freshness::StaleBoth,
        _ => crate::dfm_overlay::Freshness::Unknown,
    };
    app.viewport.load_dfm_run(&run, freshness);
}

/// A message that asks for the check in words, so chat and the button share one entry.
pub fn is_check_request(text: &str) -> bool {
    let text = text.trim().trim_end_matches(['.', '。', '!', '！', '?', '？']).to_lowercase();
    matches!(
        text.as_str(),
        "检查一下当前设计"
            | "检查当前设计"
            | "检查一下当前钣金件"
            | "检查当前钣金件"
            | "检查一下钣金件"
            | "检查钣金件"
            | "帮我检查一下当前钣金件"
            | "帮我检查当前钣金件"
            | "check this design"
            | "check the current design"
            | "check this sheet metal part"
            | "check the sheet metal part"
    )
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
