//! The built-in agent's connections: which plans are signed in, which API keys and custom
//! addresses it was handed, and the sign-in under way. The agent (Mastra Code's ACP agent with
//! Intcad's `_intcad/connections/*` methods) owns that state and reports it whole as a snapshot;
//! the app keeps the last one, hands it to the page and carries out the page's actions.
//!
//! Keys are held by the system keychain only. The app reads them off the window thread for each
//! `setup` and passes them on: no key is kept in preferences, in the conversation, in the status
//! the control channel reports or in anything written to a log.
//!
//! The first half of this file is plain data, read from the agent as hostile input; the second
//! half is what the app does with it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The built-in agent's id and name in the agent list.
pub const AGENT_ID: &str = "mastra";
pub const AGENT_NAME: &str = "Mastra";

/// The providers the built-in agent takes an API key for, in the order the page lists them.
pub const KEY_IDS: [&str; 5] = ["anthropic", "openai", "google", "openrouter", "deepseek"];

/// Longest API key or pasted sign-in code taken from the page (characters).
pub const MAX_SECRET: usize = 4096;
/// Longest name of a custom address (characters).
pub const MAX_NAME: usize = 40;
/// Longest address of a custom address (characters).
pub const MAX_URL: usize = 400;
/// Most models one custom address names, and the longest name of one (characters).
pub const MAX_MODELS: usize = 16;
pub const MAX_MODEL: usize = 120;
/// Most custom addresses kept.
pub const MAX_CUSTOM: usize = 32;

/// Most rows of one kind read from a snapshot, the longest text in one, and the most release dates.
const MAX_ROWS: usize = 64;
const MAX_TEXT: usize = 512;
const MAX_DATES: usize = 5000;

/// A custom address as preferences keep it. Its key, when it has one, is in the system keychain.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomAddress {
    pub name: String,
    pub url: String,
    pub models: Vec<String>,
    /// A key is stored for it in the system keychain.
    pub key: bool,
}

/// Where the keychain keeps a provider's API key. The names are the built-in agent's own, so
/// removing an added agent (which deletes the entry named by its id) can never take one.
pub fn key_secret(provider: &str) -> String {
    format!("mastra/key/{provider}")
}

/// Where the keychain keeps a custom address's key.
pub fn custom_secret(name: &str) -> String {
    format!("mastra/custom/{name}")
}

fn text(value: &Value, name: &str) -> Option<String> {
    value.get(name).and_then(Value::as_str).map(|s| s.chars().take(MAX_TEXT).collect())
}

fn rows<T>(value: &Value, name: &str, read: impl Fn(&Value) -> Option<T>) -> Option<Vec<T>> {
    Some(value.get(name)?.as_array()?.iter().take(MAX_ROWS).filter_map(read).collect())
}

/// Whether a plan is signed in. `Expired`: a turn failed because the sign-in was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanState {
    In,
    Out,
    Expired,
}

impl PlanState {
    fn parse(s: &str) -> Option<PlanState> {
        Some(match s {
            "in" => PlanState::In,
            "out" => PlanState::Out,
            "expired" => PlanState::Expired,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            PlanState::In => "in",
            PlanState::Out => "out",
            PlanState::Expired => "expired",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub state: PlanState,
    pub account: Option<String>,
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub id: String,
    pub name: String,
    pub saved: bool,
    pub shadowed: bool,
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Custom {
    pub id: String,
    pub name: String,
    pub url: String,
    pub models: Vec<String>,
    pub keyed: bool,
    pub prefix: String,
}

/// How far a sign-in has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Browser,
    Device,
    Code,
}

impl Phase {
    pub fn id(self) -> &'static str {
        match self {
            Phase::Starting => "starting",
            Phase::Browser => "browser",
            Phase::Device => "device",
            Phase::Code => "code",
        }
    }
}

/// The sign-in under way. The address the agent opened is not kept: the page has no use for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub id: String,
    pub phase: Phase,
    pub user_code: Option<String>,
    /// The pasted code had not the shape of one; the sign-in is still under way.
    pub incomplete: bool,
}

/// Why the last sign-in could not finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginProblemKind {
    Port,
    Offline,
    Failed,
}

