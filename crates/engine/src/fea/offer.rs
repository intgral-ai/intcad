//! How far this build offers the stress estimate: the release switch (spec decision 32).
//!
//! The stress estimate merges into `develop` before it works end to end in an installed app, so
//! a release keeps it out of sight until bar 3 is accepted (a person's run by hand, #270) and
//! then bar 3b (an agent's run, admitted and charged). Each level adds to the one before:
//! - [`Offer::Off`]: no Stress button, and an agent's estimate is refused before it starts.
//! - [`Offer::ByHand`]: the Stress button opens the Simulation panel for a run by hand (#270);
//!   an agent's estimate is still refused.
//! - [`Offer::Agents`]: an agent may ask for an estimate too, through the chat's Stress estimate
//!   mode and `inspect.stress_estimate`; it is then admitted and charged as any agent call is.
//!
//! The app's preference `stress_estimate` holds `auto` (the default), `off`, `by_hand` or
//! `agents`. `auto` follows the build, so a later release that changes the default reaches people
//! who never set it. Every preference is saved whole, so a saved `off` would never change.
//! A person's own run by hand (the free GUI entry) is never refused here: the switch hides the
//! controls that start it instead.

use serde_json::Value;

/// The preference value that follows the build ([`Offer::for_build`]).
pub const AUTO: &str = "auto";

/// The environment variable that sets `auto`'s level for one run: `off`, `by_hand` or `agents`.
/// The headless `intcad-cli mcp` has no preferences of its own, so this is how a tester turns the
/// estimate on there.
pub const ENV: &str = "INTCAD_STRESS_ESTIMATE";

/// How far the stress estimate is offered. Ordered: each level includes the ones before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Offer {
    /// Out of sight: no Stress button, no Simulation panel setup, no agent route.
    Off,
    /// A person's run by hand from the Simulation panel; an agent's estimate is refused.
    ByHand,
    /// A person's run by hand, and an agent's estimate.
    Agents,
}

impl Offer {
    /// The preference values a person may set, `auto` first.
    pub const VALUES: [&'static str; 4] = [AUTO, "off", "by_hand", "agents"];

    /// A level by its preference name; `auto` and anything unknown are `None`.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim() {
            "off" => Some(Self::Off),
            "by_hand" => Some(Self::ByHand),
            "agents" => Some(Self::Agents),
            _ => None,
        }
    }

    /// The level's preference name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ByHand => "by_hand",
            Self::Agents => "agents",
        }
    }

    /// What `auto` means for a build: nothing in a release build, or in the browser, where the
    /// estimate cannot run (it starts gmsh and CalculiX on this computer); everything in a
    /// development build or a test, so the stack's own tests and runs by hand keep their meaning.
    pub fn for_build(release: bool, browser: bool) -> Self {
        if release || browser { Self::Off } else { Self::Agents }
    }

    /// The level a saved preference gives: `auto` (or nothing) is `env`'s level when it names
    /// one, else the build's; a level's name is that level; anything else is `Off`, so a value
    /// this build does not know never turns the estimate on.
    pub fn resolve(setting: &str, build: Self, env: Option<&str>) -> Self {
        match setting.trim() {
            "" | AUTO => env.and_then(Self::parse).unwrap_or(build),
            other => Self::parse(other).unwrap_or(Self::Off),
        }
    }

    /// This build's `auto`, with [`ENV`] read now.
    pub fn this_build() -> Self {
        let env = std::env::var(ENV).ok();
        Self::resolve(AUTO, Self::for_build(release_build(), cfg!(target_arch = "wasm32")), env.as_deref())
    }

    /// A person can start the estimate from the Stress button.
    pub fn button(self) -> bool {
        self >= Self::ByHand
    }

    /// An agent may ask for the estimate.
    pub fn agents(self) -> bool {
        self >= Self::Agents
    }
}

/// The release workflow built this binary: its `SOLIDCRAFT_VERSION` is set, and it is not a
/// debug build. The same test as the app's update check (`update::release_build`).
pub fn release_build() -> bool {
    option_env!("SOLIDCRAFT_VERSION").is_some_and(|v| !v.is_empty()) && !cfg!(debug_assertions)
}

/// Whether `id` names one of the estimate's commands (the estimate itself, or showing its
/// result), resolved as the engine resolves an id: trimmed, exact, then ignoring ASCII case.
pub fn is_estimate_command(id: &str) -> bool {
    crate::find_command(id).is_some_and(|c| c.id == "inspect.stress_estimate" || c.id.starts_with("inspect.stress_estimate."))
}

