//! The agents the chat can use: the installed ones it finds by itself (Claude Code, Codex,
//! Antigravity, OpenCode, …) and the ones the user adds (another ACP command, or an
//! OpenAI-compatible endpoint such as Ollama, LM Studio or OpenAI, or an Anthropic-compatible
//! one). Also the permission modes. Preferences only: no design data, no keys (those are in the
//! system keychain).

use serde::{Deserialize, Serialize};

/// How much the agent may do without asking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Every change to the design, file and destructive action asks first.
    Ask,
    /// Modelling commands run; files and destructive actions still ask (the default).
    #[default]
    Edits,
    /// Nothing asks (crash reports and the folder limits still hold).
    All,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Ask, Mode::Edits, Mode::All];
    pub fn id(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Edits => "edits",
            Mode::All => "all",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.id() == s)
    }
    pub fn label(self) -> &'static str {
        crate::i18n::tr(match self {
            Mode::Ask => "ui.ai_mode_ask",
            Mode::Edits => "ui.ai_mode_edits",
            Mode::All => "ui.ai_mode_all",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A program that speaks the Agent Client Protocol.
    #[default]
    Acp,
    /// An OpenAI-compatible chat completions endpoint.
    Api,
    /// An Anthropic-compatible Messages endpoint.
    Anthropic,
}

impl Kind {
    /// The id written in preferences and on the control channel.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Acp => "acp",
            Kind::Api => "api",
            Kind::Anthropic => "anthropic",
        }
    }
    /// Whether the agent is an endpoint SolidCraft talks to itself (either protocol).
    pub fn is_api(self) -> bool {
        matches!(self, Kind::Api | Kind::Anthropic)
    }
    /// The wire protocol of an added endpoint; `None` for an ACP agent.
    pub fn protocol(self) -> Option<Protocol> {
        match self {
            Kind::Acp => None,
            Kind::Api => Some(Protocol::OpenAi),
            Kind::Anthropic => Some(Protocol::Anthropic),
        }
    }
}

/// How a chat turn executes. This is separate from the AI connection it uses: the connection and
/// model stay the same, and only the thing that answers the message changes. The direct choice is
/// the default, so preferences written before this choice existed keep their behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Execution {
    /// Talk to the selected AI connection (or ACP agent) directly, as before.
    #[default]
    Direct,
    /// Hand the turn to a Mastra workflow running in the standalone runtime; the workflow uses
    /// the same selected AI connection for its model calls.
    Mastra,
}

impl Execution {
    pub const ALL: [Execution; 2] = [Execution::Direct, Execution::Mastra];

    /// The id written in preferences.
    pub fn id(self) -> &'static str {
        match self {
            Execution::Direct => "direct",
            Execution::Mastra => "mastra",
        }
    }
    pub fn parse(s: &str) -> Option<Execution> {
        Execution::ALL.into_iter().find(|e| e.id() == s)
    }
    pub fn label(self) -> &'static str {
        crate::i18n::tr(match self {
            Execution::Direct => "ui.mastra_execution_direct",
            Execution::Mastra => "ui.mastra_execution_workflow",
        })
    }
}

/// The wire protocol of an endpoint the user added.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// `{base}/chat/completions`, as Ollama, LM Studio, OpenAI and their kin speak.
    #[default]
    OpenAi,
    /// `{base}/v1/messages`, as the Anthropic Messages API and compatible endpoints speak.
    Anthropic,
}

impl Protocol {
    pub fn kind(self) -> Kind {
        match self {
            Protocol::OpenAi => Kind::Api,
            Protocol::Anthropic => Kind::Anthropic,
        }
    }
}

/// An agent the user added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// ACP: the command line.
    pub command: String,
    /// API: the endpoint (`…/v1`) and model.
    pub base_url: String,
    pub model: String,
    /// Added with the Ollama preset (even if its display name changes).
    pub ollama: bool,
    /// API: a key is stored for it in the system keychain.
    pub key: bool,
    /// API: how long one model request may take, in seconds (clamped to
    /// [`MIN_REQUEST_TIMEOUT_S`]..=[`MAX_REQUEST_TIMEOUT_S`] where it is used).
    pub request_timeout_s: u64,
    /// API: the reasoning effort to send as `reasoning_effort`; empty sends nothing and the
    /// provider's own default applies (trimmed and length-capped by [`reasoning_effort`]).
    pub reasoning_effort: String,
}

