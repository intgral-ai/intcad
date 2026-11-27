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
    /// The local port the browser calls back to is taken.
    Port,
    /// The system would not open that port.
    Listen,
    /// The person did not come back from the browser in the sign-in's time.
    Timeout,
    Offline,
    /// The sign-in service refused the last step.
    Refused,
    Failed,
}

impl LoginProblemKind {
    pub fn id(self) -> &'static str {
        match self {
            LoginProblemKind::Port => "port",
            LoginProblemKind::Listen => "listen",
            LoginProblemKind::Timeout => "timeout",
            LoginProblemKind::Offline => "offline",
            LoginProblemKind::Refused => "refused",
            LoginProblemKind::Failed => "failed",
        }
    }
}

/// How far a sign-in that failed had got (INT-1521).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Before the browser was opened.
    Open,
    /// Waiting for the browser.
    Wait,
    /// The last step, with the sign-in service.
    Exchange,
    /// Saving what it gave.
    Store,
}

/// Who holds the port a sign-in needs, as far as the agent could tell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortHolder {
    /// A sign-in of another design of this app.
    Here,
    /// A sign-in of another Intcad.
    Intcad,
}

/// Longest error code read from a sign-in's problem (characters).
const MAX_WORD: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginProblem {
    pub id: String,
    pub kind: LoginProblemKind,
    /// For [`LoginProblemKind::Port`]: who holds it, when the agent found out.
    pub holder: Option<PortHolder>,
    /// For [`LoginProblemKind::Listen`]: the port and the system's code for the refusal (`EACCES`).
    /// At [`Step::Exchange`]: the sign-in service's error code, or the system's when it had none.
    pub port: Option<u16>,
    pub code: Option<String>,
    pub step: Option<Step>,
    /// The sign-in service's HTTP status at [`Step::Exchange`].
    pub status: Option<u16>,
    /// Where the agent wrote the sign-in down.
    pub log: Option<String>,
}

/// A short word from the agent for a sentence: printable, cut to size, `None` when empty.
fn word(value: &Value, name: &str) -> Option<String> {
    let word: String = value.get(name)?.as_str()?.chars().filter(|c| !c.is_control()).take(MAX_WORD).collect();
    let word = word.trim();
    (!word.is_empty()).then(|| word.to_string())
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
            let holder = problem.get("holder").and_then(|holder| match holder.get("kind").and_then(Value::as_str) {
                Some("here") => Some(PortHolder::Here),
                Some("intcad") => Some(PortHolder::Intcad),
                _ => None,
            });
            Some(LoginProblem {
                id: text(problem, "id").filter(|id| !id.is_empty())?,
                kind: match problem.get("kind").and_then(Value::as_str) {
                    Some("port") => LoginProblemKind::Port,
                    Some("listen") => LoginProblemKind::Listen,
                    Some("timeout") => LoginProblemKind::Timeout,
                    Some("offline") => LoginProblemKind::Offline,
                    Some("refused") => LoginProblemKind::Refused,
                    _ => LoginProblemKind::Failed,
                },
                holder,
                port: problem.get("port").and_then(Value::as_u64).and_then(|port| u16::try_from(port).ok()),
                code: word(problem, "code"),
                step: match problem.get("step").and_then(Value::as_str) {
                    Some("open") => Some(Step::Open),
                    Some("wait") => Some(Step::Wait),
                    Some("exchange") => Some(Step::Exchange),
                    Some("store") => Some(Step::Store),
                    _ => None,
                },
                status: problem.get("status").and_then(Value::as_u64).and_then(|status| u16::try_from(status).ok()).filter(|status| (100..600).contains(status)),
                log: text(problem, "log").filter(|log| !log.is_empty() && !log.chars().any(char::is_control)),
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

    /// Whether a plan is signed in, signed out or expired in `now` and was not so here. Mastra
    /// keeps its sign-ins in a file every agent of the app reads, so such a change is one the
    /// other conversations' agents have to hear of.
    pub fn plans_differ(&self, now: &Snapshot) -> bool {
        self.plans.iter().any(|plan| now.plans.iter().find(|other| other.id == plan.id).is_some_and(|other| other.state != plan.state))
    }

    /// Whether a plan that is signed in, or was until its sign-in was refused, gives the models
    /// under `prefix`. A turn refused there was refused for that sign-in; anywhere else it went
    /// through a key or a custom address.
    pub fn plan_held(&self, prefix: &str) -> bool {
        self.plans.iter().any(|plan| plan.prefix == prefix && plan.state != PlanState::Out)
    }

    /// The name, in the user's words, of the connection that gives the models under `prefix`: a
    /// plan that is or was signed in before a key (the plan serves the models where both do), a
    /// key before a custom address, and a provider without a key before a plan that is signed
    /// out (a turn did not go through that plan). A prefix no connection has is shown as it is.
    pub fn name_of(&self, prefix: &str) -> String {
        let plan = |used: bool| self.plans.iter().find(|plan| plan.prefix == prefix && (plan.state != PlanState::Out) == used).map(plan_name);
        let key = |saved: bool| self.keys.iter().find(|key| key.prefix == prefix && key.saved == saved).map(|key| key.name.clone());
        let custom = || self.custom.iter().find(|custom| custom.prefix == prefix).map(|custom| custom.name.clone());
        plan(true)
            .or_else(|| key(true))
            .or_else(custom)
            .or_else(|| key(false))
            .or_else(|| plan(false))
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
    /// A plan's sign-in was refused: signing in again mends it.
    Auth,
    /// An API key was refused, a provider's or a custom address's. The agent calls this `auth`
    /// too; the app tells the two apart by the connection ([`TurnProblem::as_told`]), since
    /// there is no sign-in to do again for a key.
    Key,
    /// The plan or key has nothing left.
    Quota,
    /// Too many requests for now.
    Rate,
    /// The provider could not be reached.
    Network,
    /// This connection may not use that model.
    Model,
    /// The model cannot read the message's pictures or documents (INT-1513).
    Attachments,
    /// Anything else: shown in the agent's own words, as any agent's error is.
    Other,
}

impl TurnKind {
    pub fn id(self) -> &'static str {
        match self {
            TurnKind::Auth => "auth",
            TurnKind::Key => "key",
            TurnKind::Quota => "quota",
            TurnKind::Rate => "rate",
            TurnKind::Network => "network",
            TurnKind::Model => "model",
            TurnKind::Attachments => "attachments",
            TurnKind::Other => "other",
        }
    }
    /// The catalog's sentence for it (`{name}`: the connection). `None`: the agent's own words.
    pub fn sentence(self) -> Option<&'static str> {
        Some(match self {
            TurnKind::Auth => "ui.agent_chat_problem_auth",
            TurnKind::Key => "ui.agent_chat_problem_key",
            TurnKind::Quota => "ui.agent_chat_problem_quota",
            TurnKind::Rate => "ui.agent_chat_problem_rate",
            TurnKind::Network => "ui.agent_chat_problem_network",
            TurnKind::Model => "ui.agent_chat_problem_model",
            TurnKind::Attachments => "ui.agent_chat_problem_attachments",
            TurnKind::Other => return None,
        })
    }
    /// That sentence about the connection called `name`, in the language of whoever reads it now.
    /// It is worded where it is shown, not where the failure arrives: the failure can arrive
    /// outside a frame (a control-channel call), where the app's language is not set.
    pub fn words(self, name: &str) -> String {
        self.sentence().map(|sentence| crate::i18n::format(sentence, &[("name", name)])).unwrap_or_default()
    }
}

/// A failed turn: why, and the prefix of the connection it went through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnProblem {
    pub kind: TurnKind,
    pub prefix: String,
}

