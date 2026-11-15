//! Wire shapes for `dfm.check`, `dfm.cancel`, `dfm.status` and `inspect.dfm`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Run a DFM check for the open project (no parameters).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DfmCheckParams {}

/// The check was captured, frozen and queued.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DfmCheckReply {
    pub project_id: String,
    pub project_name: String,
    pub candidate_digest: String,
    pub config_digest: String,
    /// How many native sheet bodies were found and will each get a `dfm.sheet` check.
    pub sheets: usize,
    /// `true` when an earlier check had to be stopped first.
    pub replaces_running: bool,
}

/// Cancel the live check (no parameters).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DfmCancelParams {}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DfmCancelReply {
    /// `true` when a check was running and the cancellation was signalled.
    pub cancelled: bool,
}

/// Read the live check state (no parameters).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DfmStatusParams {}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DfmStatusReply {
    pub running: bool,
    pub active_run: Option<String>,
    pub last_run: Option<String>,
    pub last_outcome: Option<String>,
    pub last_reason: Option<String>,
    pub error: Option<String>,
}

/// Read a project's saved DFM report. `run` defaults to the newest complete run.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct InspectDfmParams {
    #[serde(default)]
    pub run: Option<String>,
    /// The live candidate digest, when the caller has one. Without it, freshness is unknown
    /// across sessions; in the same session the captured revision is used.
    #[serde(default)]
    pub model_digest: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InspectDfmReply {
    pub available: bool,
    pub running: bool,
    pub run_id: Option<String>,
    pub outcome: Option<String>,
    pub reason: Option<String>,
    pub model_version: Option<String>,
    pub candidate_digest: Option<String>,
    pub config_digest: Option<String>,
    /// `current`, `staleModel`, `staleConfig`, `staleBoth` or `unknown`.
    pub freshness: Option<String>,
    /// The exact CONTRACT §2 `findings.json` object A renders and B locates.
    pub findings: Option<serde_json::Value>,
    pub summary: Option<String>,
    pub source_revision: Option<String>,
}