impl Default for AgentConfig {
    fn default() -> AgentConfig {
        AgentConfig {
            id: String::new(),
            name: String::new(),
            kind: Kind::default(),
            command: String::new(),
            base_url: String::new(),
            model: String::new(),
            ollama: false,
            key: false,
            request_timeout_s: DEFAULT_REQUEST_TIMEOUT_S,
            reasoning_effort: String::new(),
        }
    }
}

/// How long one model request to an endpoint may take by default, in seconds.
pub const DEFAULT_REQUEST_TIMEOUT_S: u64 = 300;
/// The shortest request timeout an endpoint can have.
pub const MIN_REQUEST_TIMEOUT_S: u64 = 30;
/// The longest request timeout an endpoint can have (an hour).
pub const MAX_REQUEST_TIMEOUT_S: u64 = 3600;

/// A request timeout in range: seconds from a preferences file, the dialog or the control
/// channel, rounded and clamped; anything not a finite number is the default.
pub fn request_timeout_s(secs: f64) -> u64 {
    if !secs.is_finite() {
        return DEFAULT_REQUEST_TIMEOUT_S;
    }
    // Clamped first, so the cast is exact.
    secs.round().clamp(MIN_REQUEST_TIMEOUT_S as f64, MAX_REQUEST_TIMEOUT_S as f64) as u64
}

/// The longest reasoning effort an endpoint can be given (no provider names one near this).
pub const MAX_REASONING_EFFORT_LEN: usize = 64;

/// A reasoning effort as stored: trimmed, and capped so a hand-edited preferences file or the
/// control channel cannot carry an absurd value into every request. Empty means "send nothing".
pub fn reasoning_effort(raw: &str) -> String {
    raw.trim().chars().take(MAX_REASONING_EFFORT_LEN).collect()
}

/// An agent as the picker and the switcher show it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// ACP: what to start.
    pub argv: Vec<String>,
    pub version: Option<String>,
    /// `login_found`, `key`, `signed_out`, `unknown` from discovery, `signed_in` after a
    /// successful connection, or empty for an untested custom agent.
    pub login: &'static str,
    /// Found installed (not added by the user).
    pub found: bool,
    pub base_url: String,
    pub model: String,
    pub key: bool,
    /// API: seconds one model request may take (in range).
    pub request_timeout_s: u64,
    /// API: the reasoning effort to send, empty for none (trimmed, in length).
    pub reasoning_effort: String,
    /// Found installed, but its program didn't start when asked its version: why (#9034).
    pub problem: Option<String>,
    /// Found installed: the command that installs it again.
    pub install: &'static str,
    /// Found installed, though its maker has retired it: used when picked, and the default only
    /// while no other agent can be.
    pub retired: bool,
}

impl Entry {
    fn from_config(c: &AgentConfig) -> Entry {
        Entry {
            id: c.id.clone(),
            name: if c.name.trim().is_empty() { c.id.clone() } else { c.name.clone() },
            kind: c.kind,
            argv: super::acp::split_command(&c.command),
            version: None,
            login: "",
            found: false,
            base_url: c.base_url.clone(),
            model: c.model.clone(),
            key: c.key,
            request_timeout_s: c.request_timeout_s.clamp(MIN_REQUEST_TIMEOUT_S, MAX_REQUEST_TIMEOUT_S),
            reasoning_effort: reasoning_effort(&c.reasoning_effort),
            problem: None,
            install: "",
            retired: false,
        }
    }
    /// What is known of an installed agent without starting it: the catalog key of the word its
    /// row in the picker shows. "Signed in" only after a connection from this window succeeded,
    /// and installed is all that is claimed otherwise. `None` for an agent the user added.
    pub fn status(&self) -> Option<&'static str> {
        self.found.then_some(match (&self.problem, self.retired, self.login) {
            (Some(_), _, _) => "ui.agent_chat_does_not_start",
            (None, true, _) => "ui.agent_chat_retired",
            (None, false, "signed_in") => "ui.agent_chat_signed_in",
            (None, false, "signed_out") => "ui.agent_chat_not_signed_in",
            _ => "ui.agent_chat_installed",
        })
    }
    /// A short line under the name: the version, the model, the command, or why it can't start.
    pub fn detail(&self) -> String {
        match self.kind {
            Kind::Api | Kind::Anthropic => format!("{} · {}", self.model, self.base_url),
            Kind::Acp => match (&self.problem, &self.version) {
                (Some(p), _) => crate::i18n::format("ui.agent_chat_cannot_start", &[("error", p)]),
                (None, Some(v)) => v.clone(),
                (None, None) => super::panel::short_command(&self.argv.join(" ")),
            },
        }
    }
    /// What to tell someone starting an agent that was found but didn't start (#9034): the reason,
    /// and how to install it again. `None` when nothing was seen wrong.
    pub fn problem_hint(&self) -> Option<String> {
        let problem = self.problem.as_deref()?;
        let program = self.argv.first().map(String::as_str).unwrap_or_default();
        Some(crate::i18n::format(
            "ui.agent_chat_reinstall",
            &[("agent", &self.name), ("program", program), ("error", problem), ("install", self.install)],
        ))
    }
}