impl LoginProblemKind {
    pub fn id(self) -> &'static str {
        match self {
            LoginProblemKind::Port => "port",
            LoginProblemKind::Offline => "offline",
            LoginProblemKind::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginProblem {
    pub id: String,
    pub kind: LoginProblemKind,
}

/// Everything the agent says about its connections, as it last said it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub plans: Vec<Plan>,
    pub keys: Vec<Key>,
    pub custom: Vec<Custom>,
    pub login: Option<Login>,
    pub problem: Option<LoginProblem>,
    /// Release date (`YYYY-MM-DD`) by model id.
    pub dates: BTreeMap<String, String>,
}

impl Snapshot {
    /// Read a snapshot as the agent sent it. It comes from a child process, so everything is
    /// counted and cut to size, and a row that does not read is left out. `None`: not a snapshot.
    pub fn parse(value: &Value) -> Option<Snapshot> {
        let flag = |row: &Value, name: &str| row.get(name).and_then(Value::as_bool).unwrap_or(false);
        let plans = rows(value, "plans", |row| {
            Some(Plan {
                id: text(row, "id").filter(|id| !id.is_empty())?,
                name: text(row, "name").unwrap_or_default(),
                state: row.get("state").and_then(Value::as_str).and_then(PlanState::parse)?,
                account: text(row, "account").filter(|account| !account.is_empty()),
                prefix: text(row, "prefix").unwrap_or_default(),
            })
        })?;
        let keys = rows(value, "keys", |row| {
            Some(Key {
                id: text(row, "id").filter(|id| !id.is_empty())?,
                name: text(row, "name").unwrap_or_default(),
                saved: flag(row, "saved"),
                shadowed: flag(row, "shadowed"),
                prefix: text(row, "prefix").unwrap_or_default(),
            })
        })?;
        let custom = rows(value, "custom", |row| {
            Some(Custom {
                id: text(row, "id").filter(|id| !id.is_empty())?,
                name: text(row, "name").unwrap_or_default(),
                url: text(row, "url").unwrap_or_default(),
                models: row
                    .get("models")
                    .and_then(Value::as_array)
                    .map(|models| {
                        models.iter().take(MAX_ROWS).filter_map(|model| model.as_str().map(|s| s.chars().take(MAX_TEXT).collect())).collect()
                    })
                    .unwrap_or_default(),
                keyed: flag(row, "keyed"),
                prefix: text(row, "prefix").unwrap_or_default(),
            })
        })
        .unwrap_or_default();
        let login = value.get("login").filter(|login| login.is_object()).and_then(|login| {
            Some(Login {
                id: text(login, "id").filter(|id| !id.is_empty())?,
                // A phase this version does not know still is a sign-in under way, with nothing to type yet.
                phase: match login.get("phase").and_then(Value::as_str) {
                    Some("browser") => Phase::Browser,
                    Some("device") => Phase::Device,
                    Some("code") => Phase::Code,
                    _ => Phase::Starting,
                },
                user_code: text(login, "userCode").filter(|code| !code.is_empty()),
                incomplete: login.get("error").and_then(Value::as_str) == Some("incomplete"),
            })
        });
        let problem = value.get("problem").filter(|problem| problem.is_object()).and_then(|problem| {
            Some(LoginProblem {
                id: text(problem, "id").filter(|id| !id.is_empty())?,
                kind: match problem.get("kind").and_then(Value::as_str) {
                    Some("port") => LoginProblemKind::Port,
                    Some("offline") => LoginProblemKind::Offline,
                    _ => LoginProblemKind::Failed,
                },
            })
        });
        let dates = value
            .get("dates")
            .and_then(Value::as_object)
            .map(|dates| {
                dates
                    .iter()
                    .take(MAX_DATES)
                    .filter(|(model, _)| model.chars().count() <= MAX_TEXT)
                    .filter_map(|(model, date)| Some((model.clone(), date.as_str().filter(|date| date.len() <= 32)?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Some(Snapshot { plans, keys, custom, login, problem, dates })
    }

    /// A plan signed in, a key saved, or a custom address: there is something to talk to.
    pub fn connected(&self) -> bool {
        self.plans.iter().any(|plan| plan.state == PlanState::In) || self.keys.iter().any(|key| key.saved) || !self.custom.is_empty()
    }

    /// Whether the plan that gives the models under `prefix` is signed in. A prefix is one opaque
    /// string and is compared whole, never taken apart.
    pub fn plan_in(&self, prefix: &str) -> bool {
        self.plans.iter().any(|plan| plan.prefix == prefix && plan.state == PlanState::In)
    }

    /// The name, in the user's words, of the connection that gives the models under `prefix`: a
    /// plan that is or was signed in before a key (the plan serves the models where both do), a
    /// key before a custom address. A prefix no connection has is shown as it is.
    pub fn name_of(&self, prefix: &str) -> String {
        let plan = |used: bool| self.plans.iter().find(|plan| plan.prefix == prefix && (plan.state != PlanState::Out) == used).map(plan_name);
        let key = |saved: bool| self.keys.iter().find(|key| key.prefix == prefix && key.saved == saved).map(|key| key.name.clone());
        let custom = || self.custom.iter().find(|custom| custom.prefix == prefix).map(|custom| custom.name.clone());
        plan(true)
            .or_else(|| key(true))
            .or_else(custom)
            .or_else(|| plan(false))
            .or_else(|| key(false))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| if prefix.is_empty() { AGENT_NAME.to_string() } else { prefix.to_string() })
    }
}

/// A plan's name in the app's own words, by its id; a plan the app has no words for keeps the
/// agent's name.
pub fn plan_name(plan: &Plan) -> String {
    let own = match plan.id.as_str() {
        "openai-codex" => Some("ui.agent_chat_plan_openai_codex"),
        "anthropic" => Some("ui.agent_chat_plan_anthropic"),
        "kimi-for-coding" => Some("ui.agent_chat_plan_kimi_for_coding"),
        "xai" => Some("ui.agent_chat_plan_xai"),
        "github-copilot" => Some("ui.agent_chat_plan_github_copilot"),
        _ => None,
    };
    match own {
        Some(key) => crate::i18n::tr(key).to_string(),
        None if plan.name.is_empty() => plan.id.clone(),
        None => plan.name.clone(),
    }
}

/// Why a turn failed, as the built-in agent tells it in the error's `data.intcad`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnKind {
    /// The sign-in or key was refused.
    Auth,
    /// The plan or key has nothing left.
    Quota,
    /// Too many requests for now.
    Rate,
    /// The provider could not be reached.
    Network,
    /// This connection may not use that model.
    Model,
    /// Anything else: shown in the agent's own words, as any agent's error is.
    Other,
}

impl TurnKind {
    pub fn id(self) -> &'static str {
        match self {
            TurnKind::Auth => "auth",
            TurnKind::Quota => "quota",
            TurnKind::Rate => "rate",
            TurnKind::Network => "network",
            TurnKind::Model => "model",
            TurnKind::Other => "other",
        }
    }
    /// The catalog's sentence for it (`{name}`: the connection). `None`: the agent's own words.
    pub fn sentence(self) -> Option<&'static str> {
        Some(match self {
            TurnKind::Auth => "ui.agent_chat_problem_auth",
            TurnKind::Quota => "ui.agent_chat_problem_quota",
            TurnKind::Rate => "ui.agent_chat_problem_rate",
            TurnKind::Network => "ui.agent_chat_problem_network",
            TurnKind::Model => "ui.agent_chat_problem_model",
            TurnKind::Other => return None,
        })
    }
}

/// A failed turn: why, and the prefix of the connection it went through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnProblem {
    pub kind: TurnKind,
    pub prefix: String,
}

impl TurnProblem {
    /// Read the `data` of a JSON-RPC error. `None`: the error says nothing of the kind.
    pub fn parse(data: Option<&Value>) -> Option<TurnProblem> {
        let told = data?.get("intcad").filter(|told| told.is_object())?;
        let kind = match told.get("kind").and_then(Value::as_str)? {
            "auth" => TurnKind::Auth,
            "quota" => TurnKind::Quota,
            "rate" => TurnKind::Rate,
            "network" => TurnKind::Network,
            "model" => TurnKind::Model,
            _ => TurnKind::Other,
        };
        Some(TurnProblem { kind, prefix: text(told, "prefix").unwrap_or_default() })
    }
}

/// A custom address as `setup` hands it over, with its key.
#[derive(Clone, PartialEq, Eq)]
pub struct SetupCustom {
    pub name: String,
    pub url: String,
    pub api_key: Option<String>,
    pub models: Vec<String>,
}

/// Everything `setup` hands the agent: every key and custom address, each time.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Setup {
    /// `(provider, key)`.
    pub keys: Vec<(String, String)>,
    pub custom: Vec<SetupCustom>,
}

