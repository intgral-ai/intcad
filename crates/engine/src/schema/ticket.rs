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

/// Draft one ticket. A draft waits for a person to confirm it in the Tickets panel.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TicketDraftParams {
    /// The part's name and what it must meet, in a few words.
    pub title: String,
    /// What the ticket asks for, in full.
    #[serde(default)]
    pub text: Option<String>,
    /// The requirement lines it carries, one each, with numbers and units.
    #[serde(default)]
    pub requirements: Vec<String>,
    /// The top ticket's id. Left out: the new ticket goes under the top ticket, or becomes the top
    /// ticket when the project has none yet.
    #[serde(default)]
    pub parent: Option<String>,
    /// The part it stands for: `{kind: component | body, id}`.
    #[serde(default)]
    pub part: Option<Part>,
}

/// Ask for the check of the ticket in progress, or say it is blocked or taken up again.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TicketUpdateParams {
    pub id: String,
    /// `check` (the changes are made; run this ticket's check), `block` or `unblock`.
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