/// The built-in agent, as found when the app starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Builtin {
    /// Its program: an absolute path.
    pub program: std::path::PathBuf,
    /// What the program is started with: nothing for a program named by the environment variable,
    /// the agent's entry file for the Node carried in the package.
    pub args: Vec<String>,
    /// The folder of the app's own where it keeps its sign-ins and its other data, so it shares
    /// none with a Mastra Code the user installed (`None`: no such folder could be named, and it
    /// keeps them where it does by itself).
    pub data: Option<std::path::PathBuf>,
}

/// Look for the built-in agent, once, when the app starts: the environment variable
/// `SOLIDCRAFT_MASTRA_AGENT` names its program, and without it the package the app runs from may
/// carry one ([`packaged_builtin`]). `None`: there is none, and the chat uses the agents it finds
/// and the ones the user added, as it did before there was a built-in one.
/// Tests name the agent themselves ([`crate::SolveApp::builtin_agent`]): a variable left in a
/// developer's shell must not change what they see.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn look_for_builtin() -> Option<Builtin> {
    if cfg!(test) {
        return None;
    }
    let (program, args) = match builtin_program(solidcraft_engine::geom::env::var("MASTRA_AGENT")) {
        Some(program) => (program, Vec::new()),
        None => packaged_builtin(&std::env::current_exe().ok()?)?,
    };
    Some(Builtin { program, args, data: builtin_data(std::env::var_os("SOLIDCRAFT_SETTINGS_DIR"), solidcraft_engine::recovery::data_dir()) })
}

/// The browser build starts no agent.
#[cfg(target_arch = "wasm32")]
pub(crate) fn look_for_builtin() -> Option<Builtin> {
    None
}

/// The program a `SOLIDCRAFT_MASTRA_AGENT` value names: an absolute path to a file. Anything
/// else is no built-in agent, so a mistyped value leaves the chat as it was.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn builtin_program(value: Option<String>) -> Option<std::path::PathBuf> {
    let program = std::path::PathBuf::from(value.filter(|value| !value.trim().is_empty())?);
    (program.is_absolute() && program.is_file()).then_some(program)
}

/// The folder a package carries the built-in agent in, beside the app by the rules the Mastra
/// workflow runtime's folder follows ([`super::packaged_runtime::roots_named`]). It is a folder of
/// its own: `mastra/` is that runtime's, with a receipt this one does not have.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) const PACKAGED_AGENT_DIR: &str = "mastra-agent";

/// The built-in agent a package carries for the app at `exe`: its own Node and the entry file that
/// Node is started with. The folder is what says the package carries the agent. A package that
/// has it has the built-in agent and nothing else, also when the folder is damaged: the agent then
/// fails to start and the Assistant shows that, and never goes back to the agents found on the
/// computer. A package without the folder has no built-in agent, and the chat is as it was.
/// Nothing outside the package is looked at: not `PATH`, not a Node the user installed.
#[cfg(not(target_arch = "wasm32"))]
fn packaged_builtin(exe: &std::path::Path) -> Option<(std::path::PathBuf, Vec<String>)> {
    let node = if cfg!(windows) { "node.exe" } else { "node" };
    let roots = super::packaged_runtime::roots_named(exe, PACKAGED_AGENT_DIR);
    // The place that holds the Node program, else the first place where the folder is at all.
    let root = roots.iter().find(|root| root.join(node).is_file()).or_else(|| roots.iter().find(|root| root.is_dir()))?;
    Some((root.join(node), vec![root.join("agent").join("main.mjs").to_string_lossy().into_owned()]))
}