impl std::fmt::Debug for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // It holds keys: only which providers and addresses it names may reach a log.
        f.debug_struct("Setup")
            .field("keys", &self.keys.iter().map(|(provider, _)| provider.as_str()).collect::<Vec<_>>())
            .field("custom", &self.custom.iter().map(|custom| (custom.name.as_str(), custom.api_key.is_some())).collect::<Vec<_>>())
            .finish()
    }
}

/// Which `_intcad/connections/*` request: what a failed one is reported by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Setup,
    List,
    Login,
    Code,
    Reopen,
    Cancel,
    Logout,
}

impl What {
    pub fn method(self) -> &'static str {
        match self {
            What::Setup => "_intcad/connections/setup",
            What::List => "_intcad/connections/list",
            What::Login => "_intcad/connections/login",
            What::Code => "_intcad/connections/code",
            What::Reopen => "_intcad/connections/reopen",
            What::Cancel => "_intcad/connections/cancel",
            What::Logout => "_intcad/connections/logout",
        }
    }
}

/// The notification the agent sends whenever its snapshot changes.
pub const CHANGED: &str = "_intcad/connections/changed";

/// One request to the built-in agent about its connections. Each is answered with the snapshot.
#[derive(Clone, PartialEq, Eq)]
pub enum Request {
    Setup(Setup),
    List,
    Login(String),
    Code { id: String, code: String },
    Reopen,
    Cancel,
    Logout(String),
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A pasted sign-in code is as good as a key until it is used: it is not printed either.
        match self {
            Request::Setup(setup) => setup.fmt(f),
            Request::Code { id, .. } => f.debug_struct("Code").field("id", id).finish_non_exhaustive(),
            Request::Login(id) => f.debug_tuple("Login").field(id).finish(),
            Request::Logout(id) => f.debug_tuple("Logout").field(id).finish(),
            Request::List => f.write_str("List"),
            Request::Reopen => f.write_str("Reopen"),
            Request::Cancel => f.write_str("Cancel"),
        }
    }
}

impl Request {
    pub fn what(&self) -> What {
        match self {
            Request::Setup(_) => What::Setup,
            Request::List => What::List,
            Request::Login(_) => What::Login,
            Request::Code { .. } => What::Code,
            Request::Reopen => What::Reopen,
            Request::Cancel => What::Cancel,
            Request::Logout(_) => What::Logout,
        }
    }

    /// The request's JSON-RPC parameters.
    pub fn params(&self) -> Value {
        match self {
            Request::Setup(setup) => {
                let keys: serde_json::Map<String, Value> =
                    setup.keys.iter().map(|(provider, key)| (provider.clone(), Value::from(key.as_str()))).collect();
                let custom: Vec<Value> = setup
                    .custom
                    .iter()
                    .map(|custom| {
                        let mut row = json!({"name": custom.name, "url": custom.url, "models": custom.models});
                        if let (Some(key), Some(row)) = (&custom.api_key, row.as_object_mut()) {
                            row.insert("apiKey".into(), Value::from(key.as_str()));
                        }
                        row
                    })
                    .collect();
                json!({"keys": keys, "custom": custom})
            }
            Request::Login(id) | Request::Logout(id) => json!({"id": id}),
            Request::Code { id, code } => json!({"id": id, "code": code}),
            Request::List | Request::Reopen | Request::Cancel => json!({}),
        }
    }
}

/// A custom address as the page asks for it, checked: a name of 1 to 40 characters, an `http` or
/// `https` address of at most 400, 1 to 16 model names of at most 120 each, and a key of at most
/// 4096 when one is given. The reason for a refusal never repeats a value.
pub fn custom_address(name: &str, url: &str, key: Option<&str>, models: &[Value]) -> Result<(CustomAddress, Option<String>), &'static str> {
    let plain = |s: &str| !s.chars().any(char::is_control);
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || !plain(name) {
        return Err("a custom address needs a name of 1 to 40 characters");
    }
    let url = super::agents::normalize_url(url).map_err(|_| "the address must start with http:// or https://")?;
    if url.chars().count() > MAX_URL || !plain(&url) {
        return Err("the address is too long");
    }
    if models.is_empty() || models.len() > MAX_MODELS {
        return Err("a custom address needs 1 to 16 model names");
    }
    let mut named: Vec<String> = Vec::new();
    for model in models {
        let model = model.as_str().map(str::trim).filter(|model| !model.is_empty() && model.chars().count() <= MAX_MODEL && plain(model));
        let model = model.ok_or("a model name has 1 to 120 characters")?;
        if !named.iter().any(|known| known == model) {
            named.push(model.to_string());
        }
    }
    let key = key.map(str::trim).filter(|key| !key.is_empty());
    if key.is_some_and(|key| key.chars().count() > MAX_SECRET || !plain(key)) {
        return Err("the key is too long");
    }
    Ok((CustomAddress { name: name.to_string(), url, models: named, key: key.is_some() }, key.map(str::to_string)))
}