impl TurnProblem {
    /// The failure as the user is told it. The agent says `auth` for every refused credential;
    /// only a plan has a sign-in to do again, so `auth` stays for a plan that is signed in or was
    /// until now, and is a refused key for everything else: a saved key, a custom address, or a
    /// connection nothing is known of.
    pub fn as_told(mut self, snapshot: Option<&Snapshot>) -> TurnProblem {
        if self.kind == TurnKind::Auth && !snapshot.is_some_and(|snapshot| snapshot.plan_held(&self.prefix)) {
            self.kind = TurnKind::Key;
        }
        self
    }

    /// Read the `data` of a JSON-RPC error. `None`: the error says nothing of the kind. (`key` is
    /// not a word the agent uses: [`TurnProblem::as_told`] makes it.)
    pub fn parse(data: Option<&Value>) -> Option<TurnProblem> {
        let told = data?.get("intcad").filter(|told| told.is_object())?;
        let kind = match told.get("kind").and_then(Value::as_str)? {
            "auth" => TurnKind::Auth,
            "quota" => TurnKind::Quota,
            "rate" => TurnKind::Rate,
            "network" => TurnKind::Network,
            "model" => TurnKind::Model,
            "attachments" => TurnKind::Attachments,
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
    /// What the browser shows once the ChatGPT sign-in page has called back, in the user's
    /// language (`words.callback`). Empty: the agent's own words.
    pub callback: String,
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
                let mut params = json!({"keys": keys, "custom": custom});
                if let (false, Some(params)) = (setup.callback.is_empty(), params.as_object_mut()) {
                    params.insert("words".into(), json!({"callback": setup.callback}));
                }
                params
            }
            Request::Login(id) | Request::Logout(id) => json!({"id": id}),
            Request::Code { id, code } => json!({"id": id, "code": code}),
            Request::List | Request::Reopen | Request::Cancel => json!({}),
        }
    }
}

/// Strip from both ends what JavaScript's `trim()` strips. The agent trims a name that way, and
/// the two sides have to agree on which name is empty: Rust's own `trim` leaves U+FEFF (the
/// byte-order mark a paste can carry) where JavaScript takes it.
fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| {
        matches!(
            c,
            '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
        )
    })
}

/// Why a custom address is not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unfit {
    /// The name or the address is missing or out of range, or the address is no `http` or
    /// `https` address with a host.
    NameOrAddress,
    /// No model is named.
    NoModel,
    /// More than 16 models, or a name of more than 120 characters.
    Models,
    /// The address carries a user name, a password, a query or a fragment: a secret there would
    /// be kept in preferences and shown, where a key is not.
    AddressCarriesMore,
    /// An address of that name is kept already.
    Exists,
    /// As many addresses as can be kept are kept already.
    Full,
    /// Its key would travel in the clear to another computer.
    KeyInTheClear,
    /// What came as its key is no key.
    Key,
}

impl Unfit {
    /// The catalog's sentence for the page (`connect.error`).
    pub fn sentence(self) -> &'static str {
        match self {
            Unfit::NameOrAddress => "ui.agent_chat_conn_custom_need_both",
            Unfit::NoModel => "ui.agent_chat_conn_custom_need_model",
            Unfit::Models => "ui.agent_chat_conn_custom_too_many",
            Unfit::AddressCarriesMore => "ui.agent_chat_conn_custom_plain_url",
            Unfit::Exists => "ui.agent_chat_conn_custom_exists",
            Unfit::Full => "ui.agent_chat_conn_custom_full",
            Unfit::KeyInTheClear => "ui.ai_key_needs_https",
            Unfit::Key => REFUSED,
        }
    }

    /// The reason for the host's log. It repeats nothing that was typed.
    pub fn why(self) -> &'static str {
        match self {
            Unfit::NameOrAddress => "a custom address needs a name of 1 to 40 characters and an http or https address",
            Unfit::NoModel => "a custom address needs a model name",
            Unfit::Models => "a custom address takes at most 16 model names of at most 120 characters",
            Unfit::AddressCarriesMore => "the address carries a user name, a password, a query or a fragment",
            Unfit::Exists => "an address of that name is kept already",
            Unfit::Full => "no room for another custom address",
            Unfit::KeyInTheClear => "a key is only sent over HTTPS or to this computer",
            Unfit::Key => "the key is out of range",
        }
    }
}

/// The sentence for something the agent, or the app, could not do and has no better words for.
pub const REFUSED: &str = "ui.agent_chat_conn_refused";