/// Whether a request asks for the estimate, by any route a request takes: `execute` (a command
/// and its parameters), `batch` and `engine.script` (a list of commands), or `ui.start` (a
/// command started in the window). `method` is the control method (`engine.execute`, …) or the
/// MCP tool's name (`execute`, …); the parameters are the request's own. A step names its
/// command in `command` or `id`, as the control channel reads it.
pub fn asks_for_estimate(method: &str, params: &Value) -> bool {
    let named = |v: &Value| [v.get("command"), v.get("id")].into_iter().flatten().filter_map(Value::as_str).any(is_estimate_command);
    let listed = |list: Option<&Vec<Value>>| list.is_some_and(|steps| steps.iter().any(named));
    match method {
        "engine.execute" | "execute" | "command" => named(params),
        "engine.batch" | "batch" => listed(params.get("commands").and_then(Value::as_array)),
        "engine.script" => listed(params.as_array().or_else(|| params.get("commands").and_then(Value::as_array))),
        "ui.start" => named(params),
        _ => false,
    }
}

/// The refusal an agent gets while the switch is below [`Offer::Agents`]: plain words, and that
/// asking again will not help.
pub fn agent_refusal(offer: Offer) -> String {
    format!(
        "the stress estimate is not available to agents in this build (stress_estimate is {}): do not ask for it again; tell the \
         user it is not offered to agents yet",
        offer.name()
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// S6: `auto` follows the build, then the environment; a named level wins; anything unknown
    /// is off.
    #[test]
    fn auto_follows_the_build_and_an_unknown_value_is_off() {
        assert_eq!(Offer::for_build(true, false), Offer::Off, "a release build");
        assert_eq!(Offer::for_build(false, true), Offer::Off, "the browser cannot run it");
        assert_eq!(Offer::for_build(false, false), Offer::Agents, "a development build or a test");
        for build in [Offer::Off, Offer::Agents] {
            assert_eq!(Offer::resolve(AUTO, build, None), build);
            assert_eq!(Offer::resolve("", build, None), build, "nothing saved yet");
            assert_eq!(Offer::resolve(AUTO, build, Some("by_hand")), Offer::ByHand, "the environment sets auto's level");
            assert_eq!(Offer::resolve(AUTO, build, Some("sideways")), build, "an unknown environment value is ignored");
            assert_eq!(Offer::resolve("agents", build, Some("off")), Offer::Agents, "a set preference wins over the environment");
            assert_eq!(Offer::resolve("by_hand", build, None), Offer::ByHand);
            assert_eq!(Offer::resolve("off", build, None), Offer::Off);
            assert_eq!(Offer::resolve("on", build, None), Offer::Off, "unknown values are off");
            assert_eq!(Offer::resolve("Agents!", build, None), Offer::Off);
        }
        assert!(!Offer::Off.button() && !Offer::Off.agents());
        assert!(Offer::ByHand.button() && !Offer::ByHand.agents());
        assert!(Offer::Agents.button() && Offer::Agents.agents());
        for v in Offer::VALUES.iter().skip(1) {
            assert_eq!(Offer::parse(v).map(Offer::name), Some(*v));
        }
    }

    /// Every route a request takes to the estimate is recognised, and nothing else is.
    #[test]
    fn a_request_for_the_estimate_is_recognised_on_every_route() {
        let estimate = json!({"command": "inspect.stress_estimate", "params": {"dry_run": true}});
        let show = json!({"command": "inspect.stress_estimate.show", "params": {"show": "stress"}});
        let other = json!({"command": "inspect.measure", "params": {}});
        for method in ["engine.execute", "execute", "command"] {
            assert!(asks_for_estimate(method, &estimate), "{method}");
            assert!(asks_for_estimate(method, &show), "{method}: showing the result");
            assert!(!asks_for_estimate(method, &other), "{method}");
        }
        for method in ["engine.batch", "batch"] {
            assert!(asks_for_estimate(method, &json!({"commands": [other, estimate]})), "{method}");
            assert!(!asks_for_estimate(method, &json!({"commands": [other]})), "{method}");
        }
        assert!(asks_for_estimate("engine.script", &json!([other, estimate])));
        assert!(asks_for_estimate("engine.script", &json!({"commands": [show]})));
        assert!(!asks_for_estimate("engine.script", &json!([other])));
        assert!(asks_for_estimate("ui.start", &json!({"command": "inspect.stress_estimate"})));
        assert!(!asks_for_estimate("ui.start", &json!({"command": "inspect.measure"})));
        assert!(!asks_for_estimate("ui.screenshot", &estimate), "a method that runs no command");
        assert!(!is_estimate_command("inspect.stress_estimates"), "a prefix of another id is not the estimate");
        // Spelt as the engine still runs it: other case, spaces around it, or under `id`.
        assert!(asks_for_estimate("engine.execute", &json!({"command": " Inspect.Stress_Estimate "})));
        assert!(asks_for_estimate("engine.execute", &json!({"id": "inspect.stress_estimate"})));
        assert!(asks_for_estimate("engine.batch", &json!({"commands": [{"id": "INSPECT.STRESS_ESTIMATE.SHOW"}]})));
    }

    #[test]
    fn the_agents_refusal_says_not_to_ask_again() {
        let words = agent_refusal(Offer::ByHand);
        assert!(words.contains("not available to agents") && words.contains("by_hand") && words.contains("do not ask for it again"), "{words}");
    }
}