/// Where the built-in agent keeps its data: `mastra` in the app's settings folder. `chosen` is
/// the settings folder a run was given for itself (`SOLIDCRAFT_SETTINGS_DIR`), which the app's
/// other files follow too; `own` is the app's per-user folder otherwise.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn builtin_data(chosen: Option<std::ffi::OsString>, own: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    let settings = chosen.map(std::path::PathBuf::from).filter(|folder| folder.is_absolute()).or(own)?;
    Some(settings.join("mastra"))
}

/// Whether the chat has its built-in agent. With it the agent list is that one agent: nothing
/// is looked for on the computer, and the agents of an older version's preferences are not offered.
pub fn is_builtin(app: &crate::SolveApp) -> bool {
    app.builtin_agent.is_some()
}

/// The built-in agent as the agent list shows it.
fn builtin(app: &crate::SolveApp) -> Option<Entry> {
    let found = app.builtin_agent.as_ref()?;
    Some(Entry {
        id: super::connections::AGENT_ID.to_string(),
        name: super::connections::AGENT_NAME.to_string(),
        kind: Kind::Acp,
        argv: std::iter::once(found.program.to_string_lossy().into_owned()).chain(found.args.iter().cloned()).collect(),
        version: None,
        login: "",
        found: false,
        base_url: String::new(),
        model: String::new(),
        key: false,
        request_timeout_s: DEFAULT_REQUEST_TIMEOUT_S,
        reasoning_effort: String::new(),
        problem: None,
        install: "",
        retired: false,
    })
}

/// Every agent the chat can use: the built-in one alone when there is one, else the installed
/// ones found, then the ones the user added.
pub fn entries(app: &crate::SolveApp) -> Vec<Entry> {
    if let Some(entry) = builtin(app) {
        return vec![entry];
    }
    let mut out = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(d) = &app.agent_chat.detection {
        out.extend(d.found.iter().map(|f| Entry {
            id: f.id.to_string(),
            name: f.name.to_string(),
            kind: Kind::Acp,
            argv: f.argv.clone(),
            version: f.version.clone(),
            login: f.login.id(),
            found: true,
            base_url: String::new(),
            model: String::new(),
            key: false,
            request_timeout_s: DEFAULT_REQUEST_TIMEOUT_S,
            reasoning_effort: String::new(),
            problem: f.problem.clone(),
            install: f.install,
            retired: super::discover::KNOWN.iter().any(|known| known.id == f.id && known.retired),
        }));
    }
    out.extend(app.preferences.agents.iter().map(Entry::from_config));
    for entry in &mut out {
        if entry.kind == Kind::Acp
            && let Some(signed_in) = app.agent_chat.login_results.get(&entry.id)
        {
            entry.login = if *signed_in { "signed_in" } else { "signed_out" };
        }
    }
    out
}

/// The agent used while none is picked or saved as the default: the first that can be expected
/// to work (its program starts, and its maker has not retired it), else the first whose program
/// starts, else the first. Found agents come in the order of `discover::KNOWN`, so Claude Code is
/// looked for first and Codex second, then the agents the user added. It was the first entry
/// whatever its state, which made a retired command line without an ACP mode the default while a
/// working agent stood next in the list. Whether a program speaks ACP is not checked here.
pub fn default_of(all: &[Entry]) -> Option<&Entry> {
    all.iter().find(|e| e.problem.is_none() && !e.retired).or_else(|| all.iter().find(|e| e.problem.is_none())).or_else(|| all.first())
}

/// The agent a conversation uses: the one picked in the panel, else the saved default
/// (`agent_default`, which names any listed agent, a retired one too), else [`default_of`].
pub fn current(app: &crate::SolveApp) -> Option<Entry> {
    let all = entries(app);
    let pick = |id: &str| all.iter().find(|e| e.id == id).cloned();
    app.agent_chat.agent_id.as_deref().and_then(pick).or_else(|| pick(&app.preferences.agent_default)).or_else(|| default_of(&all).cloned())
}

/// The permission mode for an agent: its own, else the global one.
pub fn mode_for(p: &crate::prefs::Prefs, id: &str) -> Mode {
    p.agent_permissions.get(id).copied().unwrap_or(p.ai_permissions)
}

/// Whether `tool` is always allowed for `agent`.
pub fn always_allowed(p: &crate::prefs::Prefs, agent: &str, tool: &str) -> bool {
    p.agent_always_allow.get(agent).is_some_and(|t| t.iter().any(|x| x == tool))
}