/// An address a custom connection can have: `http` or `https`, a host, nothing before the host
/// and no query or fragment after the path, within the length limit. A short one (`http://pi`)
/// is as good as any. Slashes at its end are left off, as every base address is kept.
fn custom_url(url: &str) -> Result<String, Unfit> {
    let url = js_trim(url).trim_end_matches('/');
    if url.chars().count() > MAX_URL || url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return Err(Unfit::NameOrAddress);
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")).ok_or(Unfit::NameOrAddress)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') || url.contains(['?', '#']) {
        return Err(Unfit::AddressCarriesMore);
    }
    // What stands before a port: a name, an address, or an IPv6 address in its brackets.
    let host = match authority.strip_prefix('[') {
        Some(inner) => inner.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    if host.is_empty() {
        return Err(Unfit::NameOrAddress);
    }
    Ok(url.to_string())
}

/// A custom address as it was typed, checked: a name of 1 to 40 characters, an address
/// ([`custom_url`]) of at most 400, 1 to 16 model names of at most 120 each (`None`: an entry
/// that is no text), and a key of at most 4096 when one is given, which only goes over HTTPS or
/// to this computer. Nothing typed is quietly left out, except a model named twice.
pub fn custom_address(name: &str, url: &str, key: Option<&str>, models: &[Option<&str>]) -> Result<(CustomAddress, Option<String>), Unfit> {
    let plain = |s: &str| !s.chars().any(char::is_control);
    let name = js_trim(name);
    if name.is_empty() || name.chars().count() > MAX_NAME || !plain(name) {
        return Err(Unfit::NameOrAddress);
    }
    let url = custom_url(url)?;
    if models.is_empty() {
        return Err(Unfit::NoModel);
    }
    if models.len() > MAX_MODELS {
        return Err(Unfit::Models);
    }
    let mut named: Vec<String> = Vec::new();
    for model in models {
        let model = js_trim(model.ok_or(Unfit::Models)?);
        if model.is_empty() {
            return Err(Unfit::NoModel);
        }
        if model.chars().count() > MAX_MODEL || !plain(model) {
            return Err(Unfit::Models);
        }
        if !named.iter().any(|known| known == model) {
            named.push(model.to_string());
        }
    }
    let key = key.map(js_trim).filter(|key| !key.is_empty());
    if key.is_some_and(|key| key.chars().count() > MAX_SECRET || !plain(key)) {
        return Err(Unfit::Key);
    }
    if key.is_some() && !super::agents::key_safe_for(&url) {
        return Err(Unfit::KeyInTheClear);
    }
    Ok((CustomAddress { name: name.to_string(), url, models: named, key: key.is_some() }, key.map(str::to_string)))
}

/// The custom addresses of the preferences that can be handed to the agent: each checked as one
/// that is typed is, since a preferences file can be written by hand or by another version. One
/// that does not pass is left out here and left alone there.
fn fit_addresses(kept: &[CustomAddress]) -> Vec<CustomAddress> {
    let mut fit: Vec<CustomAddress> = Vec::new();
    for address in kept {
        if fit.len() >= MAX_CUSTOM {
            break;
        }
        let models: Vec<Option<&str>> = address.models.iter().map(|model| Some(model.as_str())).collect();
        let Ok((mut checked, _)) = custom_address(&address.name, &address.url, None, &models) else { continue };
        checked.key = address.key;
        let in_the_clear = checked.key && !super::agents::key_safe_for(&checked.url);
        if !in_the_clear && !fit.iter().any(|known| known.name == checked.name) {
            fit.push(checked);
        }
    }
    fit
}

// ---- What the app does with it ----

use super::State;
use super::backend::Secrets;
use crate::SolveApp;
use crate::i18n::{format as trf, tr};

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
    /// A sign-in was asked for and no snapshot has come since: it may be under way already.
    login_asked: bool,
    /// What the last action could not do: the catalog id of its sentence. It is worded where it
    /// is shown, as a failed turn is, because it can be set outside a frame.
    pub error: Option<&'static str>,
    /// The keys saved here, or refused, counted. The page's key dialog waits for it to move on
    /// from the count it saw at Save, then reads `error` for how it went: an update that worked
    /// leaves the rest of the state as it was, and so does a refusal like the one before it.
    pub key_saves: u64,
    pub session: Session,
    /// The agent refused a session since its last snapshot: it is not asked again before the next.
    pub refused: bool,
    /// The keys being read from the keychain for the next `setup`, and whether one could not be read.
    reading: Option<std::sync::mpsc::Receiver<(Setup, bool)>>,
    /// The agent can update the shared sign-in store before its sign-out reply arrives.
    pub(super) logout_pending: bool,
    pub kept: Option<Kept>,
    /// The count of changes ([`SolveApp::builtin_changes`]) this conversation's agent was last
    /// set up at. Behind it, the agent holds keys, addresses or sign-ins that are no longer so.
    set_up_at: u64,
    /// The setup under way only catches up with a change made in another conversation: the
    /// snapshot it brings is not a change of this one's to count again.
    catching_up: bool,
    /// A plan was signed in or out here since [`settle`] last looked.
    plans_changed: bool,
    /// The last `setup` handed to this conversation's agent: next-step suggestions hand the same
    /// to their own process of the agent. It holds keys, as the agent's environment does.
    pub(super) last_setup: Option<Setup>,
    /// The setups handed to this conversation's agent, counted: next-step suggestions hand each
    /// one on to their own process of the agent.
    pub(super) setups: u64,
}

impl Conn {
    /// The keys of a setup are being read: the setup follows.
    pub(super) fn reading_keys(&self) -> bool {
        self.reading.is_some()
    }

    pub(super) fn setup_current(&self, changes: u64) -> bool {
        self.set_up_at == changes && !self.reading_keys() && !self.logout_pending
    }

    #[cfg(test)]
    pub(super) fn test_setup_at(&mut self, changes: u64) {
        self.set_up_at = changes;
    }

    /// Take in what the agent now says of its connections.
    pub(super) fn take_in(&mut self, snapshot: Snapshot) {
        let catching_up = std::mem::take(&mut self.catching_up);
        if !catching_up && self.snapshot.as_ref().is_some_and(|before| before.plans_differ(&snapshot)) {
            self.plans_changed = true;
        }
        self.snapshot = Some(snapshot);
        self.refused = false;
        self.login_asked = false;
    }