// ---- What the app does with it ----

use super::State;
use super::backend::Secrets;
use crate::SolveApp;
use crate::i18n::tr;

/// Whether the agent has a session to answer in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Session {
    #[default]
    None,
    /// Asked for; the agent has not answered yet.
    Opening,
    Open,
}

/// A failed turn whose message is kept (the chat's last message) for sending again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Kept {
    pub kind: TurnKind,
    pub prefix: String,
    /// The plan was seen not signed in since the turn failed. Seeing it signed in after that
    /// means the user signed in again; without it, a snapshot from before the failure would
    /// send the message straight back into the same refusal.
    lapsed: bool,
}

/// The app's side of the built-in agent's connections, for one conversation.
#[derive(Default)]
pub(crate) struct Conn {
    /// The running agent is the built-in one.
    pub active: bool,
    /// The agent's last snapshot (`None`: it has sent none since it started).
    pub snapshot: Option<Snapshot>,
    /// A key or custom address the app could not keep, in the user's language.
    pub error: Option<String>,
    pub session: Session,
    /// The agent refused a session since its last snapshot: it is not asked again before the next.
    pub refused: bool,
    /// The keys being read from the keychain for the next `setup`, and whether one could not be read.
    reading: Option<std::sync::mpsc::Receiver<(Setup, bool)>>,
    pub kept: Option<Kept>,
}

impl Conn {
    /// Keep the message of a turn that failed for `problem`.
    pub(super) fn keep(&mut self, problem: &TurnProblem) {
        let lapsed = self.snapshot.as_ref().is_some_and(|snapshot| !snapshot.plan_in(&problem.prefix));
        self.kept = Some(Kept { kind: problem.kind, prefix: problem.prefix.clone(), lapsed });
    }
}

/// Read every key the built-in agent is handed. `true` with it: the keychain refused one.
fn read_setup(secrets: Option<&dyn Secrets>, custom: &[CustomAddress]) -> (Setup, bool) {
    let mut failed = false;
    let mut read = |id: String| match secrets.map(|secrets| secrets.get(&id)) {
        Some(Ok(key)) => key.filter(|key| !key.trim().is_empty()),
        Some(Err(_)) => {
            failed = true;
            None
        }
        // A host without a keychain kept no key.
        None => None,
    };
    let keys: Vec<(String, String)> =
        KEY_IDS.iter().filter_map(|provider| read(key_secret(provider)).map(|key| (provider.to_string(), key))).collect();
    let custom: Vec<SetupCustom> = custom
        .iter()
        .take(MAX_CUSTOM)
        .map(|address| SetupCustom {
            name: address.name.clone(),
            url: address.url.clone(),
            api_key: if address.key { read(custom_secret(&address.name)) } else { None },
            models: address.models.clone(),
        })
        .collect();
    (Setup { keys, custom }, failed)
}

fn send(app: &mut SolveApp, request: Request) {
    if let Some(backend) = app.agent_chat.backend.as_mut() {
        backend.connections(request);
    }
}

/// Hand the running built-in agent every key and custom address again. The keys are read off the
/// window thread, since the keychain may ask the user first and takes as long as they do; the
/// request goes once they are read ([`poll`]). A newer read replaces one still under way.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn begin_setup(app: &mut SolveApp, ctx: &egui::Context) {
    if !app.agent_chat.conn.active || app.agent_chat.backend.is_none() {
        return;
    }
    let secrets = app.services.secrets.clone();
    let custom = app.preferences.mastra_custom.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let wake = ctx.clone();
    let spawned = std::thread::Builder::new().name("mastra-keys".into()).spawn(move || {
        let _ = tx.send(read_setup(secrets.as_deref(), &custom));
        wake.request_repaint();
    });
    if spawned.is_ok() {
        app.agent_chat.conn.reading = Some(rx);
    } else {
        // Nothing to read on: the agent still starts, with the addresses and without keys.
        app.agent_chat.conn.reading = None;
        app.agent_chat.conn.error = Some(tr("ui.host_ai_key_unavailable").to_string());
        let (setup, _) = read_setup(None, &app.preferences.mastra_custom);
        send(app, Request::Setup(setup));
    }
}

/// The browser build has no agent process to set up.
#[cfg(target_arch = "wasm32")]
pub(super) fn begin_setup(_app: &mut SolveApp, _ctx: &egui::Context) {}

/// Take in the keys once they are read, and hand them to the agent.
pub(super) fn poll(app: &mut SolveApp) {
    use std::sync::mpsc::TryRecvError;
    let Some(reading) = &app.agent_chat.conn.reading else { return };
    let (setup, failed) = match reading.try_recv() {
        Ok(read) => read,
        Err(TryRecvError::Empty) => return,
        // The reader ended without an answer: the agent starts without keys rather than not at all.
        Err(TryRecvError::Disconnected) => (read_setup(None, &app.preferences.mastra_custom).0, true),
    };
    app.agent_chat.conn.reading = None;
    if failed {
        app.agent_chat.conn.error = Some(tr("ui.host_ai_key_unavailable").to_string());
    }
    send(app, Request::Setup(setup));
}

