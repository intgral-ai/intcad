//! Wire shapes for `ticket.list`, `ticket.inspect`, `ticket.draft` and `ticket.update`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::project::tickets::{Event, Part, Ticket};

/// List the project's part tickets (no parameters).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TicketListParams {}

/// One ticket by its id.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TicketInspectParams {
    /// The ticket's id, such as `T-2`.
    pub id: String,
}

/// Draft one ticket: a piece of work. A draft waits for a person to accept it in the Tickets panel.
/// Unknown keys are refused, so a draft in the old one-`part` shape does not lose its part.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TicketDraftParams {
    /// What is to be done, in a few words.
    pub title: String,
    /// What the ticket asks for, in full.
    #[serde(default)]
    pub text: Option<String>,
    /// The requirement lines it carries, one each, with numbers and units; may be empty.
    #[serde(default)]
    pub requirements: Vec<String>,
    /// The ticket it is a sub-ticket of (one level deep). Left out: a ticket of its own; there is
    /// no top ticket.
    #[serde(default)]
    pub parent: Option<String>,
    /// The parts it bears on, none, one or several: `[{kind: component | body, id}]`.
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Files already in the project folder it points to, as paths relative to that folder (never
    /// inside `.intcad/`).
    #[serde(default)]
    pub attachments: Vec<String>,
    /// The tickets it waits for, such as `["T-3"]`: the Assistant takes it once those are done.
    #[serde(default)]
    pub waits: Vec<String>,
    /// Whether a check applies to it; left out: one does. `false` for a ticket no check can
    /// judge (a discussion of a constraint, say).
    #[serde(default)]
    pub checks: Option<bool>,
    /// Ask the app to attach a picture of the 3D view as it is now.
    #[serde(default)]
    pub view: bool,
}

/// Say the ticket the Assistant works is finished, or that it is blocked or taken up again.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TicketUpdateParams {
    pub id: String,
    /// `finished` (its changes are made; a person starts its check, or confirms it when no check
    /// applies), `block` (with the reason) or `unblock`.
    pub action: String,
    /// Why it is blocked, or a note for the record.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Every ticket of the project.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TicketsReply {
    pub tickets: Vec<Ticket>,
}

/// One ticket and its record.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TicketReply {
    pub ticket: Ticket,
    pub events: Vec<Event>,
}

#[cfg(test)]
#[path = "ticket_tests.rs"]
mod tests;