    /// Keep the message of a turn that failed for `problem`.
    pub(super) fn keep(&mut self, problem: &TurnProblem) {
        let lapsed = self.snapshot.as_ref().is_some_and(|snapshot| !snapshot.plan_in(&problem.prefix));
        self.kept = Some(Kept { kind: problem.kind, prefix: problem.prefix.clone(), lapsed });
    }
}

/// Read every key the built-in agent is handed, for the addresses that can be handed over
/// ([`fit_addresses`]). `true` with it: the keychain refused one.
fn read_setup(secrets: Option<&dyn Secrets>, custom: &[CustomAddress], callback: String) -> (Setup, bool) {
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
    let custom: Vec<SetupCustom> = fit_addresses(custom)
        .into_iter()
        .map(|address| SetupCustom {
            api_key: if address.key { read(custom_secret(&address.name)) } else { None },
            name: address.name,
            url: address.url,
            models: address.models,
        })
        .collect();
    (Setup { keys, custom, callback }, failed)
}

/// What the browser is to show when the ChatGPT sign-in page calls back, in the user's language.
/// A setup can be begun outside a frame, where the language is not set: it is set here.
fn callback_words(app: &SolveApp) -> String {
    let _language = crate::i18n::scope(&app.preferences.language);
    tr("ui.agent_chat_conn_callback").to_string()
}

fn send(app: &mut SolveApp, request: Request) {
    if let Request::Setup(setup) = &request {
        app.agent_chat.conn.last_setup = Some(setup.clone());
        app.agent_chat.conn.setups = app.agent_chat.conn.setups.wrapping_add(1);
    }
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
    app.agent_chat.conn.set_up_at = app.builtin_changes;
    let secrets = app.services.secrets.clone();
    let custom = app.preferences.mastra_custom.clone();
    let callback = callback_words(app);
    let (tx, rx) = std::sync::mpsc::channel();
    let wake = ctx.clone();
    let spawned = std::thread::Builder::new().name("mastra-keys".into()).spawn(move || {
        let _ = tx.send(read_setup(secrets.as_deref(), &custom, callback));
        wake.request_repaint();
    });
    if spawned.is_ok() {
        app.agent_chat.conn.reading = Some(rx);
    } else {
        // Nothing to read on: the agent still starts, with the addresses and without keys.
        app.agent_chat.conn.reading = None;
        app.agent_chat.conn.error = Some("ui.host_ai_key_unavailable");
        let (setup, _) = read_setup(None, &app.preferences.mastra_custom, callback_words(app));
        send(app, Request::Setup(setup));
    }
}

/// The browser build has no agent process to set up.
#[cfg(target_arch = "wasm32")]
pub(super) fn begin_setup(_app: &mut SolveApp, _ctx: &egui::Context) {}

/// A key or a custom address was kept or removed: every conversation's agent is behind until it
/// is set up again, and this one's is set up now.
fn changed(app: &mut SolveApp, ctx: &egui::Context) {
    app.builtin_changes = app.builtin_changes.wrapping_add(1);
    // A key or address is a provider change: the turn's desktop tool authority does not survive it.
    if let Some(backend) = app.agent_chat.backend.as_mut() {
        backend.retire_bridge_authority();
    }
    begin_setup(app, ctx);
}

/// The conversation's turn each frame it is the one shown. Its agent is set up again when a key,
/// a custom address or a sign-in was changed in another conversation since (each design keeps
/// an agent of its own, and all of them read the same keychain and the same sign-ins). And the
/// keys of a setup are taken in once they are read, and handed to the agent.
pub(super) fn poll(app: &mut SolveApp, ctx: &egui::Context) {
    use std::sync::mpsc::TryRecvError;
    let conn = &app.agent_chat.conn;
    if conn.active && app.agent_chat.backend.is_some() && conn.set_up_at != app.builtin_changes {
        app.agent_chat.conn.catching_up = true;
        begin_setup(app, ctx);
    }
    let Some(reading) = &app.agent_chat.conn.reading else { return };
    let (setup, failed) = match reading.try_recv() {
        Ok(read) => read,
        Err(TryRecvError::Empty) => return,
        // The reader ended without an answer: the agent starts without keys rather than not at all.
        Err(TryRecvError::Disconnected) => (read_setup(None, &app.preferences.mastra_custom, callback_words(app)).0, true),
    };
    app.agent_chat.conn.reading = None;
    if failed {
        app.agent_chat.conn.error = Some("ui.host_ai_key_unavailable");
    }
    send(app, Request::Setup(setup));
}

/// Bring the conversation in line with the agent's last snapshot. With nothing connected the
/// conversation needs a sign-in and has no session (the agent refuses one then); once something
/// is connected a session is opened, once, and the message kept from a refused sign-in is sent
/// again when that plan is signed in.
pub(super) fn settle(app: &mut SolveApp) {
    // A plan signed in or out here: the other conversations' agents are behind, this one's is not.
    if std::mem::take(&mut app.agent_chat.conn.plans_changed) {
        app.builtin_changes = app.builtin_changes.wrapping_add(1);
        app.agent_chat.conn.set_up_at = app.builtin_changes;
        // A plan signed in or out: a CAD call already in flight may no longer draw on it.
        if let Some(backend) = app.agent_chat.backend.as_mut() {
            backend.retire_bridge_authority();
        }
    }
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
    if super::agents::builtin_current(app) { page(&app.agent_chat.conn) } else { Value::Null }
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
        .map(|key| {
            json!({
                "id": key.id, "name": key.name, "saved": key.saved, "shadowed": key.shadowed, "prefix": key.prefix,
                // The dialog's complete title: the provider's name is a value, never a template.
                "title": trf("ui.agent_chat_conn_key_title", &[("name", &key.name)]),
            })
        })
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
        "problem": snapshot.problem.as_ref().map(|problem| json!({"id": problem.id, "kind": problem.kind.id(), "text": problem_text(problem)})),
        "error": conn.error.map(tr),
        "keySaves": conn.key_saves,
        "dates": snapshot.dates,
        "connected": snapshot.connected(),
    })
}