/// Bring the conversation in line with the agent's last snapshot. With nothing connected the
/// conversation needs a sign-in and has no session (the agent refuses one then); once something
/// is connected a session is opened, once, and the message kept from a refused sign-in is sent
/// again when that plan is signed in.
pub(super) fn settle(app: &mut SolveApp) {
    let chat = &mut app.agent_chat;
    if !chat.conn.active || chat.backend.is_none() {
        return;
    }
    let Some(snapshot) = chat.conn.snapshot.as_ref() else { return };
    let connected = snapshot.connected();
    let signed_in = chat.conn.kept.as_ref().filter(|kept| kept.kind == TurnKind::Auth).map(|kept| snapshot.plan_in(&kept.prefix));
    let state = chat.state();
    // A session asked for is answered either way; a running turn ends by itself. Both are settled then.
    let opening = state == State::Starting && chat.conn.session == Session::Opening;
    if !connected && !opening && matches!(state, State::Starting | State::Ready | State::NeedsAuth) {
        if chat.conn.session != Session::None {
            chat.conn.session = Session::None;
            if let Some(backend) = chat.backend.as_mut() {
                backend.forget_session();
            }
            chat.clear_config();
        }
        chat.state = Some(State::NeedsAuth);
    } else if connected && chat.conn.session == Session::None && !chat.conn.refused && matches!(state, State::Starting | State::NeedsAuth) {
        if let Some(backend) = chat.backend.as_mut() {
            backend.new_session();
        }
        chat.conn.session = Session::Opening;
        chat.state = Some(State::Starting);
    }
    match (signed_in, chat.conn.kept.as_mut()) {
        (Some(false), Some(kept)) => kept.lapsed = true,
        (Some(true), Some(kept)) if kept.lapsed => resend(app),
        _ => {}
    }
}

/// Send the kept message again, in its own place in the conversation. While its session is still
/// being opened it waits for that, as a message typed before the agent was ready does.
fn resend(app: &mut SolveApp) {
    let chat = &mut app.agent_chat;
    chat.conn.kept = None;
    match chat.state() {
        State::Ready if chat.turn.is_none() => {
            if let Some((text, image, at, files)) = chat.last.take() {
                super::start_turn(app, text, image, at, files);
            }
        }
        State::Starting | State::NeedsAuth if chat.queued.is_none() => chat.queued = chat.last.take(),
        _ => {}
    }
}

/// What the page and the control channel's status are told of the connections, while the
/// built-in agent is the agent (`null` otherwise).
pub(super) fn told_of(app: &SolveApp) -> Value {
    if super::agents::is_builtin(app) { page(&app.agent_chat.conn) } else { Value::Null }
}

/// The agent's snapshot with the plans named in the app's words, and what the app itself has to
/// say. Before the first snapshot the lists are empty. No key is in it: the agent sends none
/// back, and the app holds none.
fn page(conn: &Conn) -> Value {
    let none = Snapshot::default();
    let snapshot = conn.snapshot.as_ref().unwrap_or(&none);
    let plans: Vec<Value> = snapshot
        .plans
        .iter()
        .map(|plan| json!({"id": plan.id, "name": plan_name(plan), "state": plan.state.id(), "account": plan.account, "prefix": plan.prefix}))
        .collect();
    let keys: Vec<Value> = snapshot
        .keys
        .iter()
        .map(|key| json!({"id": key.id, "name": key.name, "saved": key.saved, "shadowed": key.shadowed, "prefix": key.prefix}))
        .collect();
    let custom: Vec<Value> = snapshot
        .custom
        .iter()
        .map(|custom| json!({"id": custom.id, "name": custom.name, "url": custom.url, "models": custom.models, "keyed": custom.keyed, "prefix": custom.prefix}))
        .collect();
    let login = snapshot.login.as_ref().map(
        |login| json!({"id": login.id, "phase": login.phase.id(), "userCode": login.user_code, "error": login.incomplete.then_some("incomplete")}),
    );
    json!({
        "plans": plans,
        "keys": keys,
        "custom": custom,
        "login": login,
        "problem": snapshot.problem.as_ref().map(|problem| json!({"id": problem.id, "kind": problem.kind.id()})),
        "error": conn.error,
        "dates": snapshot.dates,
        "connected": snapshot.connected(),
    })
}

/// The snapshot of the running built-in agent, for the actions that ask it for something.
fn told(app: &SolveApp) -> Option<&Snapshot> {
    let chat = &app.agent_chat;
    if !chat.conn.active || chat.backend.is_none() {
        return None;
    }
    chat.conn.snapshot.as_ref()
}

/// Carry out a change to what the keychain holds; whether it took. A keychain that is missing
/// or refuses is told to the page in the user's language (`connect.error`); the keychain's own
/// words are not, and nothing of a key is.
fn change(app: &mut SolveApp, change: impl FnOnce(&dyn Secrets) -> Result<(), String>) -> bool {
    let kept = match &app.services.secrets {
        Some(secrets) => change(secrets.as_ref()).map_err(|_| "ui.host_ai_key_unavailable"),
        None => Err("ui.ai_no_keychain"),
    };
    app.agent_chat.conn.error = kept.err().map(|why| tr(why).to_string());
    kept.is_ok()
}

/// The actions of the Connections page.
pub(super) const ACTIONS: [&str; 10] = [
    "conn_login",
    "conn_code",
    "conn_reopen",
    "conn_cancel",
    "conn_logout",
    "conn_key",
    "conn_key_remove",
    "conn_custom",
    "conn_custom_remove",
    "conn_retry",
];

