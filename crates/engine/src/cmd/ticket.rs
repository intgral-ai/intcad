//! `ticket.list`, `ticket.inspect`, `ticket.draft` and `ticket.update`: the agent's way to the
//! project's part tickets (`crate::project::tickets`).
//!
//! These four are all there is. Drafting makes drafts only; an update says the ticket the
//! Assistant works is finished, or that it is blocked or taken up again. Accepting or declining a
//! draft, handing a ticket over, starting its check, setting a state, finishing, editing,
//! deleting and restoring are a person's buttons in the Tickets panel, so no command does them.
//! Every step a command makes is recorded as the agent's.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::CommandSpec;
use crate::params::bad;
use crate::project::Project;
use crate::project::tickets::{self, Draft, TicketError};
use crate::schema::TypedContract;
use crate::schema::ticket::{TicketDraftParams, TicketInspectParams, TicketListParams, TicketReply, TicketUpdateParams, TicketsReply};
use crate::{Result, Session};

const LIST_CONTRACT: TypedContract<TicketListParams> = TypedContract::new();
const INSPECT_CONTRACT: TypedContract<TicketInspectParams> = TypedContract::new();
const DRAFT_CONTRACT: TypedContract<TicketDraftParams> = TypedContract::new();
const UPDATE_CONTRACT: TypedContract<TicketUpdateParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("ticket.list", "List Part Tickets", crate::checked!("ticket.list", list))
        .icon("settings")
        .noundo()
        .doc_only()
        .contract(&LIST_CONTRACT)
        .output(crate::schema::output_schema_for::<TicketsReply>)
        .params(
            "→ tickets: every part ticket of the open project by number, each with id, parent, title, text, requirements, parts, attachments, waits, \
             worker, checks, state, check and deleted (a person deleted it; it can be restored)",
        ),
    CommandSpec::new("ticket.inspect", "Inspect Part Ticket", crate::checked!("ticket.inspect", inspect))
        .icon("settings")
        .noundo()
        .doc_only()
        .contract(&INSPECT_CONTRACT)
        .output(crate::schema::output_schema_for::<TicketReply>)
        .params("id: T-2 → ticket, and events: its record, oldest first (who, when, what changed)"),
    CommandSpec::new("ticket.draft", "Draft Part Ticket", crate::checked!("ticket.draft", draft))
        .icon("settings")
        .noundo()
        .doc_only()
        .contract(&DRAFT_CONTRACT)
        .output(crate::schema::output_schema_for::<TicketReply>)
        .params(
            "title; text?; requirements?: [one line each, with numbers and units]; parts?: [{kind: component | body, id}], none, one or several; \
             attachments?: [paths of files in the project folder]; parent?: a ticket's id, to draft a sub-ticket (one level deep); \
             waits?: [ids of tickets it waits for]; checks?: false when no check applies; view?: true to attach a picture of the 3D view \
             → a draft, one piece of work. There is no top ticket. A person accepts it in the Tickets panel before anyone works on it",
        ),
    CommandSpec::new("ticket.update", "Update Part Ticket", crate::checked!("ticket.update", update))
        .icon("settings")
        .noundo()
        .doc_only()
        .contract(&UPDATE_CONTRACT)
        .output(crate::schema::output_schema_for::<TicketReply>)
        .params(
            "id; action: finished (the ticket you work has its changes; a person starts its check, or confirms it when no check applies) \
             | block (reason required) | unblock; reason? → the ticket. No check starts by itself. A person accepts drafts, starts checks, \
             changes states and finishes tickets in the Tickets panel; no command does those",
        ),
];

fn root(s: &Session, id: &str) -> Result<PathBuf> {
    let path = s.path.as_deref().ok_or_else(|| bad(id, "the open design is not saved in a project; make or open a project first"))?;
    Project::locate(Path::new(path)).ok_or_else(|| bad(id, "the open design is not in a project; make or open a project first"))
}

fn refusal(id: &str, error: TicketError) -> crate::EngineError {
    bad(id, error.to_string())
}

fn reply(id: &str, root: &Path, ticket: tickets::Ticket) -> Result<Value> {
    let events = tickets::events(root, Some(&ticket.id)).map_err(|error| refusal(id, error))?;
    serde_json::to_value(TicketReply { ticket, events }).map_err(|error| bad(id, error.to_string()))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let root = root(s, "ticket.list")?;
    let tickets = tickets::list(&root).map_err(|error| refusal("ticket.list", error))?;
    serde_json::to_value(TicketsReply { tickets }).map_err(|error| bad("ticket.list", error.to_string()))
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let params: TicketInspectParams = serde_json::from_value(p.clone()).map_err(|error| bad("ticket.inspect", error.to_string()))?;
    let root = root(s, "ticket.inspect")?;
    let ticket = tickets::load(&root, params.id.trim()).map_err(|error| refusal("ticket.inspect", error))?;
    reply("ticket.inspect", &root, ticket)
}

fn draft(s: &mut Session, p: &Value) -> Result<Value> {
    let params: TicketDraftParams = serde_json::from_value(p.clone()).map_err(|error| bad("ticket.draft", error.to_string()))?;
    let root = root(s, "ticket.draft")?;
    let draft = Draft {
        title: params.title,
        text: params.text.unwrap_or_default(),
        requirements: params.requirements,
        parent: params.parent,
        parts: params.parts,
        attachments: params.attachments,
        waits: params.waits,
        checks: params.checks,
        view: params.view,
    };
    let ticket = tickets::draft(&root, draft, tickets::Actor::Agent).map_err(|error| refusal("ticket.draft", error))?;
    reply("ticket.draft", &root, ticket)
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let params: TicketUpdateParams = serde_json::from_value(p.clone()).map_err(|error| bad("ticket.update", error.to_string()))?;
    let root = root(s, "ticket.update")?;
    let id = params.id.trim();
    let ticket = match params.action.trim() {
        "finished" => tickets::finished(&root, id, params.reason),
        "block" => tickets::block(&root, id, params.reason.as_deref().unwrap_or_default()),
        "unblock" => tickets::unblock(&root, id, params.reason),
        "check" => {
            return Err(bad(
                "ticket.update",
                "no check starts by itself: say `finished`, and a person starts the ticket's check in the Tickets panel",
            ));
        }
        "confirm" | "confirmed" | "accept" | "decline" | "done" | "complete" | "start" | "state" | "reopen" | "delete" | "restore" => {
            return Err(bad(
                "ticket.update",
                "only a person accepts, starts, changes the state of, finishes, deletes or restores a ticket, with the buttons in the Tickets panel",
            ));
        }
        other => return Err(bad("ticket.update", format!("`{other}` is not an action; use finished, block or unblock"))),
    }
    .map_err(|error| refusal("ticket.update", error))?;
    reply("ticket.update", &root, ticket)
}