/// Remember an "always allow" answer.
pub fn remember(p: &mut crate::prefs::Prefs, agent: &str, tool: &str) {
    let list = p.agent_always_allow.entry(agent.to_string()).or_default();
    if !list.iter().any(|x| x == tool) && list.len() < 500 {
        list.push(tool.chars().take(200).collect());
    }
}

/// A free id for a new agent: `prefix-1`, `prefix-2`, …
pub fn new_id(p: &crate::prefs::Prefs, prefix: &str) -> String {
    (1..).map(|n| format!("{prefix}-{n}")).find(|id| !p.agents.iter().any(|a| &a.id == id)).unwrap_or_else(|| prefix.to_string())
}

/// An endpoint people commonly add, with the base URL to start from.
pub struct ApiPreset {
    pub name: &'static str,
    pub base_url: &'static str,
}

/// OpenAI-compatible endpoints people use.
pub const OPENAI_PRESETS: &[ApiPreset] = &[
    ApiPreset { name: "Ollama", base_url: "http://localhost:11434/v1" },
    ApiPreset { name: "LM Studio", base_url: "http://localhost:1234/v1" },
    ApiPreset { name: "OpenClaw", base_url: "http://localhost:18789/v1" },
    ApiPreset { name: "OpenAI", base_url: "https://api.openai.com/v1" },
    ApiPreset { name: "Mistral AI", base_url: "https://api.mistral.ai/v1" },
    ApiPreset { name: "Mammouth AI", base_url: "https://api.mammouth.ai/v1" },
    ApiPreset { name: "OpenCode Go", base_url: "https://opencode.ai/zen/go/v1" },
    ApiPreset { name: "DeepSeek", base_url: "https://api.deepseek.com" },
    ApiPreset { name: "Qwen", base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1" },
    ApiPreset { name: "OpenRouter", base_url: "https://openrouter.ai/api/v1" },
];

/// Anthropic-compatible endpoints people use. The base URL is the one their documentation calls
/// `ANTHROPIC_BASE_URL`: the `/v1` is added where needed.
pub const ANTHROPIC_PRESETS: &[ApiPreset] = &[
    ApiPreset { name: "Anthropic", base_url: "https://api.anthropic.com/v1" },
    ApiPreset { name: "DeepSeek", base_url: "https://api.deepseek.com/anthropic" },
    ApiPreset { name: "Qwen", base_url: "https://dashscope-intl.aliyuncs.com/apps/anthropic" },
];

/// The presets offered for an endpoint protocol.
pub fn presets(protocol: Protocol) -> &'static [ApiPreset] {
    match protocol {
        Protocol::OpenAi => OPENAI_PRESETS,
        Protocol::Anthropic => ANTHROPIC_PRESETS,
    }
}

/// Whether a key may be sent to `url`: only over HTTPS, or to this computer.
pub fn key_safe_for(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    if u.starts_with("https://") {
        return true;
    }
    let Some(rest) = u.strip_prefix("http://") else { return false };
    // The host part ends where the HTTP client ends it, at the first `/`, `?` or `#`.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    local_authority(authority)
}