/// Why a sign-in could not finish, as the page says it under the plan's row: which step and, at
/// the step with the sign-in service, its status and code (INT-1521). Where the agent wrote the
/// sign-in down, the sentence says where.
fn problem_text(problem: &LoginProblem) -> String {
    let detail = [problem.status.map(|status| format!("HTTP {status}")), problem.code.clone()].into_iter().flatten().collect::<Vec<_>>().join(", ");
    let with = |id: &'static str| trf(id, &[("detail", if detail.is_empty() { "?" } else { &detail })]);
    let sentence = match (problem.kind, &problem.holder) {
        (LoginProblemKind::Port, Some(PortHolder::Here)) => tr("ui.agent_chat_conn_problem_port_here").to_string(),
        (LoginProblemKind::Port, Some(PortHolder::Intcad)) => tr("ui.agent_chat_conn_problem_port_intcad").to_string(),
        (LoginProblemKind::Port, None) => tr("ui.agent_chat_conn_problem_port").to_string(),
        (LoginProblemKind::Listen, _) => {
            let port = problem.port.map(|port| port.to_string()).unwrap_or_else(|| "1457".into());
            trf("ui.agent_chat_conn_problem_listen", &[("port", &port), ("code", problem.code.as_deref().unwrap_or("?"))])
        }
        (LoginProblemKind::Timeout, _) => tr("ui.agent_chat_conn_problem_timeout").to_string(),
        (LoginProblemKind::Refused, _) => with("ui.agent_chat_conn_problem_exchange_refused"),
        (LoginProblemKind::Offline, _) if problem.step == Some(Step::Exchange) => with("ui.agent_chat_conn_problem_exchange_offline"),
        (LoginProblemKind::Offline, _) => tr("ui.agent_chat_conn_problem_offline").to_string(),
        (LoginProblemKind::Failed, _) => match problem.step {
            Some(Step::Open) => tr("ui.agent_chat_conn_problem_step_open").to_string(),
            Some(Step::Wait) => tr("ui.agent_chat_conn_problem_step_wait").to_string(),
            Some(Step::Exchange) if !detail.is_empty() => with("ui.agent_chat_conn_problem_exchange_failed"),
            Some(Step::Store) => with("ui.agent_chat_conn_problem_store"),
            _ => tr("ui.agent_chat_conn_problem_failed").to_string(),
        },
    };
    match &problem.log {
        Some(path) => format!("{sentence} {}", trf("ui.agent_chat_conn_problem_log", &[("path", path)])),
        None => sentence,
    }
}

/// End the sign-in this conversation's agent has under way, or was asked for and has not
/// answered yet: the conversation is leaving the window (another design is shown), or another
/// design's sign-in needs the port it holds (INT-1522).
pub(crate) fn cancel_sign_in(chat: &mut super::AgentChat) {
    let conn = &mut chat.conn;
    let under_way = conn.login_asked || conn.snapshot.as_ref().is_some_and(|snapshot| snapshot.login.is_some());
    if !conn.active || !under_way {
        return;
    }
    conn.login_asked = false;
    if let Some(backend) = chat.backend.as_mut() {
        backend.connections(Request::Cancel);
    }
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
    app.agent_chat.conn.error = kept.err();
    kept.is_ok()
}

