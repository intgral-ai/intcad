//! `ticket.list`, `ticket.inspect`, `ticket.draft` and `ticket.update`: the agent's way to the
//! project's part tickets (`crate::project::tickets`).
//!
//! These four are all there is. Drafting makes drafts only; an update asks for the check of the
//! ticket in progress or says it is blocked. Confirming a draft, starting a ticket and marking it
//! done are a person's buttons in the Tickets panel, so no command does them. Every step a command
//! makes is recorded as the agent's.

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
        .params("→ tickets: every part ticket of the open project, the top ticket first, each with id, parent, title, text, requirements, part, state and check"),
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
            "title; text?; requirements?: [one line each, with numbers and units]; parent?: the top ticket's id; part?: {kind: component | body, id} \
             → a draft. A person confirms it in the Tickets panel before anyone works on it. The first ticket of a project is its top ticket; \
             later ones go under it, one per part",
        ),
    CommandSpec::new("ticket.update", "Update Part Ticket", crate::checked!("ticket.update", update))
        .icon("settings")
        .noundo()
        .doc_only()
        .contract(&UPDATE_CONTRACT)
        .output(crate::schema::output_schema_for::<TicketReply>)
        .params(
            "id; action: check (the ticket in progress has its changes: run its check; it then waits for a person) | block (reason required) | unblock; \
             reason? → the ticket. A person confirms drafts and marks tickets done in the Tickets panel; no command does either",
        ),
];

fn root(s: &Session, id: &str) -> Result<PathBuf> {
    let path = s.path.as_deref().ok_or_else(|| bad(id, "the open design is not saved in a project; make or open a project first"))?;
    Ok(Project::locate(Path::new(path)).ok_or_else(|| bad(id, "the open design is not in a project; make or open a project first"))?)
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
        part: params.part,
    };
    let ticket = tickets::draft(&root, draft, tickets::Actor::Agent).map_err(|error| refusal("ticket.draft", error))?;
    reply("ticket.draft", &root, ticket)
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let params: TicketUpdateParams = serde_json::from_value(p.clone()).map_err(|error| bad("ticket.update", error.to_string()))?;
    let root = root(s, "ticket.update")?;
    let id = params.id.trim();
    let ticket = match params.action.trim() {
        "check" => tickets::ask_check(&root, id, tickets::Actor::Agent, params.reason),
        "block" => tickets::block(&root, id, params.reason.as_deref().unwrap_or_default()),
        "unblock" => tickets::unblock(&root, id, params.reason),
        "confirm" | "confirmed" | "done" | "complete" | "start" => {
            return Err(bad("ticket.update", "only a person confirms, starts or marks a ticket done, with the buttons in the Tickets panel"));
        }
        other => return Err(bad("ticket.update", format!("`{other}` is not an action; use check, block or unblock"))),
    }
    .map_err(|error| refusal("ticket.update", error))?;
    reply("ticket.update", &root, ticket)
}