/// `authority` is exactly this computer's name with at most a port. No user name goes before it
/// and no stray character after it, as URL parsers don't all read those the same way.
fn local_authority(authority: &str) -> bool {
    let (host, port) = match authority.strip_prefix('[') {
        Some(inner) => {
            let Some((ip, after)) = inner.split_once(']') else { return false };
            if ip != "::1" {
                return false;
            }
            ("[::1]", after)
        }
        None => {
            let end = authority.find(':').unwrap_or(authority.len());
            authority.split_at(end)
        }
    };
    let port_ok = match port.strip_prefix(':') {
        Some(digits) => digits.bytes().all(|b| b.is_ascii_digit()) && digits.parse::<u16>().is_ok(),
        None => port.is_empty(),
    };
    port_ok && matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// A base URL the API backend accepts: `http(s)://host…`, without a trailing slash.
pub fn normalize_url(url: &str) -> Result<String, String> {
    let u = url.trim().trim_end_matches('/').to_string();
    let lower = u.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) || u.len() < 10 || u.contains(char::is_whitespace) {
        return Err(crate::i18n::tr("ui.ai_bad_url").to_string());
    }
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_only_over_https_or_to_this_computer() {
        assert!(key_safe_for("https://api.openai.com/v1"));
        assert!(key_safe_for("https://api.mammouth.ai/v1"));
        assert!(key_safe_for("http://localhost:11434/v1"));
        assert!(key_safe_for("http://127.0.0.1:1234/v1"));
        assert!(key_safe_for("http://[::1]:8080/v1"));
        assert!(!key_safe_for("http://192.168.1.20:11434/v1"));
        assert!(!key_safe_for("http://localhost.evil.example/v1"));
        assert!(!key_safe_for("http://user@evil.example/v1"));
        assert!(!key_safe_for("ftp://x"));
    }

    /// Plain HTTP only reaches this computer: the check reads the host where the HTTP client does,
    /// and takes nothing around the name that URL parsers could read differently.
    #[test]
    fn a_key_over_http_needs_a_url_that_can_only_mean_this_computer() {
        for url in [
            "http://localhost",
            "HTTP://LOCALHOST:11434/v1",
            " http://localhost:11434/v1/ ",
            "http://127.0.0.1:1234/v1?x=1",
            "http://[::1]/v1",
            "http://[::1]:8080/v1",
            "http://localhost:65535/@models",
        ] {
            assert!(key_safe_for(url), "{url}");
        }
        for url in [
            "http://evil.example#@localhost/v1",
            "http://evil.example?@localhost/v1",
            "http://evil.example/@localhost/v1",
            "http://evil.example\\@localhost/v1",
            "http://localhost\\@evil.example/v1",
            "http://localhost\\.evil.example/v1",
            "http://localhost@evil.example/v1",
            "http://localhost:1234@evil.example/v1",
            "http://user@localhost/v1",
            "http://user:secret@localhost:1234/v1",
            "http://[::1]@evil.example/v1",
            "http://[::1].evil.example/v1",
            "http://[::1]evil.example/v1",
            "http://[::2]/v1",
            "http://[::1/v1",
            "http://127.0.0.1.evil.example/v1",
            "http://localhost.evil.example:1234/v1",
            "http://localhost:80.evil.example/v1",
            "http://localhost:abc/v1",
            "http://localhost:/v1",
            "http://localhost:65536/v1",
            "http://local\thost.evil.example/v1",
            "http://localhost evil.example/v1",
            "http://localhost%2eevil.example/v1",
            "http://",
            "http:///localhost/v1",
            "http:/localhost/v1",
            "//localhost/v1",
            "localhost:1234/v1",
        ] {
            assert!(!key_safe_for(url), "{url}");
        }
    }

    /// The built-in agent is named by an absolute path to a file and nothing looser, and keeps
    /// its data in `mastra` under the settings folder: the one a run was given, else the app's own.
    #[test]
    fn the_built_in_agent_is_an_absolute_file_with_a_data_folder_of_the_apps() {
        let me = std::env::current_exe().expect("this test's own program");
        assert_eq!(builtin_program(Some(me.to_string_lossy().into_owned())), Some(me.clone()));
        let folder = me.parent().expect("its folder").to_string_lossy().into_owned();
        for not_one in [
            None,
            Some(String::new()),
            Some("   ".into()),
            Some("mastra-agent".into()),
            Some("./mastra-agent".into()),
            Some(folder),
            Some("/no/such/program".into()),
        ] {
            assert_eq!(builtin_program(not_one.clone()), None, "{not_one:?}");
        }
        let own = std::path::PathBuf::from(if cfg!(windows) { r"C:\Users\u\AppData\Local\Intcad" } else { "/home/u/.local/share/intcad" });
        let given = std::path::PathBuf::from(if cfg!(windows) { r"D:\run\settings" } else { "/tmp/run/settings" });
        assert_eq!(builtin_data(None, Some(own.clone())), Some(own.join("mastra")));
        assert_eq!(
            builtin_data(Some(given.clone().into_os_string()), Some(own.clone())),
            Some(given.join("mastra")),
            "a run's own settings folder wins"
        );
        assert_eq!(builtin_data(Some("relative/settings".into()), Some(own.clone())), Some(own.join("mastra")), "a relative folder is not one");
        assert_eq!(builtin_data(None, None), None);
        // A test never finds one by itself, whatever the shell it runs in holds.
        assert_eq!(look_for_builtin(), None);
    }

    /// A package carries the built-in agent in a folder beside the app: its own Node and the
    /// entry file. A package made without the folder behaves as it did. One that has the folder
    /// has the built-in agent whatever is missing inside it, so a damaged package shows the
    /// agent failing and never the agents found on the computer.
    #[test]
    fn a_package_with_the_agents_folder_has_the_built_in_agent_and_one_without_it_has_none() {
        let dir = std::env::temp_dir().join(format!("intcad-packaged-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // An app laid out as each system's package lays it out: the program two folders down, so
        // every rule of `roots_named` has a place inside `dir`.
        let exe = dir.join("Contents").join("MacOS").join("intcad");
        let root = super::super::packaged_runtime::roots_named(&exe, PACKAGED_AGENT_DIR).into_iter().next().expect("a place for the folder");
        assert!(root.starts_with(&dir) && root.ends_with(PACKAGED_AGENT_DIR), "{root:?}");
        let node = root.join(if cfg!(windows) { "node.exe" } else { "node" });
        let entry = root.join("agent").join("main.mjs");
        let found = Some((node.clone(), vec![entry.to_string_lossy().into_owned()]));
        assert_eq!(packaged_builtin(&exe), None, "no folder: no built-in agent");
        // The folder alone, with nothing in it: the agent is the package's, and will fail to start.
        std::fs::create_dir_all(&root).expect("make the folder");
        assert_eq!(packaged_builtin(&exe), found, "an empty folder");
        for file in [&node, &entry] {
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("make the folder");
            std::fs::write(file, b"").expect("write the file");
        }
        assert_eq!(packaged_builtin(&exe), found, "a whole folder");
        // A file where the folder should be is no folder.
        std::fs::remove_dir_all(&root).expect("take the folder away");
        std::fs::write(&root, b"").expect("write a file in its place");
        assert_eq!(packaged_builtin(&exe), None, "a file of that name");
        std::fs::remove_file(&root).expect("take the file away");
        std::fs::create_dir_all(&root).expect("make the folder again");
        // The other runtime's folder is not this one's.
        assert!(!super::super::packaged_runtime::roots(&exe).contains(&root));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn modes_and_ids_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.id()), Some(m));
        }
        let mut p = crate::prefs::Prefs::default();
        assert_eq!(p.ai_permissions, Mode::Edits, "model edits run, files ask: the default");
        p.agent_permissions.insert("codex".into(), Mode::Ask);
        assert_eq!(mode_for(&p, "codex"), Mode::Ask);
        assert_eq!(mode_for(&p, "claude"), Mode::Edits);
        remember(&mut p, "claude", "mcp__intcad__save");
        remember(&mut p, "claude", "mcp__intcad__save");
        assert!(always_allowed(&p, "claude", "mcp__intcad__save") && !always_allowed(&p, "codex", "mcp__intcad__save"));
        assert_eq!(p.agent_always_allow["claude"].len(), 1);
        p.agents.push(AgentConfig { id: "api-1".into(), ..Default::default() });
        assert_eq!(new_id(&p, "api"), "api-2");
        assert_eq!(normalize_url(" http://localhost:11434/v1/ ").as_deref(), Ok("http://localhost:11434/v1"));
        assert!(normalize_url("localhost:11434").is_err());
    }

    #[test]
    fn endpoint_kinds_keep_their_protocol_and_presets() {
        assert_eq!(serde_json::to_string(&Kind::Api).unwrap(), "\"api\"");
        assert_eq!(serde_json::to_string(&Kind::Anthropic).unwrap(), "\"anthropic\"");
        assert_eq!(serde_json::from_str::<Kind>("\"anthropic\"").unwrap(), Kind::Anthropic);
        assert!(Kind::Anthropic.is_api() && !Kind::Acp.is_api());
        assert!(Kind::Acp.protocol().is_none());
        assert_eq!(Kind::Anthropic.protocol(), Some(Protocol::Anthropic));
        assert_eq!(Protocol::Anthropic.kind(), Kind::Anthropic);
        assert_eq!(Kind::Api.id(), "api");
        assert_eq!(Kind::Anthropic.id(), "anthropic");
        assert!(presets(Protocol::OpenAi).iter().all(|p| !p.name.is_empty() && p.base_url.starts_with("http")));
        assert!(presets(Protocol::OpenAi).iter().any(|p| p.name == "Mammouth AI" && key_safe_for(p.base_url)));
        assert!(presets(Protocol::OpenAi).iter().any(|p| p.name == "Mistral AI" && p.base_url == "https://api.mistral.ai/v1"));
        assert!(presets(Protocol::Anthropic).iter().any(|p| p.base_url == "https://api.anthropic.com/v1"));
        for name in ["DeepSeek", "Qwen"] {
            assert!(presets(Protocol::OpenAi).iter().any(|p| p.name == name), "OpenAI-compatible preset for {name}");
            assert!(presets(Protocol::Anthropic).iter().any(|p| p.name == name), "Anthropic-compatible preset for {name}");
        }
        assert!(presets(Protocol::Anthropic).iter().any(|p| p.base_url == "https://api.deepseek.com/anthropic"));
        assert!(presets(Protocol::Anthropic).iter().any(|p| p.base_url == "https://dashscope-intl.aliyuncs.com/apps/anthropic"));
    }

    /// The first preset is the one an Add dialog opens with, and the one taken for Ollama.
    #[test]
    fn ollama_stays_the_first_preset_and_every_preset_can_take_a_key() {
        assert_eq!(OPENAI_PRESETS.first().map(|p| p.name), Some("Ollama"));
        assert_eq!(OPENAI_PRESETS.last().map(|p| p.name), Some("OpenRouter"));
        assert!(OPENAI_PRESETS.iter().any(|p| p.name == "OpenClaw"));
        for preset in OPENAI_PRESETS.iter().chain(ANTHROPIC_PRESETS) {
            assert_eq!(normalize_url(preset.base_url).as_deref(), Ok(preset.base_url), "{}", preset.name);
            assert!(key_safe_for(preset.base_url), "{}", preset.name);
        }
    }

    #[test]
    fn request_timeouts_default_to_300_s_and_stay_between_30_s_and_an_hour() {
        assert_eq!(AgentConfig::default().request_timeout_s, 300);
        assert_eq!(request_timeout_s(600.4), 600);
        assert_eq!(request_timeout_s(1.0), 30);
        assert_eq!(request_timeout_s(-5.0), 30);
        assert_eq!(request_timeout_s(1e300), 3600);
        assert_eq!(request_timeout_s(f64::NAN), 300);
        assert_eq!(request_timeout_s(f64::INFINITY), 300);
        // An older preferences file has no timeout: the default; a hand-edited one is clamped where used.
        let old: AgentConfig = serde_json::from_str(r#"{"id":"api-1","kind":"api","base_url":"http://localhost:11434/v1","model":"m"}"#).unwrap();
        assert_eq!(old.request_timeout_s, 300);
        let wild: AgentConfig = serde_json::from_str(r#"{"id":"api-2","kind":"api","request_timeout_s":99999}"#).unwrap();
        assert_eq!(Entry::from_config(&wild).request_timeout_s, 3600);
        let saved = AgentConfig { id: "api-3".into(), kind: Kind::Api, request_timeout_s: 1200, ..Default::default() };
        let back: AgentConfig = serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(back, saved);
    }

    #[test]
    fn reasoning_effort_defaults_to_empty_and_is_trimmed_and_capped() {
        assert!(AgentConfig::default().reasoning_effort.is_empty());
        // An older preferences file has no effort: empty, so nothing is sent.
        let old: AgentConfig = serde_json::from_str(r#"{"id":"api-1","kind":"api","base_url":"http://localhost:11434/v1","model":"m"}"#).unwrap();
        assert!(old.reasoning_effort.is_empty());
        assert_eq!(reasoning_effort("  high "), "high");
        assert!(reasoning_effort("   ").is_empty());
        let long = reasoning_effort(&"x".repeat(500));
        assert_eq!(long.chars().count(), MAX_REASONING_EFFORT_LEN);
        let saved = AgentConfig { id: "api-2".into(), kind: Kind::Api, reasoning_effort: "max".into(), ..Default::default() };
        let back: AgentConfig = serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(back, saved);
        // The entry carries it trimmed; a hand-edited over-long one is capped where used.
        assert_eq!(Entry::from_config(&saved).reasoning_effort, "max");
        let wild: AgentConfig = serde_json::from_str(&format!(r#"{{"id":"api-3","kind":"api","reasoning_effort":"{}"}}"#, "y".repeat(500))).unwrap();
        assert_eq!(Entry::from_config(&wild).reasoning_effort.chars().count(), MAX_REASONING_EFFORT_LEN);
    }
}