/// Refuse an action for something the user typed, and tell them: the page shows `sentence`
/// (`connect.error`), where a refusal that only reached the host's log would leave them waiting.
fn refuse(app: &mut SolveApp, ctx: &egui::Context, sentence: &'static str, why: String) -> Result<(), String> {
    app.agent_chat.conn.error = Some(sentence);
    ctx.request_repaint();
    Err(why)
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
    if !super::agents::builtin_current(app) {
        return Err(format!("{kind}: this agent has no connections"));
    }
    // What the last action could not do is no longer the news.
    app.agent_chat.conn.error = None;
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
            // A sign-in left under way in another design would hold the port this one needs.
            crate::documents::cancel_background_sign_ins(app);
            app.agent_chat.conn.login_asked = true;
            send(app, Request::Login(id.to_string()));
        }
        "conn_code" => {
            let id = text("id").ok_or("conn_code: no id")?;
            let code = text("code").map(js_trim).filter(|code| !code.is_empty() && code.chars().count() <= MAX_SECRET);
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
            if app.agent_chat.conn.logout_pending {
                return Err("conn_logout: a sign-out is already under way".into());
            }
            app.agent_chat.conn.logout_pending = true;
            send(app, Request::Logout(id.to_string()));
        }
        "conn_key" => {
            // Every save is answered, kept or refused, so the dialog that asked stops waiting.
            app.agent_chat.conn.key_saves = app.agent_chat.conn.key_saves.wrapping_add(1);
            let Ok(provider) = provider() else { return refuse(app, ctx, REFUSED, "conn_key: provider must be one of the five".into()) };
            let key =
                text("key").map(js_trim).filter(|key| !key.is_empty() && key.chars().count() <= MAX_SECRET && !key.chars().any(char::is_control));
            let Some(key) = key else { return refuse(app, ctx, REFUSED, "conn_key: a key has 1 to 4096 characters".into()) };
            if change(app, |secrets| secrets.set(&key_secret(provider), key)) {
                changed(app, ctx);
            }
        }
        "conn_key_remove" => {
            let provider = provider()?;
            if change(app, |secrets| secrets.delete(&key_secret(provider))) {
                changed(app, ctx);
            }
        }
        "conn_custom" => {
            // Only what can be handed over counts. An entry that cannot (written by hand, or by
            // another version) is never shown, so it takes neither a name nor room.
            let kept = fit_addresses(&app.preferences.mastra_custom);
            let checked = (|| {
                let (Some(name), Some(url)) = (text("name"), text("url")) else { return Err(Unfit::NameOrAddress) };
                let models = action.get("models").and_then(Value::as_array).ok_or(Unfit::NoModel)?;
                let key = match action.get("key") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(key)) => Some(key.as_str()),
                    Some(_) => return Err(Unfit::Key),
                };
                let models: Vec<Option<&str>> = models.iter().map(Value::as_str).collect();
                let (address, key) = custom_address(name, url, key, &models)?;
                // An address is added, never written over: writing over one would drop its key.
                if kept.iter().any(|known| known.name == address.name) {
                    return Err(Unfit::Exists);
                }
                if kept.len() >= MAX_CUSTOM {
                    return Err(Unfit::Full);
                }
                Ok((address, key))
            })();
            let (address, key) = match checked {
                Ok(checked) => checked,
                Err(unfit) => return refuse(app, ctx, unfit.sentence(), format!("conn_custom: {}", unfit.why())),
            };
            // The key goes first: an address is not kept with a key the keychain did not take.
            let secret = custom_secret(&address.name);
            if key.is_none_or(|key| change(app, |secrets| secrets.set(&secret, &key))) {
                // An entry of this name that could not be handed over makes way, with its key:
                // it was never shown, so nobody could have removed it.
                let (same, others): (Vec<CustomAddress>, Vec<CustomAddress>) =
                    std::mem::take(&mut app.preferences.mastra_custom).into_iter().partition(|other| js_trim(&other.name) == address.name);
                app.preferences.mastra_custom = others;
                for old in same {
                    let stale = custom_secret(&old.name);
                    if old.key
                        && !(address.key && stale == secret)
                        && let Some(secrets) = &app.services.secrets
                    {
                        // Left behind it would only lie in the keychain: nothing reads it again.
                        let _ = secrets.delete(&stale);
                    }
                }
                app.preferences.mastra_custom.push(address);
                changed(app, ctx);
            }
        }
        "conn_custom_remove" => {
            let id = text("id").ok_or("conn_custom_remove: no id")?;
            // The page names an address by the id the agent gave it; preferences know it by its name.
            let named = app.agent_chat.conn.snapshot.as_ref().and_then(|snapshot| snapshot.custom.iter().find(|custom| custom.id == id));
            let name = named.map_or_else(|| id.to_string(), |custom| custom.name.clone());
            let at = app.preferences.mastra_custom.iter().position(|kept| kept.name == name).ok_or("conn_custom_remove: no such custom address")?;
            let address = app.preferences.mastra_custom.remove(at);
            if address.key
                && let Some(secrets) = &app.services.secrets
                && secrets.delete(&custom_secret(&address.name)).is_err()
            {
                app.agent_chat.conn.error = Some("ui.host_ai_key_unavailable");
            }
            changed(app, ctx);
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
        assert_eq!((plan_name(&snapshot.plans[0]).as_str(), plan_name(&snapshot.plans[1]).as_str()), ("ChatGPT", "Some New Plan"));
        assert!(snapshot.keys[0].saved && snapshot.keys[0].shadowed && !snapshot.keys[1].saved);
        assert_eq!(snapshot.custom[0].models, ["llama3.2"]);
        let login = snapshot.login.as_ref().expect("the sign-in under way");
        assert_eq!((login.id.as_str(), login.phase, login.user_code.as_deref(), login.incomplete), ("anthropic", Phase::Code, None, true));
        assert!(!format!("{snapshot:?}").contains("claude.ai"), "the address is not kept");
        let port = LoginProblem { id: "openai-codex".into(), kind: LoginProblemKind::Port, holder: None, port: None, code: None, step: None, status: None, log: None };
        assert_eq!(snapshot.problem, Some(port));
        assert_eq!(snapshot.dates.len(), 1, "a date that is no text is left out");
        // Something is connected by the saved key and the custom address, though the plan expired.
        assert!(snapshot.connected() && !snapshot.plan_in("openai"));
    }

    /// INT-1522: who holds the sign-in's port, and the port and code of one the system would not
    /// open, are read; a code is a short word, and anything else of them is left out.
    #[test]
    fn a_sign_in_problem_says_who_holds_the_port_or_what_the_system_answered() {
        let problem = |value: Value| Snapshot::parse(&json!({"plans": [], "keys": [], "problem": value})).and_then(|snapshot| snapshot.problem);
        let holder = |value: Value| problem(json!({"id": "openai-codex", "kind": "port", "holder": value})).and_then(|problem| problem.holder);
        assert_eq!(holder(json!({"kind": "here"})), Some(PortHolder::Here));
        assert_eq!(holder(json!({"kind": "intcad"})), Some(PortHolder::Intcad));
        for none in [json!({"kind": "program", "name": "Codex"}), json!({"kind": "someone"}), json!("Codex"), json!(null)] {
            assert_eq!(holder(none.clone()), None, "{none}");
        }
        let listen = problem(json!({"id": "openai-codex", "kind": "listen", "port": 1457, "code": " EACCES\u{7} "})).expect("a problem");
        assert_eq!((listen.kind, listen.port, listen.code.as_deref()), (LoginProblemKind::Listen, Some(1457), Some("EACCES")));
        let odd = problem(json!({"id": "openai-codex", "kind": "listen", "port": 99999, "code": 13})).expect("a problem");
        assert_eq!((odd.port, odd.code), (None, None));
        let long = problem(json!({"id": "openai-codex", "kind": "listen", "code": "x".repeat(500)})).expect("a problem");
        assert_eq!(long.code, Some("x".repeat(MAX_WORD)));
        assert_eq!(problem(json!({"id": "openai-codex", "kind": "timeout"})).map(|problem| problem.kind), Some(LoginProblemKind::Timeout));
        // INT-1521: the step, the service's status and the log's path; a status that is none, a
        // step this version does not know and a path with a line break in it are left out.
        let at = problem(json!({"id": "openai-codex", "kind": "refused", "step": "exchange", "status": 403, "code": "unsupported_country", "log": "/a/sign-in.log"}))
            .expect("a problem");
        assert_eq!((at.kind, at.step, at.status, at.log.as_deref()), (LoginProblemKind::Refused, Some(Step::Exchange), Some(403), Some("/a/sign-in.log")));
        let odd = problem(json!({"id": "openai-codex", "kind": "refused", "step": "teleport", "status": 7, "log": "/a\nb"})).expect("a problem");
        assert_eq!((odd.step, odd.status, odd.log), (None, None, None));
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
        assert_eq!(snapshot.name_of("openai"), "ChatGPT", "the plan wins over the key with the same prefix");
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
        for kind in [TurnKind::Auth, TurnKind::Key, TurnKind::Quota, TurnKind::Rate, TurnKind::Network, TurnKind::Model, TurnKind::Attachments] {
            let sentence = kind.sentence().expect("a sentence of the app's own");
            assert!(!tr(sentence).starts_with("ui."), "{sentence} is in the catalog");
        }
        // `key` is the app's own reading of `auth`, never the agent's word.
        assert_eq!(read(json!({"intcad": {"kind": "key", "prefix": "openai"}})).map(|problem| problem.kind), Some(TurnKind::Other));
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
        let setup = Setup { keys: vec![("openai".into(), "sk-openai-never-shown".into())], custom: vec![custom], ..Default::default() };
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

    /// B11: a custom address is taken only whole and in range, each refusal has its own sentence
    /// for the page, and the reason for the host's log repeats nothing that was typed.
    #[test]
    fn a_custom_address_is_taken_only_in_range() {
        let models = [Some("llama3.2"), Some(" qwen3 "), Some("llama3.2")];
        let (address, key) = custom_address("  my-llm ", " http://localhost:11434/v1/ ", Some("  k  "), &models).expect("a custom address");
        let kept = CustomAddress {
            name: "my-llm".into(),
            url: "http://localhost:11434/v1".into(),
            models: vec!["llama3.2".into(), "qwen3".into()],
            key: true,
        };
        assert_eq!((address, key.as_deref()), (kept, Some("k")), "trimmed, and a model named twice is named once");
        let (plain, none) = custom_address("local", "https://llm.example/v1", Some("   "), &models[..1]).expect("a custom address");
        assert!(!plain.key && none.is_none(), "an empty key is no key");
        // A short address is an address: a host is all it needs.
        for (typed, kept) in [
            ("http://pi", "http://pi"),
            ("HTTP://Pi:8080/", "HTTP://Pi:8080"),
            ("http://[::1]:11434/v1", "http://[::1]:11434/v1"),
            ("https://a.b/x@y/v1", "https://a.b/x@y/v1"),
        ] {
            assert_eq!(custom_address("n", typed, None, &[Some("m")]).map(|(address, _)| address.url).as_deref(), Ok(kept), "{typed}");
        }
        // The name is trimmed as the agent trims it: a byte-order mark is no name.
        assert_eq!(
            custom_address("\u{feff} my-llm\u{feff}\u{3000}", "http://pi", None, &[Some("m")]).map(|(address, _)| address.name).as_deref(),
            Ok("my-llm")
        );
        assert_eq!(js_trim("\u{feff}\u{a0}\t x\u{2028}\u{feff}"), "x");
        assert_eq!(js_trim("\u{85}x"), "\u{85}x", "what JavaScript leaves is left");

        let ok = [Some("m")];
        let seventeen: Vec<String> = (0..17).map(|n| format!("m{n}")).collect();
        let seventeen: Vec<Option<&str>> = seventeen.iter().map(|model| Some(model.as_str())).collect();
        let (long_name, long_url, long_model, long_key) =
            ("n".repeat(41), format!("https://llm.example/{}", "p".repeat(400)), "m".repeat(121), "k".repeat(4097));
        let local = "http://localhost:11434/v1";
        let refused = [
            (custom_address("", local, None, &ok), Unfit::NameOrAddress),
            (custom_address("   ", local, None, &ok), Unfit::NameOrAddress),
            (custom_address("\u{feff}", local, None, &ok), Unfit::NameOrAddress),
            (custom_address(&long_name, local, None, &ok), Unfit::NameOrAddress),
            (custom_address("tab\tname", local, None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", "localhost:11434", None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", "ftp://localhost/v1", None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", "http://", None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", "http://:8080/v1", None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", "http://local host/v1", None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", "http://localhost\\@evil.example/v1", None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", &long_url, None, &ok), Unfit::NameOrAddress),
            (custom_address("my-llm", "https://user:sk-in-the-address@llm.example/v1", None, &ok), Unfit::AddressCarriesMore),
            (custom_address("my-llm", "https://token@llm.example/v1", None, &ok), Unfit::AddressCarriesMore),
            (custom_address("my-llm", "https://llm.example/v1?api_key=sk-in-the-address", None, &ok), Unfit::AddressCarriesMore),
            (custom_address("my-llm", "https://llm.example/v1#sk-in-the-address", None, &ok), Unfit::AddressCarriesMore),
            (custom_address("my-llm", local, None, &[]), Unfit::NoModel),
            (custom_address("my-llm", local, None, &[Some("m"), Some("  ")]), Unfit::NoModel),
            (custom_address("my-llm", local, None, &seventeen), Unfit::Models),
            (custom_address("my-llm", local, None, &[Some("m"), Some(long_model.as_str())]), Unfit::Models),
            (custom_address("my-llm", local, None, &[Some("m"), None]), Unfit::Models),
            (custom_address("my-llm", local, Some(long_key.as_str()), &ok), Unfit::Key),
            (custom_address("my-llm", "http://192.168.1.20:11434/v1", Some("k"), &ok), Unfit::KeyInTheClear),
            (custom_address("my-llm", "http://pi", Some("k"), &ok), Unfit::KeyInTheClear),
        ];
        for (n, (result, unfit)) in refused.iter().enumerate() {
            assert_eq!(result.as_ref().err(), Some(unfit), "case {n}");
        }
        assert!(
            custom_address("my-llm", "http://192.168.1.20:11434/v1", None, &ok).is_ok(),
            "without a key, plain HTTP to another computer is the user's to choose"
        );
        let limits =
            custom_address(&"n".repeat(40), &format!("https://llm.example/{}", "p".repeat(380)), Some("k".repeat(4096).as_str()), &seventeen[..16]);
        assert!(limits.is_ok(), "the limits themselves are in range");
        for unfit in [
            Unfit::NameOrAddress,
            Unfit::NoModel,
            Unfit::Models,
            Unfit::AddressCarriesMore,
            Unfit::Exists,
            Unfit::Full,
            Unfit::KeyInTheClear,
            Unfit::Key,
        ] {
            assert!(!tr(unfit.sentence()).starts_with("ui."), "{unfit:?} has its sentence in the catalog");
            let why = unfit.why();
            assert!(!why.is_empty() && !why.contains("my-llm") && !why.contains("localhost") && !why.contains("sk-"), "{why}");
        }
    }

    /// What preferences hold is checked as what is typed is before it is handed to the agent: a
    /// file written by hand, or by another version, hands over nothing the form would refuse.
    /// What does not pass is left out of the hand-over and left alone in the list.
    #[test]
    fn custom_addresses_from_preferences_are_checked_before_they_are_handed_over() {
        let address = |name: &str, url: &str, models: &[&str], key: bool| CustomAddress {
            name: name.into(),
            url: url.into(),
            models: models.iter().map(|model| model.to_string()).collect(),
            key,
        };
        let kept = vec![
            address("good", "http://localhost:11434/v1", &["llama3.2"], true),
            address("\u{feff}", "http://localhost:11434/v1", &["m"], false),
            address("secret-in-address", "https://user:sk-in-the-address@llm.example/v1", &["m"], false),
            address("query", "https://llm.example/v1?api_key=sk-in-the-address", &["m"], false),
            address("no-models", "https://llm.example/v1", &[], false),
            address("key-in-the-clear", "http://192.168.1.20:11434/v1", &["m"], true),
            address("good", "https://second.example/v1", &["m"], false),
            address(" pi ", "http://pi/", &["m", "m"], false),
        ];
        let before = kept.clone();
        let fit = fit_addresses(&kept);
        assert_eq!(fit, [address("good", "http://localhost:11434/v1", &["llama3.2"], true), address("pi", "http://pi", &["m"], false)]);
        assert_eq!(kept, before, "the list itself is not rewritten");
        let (setup, failed) = read_setup(None, &kept, "Go back to Intcad.".into());
        assert_eq!(setup.custom.iter().map(|custom| custom.name.as_str()).collect::<Vec<_>>(), ["good", "pi"]);
        assert!(!failed && !Request::Setup(setup).params().to_string().contains("sk-in-the-address"));
        let many: Vec<CustomAddress> = (0..40).map(|n| address(&format!("a{n}"), "http://pi", &["m"], false)).collect();
        assert_eq!(fit_addresses(&many).len(), MAX_CUSTOM);
    }

    /// A plan that changed hands is told from a snapshot that says the same again.
    #[test]
    fn a_plan_signed_in_or_out_is_a_change_and_the_same_again_is_not() {
        let before = Snapshot::parse(&told()).expect("a snapshot");
        assert!(!before.plans_differ(&before));
        let mut keys_only = told();
        keys_only["keys"][0]["saved"] = json!(false);
        keys_only["login"] = json!(null);
        assert!(
            !before.plans_differ(&Snapshot::parse(&keys_only).expect("a snapshot")),
            "keys and a sign-in under way are not a plan changing hands"
        );
        for state in ["in", "out"] {
            let mut now = told();
            now["plans"][0]["state"] = json!(state);
            assert!(before.plans_differ(&Snapshot::parse(&now).expect("a snapshot")), "{state}");
        }
        let mut conn = Conn::default();
        conn.take_in(before.clone());
        assert!(!conn.plans_changed, "the first word is no change");
        let mut signed_out = told();
        signed_out["plans"][0]["state"] = json!("out");
        let signed_out = Snapshot::parse(&signed_out).expect("a snapshot");
        conn.take_in(signed_out.clone());
        assert!(conn.plans_changed);
        // Catching up with a change made elsewhere is not a change to tell the others of again.
        let mut behind = Conn::default();
        behind.take_in(before);
        behind.catching_up = true;
        behind.take_in(signed_out);
        assert!(!behind.plans_changed && !behind.catching_up);
    }

    /// A refused credential is a sign-in's only on a plan that is signed in or was until now:
    /// through a saved key, a custom address or a connection nothing is known of it is the key's.
    #[test]
    fn a_refused_credential_is_the_keys_unless_a_plan_holds_the_prefix() {
        let snapshot = Snapshot::parse(&told()).expect("a snapshot");
        let auth = |prefix: &str| TurnProblem { kind: TurnKind::Auth, prefix: prefix.into() };
        // `openai` is the expired ChatGPT plan's: the sign-in was refused, though a key is saved too.
        assert_eq!(auth("openai").as_told(Some(&snapshot)).kind, TurnKind::Auth);
        for prefix in ["google", "mastracode/my-llm", "mastracode", "new", "nowhere", ""] {
            assert_eq!(auth(prefix).as_told(Some(&snapshot)).kind, TurnKind::Key, "{prefix}");
        }
        assert_eq!(auth("openai").as_told(None).kind, TurnKind::Key, "nothing is known of the connections");
        let mut signed_out = told();
        signed_out["plans"][0]["state"] = json!("out");
        let signed_out = Snapshot::parse(&signed_out).expect("a snapshot");
        assert_eq!(auth("openai").as_told(Some(&signed_out)).kind, TurnKind::Key, "signed out, the saved key served the turn");
        assert_eq!(signed_out.name_of("openai"), "OpenAI");
        // A provider without a key is named before a plan that is signed out: the turn did not
        // go through that plan.
        let mut bare = told();
        bare["plans"][0]["state"] = json!("out");
        bare["keys"][0]["saved"] = json!(false);
        assert_eq!(Snapshot::parse(&bare).expect("a snapshot").name_of("openai"), "OpenAI");
        // Every other kind is told as the agent says it.
        for kind in [TurnKind::Quota, TurnKind::Rate, TurnKind::Network, TurnKind::Model, TurnKind::Other] {
            assert_eq!(TurnProblem { kind, prefix: "google".into() }.as_told(Some(&snapshot)).kind, kind);
        }
    }

    #[test]
    fn the_keychain_names_are_the_built_in_agents_own() {
        assert_eq!(key_secret("openai"), "mastra/key/openai");
        assert_eq!(custom_secret("my-llm"), "mastra/custom/my-llm");
        assert_eq!(KEY_IDS, ["anthropic", "openai", "google", "openrouter", "deepseek"]);
    }

    /// B10 (next-step suggestions): the setup handed to the chat's agent is kept, so the guessing
    /// process of the same agent can be handed the same; another request does not replace it.
    #[test]
    fn the_setup_handed_to_the_agent_is_kept_for_the_guessing_process() {
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        let setup = Setup { keys: vec![("anthropic".into(), "k-chat".into())], ..Default::default() };
        send(&mut app, Request::Setup(setup.clone()));
        send(&mut app, Request::List);
        assert_eq!(app.agent_chat.conn.last_setup, Some(setup));
    }
}