/// One of the page's `conn_*` actions (`kind`), checked as every action of the page is: against
/// the agent's last snapshot, and field by field. A refusal's reason goes to the host's log, so it
/// never repeats a value the page sent. The sign-in actions need the running agent; a key or a
/// custom address can be kept while it is stopped and is handed over when it next starts.
pub(super) fn act(app: &mut SolveApp, ctx: &egui::Context, kind: &str, action: &Value) -> Result<(), String> {
    // The same reason the page's other actions give, whoever asks (the page, the control channel).
    if !ACTIONS.contains(&kind) {
        return Err(format!("unknown action {kind}"));
    }
    if !super::agents::is_builtin(app) {
        return Err(format!("{kind}: this agent has no connections"));
    }
    let text = |name: &str| action.get(name).and_then(Value::as_str);
    let provider =
        || text("provider").filter(|provider| KEY_IDS.contains(provider)).ok_or_else(|| format!("{kind}: provider must be one of the five"));
    let silent = || format!("{kind}: the agent has not said what it offers");
    match kind {
        "conn_login" => {
            let id = text("id").ok_or("conn_login: no id")?;
            let snapshot = told(app).ok_or_else(silent)?;
            if !snapshot.plans.iter().any(|plan| plan.id == id) {
                return Err("conn_login: not one of the plans".into());
            }
            if snapshot.login.is_some() {
                return Err("conn_login: a sign-in is already under way".into());
            }
            send(app, Request::Login(id.to_string()));
        }
        "conn_code" => {
            let id = text("id").ok_or("conn_code: no id")?;
            let code = text("code").map(str::trim).filter(|code| !code.is_empty() && code.chars().count() <= MAX_SECRET);
            let code = code.ok_or("conn_code: a code has 1 to 4096 characters")?;
            if !told(app).ok_or_else(silent)?.login.as_ref().is_some_and(|login| login.id == id) {
                return Err("conn_code: no sign-in for that plan is under way".into());
            }
            send(app, Request::Code { id: id.to_string(), code: code.to_string() });
        }
        "conn_reopen" => {
            if told(app).ok_or_else(silent)?.login.is_none() {
                return Err("conn_reopen: no sign-in is under way".into());
            }
            send(app, Request::Reopen);
        }
        // Cancel also clears the note of a sign-in that could not finish, so it needs none under way.
        "conn_cancel" => {
            told(app).ok_or_else(silent)?;
            send(app, Request::Cancel);
        }
        "conn_logout" => {
            let id = text("id").ok_or("conn_logout: no id")?;
            if !told(app).ok_or_else(silent)?.plans.iter().any(|plan| plan.id == id && plan.state != PlanState::Out) {
                return Err("conn_logout: not a plan that is signed in".into());
            }
            send(app, Request::Logout(id.to_string()));
        }
        "conn_key" => {
            let provider = provider()?;
            let key =
                text("key").map(str::trim).filter(|key| !key.is_empty() && key.chars().count() <= MAX_SECRET && !key.chars().any(char::is_control));
            let key = key.ok_or("conn_key: a key has 1 to 4096 characters")?;
            if change(app, |secrets| secrets.set(&key_secret(provider), key)) {
                begin_setup(app, ctx);
            }
        }
        "conn_key_remove" => {
            let provider = provider()?;
            if change(app, |secrets| secrets.delete(&key_secret(provider))) {
                begin_setup(app, ctx);
            }
        }
        "conn_custom" => {
            let (name, url) = (text("name").ok_or("conn_custom: no name")?, text("url").ok_or("conn_custom: no url")?);
            let models = action.get("models").and_then(Value::as_array).ok_or("conn_custom: no models")?;
            let key = match action.get("key") {
                None | Some(Value::Null) => None,
                Some(Value::String(key)) => Some(key.as_str()),
                Some(_) => return Err("conn_custom: key must be text".into()),
            };
            let (address, key) = custom_address(name, url, key, models).map_err(|why| format!("conn_custom: {why}"))?;
            let known = app.preferences.mastra_custom.iter().position(|kept| kept.name == address.name);
            if known.is_none() && app.preferences.mastra_custom.len() >= MAX_CUSTOM {
                return Err("conn_custom: no room for another custom address".into());
            }
            // The app's rule for every key it holds: over HTTPS, or to this computer.
            if key.is_some() && !super::agents::key_safe_for(&address.url) {
                app.agent_chat.conn.error = Some(tr("ui.ai_key_needs_https").to_string());
                return Ok(());
            }
            let had_key = known.and_then(|at| app.preferences.mastra_custom.get(at)).is_some_and(|kept| kept.key);
            let secret = custom_secret(&address.name);
            // The key goes first: an address is not kept with a key the keychain did not take.
            let stored = match key {
                Some(key) => change(app, |secrets| secrets.set(&secret, &key)),
                // An address given again without a key leaves none behind.
                None if had_key => change(app, |secrets| secrets.delete(&secret)),
                None => {
                    app.agent_chat.conn.error = None;
                    true
                }
            };
            if stored {
                match known.and_then(|at| app.preferences.mastra_custom.get_mut(at)) {
                    Some(kept) => *kept = address,
                    None => app.preferences.mastra_custom.push(address),
                }
                begin_setup(app, ctx);
            }
        }
        "conn_custom_remove" => {
            let id = text("id").ok_or("conn_custom_remove: no id")?;
            // The page names an address by the id the agent gave it; preferences know it by its name.
            let named = app.agent_chat.conn.snapshot.as_ref().and_then(|snapshot| snapshot.custom.iter().find(|custom| custom.id == id));
            let name = named.map_or_else(|| id.to_string(), |custom| custom.name.clone());
            let at = app.preferences.mastra_custom.iter().position(|kept| kept.name == name).ok_or("conn_custom_remove: no such custom address")?;
            let address = app.preferences.mastra_custom.remove(at);
            app.agent_chat.conn.error = None;
            if address.key
                && let Some(secrets) = &app.services.secrets
                && secrets.delete(&custom_secret(&address.name)).is_err()
            {
                app.agent_chat.conn.error = Some(tr("ui.host_ai_key_unavailable").to_string());
            }
            begin_setup(app, ctx);
        }
        "conn_retry" => {
            let chat = &app.agent_chat;
            if chat.conn.kept.is_none() || chat.last.is_none() {
                return Err("conn_retry: no message is kept".into());
            }
            if chat.state() != State::Ready || chat.turn.is_some() {
                return Err("conn_retry: the agent is not ready for a message".into());
            }
            resend(app);
        }
        _ => return Err(format!("unknown action {kind}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn told() -> Value {
        json!({
            "plans": [
                {"id": "openai-codex", "name": "ChatGPT Plus/Pro (Codex Subscription)", "state": "expired", "account": "a@example.com", "prefix": "openai"},
                {"id": "some-new-plan", "name": "Some New Plan", "state": "out", "account": null, "prefix": "new"},
            ],
            "keys": [{"id": "openai", "name": "OpenAI", "saved": true, "shadowed": true, "prefix": "openai"}, {"id": "google", "name": "Google", "saved": false, "shadowed": false, "prefix": "google"}],
            "custom": [{"id": "my-llm", "name": "my-llm", "url": "http://localhost:11434/v1", "models": ["llama3.2"], "keyed": false, "prefix": "mastracode/my-llm"}],
            "login": {"id": "anthropic", "phase": "code", "url": "https://claude.ai/oauth/authorize?state=secret", "userCode": null, "error": "incomplete"},
            "problem": {"id": "openai-codex", "kind": "port"},
            "dates": {"openai/gpt-6.2-sol": "2026-11-17", "broken": 7},
        })
    }

    /// The snapshot is read field by field, the sign-in's address is not kept, and a plan the
    /// app has no words for keeps the agent's name.
    #[test]
    fn a_snapshot_is_read_as_the_agent_sends_it() {
        let snapshot = Snapshot::parse(&told()).expect("a snapshot");
        assert_eq!(snapshot.plans.len(), 2);
        assert_eq!((snapshot.plans[0].state, snapshot.plans[0].account.as_deref()), (PlanState::Expired, Some("a@example.com")));
        assert_eq!((plan_name(&snapshot.plans[0]).as_str(), plan_name(&snapshot.plans[1]).as_str()), ("ChatGPT plan", "Some New Plan"));
        assert!(snapshot.keys[0].saved && snapshot.keys[0].shadowed && !snapshot.keys[1].saved);
        assert_eq!(snapshot.custom[0].models, ["llama3.2"]);
        let login = snapshot.login.as_ref().expect("the sign-in under way");
        assert_eq!((login.id.as_str(), login.phase, login.user_code.as_deref(), login.incomplete), ("anthropic", Phase::Code, None, true));
        assert!(!format!("{snapshot:?}").contains("claude.ai"), "the address is not kept");
        assert_eq!(snapshot.problem, Some(LoginProblem { id: "openai-codex".into(), kind: LoginProblemKind::Port }));
        assert_eq!(snapshot.dates.len(), 1, "a date that is no text is left out");
        // Something is connected by the saved key and the custom address, though the plan expired.
        assert!(snapshot.connected() && !snapshot.plan_in("openai"));
    }

    /// What comes from the agent's process is hostile: nothing of it panics, a row that does not
    /// read is left out, and lists and texts are cut to size.
    #[test]
    fn a_hostile_snapshot_is_cut_to_size_or_refused() {
        for not_one in
            [json!(null), json!([]), json!("plans"), json!({"plans": [], "keys": 3}), json!({"keys": []}), json!({"plans": {}, "keys": []})]
        {
            assert_eq!(Snapshot::parse(&not_one), None, "{not_one}");
        }
        let long = "x".repeat(100_000);
        let many: Vec<Value> = (0..5000).map(|n| json!({"id": format!("p{n}"), "name": long, "state": "in", "prefix": long})).collect();
        let rows = json!([{"id": 7}, {"state": "in"}, {"id": "", "state": "in"}, {"id": "a", "state": "signed-in"}, 3, null]);
        let odd = Snapshot::parse(&json!({"plans": rows, "keys": [{"id": "k"}], "custom": "none", "login": "soon", "problem": [], "dates": []}))
            .expect("a snapshot");
        assert!(odd.plans.is_empty() && odd.custom.is_empty() && odd.login.is_none() && odd.problem.is_none() && odd.dates.is_empty());
        assert_eq!(odd.keys, [Key { id: "k".into(), name: String::new(), saved: false, shadowed: false, prefix: String::new() }]);
        assert!(!odd.connected());
        let big = Snapshot::parse(&json!({"plans": many, "keys": []})).expect("a snapshot");
        assert_eq!(big.plans.len(), MAX_ROWS);
        assert!(big.plans.iter().all(|plan| plan.name.chars().count() == MAX_TEXT && plan.prefix.chars().count() == MAX_TEXT));
        // A phase or a kind this version does not know still reads as a sign-in and a problem.
        let later = Snapshot::parse(
            &json!({"plans": [], "keys": [], "login": {"id": "xai", "phase": "passkey"}, "problem": {"id": "xai", "kind": "quantum"}}),
        )
        .expect("a snapshot");
        assert_eq!(later.login.map(|login| login.phase), Some(Phase::Starting));
        assert_eq!(later.problem.map(|problem| problem.kind), Some(LoginProblemKind::Failed));
    }

    /// A prefix is one opaque string: a custom address's has a slash in it and is compared whole.
    /// Where a plan and a key give the same models, the plan that is or was signed in is named.
    #[test]
    fn a_connection_is_named_by_its_whole_prefix() {
        let snapshot = Snapshot::parse(&told()).expect("a snapshot");
        assert_eq!(snapshot.name_of("openai"), "ChatGPT plan", "the plan wins over the key with the same prefix");
        assert_eq!(snapshot.name_of("mastracode/my-llm"), "my-llm");
        assert_eq!(snapshot.name_of("mastracode"), "mastracode", "half a prefix names no connection");
        assert_eq!(snapshot.name_of("my-llm"), "my-llm", "nor does its other half: the prefix itself is shown");
        assert_eq!(snapshot.name_of("google"), "Google");
        assert_eq!(snapshot.name_of(""), AGENT_NAME);
        let mut signed_out = told();
        signed_out["plans"][0]["state"] = json!("out");
        assert_eq!(Snapshot::parse(&signed_out).expect("a snapshot").name_of("openai"), "OpenAI", "signed out, the saved key serves the models");
    }

    /// Why a turn failed is read from the error's `data.intcad` and from nowhere else.
    #[test]
    fn a_failed_turns_reason_is_read_from_the_errors_data() {
        let read = |data: Value| TurnProblem::parse(Some(&data));
        assert_eq!(
            read(json!({"intcad": {"kind": "quota", "prefix": "mastracode/my-llm"}})),
            Some(TurnProblem { kind: TurnKind::Quota, prefix: "mastracode/my-llm".into() })
        );
        assert_eq!(read(json!({"intcad": {"kind": "something-new"}})), Some(TurnProblem { kind: TurnKind::Other, prefix: String::new() }));
        for nothing in [json!(null), json!({}), json!({"intcad": "auth"}), json!({"intcad": {"prefix": "openai"}}), json!({"kind": "auth"})] {
            assert_eq!(read(nothing.clone()), None, "{nothing}");
        }
        assert_eq!(TurnProblem::parse(None), None);
        assert!(TurnKind::Other.sentence().is_none());
        for kind in [TurnKind::Auth, TurnKind::Quota, TurnKind::Rate, TurnKind::Network, TurnKind::Model] {
            let sentence = kind.sentence().expect("a sentence of the app's own");
            assert!(!tr(sentence).starts_with("ui."), "{sentence} is in the catalog");
        }
    }

    /// B10: what holds a key or a pasted code never prints it, so no log line can carry one.
    #[test]
    fn requests_that_hold_a_secret_do_not_print_it() {
        let custom = SetupCustom {
            name: "my-llm".into(),
            url: "http://localhost:11434/v1".into(),
            api_key: Some("sk-custom-never-shown".into()),
            models: vec!["llama3.2".into()],
        };
        let setup = Setup { keys: vec![("openai".into(), "sk-openai-never-shown".into())], custom: vec![custom] };
        let request = Request::Setup(setup.clone());
        // The request does carry them: the parameters are what goes to the agent.
        let params = request.params().to_string();
        assert!(params.contains("sk-openai-never-shown") && params.contains("sk-custom-never-shown"));
        for printed in [format!("{setup:?}"), format!("{request:?}"), format!("{:#?}", Some(&request))] {
            assert!(!printed.contains("never-shown"), "{printed}");
            assert!(printed.contains("openai") && printed.contains("my-llm"), "which providers and addresses is no secret: {printed}");
        }
        let code = format!("{:?}", Request::Code { id: "anthropic".into(), code: "abc#never-shown".into() });
        assert!(code.contains("anthropic") && !code.contains("never-shown"), "{code}");
    }

    /// B11: a custom address is taken only whole and in range, and the reason for a refusal
    /// repeats nothing that was typed.
    #[test]
    fn a_custom_address_is_taken_only_in_range() {
        let models = [json!("llama3.2"), json!(" qwen3 "), json!("llama3.2")];
        let (address, key) = custom_address("  my-llm ", " http://localhost:11434/v1/ ", Some("  k  "), &models).expect("a custom address");
        assert_eq!(
            address,
            CustomAddress {
                name: "my-llm".into(),
                url: "http://localhost:11434/v1".into(),
                models: vec!["llama3.2".into(), "qwen3".into()],
                key: true
            }
        );
        assert_eq!(key.as_deref(), Some("k"));
        let (plain, none) = custom_address("local", "https://llm.example/v1", Some("   "), &models[..1]).expect("a custom address");
        assert!(!plain.key && none.is_none(), "an empty key is no key");
        let ok = [json!("m")];
        let seventeen: Vec<Value> = (0..17).map(|n| json!(format!("m{n}"))).collect();
        let refused = [
            custom_address("", "http://localhost:11434/v1", None, &ok),
            custom_address("   ", "http://localhost:11434/v1", None, &ok),
            custom_address(&"n".repeat(41), "http://localhost:11434/v1", None, &ok),
            custom_address("tab\tname", "http://localhost:11434/v1", None, &ok),
            custom_address("my-llm", "localhost:11434", None, &ok),
            custom_address("my-llm", "ftp://localhost/v1", None, &ok),
            custom_address("my-llm", &format!("https://llm.example/{}", "p".repeat(400)), None, &ok),
            custom_address("my-llm", "http://localhost:11434/v1", None, &[]),
            custom_address("my-llm", "http://localhost:11434/v1", None, &seventeen),
            custom_address("my-llm", "http://localhost:11434/v1", None, &[json!("")]),
            custom_address("my-llm", "http://localhost:11434/v1", None, &[json!(7)]),
            custom_address("my-llm", "http://localhost:11434/v1", None, &[json!("m".repeat(121))]),
            custom_address("my-llm", "http://localhost:11434/v1", Some(&"k".repeat(4097)), &ok),
        ];
        for (n, result) in refused.iter().enumerate() {
            let why = result.as_ref().expect_err(&format!("case {n} is refused"));
            assert!(!why.contains("my-llm") && !why.contains("localhost") && !why.contains("kkkk"), "{why}");
        }
        assert!(
            custom_address(&"n".repeat(40), &format!("https://llm.example/{}", "p".repeat(380)), Some(&"k".repeat(4096)), &seventeen[..16]).is_ok(),
            "the limits themselves are in range"
        );
    }

    #[test]
    fn the_keychain_names_are_the_built_in_agents_own() {
        assert_eq!(key_secret("openai"), "mastra/key/openai");
        assert_eq!(custom_secret("my-llm"), "mastra/custom/my-llm");
        assert_eq!(KEY_IDS, ["anthropic", "openai", "google", "openrouter", "deepseek"]);
    }
}
