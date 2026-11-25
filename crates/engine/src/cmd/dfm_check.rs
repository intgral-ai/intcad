//! `dfm.check`, `dfm.cancel`, `dfm.status` and `inspect.dfm`: the manual entry, the cancel, and
//! the report the project keeps.
//!
//! `dfm.check` captures the snapshot and freezes the project on this thread, then hands the work
//! to the background service; it never runs geometry here. `inspect.dfm` is read-only and never
//! returns factory rows.

use serde_json::Value;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::TypedContract;
use crate::schema::dfm::{
    DfmCancelParams, DfmCancelReply, DfmCheckParams, DfmCheckReply, DfmStatusParams, DfmStatusReply, InspectDfmParams, InspectDfmReply,
};
use crate::{Result, Session};

const CHECK_CONTRACT: TypedContract<DfmCheckParams> = TypedContract::new();
const CANCEL_CONTRACT: TypedContract<DfmCancelParams> = TypedContract::new();
const STATUS_CONTRACT: TypedContract<DfmStatusParams> = TypedContract::new();
const INSPECT_CONTRACT: TypedContract<InspectDfmParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("dfm.check", "Run DFM Check", crate::checked!("dfm.check", check))
        .at("SOLID", "INSPECT")
        .icon("render_check")
        .noundo()
        .contract(&CHECK_CONTRACT)
        .output(crate::schema::output_schema_for::<DfmCheckReply>)
        .params(
            "→ the open project's DFM check starts in the background: one dfm.sheet check per native sheet body, based on a \
                 snapshot captured now and the project's frozen process, material, thickness and data. Returns the project identity, \
                 the model and configuration digests and the sheet count. Saving and editing stay usable; dfm.status reports progress",
        ),
    CommandSpec::new("dfm.cancel", "Cancel DFM Check", crate::checked!("dfm.cancel", cancel))
        .icon("stop")
        .noundo()
        .contract(&CANCEL_CONTRACT)
        .output(crate::schema::output_schema_for::<DfmCancelReply>)
        .params("→ cancelled: true when a check was running; its record becomes cancelled and the previous complete report stays latest"),
    CommandSpec::new("dfm.status", "DFM Check Status", crate::checked!("dfm.status", status))
        .icon("status_pin")
        .noundo()
        .contract(&STATUS_CONTRACT)
        .output(crate::schema::output_schema_for::<DfmStatusReply>)
        .params("→ running, activeRun, lastRun, lastOutcome, lastReason, error"),
    CommandSpec::new("inspect.dfm", "DFM Report", crate::checked!("inspect.dfm", inspect))
        .icon("render_check")
        .noundo()
        .contract(&INSPECT_CONTRACT)
        .output(crate::schema::output_schema_for::<InspectDfmReply>)
        .params(
            "run?: a run id (default: the newest complete run); modelDigest?: the live candidate digest → the saved findings from \
                 CONTRACT §2, the outcome, the summary and freshness (current, staleModel, staleConfig, staleBoth or unknown). \
                 Never returns factory rows",
        ),
];

fn check(s: &mut Session, _: &Value) -> Result<Value> {
    let ticket = crate::dfm::service::start(s).map_err(|error| bad("dfm.check", error.to_string()))?;
    serde_json::to_value(DfmCheckReply {
        project_id: ticket.project_id,
        project_name: ticket.project_name,
        candidate_digest: ticket.candidate_digest,
        config_digest: ticket.config_digest,
        sheets: ticket.sheets,
        replaces_running: ticket.replaces_running,
    })
    .map_err(|error| bad("dfm.check", format!("the result could not be written: {error}")))
}

fn cancel(_: &mut Session, _: &Value) -> Result<Value> {
    Ok(serde_json::json!({ "cancelled": crate::dfm::service::cancel() }))
}

fn status(_: &mut Session, _: &Value) -> Result<Value> {
    let status = crate::dfm::service::status();
    serde_json::to_value(DfmStatusReply {
        running: status.running,
        active_run: status.active_run,
        last_run: status.last_run,
        last_outcome: status.last_outcome,
        last_reason: status.last_reason,
        error: status.error,
    })
    .map_err(|error| bad("dfm.status", format!("the result could not be written: {error}")))
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let params: InspectDfmParams = serde_json::from_value(p.clone()).map_err(|error| bad("inspect.dfm", error.to_string()))?;
    let inspection = crate::dfm::service::inspect(s, params.run.as_deref(), params.model_digest.as_deref())
        .map_err(|error| bad("inspect.dfm", error.to_string()))?;
    serde_json::to_value(InspectDfmReply {
        available: inspection.available,
        running: inspection.running,
        run_id: inspection.run_id,
        outcome: inspection.outcome,
        reason: inspection.reason,
        model_version: inspection.model_version,
        candidate_digest: inspection.candidate_digest,
        config_digest: inspection.config_digest,
        freshness: inspection.freshness,
        findings: inspection.findings,
        summary: inspection.summary,
        source_revision: inspection.source_revision,
    })
    .map_err(|error| bad("inspect.dfm", format!("the result could not be written: {error}")))
}
