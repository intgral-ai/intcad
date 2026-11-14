//! Export a finished run as one self-contained file.
//!
//! A run folder already holds everything a reviewer needs; this writes the same information as a
//! single JSON object (for a bug report or a machine) or as Markdown (for a person), without
//! reading the live project or the validation store.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::findings::{self, RunFindings};
use super::run::{RunMeta, RunOutcome};

/// The bundle's schema version.
pub const EXPORT_SCHEMA_VERSION: u32 = 1;

/// One exported run: identity, outcome, findings and the summary text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedRun {
    pub schema_version: u32,
    pub run: RunMeta,
    pub findings: RunFindings,
    /// The same text `summary.md` holds.
    pub summary: String,
    /// Whether this run was `latest` at export; a superseded run is still readable.
    pub current: bool,
}

impl ExportedRun {
    pub fn new(run: &RunMeta, findings: &RunFindings) -> ExportedRun {
        ExportedRun {
            schema_version: EXPORT_SCHEMA_VERSION,
            run: run.clone(),
            findings: findings.clone(),
            summary: findings::summary(findings),
            current: run.outcome == RunOutcome::Current,
        }
    }

    /// The exact JSON bytes of the bundle.
    pub fn json(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec_pretty(self).map_err(|error| error.to_string())
    }

    /// The Markdown document, headed by the run's identity and times.
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# DFM report `{}`\n\n", self.run.run_id));
        out.push_str(&format!("Project: `{}`\n\n", self.run.project_id));
        out.push_str(&format!("Started: {}\n\nEnded: {}\n\n", self.run.started_at, self.run.ended_at));
        out.push_str(&format!("Outcome: `{:?}`\n\n", self.run.outcome));
        if let Some(reason) = &self.run.reason {
            out.push_str(&format!("Reason: {reason}\n\n"));
        }
        out.push_str(&self.summary);
        out
    }
}

/// Write the JSON bundle atomically to `path`.
pub fn write_json(path: &Path, run: &ExportedRun) -> Result<(), String> {
    let bytes = run.json()?;
    solidcraft_io::safe_write::write_atomic(path, &bytes, false).map_err(|error| format!("{}: {error}", path.display()))
}

/// Write the Markdown bundle atomically to `path`.
pub fn write_markdown(path: &Path, run: &ExportedRun) -> Result<(), String> {
    solidcraft_io::safe_write::write_atomic(path, run.markdown().as_bytes(), false).map_err(|error| format!("{}: {error}", path.display()))
}
