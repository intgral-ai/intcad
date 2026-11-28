//! The stress estimate's release switch in the app (spec decision 32): the `stress_estimate`
//! preference read as a level ([`Offer`]), what it hides from people, and what it refuses agents.
//!
//! - **Stress button** (the ribbon's Inspect ▸ Estimate, its drop-down, command search, a typed
//!   command line, `ui.start`, shortcuts): offered from `by_hand`, where it opens the right
//!   rail's Simulation panel for a run by hand. Without the rail it opens the chat, the agent
//!   route, so there it needs `agents`.
//! - **The Simulation panel's setup** shows from `by_hand`; at `off` the panel is empty, as before
//!   the run by hand. Its "Ask the Assistant instead" needs `agents`.
//! - **An agent's estimate**: refused before it starts, by any route (the chat's gate, the held
//!   queue the packaged Agent and the bridge use, the control port, `batch`, `engine.script`,
//!   `ui.start`), and left out of `engine.commands`, until the level is `agents`.
//! - **A person's own run** (the free GUI entry) is never refused here; the switch hides what
//!   starts it.
//! - **The control port driving the window** (spec decision 37): a Run, or a typed estimate line,
//!   entered by input the port injected (`ui.click`, `ui.key`, …) is the port's request, not the
//!   person's ([`crate::stress_by_hand::StartedBy`]). It is refused as an agent's is until
//!   `agents`, and then takes the port's own charged entry.
//!
//! An agent cannot raise the level itself: `ui.prefs` from a controlled entry may not set it
//! ([`set_by_control_refusal`]). A tester sets it in the settings file, or starts the app with
//! `INTCAD_STRESS_ESTIMATE` (which sets `auto`'s level).

use std::sync::OnceLock;

use serde_json::Value;
use solidcraft_engine::entitlement::EntryCategory;
use solidcraft_engine::fea::offer::{self, Offer};

use crate::SolveApp;

/// The preference's name (`ui.prefs`, the settings file).
pub(crate) const PREFERENCE: &str = "stress_estimate";

/// `auto`'s level for this run: the build's, or `INTCAD_STRESS_ESTIMATE`'s, read once.
fn auto() -> Offer {
    static AUTO: OnceLock<Offer> = OnceLock::new();
    *AUTO.get_or_init(Offer::this_build)
}

/// The level the preference gives now.
pub(crate) fn offer(app: &SolveApp) -> Offer {
    Offer::resolve(&app.preferences.stress_estimate, auto(), None)
}

/// The Stress button is offered: from `by_hand`, where it opens the Simulation panel's run by
/// hand. Without the right rail it opens the chat instead, which is the agent route, so there it
/// needs `agents`.
pub(crate) fn button(app: &SolveApp) -> bool {
    let level = offer(app);
    if crate::right_rail::shown(app) { level.by_hand() } else { level.agents() }
}

/// The panel offers to ask the Assistant instead: the agent route.
pub(crate) fn ask_assistant(app: &SolveApp) -> bool {
    offer(app).agents()
}

/// A person may run the estimate by hand: from `by_hand`. A typed `inspect.stress_estimate {…}`
/// line is such a run (it opens no chat), so it waits only for this, not for [`button`].
pub(crate) fn by_hand(app: &SolveApp) -> bool {
    offer(app).by_hand()
}

/// Command `id` is one of the estimate's and the switch hides it from people now.
pub(crate) fn hides(app: &SolveApp, id: &str) -> bool {
    offer::is_estimate_command(id) && !button(app)
}

/// Why an agent's request for the estimate is refused now, or `None` when it may go on.
pub(crate) fn agent_refusal(app: &SolveApp, method: &str, params: &Value) -> Option<String> {
    let level = offer(app);
    (!level.agents() && offer::asks_for_estimate(method, params)).then(|| offer::agent_refusal(level))
}

/// [`agent_refusal`] for a request on the control queue: a person's own (free) entry passes.
pub(crate) fn request_refusal(app: &SolveApp, entry: EntryCategory, method: &str, params: &Value) -> Option<String> {
    if crate::control::free_entry(entry) { None } else { agent_refusal(app, method, params) }
}

/// `engine.commands`' list without the estimate's commands while agents may not ask for it.
pub(crate) fn filter_commands(app: &SolveApp, list: Value) -> Value {
    if offer(app).agents() {
        return list;
    }
    match list {
        Value::Array(items) => {
            Value::Array(items.into_iter().filter(|c| !c.get("id").and_then(Value::as_str).is_some_and(offer::is_estimate_command)).collect())
        }
        other => other,
    }
}

/// `ui.prefs` from a controlled entry (an agent, a script on the control port) may not set the
/// switch, or an agent could turn its own route on.
pub(crate) fn set_by_control_refusal(entry: EntryCategory, params: &Value) -> Option<String> {
    (!crate::control::free_entry(entry) && params.get(PREFERENCE).is_some())
        .then(|| format!("{PREFERENCE} is set by a person: in the settings file, or with {} when the app starts", offer::ENV))
}

#[cfg(test)]
#[path = "stress_switch_tests.rs"]
mod tests;
