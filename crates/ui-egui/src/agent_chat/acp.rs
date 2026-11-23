//! The client side of the Agent Client Protocol (ACP, <https://agentclientprotocol.com>), version 1:
//! JSON-RPC 2.0, one message per line over the agent's stdin/stdout. This file only builds and
//! reads messages; the process lives in `process.rs` and the state in `mod.rs`, so everything
//! here is plain data and tested without an agent.
//!
//! What Intcad says it can do (`clientCapabilities`): no file system and no terminal (the agent
//! reaches the design through Intcad's MCP server instead), and terminal login, so an agent
//! that needs one describes a command for the user to run rather than asking for a key.

use serde::Serialize;
use serde_json::{Value, json};

use super::{MAX_MESSAGE, MAX_MESSAGE_BYTES};

/// One advertised value, with an optional human-readable group label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChoice {
    pub value: String,
    pub name: String,
    pub description: String,
    pub group: Option<String>,
}
/// A supported ACP select option; identifiers and categories belong to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub current_value: String,
    pub options: Vec<ConfigChoice>,
}

/// One slash command the agent advertises for its open session (`available_commands_update`).
/// `name` is the agent's own, trimmed; `description` is its own words or empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
}

/// Parse ACP select options as a bounded authoritative snapshot.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigParse {
    Absent,
    Invalid,
    Snapshot(Vec<ConfigOption>),
}
pub fn config_options(v: &Value) -> ConfigParse {
    let Some(raw_list) = v.get("configOptions") else {
        return ConfigParse::Absent;
    };
    let Some(list) = raw_list.as_array() else {
        return ConfigParse::Invalid;
    };
    let mut out = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for x in list.iter().take(64) {
        match x.get("type").and_then(Value::as_str) {
            Some("select") => {}
            Some(_) => continue,
            None => return ConfigParse::Invalid,
        }
        let Some(id) = x.get("id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 256) else {
            return ConfigParse::Invalid;
        };
        let Some(current_value) = x.get("currentValue").and_then(Value::as_str).filter(|s| s.len() <= 256) else {
            return ConfigParse::Invalid;
        };
        let Some(raw) = x.get("options").and_then(Value::as_array) else {
            return ConfigParse::Invalid;
        };
        if !ids.insert(id.to_string()) {
            continue;
        }
        let mut choices = Vec::new();
        let mut values = std::collections::HashSet::new();
        'items: for item in raw.iter().take(128) {
            if choices.len() >= 128 {
                break;
            }
            let group =
                item.get("name").and_then(Value::as_str).or_else(|| item.get("group").and_then(Value::as_str)).map(|s| s.chars().take(256).collect());
            if let Some(cs) = item.get("options").and_then(Value::as_array) {
                for c in cs.iter().take(128) {
                    if choices.len() >= 128 {
                        break 'items;
                    }
                    let Some(value) = c.get("value").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 256) else {
                        continue;
                    };
                    if !values.insert(value.to_string()) {
                        continue;
                    }
                    choices.push(ConfigChoice {
                        value: value.into(),
                        name: c.get("name").and_then(Value::as_str).unwrap_or(value).chars().take(256).collect(),
                        description: c.get("description").and_then(Value::as_str).unwrap_or("").chars().take(1024).collect(),
                        group: group.clone(),
                    });
                }
            } else {
                let Some(value) = item.get("value").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 256) else {
                    continue;
                };
                if values.insert(value.to_string()) {
                    choices.push(ConfigChoice {
                        value: value.into(),
                        name: item.get("name").and_then(Value::as_str).unwrap_or(value).chars().take(256).collect(),
                        description: item.get("description").and_then(Value::as_str).unwrap_or("").chars().take(1024).collect(),
                        group: None,
                    });
                }
            }
        }
        if choices.is_empty() {
            return ConfigParse::Invalid;
        }
        out.push(ConfigOption {
            id: id.into(),
            name: x.get("name").and_then(Value::as_str).unwrap_or(id).chars().take(256).collect(),
            description: x.get("description").and_then(Value::as_str).unwrap_or("").chars().take(1024).collect(),
            category: x.get("category").and_then(Value::as_str).unwrap_or("").chars().take(128).collect(),
            current_value: current_value.into(),
            options: choices,
        });
    }
    ConfigParse::Snapshot(out)
}

/// The protocol version we speak.
pub const PROTOCOL_VERSION: u64 = 1;

/// JSON-RPC "method not found", our answer to every agent request we don't serve.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The error code ACP agents use for "authentication required".
pub const AUTH_REQUIRED: i64 = -32000;

/// One request or notification to send.
pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}
pub fn reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}
pub fn reply_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

pub fn initialize_params(version: &str) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "clientCapabilities": {
            "fs": {"readTextFile": false, "writeTextFile": false},
            "terminal": false,
            "auth": {"terminal": true},
            "_meta": {"terminal-auth": true},
        },
        "clientInfo": {"name": "intcad", "title": "Intcad", "version": version},
    })
}

/// The name the chat gives its MCP server (#2592): not `solidcraft`, the name docs/mcp.md suggests
/// for a user's own setup, so an agent that merges its own MCP servers with the session's (Codex
/// keeps its own entry on a clash) can't shadow the bridge with a headless session. It is what a
/// person reads beside each tool call an agent makes, so it carries the product's name; it was
/// `solidcraft-app` while the product showed that name.
pub const BRIDGE_NAME: &str = "intcad-app";

/// Whether a permission question is about a tool of the chat's own MCP server, whose calls
/// Intcad's gate checks. The agent writes both values and ACP has nothing better to go on, so
/// the test is strict: the whole title is the tool's name as the Claude adapter forms it
/// (`mcp__intcad-app__batch`), one word with no room for a command line, a path or other free
/// text, and the kind is `other` (the kind an MCP tool has) or not given. Any other title goes
/// through the user's mode and remembered answers: agents that word their MCP tools differently,
/// and a user's own MCP server, which the gate doesn't see.
pub fn is_bridge_tool(title: &str, kind: Option<&str>) -> bool {
    // Hosts name a server's tool in more than one way: `mcp__intcad-app__batch` (the ACP
    // convention) and `intcad-app_batch` (Mastra Code's, as a real session sends it).
    let tool = title
        .strip_prefix("mcp__")
        .and_then(|t| t.strip_prefix(BRIDGE_NAME))
        .and_then(|t| t.strip_prefix("__"))
        .or_else(|| title.strip_prefix(BRIDGE_NAME).and_then(|t| t.strip_prefix('_')).filter(|t| !t.starts_with('_')));
    let named = tool.is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
    named && matches!(kind, None | Some("other"))
}

/// The MCP server the agent starts to reach the design: `intcad-cli mcp --connect` bridged to
/// this app's control port. The server finds the port's token in the token file the app wrote
/// (docs/control-protocol.md), so the token never travels through the agent, and it keeps its
/// default folder limits (docs/mcp.md): no `--allow-any-path`. It is told which folder that
/// file is in ([`McpBridge::to_json`]).
#[derive(Clone, Debug, PartialEq)]
pub struct McpBridge {
    pub cli: std::path::PathBuf,
    pub port: u16,
}

impl McpBridge {
    pub fn to_json(&self) -> Value {
        // The folder of the app's token files, by name. An agent may start its servers with a
        // clean environment (Codex does): the server then knows nothing of a folder the app
        // was given (`SOLIDCRAFT_CONTROL_DIR`), looks in the default one, and the app refuses
        // every tool call as unauthorized. The folder is no secret; the token stays in its file.
        let folder = solidcraft_engine::control_auth::token_path(self.port)
            .and_then(|file| file.parent().map(|folder| json!({"name": "SOLIDCRAFT_CONTROL_DIR", "value": folder.to_string_lossy()})));
        json!({
            "name": BRIDGE_NAME,
            "command": self.cli.to_string_lossy(),
            "args": ["mcp", "--connect", format!("127.0.0.1:{}", self.port)],
            "env": folder.into_iter().collect::<Vec<_>>(),
        })
    }

    /// The same bridge for an agent inside WSL: the CLI as WSL sees it. It still runs on the
    /// Windows side (WSL starts Windows programs itself), so the port is the same.
    pub fn in_wsl(&self) -> McpBridge {
        McpBridge { cli: wsl_path(&self.cli), port: self.port }
    }
}

/// The agent runs inside WSL: its program is `wsl` or `wsl.exe` (any case, with or without a
/// folder). Such an agent can't use Windows paths, so the session hands it [`wsl_path`]s (#5546).
pub fn runs_in_wsl(argv: &[String]) -> bool {
    argv.first().is_some_and(|program| {
        let name = program.rsplit(['\\', '/']).next().unwrap_or(program);
        name.eq_ignore_ascii_case("wsl") || name.eq_ignore_ascii_case("wsl.exe")
    })
}

/// A Windows drive path as WSL shows it under its default mount root: `C:\Users\u` is
/// `/mnt/c/Users/u`. Anything else (a UNC share, a path already in that form) is kept.
pub fn wsl_path(path: &std::path::Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut chars = text.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(drive), Some(':'), Some('\\' | '/') | None) if drive.is_ascii_alphabetic() => {
            let rest: String = chars.map(|c| if c == '\\' { '/' } else { c }).collect();
            format!("/mnt/{}/{rest}", drive.to_ascii_lowercase()).into()
        }
        _ => path.to_path_buf(),
    }
}

/// What every session is told about where it is. An agent with no route for it in
/// [`session_new_params`] doesn't get it.
pub const SYSTEM_NOTE: &str = "You are running inside Intcad, a parametric 3D CAD application, as the user's assistant. \
The open design is reachable through the `intcad-app` MCP server: use its tools (list_commands, batch, execute, inspect_design, \
measure, screenshot, set_parameter, undo, skill_reference) to read and change the design the user is looking at, and verify with \
measure or inspect_design. Before building anything beyond one primitive, read `skill_reference` for `solidcraft-modeling` (its \
SKILL.md has the workflow, its reference.md the command shapes and this version's traps), work the numbers out first, and after \
each stage compare `measure` with them. Prefer one `batch` over many small calls. Saving, opening or exporting files waits for the user's approval in \
the chat, and each message allows a limited number of changes. Don't use shell or file tools: work through the Intcad tools. \
Keep answers short.";

/// The sentence of [`SYSTEM_NOTE`] that keeps an agent off its own shell and file tools.
pub const NO_FILE_TOOLS: &str = "Don't use shell or file tools: work through the Intcad tools.";

/// What the built-in agent is told in its place when it is handed a project (INT-1473): its file
/// tools reach the project folder, so they are for the project's files, and the design is still
/// built and checked only through Intcad's tools. It says nothing against the shell: the built-in
/// agent keeps it (asking before each command), decided on 2026-11-22.
pub const PROJECT_FILE_TOOLS: &str = "Use file tools only for the files of the project folder that the project brief names. \
Build, change, measure and check the design only through the Intcad tools.";

/// `note` ([`system_note`]) for a session handed a project folder: [`NO_FILE_TOOLS`] becomes
/// [`PROJECT_FILE_TOOLS`], the custom instructions after it stay as they are.
pub fn project_note(note: &str) -> String {
    note.replacen(NO_FILE_TOOLS, PROJECT_FILE_TOOLS, 1)
}

/// The most of the user's custom instructions (#8220) a session gets, in characters: the
/// Preferences field takes no more, and a longer text from an edited settings file is cut here.
pub const INSTRUCTIONS_LIMIT: usize = 4000;

/// The ACP system note for workflow-owned tasks. A task session gets no CAD bridge and no host
/// control token, so the only correct answer is a proposal for the host to validate.
pub const PROPOSAL_NOTE: &str = "You are running as a workflow task inside Intcad, a parametric 3D CAD application. \
This session has no CAD tools, no shell and no file access. Do not try to change any design. \
Answer with exactly one JSON object and no prose or code fence: \
{\"source\":{\"documentId\":\"<id>\",\"revision\":\"<revision>\"},\"commands\":[{\"command\":\"<native command>\",\"params\":{}}]}. \
Copy the source document id and revision from the task text. Give between 1 and 64 commands that a host can validate and apply. \
If the task cannot be expressed as commands, answer with an empty command list rather than guessing.";

/// [`SYSTEM_NOTE`] with the user's custom instructions (#8220) after it. Blank instructions leave
/// the note as it is.
pub fn system_note(instructions: &str) -> String {
    let text: String = instructions.trim().chars().take(INSTRUCTIONS_LIMIT).collect();
    let text = text.trim_end();
    if text.is_empty() {
        return SYSTEM_NOTE.to_string();
    }
    format!("{SYSTEM_NOTE}\n\nThe user's custom instructions, set in Intcad's preferences:\n{text}")
}

/// The Claude Code adapter's own tools that reach the computer rather than the design: turned off
/// for the session (`_meta.claudeCode.options.disallowedTools`), so it works through Intcad's.
pub const CLAUDE_DISALLOWED_TOOLS: &[&str] =
    &["Bash", "BashOutput", "KillShell", "Read", "Write", "Edit", "MultiEdit", "NotebookEdit", "Glob", "Grep", "WebFetch", "WebSearch", "Task"];

/// The `session/new` parameters. `init` is the agent's `initialize` reply: agents read the system
/// note from different places (#9811), and the standard field may only go to an agent offering it.
/// `note` is the system note ([`system_note`]). `project`, for the built-in agent only, is the
/// project folder the session may work in (INT-1473), sent as `_meta.intcad.project`. `cwd` stays
/// the app's folder; the built-in agent runs its MCP servers there and works in a folder of its own.
pub fn session_new_params(cwd: &std::path::Path, bridge: Option<&McpBridge>, init: &Value, note: &str, project: Option<&std::path::Path>) -> Value {
    let servers: Vec<Value> = bridge.map(McpBridge::to_json).into_iter().collect();
    let mut meta = json!({
        // The Claude adapter's own key, for its versions without the standard field.
        "systemPrompt": {"append": note},
        // Hosts may opt a session out of "bypass permissions" (the Claude adapter reads it):
        // every tool call the agent asks about reaches the panel.
        // Its own shell and file tools are off: the agent works through Intcad's tools.
        "claudeCode": {"options": {"allowDangerouslySkipPermissions": false, "disallowedTools": CLAUDE_DISALLOWED_TOOLS}},
    });
    if takes_antigravity_note(init)
        && let Some(meta) = meta.as_object_mut()
    {
        meta.insert("antigravity".into(), json!({"systemPrompt": {"append": note}}));
    }
    if let Some(project) = project
        && let Some(meta) = meta.as_object_mut()
    {
        meta.insert("intcad".into(), json!({"project": project.to_string_lossy()}));
    }
    let mut params = json!({
        "cwd": cwd.to_string_lossy(),
        "mcpServers": servers,
        "_meta": meta,
    });
    if takes_standard_note(init)
        && let Some(params) = params.as_object_mut()
    {
        params.insert("systemPrompt".into(), json!({"mode": "append", "content": [{"type": "text", "text": note}]}));
    }
    params
}

/// The agent offers ACP's `systemPrompt` field on its sessions.
fn takes_standard_note(init: &Value) -> bool {
    init.pointer("/agentCapabilities/sessionCapabilities/systemPrompt").is_some_and(|offer| !offer.is_null())
}

/// Antigravity offers its own key under `_meta` and doesn't read the Claude adapter's.
fn takes_antigravity_note(init: &Value) -> bool {
    init.pointer("/agentCapabilities/_meta/antigravity/systemPrompt/append").and_then(Value::as_bool).unwrap_or(false)
}

pub fn prompt_params(session: &str, parts: &[super::backend::PromptPart]) -> Value {
    json!({"sessionId": session, "prompt": super::backend::content_blocks(parts)})
}

/// The agent's modes in a `session/new` reply: the current one and each `(id, name)`.
pub fn modes(session: &Value) -> Option<(String, Vec<(String, String)>)> {
    let m = session.get("modes")?;
    let current = m.get("currentModeId").and_then(Value::as_str).unwrap_or("").to_string();
    let available: Vec<(String, String)> = m
        .get("availableModes")?
        .as_array()?
        .iter()
        .take(16)
        .filter_map(|x| {
            let id = x.get("id")?.as_str()?.to_string();
            let name = x.get("name").and_then(Value::as_str).unwrap_or(&id).to_string();
            Some((id, name))
        })
        .collect();
    (!available.is_empty()).then_some((current, available))
}

/// One model an agent offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// The agent's models in a `session/new` or `session/load` reply (`models`: `currentModelId` and
/// `availableModels: [{modelId, name, description?}]`): the current one and each model. `None`
/// when the agent reports none.
pub fn models(session: &Value) -> Option<(String, Vec<ModelInfo>)> {
    let m = session.get("models")?;
    let current = m.get("currentModelId").and_then(Value::as_str).unwrap_or("").to_string();
    let available: Vec<ModelInfo> = m
        .get("availableModels")?
        .as_array()?
        .iter()
        .take(200)
        .filter_map(|x| {
            let id = x.get("modelId").or_else(|| x.get("id"))?.as_str()?.to_string();
            let name = x.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(&id).to_string();
            let description = x.get("description").and_then(Value::as_str).unwrap_or("").to_string();
            Some(ModelInfo { id, name, description })
        })
        .collect();
    (!available.is_empty()).then_some((current, available))
}

/// `session/set_model` parameters.
pub fn set_model_params(session: &str, model: &str) -> Value {
    json!({"sessionId": session, "modelId": model})
}
pub fn set_config_option_params(session: &str, id: &str, value: &str) -> Value {
    json!({"sessionId": session, "configId": id, "value": value})
}

/// Whether the agent takes embedded documents in a message
/// (`agentCapabilities.promptCapabilities.embeddedContext`: it reads `resource` blocks).
pub fn takes_documents(init: &Value) -> bool {
    init.pointer("/agentCapabilities/promptCapabilities/embeddedContext").and_then(Value::as_bool).unwrap_or(false)
}

/// Whether the agent takes pictures in a message (`agentCapabilities.promptCapabilities.image`).
pub fn takes_images(init: &Value) -> bool {
    init.pointer("/agentCapabilities/promptCapabilities/image").and_then(Value::as_bool).unwrap_or(false)
}

/// One way to log in that the agent offers (`authMethods` in its `initialize` reply).
#[derive(Clone, Debug, PartialEq)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: AuthKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AuthKind {
    /// The agent signs in by itself when asked (`authenticate`), e.g. by opening a browser.
    Agent,
    /// The user runs the agent's own login in a terminal: the agent's command plus `args`.
    Terminal { args: Vec<String>, command: Option<(String, Vec<String>)> },
    /// The agent reads a key from an environment variable the user sets themselves.
    EnvVar { vars: Vec<String> },
}

pub fn auth_methods(init: &Value) -> Vec<AuthMethod> {
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array).map(|a| a.iter().take(64).filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    init.get("authMethods")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(32)
                .filter_map(|m| {
                    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let id = s("id");
                    if id.is_empty() {
                        return None;
                    }
                    let kind = match m.get("type").and_then(Value::as_str) {
                        Some("terminal") => {
                            let meta = m.get("_meta").and_then(|x| x.get("terminal-auth"));
                            let mut command = meta.and_then(|t| Some((t.get("command")?.as_str()?.to_string(), strings(t.get("args")))));
                            let mut args = strings(m.get("args"));
                            // Vibe's Python launcher can advertise its own path as an argument.
                            // The configured ACP command already starts Vibe and accepts --setup.
                            if id == "vibe-setup" && init.pointer("/agentInfo/name").and_then(Value::as_str) == Some("@mistralai/mistral-vibe") {
                                args = vec!["--setup".into()];
                                command = None;
                            }
                            AuthKind::Terminal { args, command }
                        }
                        Some("env_var") => AuthKind::EnvVar {
                            vars: m
                                .get("vars")
                                .and_then(Value::as_array)
                                .map(|v| v.iter().take(16).filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_string)).collect())
                                .unwrap_or_default(),
                        },
                        // A key the agent would take from the client: Intcad never handles
                        // keys, so this becomes "set it in your environment" instead.
                        _ if m.get("_meta").and_then(|x| x.get("api-key")).is_some() => {
                            let provider = m
                                .get("_meta")
                                .and_then(|x| x.get("api-key"))
                                .and_then(|k| k.get("provider"))
                                .and_then(Value::as_str)
                                .unwrap_or("agent");
                            AuthKind::EnvVar { vars: vec![format!("{}_API_KEY", provider.to_ascii_uppercase())] }
                        }
                        _ => AuthKind::Agent,
                    };
                    Some(AuthMethod { id, name: s("name"), description: s("description"), kind })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The agent's name and version from its `initialize` reply.
pub fn agent_info(init: &Value) -> String {
    let i = init.get("agentInfo");
    let s = |k: &str| i.and_then(|i| i.get(k)).and_then(Value::as_str);
    match (s("title").or(s("name")), s("version")) {
        (Some(n), Some(v)) => format!("{n} {v}"),
        (Some(n), None) => n.to_string(),
        _ => String::new(),
    }
}

/// A tool call's state as the agent reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl ToolStatus {
    pub fn parse(s: &str) -> Option<ToolStatus> {
        Some(match s {
            "pending" => ToolStatus::Pending,
            "in_progress" => ToolStatus::InProgress,
            "completed" => ToolStatus::Completed,
            "failed" => ToolStatus::Failed,
            _ => return None,
        })
    }
}

/// One choice of a permission request.
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}

/// What a message from the agent means for the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// The reply to one of our requests.
    Reply {
        id: u64,
        result: Result<Value, (i64, String)>,
    },
    /// Text the agent streams (`thought`: its reasoning, shown dimmed).
    Text {
        /// The upstream ACP session the update belongs to; empty when the peer omitted it.
        session: String,
        thought: bool,
        text: String,
    },
    /// A new tool call, or news about one (`title`/`status` when they changed).
    Tool {
        session: String,
        id: String,
        title: Option<String>,
        kind: Option<String>,
        status: Option<ToolStatus>,
        input: Option<Value>,
        output: Option<String>,
        /// A picture the call handed back, as the encoded bytes of the file ([`tool_media`]).
        picture: Option<Vec<u8>>,
    },
    /// The agent's plan: each entry with its status.
    Plan {
        session: String,
        steps: Vec<(String, String)>,
    },
    /// The agent asks before running a tool call; answered with [`permission_reply`].
    Permission {
        session: String,
        rpc: Value,
        title: String,
        /// The tool call's kind (`execute`, `edit`, `other`, …), as the agent gives it.
        kind: Option<String>,
        options: Vec<PermissionOption>,
    },
    /// A request of ours was refused, and the error says why in the built-in agent's own terms
    /// (`data.intcad`): a turn that failed for its connection. Only [`parse_builtin`] reads it.
    Refused {
        id: u64,
        error: (i64, String),
        problem: super::connections::TurnProblem,
    },
    /// The built-in agent's connections changed: its snapshot (`_intcad/connections/changed`).
    /// Only [`parse_builtin`] reads it.
    Connections(super::connections::Snapshot),
    /// The agent says whether it is logged in (an extension both adapters send).
    AuthStatus(String),
    /// The agent switched to another of its modes.
    ModeChanged(String),
    /// The agent switched to another model.
    ModelChanged(String),
    ConfigOptions {
        session: String,
        options: Vec<ConfigOption>,
    },
    /// The agent's slash commands for its session, replacing the previous list whole.
    Commands {
        session: String,
        commands: Vec<AvailableCommand>,
    },
    /// A request we don't serve (file system, terminal, …): answer "method not found".
    Unsupported {
        rpc: Value,
        method: String,
    },
    /// Anything else (usage, unknown updates): nothing to show.
    Ignored,
}

/// The longest text kept from one tool call's output.
const MAX_OUTPUT: usize = 2000;

/// The longest command name kept: the page's send limit, so a selected name is always sendable.
const MAX_COMMAND_NAME: usize = MAX_MESSAGE;
/// The longest serialized command name kept: the page action's byte budget, with room for its
/// JSON envelope (`{"type":"send","text":"…"}`).
const MAX_COMMAND_NAME_BYTES: usize = MAX_MESSAGE_BYTES.saturating_sub(64);

/// Whether a command name fits both budgets the page enforces: characters and the serialized
/// bytes of the action that would send it. A name of four-byte code points passes a character
/// cap but can still exceed the byte cap.
fn command_name_fits(name: &str) -> bool {
    name.chars().count() <= MAX_COMMAND_NAME && serde_json::to_string(name).is_ok_and(|quoted| quoted.len() <= MAX_COMMAND_NAME_BYTES)
}

fn content_text(c: &Value) -> Option<String> {
    // A content block, or a tool call content item wrapping one (`{type: "content", content}`).
    let block = if c.get("type").and_then(Value::as_str) == Some("content") { c.get("content")? } else { c };
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block.get("text").and_then(Value::as_str).map(str::to_string),
        Some("image") => Some("[image]".into()),
        Some("resource_link") => block.get("uri").and_then(Value::as_str).map(|u| format!("[{u}]")),
        Some("diff") => block.get("path").and_then(Value::as_str).map(|p| format!("[diff {p}]")),
        Some("terminal") => Some("[terminal]".into()),
        _ => None,
    }
}

/// The longest `rawInput` given as text that is read as JSON: a tool's arguments, not a document.
const MAX_RAW_INPUT: usize = 256 * 1024;

/// ACP's `rawInput`: the arguments the tool was called with. They are an object, which some agents
/// (Mastra Code's) send as its JSON text.
fn tool_input(u: &Value) -> Option<Value> {
    match u.get("rawInput")? {
        given if given.is_object() => Some(given.clone()),
        Value::String(text) if text.len() <= MAX_RAW_INPUT => serde_json::from_str::<Value>(text).ok().filter(Value::is_object),
        _ => None,
    }
}

/// What a tool call says it returned: its `content`, or where an agent sends none of that (Mastra
/// Code's) its `rawOutput`, a text as it is and anything else as its JSON.
fn tool_output(u: &Value) -> Option<String> {
    let parts: Vec<String> =
        u.get("content").and_then(Value::as_array).map(|items| items.iter().take(16).filter_map(content_text).collect()).unwrap_or_default();
    let mut s = if parts.is_empty() {
        match u.get("rawOutput")? {
            Value::Null => return None,
            Value::String(text) if text.is_empty() => return None,
            Value::String(text) => text.clone(),
            other => serde_json::to_string(other).ok()?,
        }
    } else {
        parts.join("\n")
    };
    clip_output(&mut s);
    Some(s)
}

fn clip_output(s: &mut String) {
    if s.len() > MAX_OUTPUT {
        let mut cut = MAX_OUTPUT;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
}

/// The longest result read as a file handed back as media: Mastra's own cap for such a file
/// (10 MiB) in base64, and the little that stands around it.
const MAX_MEDIA_TEXT: usize = 14 * 1024 * 1024 + 4096;

/// A file a tool handed back as media, where the agent sends it as Mastra Code does: `rawOutput`
/// is the JSON text of `{"__workspaceMedia": true, "text": "<path> (<n> bytes, <type>)",
/// "mediaType", "data": <base64>}`. Gives the line that says what it is, and the file's bytes
/// when it is a picture. Read as any other result it is two thousand characters of base64.
///
/// A picture an MCP tool returns comes the same way, as the JSON text of the tool's result:
/// `{"content": [{"type": "image", "data": <base64>, "mimeType"}, {"type": "text", "text"}]}`
/// (the app's own `screenshot` and `compare_picture`). Its line names the picture as the other
/// does, `image.png (<n> bytes, image/png)`, and what the tool said in text follows it.
fn tool_media(u: &Value) -> Option<(String, Option<Vec<u8>>)> {
    let text = u.get("rawOutput")?.as_str().filter(|text| text.len() <= MAX_MEDIA_TEXT && text.starts_with('{'))?;
    if !text.contains("\"__workspaceMedia\"") {
        return text.contains("\"image\"").then(|| mcp_picture(text)).flatten();
    }
    let media: Value = serde_json::from_str(text).ok()?;
    if media.get("__workspaceMedia") != Some(&Value::Bool(true)) {
        return None;
    }
    let mut line = media.get("text")?.as_str()?.to_string();
    clip_output(&mut line);
    let picture = media.get("mediaType").and_then(Value::as_str).is_some_and(|kind| kind.starts_with("image/"));
    let bytes = if picture { media.get("data").and_then(Value::as_str).and_then(|data| super::backend::base64_decode(data).ok()) } else { None };
    Some((line, bytes))
}

/// The first picture in an MCP tool's result given as JSON text, and the result's text. `None`
/// when it holds no picture that decodes: it is then read as any other result. The block's type
/// is its `mimeType` (MCP's name) or `mediaType`; where an agent sends neither, the bytes say.
fn mcp_picture(text: &str) -> Option<(String, Option<Vec<u8>>)> {
    let result: Value = serde_json::from_str(text).ok()?;
    let blocks = result.get("content")?.as_array()?;
    let kind = |block: &Value| block.get("type").and_then(Value::as_str).map(str::to_string);
    let (bytes, mime) = blocks.iter().filter(|block| kind(block).as_deref() == Some("image")).find_map(|block| {
        let bytes = super::backend::base64_decode(block.get("data")?.as_str()?).ok()?;
        let named = block.get("mimeType").or_else(|| block.get("mediaType")).and_then(Value::as_str);
        let mime = named.or(match bytes.as_slice() {
            [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
            [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
            _ => None,
        })?;
        mime.starts_with("image/").then(|| (bytes, mime.to_string()))
    })?;
    let mut line = format!("image.{} ({} bytes, {mime})", mime.trim_start_matches("image/"), bytes.len());
    for said in blocks.iter().filter(|block| kind(block).as_deref() == Some("text")).filter_map(|block| block.get("text")?.as_str()) {
        line.push('\n');
        line.push_str(said);
    }
    clip_output(&mut line);
    Some((line, Some(bytes)))
}

/// Read one line from the agent.
pub fn parse(line: &str) -> Option<Incoming> {
    read(line, false)
}

/// Read one line from the built-in agent: as [`parse`], and its own additions to the protocol
/// too (why a request failed, in [`Incoming::Refused`], and [`Incoming::Connections`]). No other
/// agent is read this way, so nothing another agent sends is taken for either.
pub fn parse_builtin(line: &str) -> Option<Incoming> {
    read(line, true)
}

fn read(line: &str, builtin: bool) -> Option<Incoming> {
    let msg: Value = serde_json::from_str(line.trim()).ok()?;
    let method = msg.get("method").and_then(Value::as_str);
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match (method, msg.get("id")) {
        (None, Some(id)) => {
            let id = id.as_u64()?;
            let result = match msg.get("error") {
                Some(e) => {
                    let error =
                        (e.get("code").and_then(Value::as_i64).unwrap_or(0), e.get("message").and_then(Value::as_str).unwrap_or("error").to_string());
                    if builtin && let Some(problem) = super::connections::TurnProblem::parse(e.get("data")) {
                        return Some(Incoming::Refused { id, error, problem });
                    }
                    Err(error)
                }
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            Some(Incoming::Reply { id, result })
        }
        (Some("session/request_permission"), Some(id)) => {
            let call = params.get("toolCall");
            let title = call.and_then(|c| c.get("title")).and_then(Value::as_str).unwrap_or("").to_string();
            let kind = call.and_then(|c| c.get("kind")).and_then(Value::as_str).map(str::to_string);
            let options = params
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .take(8)
                        .filter_map(|o| {
                            Some(PermissionOption {
                                id: o.get("optionId")?.as_str()?.to_string(),
                                name: o.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                                kind: o.get("kind").and_then(Value::as_str).unwrap_or("").to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Incoming::Permission {
                session: params.get("sessionId").and_then(Value::as_str).unwrap_or("").to_string(),
                rpc: id.clone(),
                title,
                kind,
                options,
            })
        }
        (Some(m), Some(id)) => Some(Incoming::Unsupported { rpc: id.clone(), method: m.to_string() }),
        (Some("session/update"), None) => Some(update(params.get("update")?, params.get("sessionId").and_then(Value::as_str).unwrap_or(""))),
        (Some(super::connections::CHANGED), None) if builtin => {
            Some(super::connections::Snapshot::parse(&params).map_or(Incoming::Ignored, Incoming::Connections))
        }
        (Some("_auth/status_update"), None) => {
            params.get("authStatus").and_then(|s| s.get("label")).and_then(Value::as_str).map(|l| Incoming::AuthStatus(l.to_string()))
        }
        _ => Some(Incoming::Ignored),
    }
}

fn update(u: &Value, session: &str) -> Incoming {
    let s = |k: &str| u.get(k).and_then(Value::as_str).map(str::to_string);
    match u.get("sessionUpdate").and_then(Value::as_str) {
        Some(kind @ ("agent_message_chunk" | "agent_thought_chunk")) => match u.get("content").and_then(content_text) {
            Some(text) => Incoming::Text { session: session.to_string(), thought: kind == "agent_thought_chunk", text },
            None => Incoming::Ignored,
        },
        Some("tool_call" | "tool_call_update") => match s("toolCallId") {
            Some(id) => {
                let (line, picture) = tool_media(u).map_or((None, None), |(line, picture)| (Some(line), picture));
                Incoming::Tool {
                    session: session.to_string(),
                    id,
                    title: s("title"),
                    kind: s("kind"),
                    status: u.get("status").and_then(Value::as_str).and_then(ToolStatus::parse),
                    input: tool_input(u),
                    output: line.or_else(|| tool_output(u)),
                    picture,
                }
            }
            None => Incoming::Ignored,
        },
        Some("current_mode_update") => match s("currentModeId").or_else(|| s("modeId")) {
            Some(m) => Incoming::ModeChanged(m),
            None => Incoming::Ignored,
        },
        Some("current_model_update") => match s("currentModelId").or_else(|| s("modelId")) {
            Some(m) => Incoming::ModelChanged(m),
            None => Incoming::Ignored,
        },
        Some("config_option_update") => match config_options(u) {
            ConfigParse::Snapshot(options) if !session.is_empty() => Incoming::ConfigOptions { session: session.into(), options },
            _ => Incoming::Ignored,
        },
        Some("available_commands_update") if !session.is_empty() => {
            let Some(list) = u.get("availableCommands").and_then(Value::as_array) else {
                return Incoming::Ignored;
            };
            let commands = list
                .iter()
                .take(256)
                .filter_map(|c| {
                    let name = c.get("name")?.as_str()?.trim();
                    (!name.is_empty() && command_name_fits(name)).then(|| AvailableCommand {
                        name: name.to_string(),
                        description: c.get("description").and_then(Value::as_str).unwrap_or("").chars().take(1024).collect(),
                    })
                })
                .collect();
            Incoming::Commands { session: session.into(), commands }
        }
        Some("plan") => Incoming::Plan {
            session: session.to_string(),
            steps: u
                .get("entries")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .take(64)
                        .map(|e| {
                            let g = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                            (g("content"), g("status"))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        _ => Incoming::Ignored,
    }
}

/// The answer to a permission request: the option picked, or `None` for "cancelled".
pub fn permission_reply(rpc: Value, option: Option<&str>) -> Value {
    let outcome = match option {
        Some(o) => json!({"outcome": "selected", "optionId": o}),
        None => json!({"outcome": "cancelled"}),
    };
    reply(rpc, json!({"outcome": outcome}))
}

/// Split a command line into the program and its arguments: whitespace separates, double or
/// single quotes group, a backslash before a quote keeps it. No shell runs it.
pub fn split_command(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\') if matches!(chars.peek(), Some('"') | Some('\'')) => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
                started = true;
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (Some(q), _) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            _ => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_taken_only_when_the_agent_says_so() {
        let yes = json!({"agentCapabilities": {"promptCapabilities": {"image": true, "embeddedContext": true}}});
        let no = json!({"agentCapabilities": {"promptCapabilities": {"image": true}}});
        assert!(takes_documents(&yes) && takes_images(&yes));
        assert!(!takes_documents(&no) && !takes_documents(&json!({})));
    }

    #[test]
    fn commands_split_like_a_shell_without_one() {
        assert_eq!(split_command("gemini --acp"), ["gemini", "--acp"]);
        assert_eq!(split_command(r#"  "C:\Program Files\x\agent.exe"  --flag 'a b' "#), [r"C:\Program Files\x\agent.exe", "--flag", "a b"]);
        assert_eq!(split_command(r#"a "" b"#), ["a", "", "b"]);
        assert!(split_command("   ").is_empty());
    }

    /// #2592: the bridge is not called `solidcraft`, the name a user's own MCP entry takes from
    /// docs/mcp.md, and the note tells the agent the name it really has.
    #[test]
    fn the_bridge_has_a_name_of_its_own_and_the_note_uses_it() {
        assert_ne!(BRIDGE_NAME, "solidcraft");
        // Nor `intcad` or `intgral`: a user may have a server of either name of their own.
        assert_ne!(BRIDGE_NAME, "intcad");
        assert_ne!(BRIDGE_NAME, "intgral");
        assert!(SYSTEM_NOTE.contains(&format!("`{BRIDGE_NAME}` MCP server")), "{SYSTEM_NOTE}");
        assert!(!SYSTEM_NOTE.contains("`solidcraft` MCP server"));
        // The panel still recognises the bridge's tools as Intcad's (agents name them after
        // their server, e.g. `mcp__intcad-app__batch`).
        assert!(BRIDGE_NAME.contains("intcad"));
        assert!(is_bridge_tool(&format!("mcp__{BRIDGE_NAME}__batch"), Some("other")));
    }

    /// Only the whole name of a tool of the chat's own server counts, whatever else a title holds.
    #[test]
    fn only_the_bridges_exact_tool_names_are_its_tools() {
        for tool in ["batch", "inspect_design", "set_parameter"] {
            assert!(is_bridge_tool(&format!("mcp__intcad-app__{tool}"), None), "{tool}");
            assert!(is_bridge_tool(&format!("intcad-app_{tool}"), None), "Mastra Code's name for {tool}");
            assert!(is_bridge_tool(&format!("intcad-app_{tool}"), Some("other")), "Mastra Code's name for {tool}");
            assert!(is_bridge_tool(&format!("mcp__intcad-app__{tool}"), Some("other")), "{tool}");
        }
        for title in [
            "",
            "solidcraft",
            "Intcad",
            "run intcad-cli; ls",
            "intcad-cli --version",
            "Edit ~/.local/share/intcad/agent-chat/work/notes.txt",
            // A user's own MCP server (docs/mcp.md), which Intcad's gate doesn't see.
            "mcp__intcad__batch",
            "mcp__intcad-live__batch",
            "mcp__other__intcad-app",
            "intcad-app_",
            "intcad-app_execute; ls",
            "intcad-app_execute && ls",
            "other_intcad-app_execute",
            "mcp__intcad-app__",
            "mcp__intcad-app",
            "intcad-app__batch",
            "intcad-app: batch",
            "Tool: intcad-app/batch",
            "mcp.intcad-app.batch",
            " mcp__intcad-app__batch",
            "mcp__intcad-app__batch ",
            "mcp__intcad-app__batch; ls",
            "mcp__intcad-app__batch && ls",
            "mcp__intcad-app__batch\nls",
            "mcp__intcad-app__../../tool",
            "mcp__INTCAD-APP__batch",
            "MCP__intcad-app__batch",
            "mcp__intcad-app__Batch",
        ] {
            assert!(!is_bridge_tool(title, None), "{title:?}");
        }
        for kind in ["execute", "edit", "delete", "move", "read", "search", "fetch", "think", ""] {
            assert!(!is_bridge_tool("mcp__intcad-app__batch", Some(kind)), "{kind:?}");
        }
    }

    #[test]
    fn the_session_gets_the_bridged_mcp_server_without_a_token_or_any_path() {
        let b = McpBridge { cli: "/opt/solidcraft/intcad-cli".into(), port: 41234 };
        let p = session_new_params(std::path::Path::new("/home/u/designs"), Some(&b), &json!({}), SYSTEM_NOTE, None);
        let s = &p["mcpServers"][0];
        assert_eq!(s["name"], BRIDGE_NAME);
        assert_eq!(s["args"], json!(["mcp", "--connect", "127.0.0.1:41234"]));
        let text = p.to_string();
        assert!(!text.contains("allow-any-path") && !text.contains("TOKEN") && !text.contains("token"), "{text}");
        // The server is told the folder of the app's token files, not left to the agent's
        // environment for it: Codex starts its servers with a clean one, and the app's tools
        // were all refused as unauthorized when the app had been given a folder of its own.
        let folder =
            solidcraft_engine::control_auth::token_path(41234).and_then(|file| file.parent().map(|folder| folder.to_string_lossy().into_owned()));
        assert!(folder.is_some(), "the app has a folder for its token files");
        assert_eq!(s["env"], json!([{"name": "SOLIDCRAFT_CONTROL_DIR", "value": folder}]));
        assert!(p["_meta"]["claudeCode"]["options"]["disallowedTools"].as_array().is_some_and(|t| t.iter().any(|x| x == "Bash")));
        assert_eq!(session_new_params(std::path::Path::new("/tmp"), None, &json!({}), SYSTEM_NOTE, None)["mcpServers"], json!([]));
    }

    /// #9811: the note goes by the routes an agent's `initialize` reply offers, and the standard
    /// field never goes to an agent that doesn't offer it, as the protocol asks.
    #[test]
    fn the_system_note_takes_every_route_the_agent_offers() {
        // With the user's custom instructions (#8220), which go wherever the note goes.
        let note = system_note("Sketch on XZ with explicit plane vectors.");
        let params = |init: Value| session_new_params(std::path::Path::new("/tmp"), None, &init, &note, None);
        let standard = json!({"mode": "append", "content": [{"type": "text", "text": note}]});

        let offers_standard = params(json!({"agentCapabilities": {"sessionCapabilities": {"systemPrompt": {}}}}));
        assert_eq!(offers_standard["systemPrompt"], standard);
        assert!(offers_standard["_meta"].get("antigravity").is_none());

        let offers_antigravity = params(json!({"agentCapabilities": {"_meta": {"antigravity": {"systemPrompt": {"append": true}}}}}));
        assert_eq!(offers_antigravity["_meta"]["antigravity"]["systemPrompt"]["append"], note);
        assert!(offers_antigravity.get("systemPrompt").is_none());

        for none in [
            json!({}),
            json!({"agentCapabilities": {"sessionCapabilities": {"systemPrompt": null}}}),
            json!({"agentCapabilities": {"_meta": {"antigravity": {"systemPrompt": {"append": false}}}}}),
        ] {
            let p = params(none);
            assert!(p.get("systemPrompt").is_none(), "{p}");
            assert!(p["_meta"].get("antigravity").is_none(), "{p}");
        }

        // Every session keeps the key older Claude adapters read, and the tools they turn off.
        for init in [json!({}), json!({"agentCapabilities": {"sessionCapabilities": {"systemPrompt": {}}}})] {
            let p = params(init);
            assert_eq!(p["_meta"]["systemPrompt"]["append"], note);
            assert!(p["_meta"]["claudeCode"]["options"]["disallowedTools"].is_array());
        }
    }

    /// #8220: the user's custom instructions follow the note, trimmed and cut at the limit; blank
    /// ones leave the note as it is.
    #[test]
    fn custom_instructions_follow_the_note_trimmed_and_capped() {
        assert_eq!(system_note(""), SYSTEM_NOTE);
        assert_eq!(system_note(" \n\t "), SYSTEM_NOTE);
        let note = system_note("\n  Use M3 screws.\nPrint flat side down.  \n");
        assert!(note.starts_with(SYSTEM_NOTE), "{note}");
        assert!(note.ends_with("preferences:\nUse M3 screws.\nPrint flat side down."), "{note}");
        let note = system_note(&"ż".repeat(INSTRUCTIONS_LIMIT + 10));
        assert_eq!(note.chars().filter(|c| *c == 'ż').count(), INSTRUCTIONS_LIMIT);
    }

    /// INT-1473: a session handed a project names its folder under `_meta.intcad` and keeps its
    /// own folder as `cwd`; its note lets file tools reach the project and still keeps the shell
    /// and the design's tools as they were. A session without one gets neither.
    #[test]
    fn a_session_handed_a_project_names_it_and_its_note_says_what_file_tools_are_for() {
        let work = std::path::Path::new("/data/agent-chat/work");
        let project = std::path::Path::new("/home/u/bracket");
        let p = session_new_params(work, None, &json!({}), SYSTEM_NOTE, Some(project));
        assert_eq!(p["_meta"]["intcad"]["project"], "/home/u/bracket");
        assert_eq!(p["cwd"], "/data/agent-chat/work");
        assert!(session_new_params(work, None, &json!({}), SYSTEM_NOTE, None)["_meta"].get("intcad").is_none());

        assert!(SYSTEM_NOTE.contains(NO_FILE_TOOLS), "the sentence the project note replaces is in the note");
        let note = project_note(&system_note("Use M3 screws."));
        assert!(!note.contains(NO_FILE_TOOLS) && note.contains(PROJECT_FILE_TOOLS), "{note}");
        assert!(
            note.contains("only through the Intcad tools") && !note.to_lowercase().contains("shell"),
            "the shell is neither offered nor forbidden: {note}"
        );
        assert!(note.ends_with("preferences:\nUse M3 screws."), "the custom instructions stay last: {note}");
        assert_eq!(project_note(PROPOSAL_NOTE), PROPOSAL_NOTE, "a note without the sentence is left alone");
    }

    /// #5546: an agent started through `wsl.exe` gets the session's paths as WSL shows them.
    #[test]
    fn an_agent_inside_wsl_gets_wsl_paths() {
        let argv = |line: &str| split_command(line);
        assert!(runs_in_wsl(&argv("wsl.exe -d Ubuntu -e bash -lc claude-agent-acp")));
        assert!(runs_in_wsl(&argv("WSL claude-agent-acp")));
        assert!(runs_in_wsl(&argv(r#""C:\Windows\System32\wsl.exe" -e claude-agent-acp"#)));
        assert!(!runs_in_wsl(&argv("claude-agent-acp")));
        assert!(!runs_in_wsl(&argv("wslconfig /l")));
        assert!(!runs_in_wsl(&[]));

        let wsl = |p: &str| wsl_path(std::path::Path::new(p)).to_string_lossy().into_owned();
        assert_eq!(wsl(r"C:\Users\u\AppData\Local\Intcad\agent-chat\work"), "/mnt/c/Users/u/AppData/Local/Intcad/agent-chat/work");
        assert_eq!(wsl(r"\\?\D:\Program Files\Intcad\intcad-cli.exe"), "/mnt/d/Program Files/Intcad/intcad-cli.exe");
        assert_eq!(wsl("E:/designs"), "/mnt/e/designs");
        assert_eq!(wsl("C:"), "/mnt/c/");
        assert_eq!(wsl("/home/u/designs"), "/home/u/designs");
        assert_eq!(wsl(r"\\server\share\x"), r"\\server\share\x");

        let b = McpBridge { cli: r"C:\Program Files\Intcad\intcad-cli.exe".into(), port: 41234 };
        let p = session_new_params(&wsl_path(std::path::Path::new(r"C:\Users\u\work")), Some(&b.in_wsl()), &json!({}), SYSTEM_NOTE, None);
        assert_eq!(p["cwd"], "/mnt/c/Users/u/work");
        assert_eq!(p["mcpServers"][0]["command"], "/mnt/c/Program Files/Intcad/intcad-cli.exe");
        assert_eq!(p["mcpServers"][0]["args"], json!(["mcp", "--connect", "127.0.0.1:41234"]));
    }

    #[test]
    fn vibe_terminal_setup_uses_the_configured_acp_launcher() {
        let mut init = json!({
            "agentInfo": {"name": "@mistralai/mistral-vibe"},
            "authMethods": [{
                "id": "vibe-setup", "type": "terminal", "name": "Register your API Key",
                "args": [r"C:\Program Files\Vibe\vibe-acp", "--setup"],
                "_meta": {"terminal-auth": {"command": "python.exe", "args": [r"C:\Program Files\Vibe\vibe-acp", "--setup"]}}
            }]
        });
        let methods = auth_methods(&init);
        assert_eq!(methods[0].kind, AuthKind::Terminal { args: vec!["--setup".into()], command: None });

        init["agentInfo"]["name"] = json!("another-agent");
        let methods = auth_methods(&init);
        let AuthKind::Terminal { args, command } = &methods[0].kind else { panic!("terminal method expected") };
        assert_eq!(args, &[r"C:\Program Files\Vibe\vibe-acp", "--setup"]);
        assert!(command.is_some());
    }

    #[test]
    fn auth_methods_of_both_adapters_are_read() {
        // As the Claude adapter offers them to a client with terminal login.
        let claude = json!({"authMethods": [{"description": "Use Claude subscription ", "name": "Claude Subscription", "id": "claude-ai-login",
            "type": "terminal", "args": ["--cli", "auth", "login", "--claudeai"],
            "_meta": {"terminal-auth": {"command": "/usr/bin/node", "args": ["/x/dist/index.js", "--cli", "auth", "login", "--claudeai"], "label": "Claude Login"}}}]});
        let m = auth_methods(&claude);
        assert_eq!(m.len(), 1);
        let AuthKind::Terminal { args, command } = &m[0].kind else { panic!("{m:?}") };
        assert_eq!(args.last().map(String::as_str), Some("--claudeai"));
        assert_eq!(command.as_ref().map(|c| c.0.as_str()), Some("/usr/bin/node"));
        // As the Codex adapter offers them.
        let codex = json!({"authMethods": [{"id": "chatgpt", "name": "Login with ChatGPT"},
            {"type": "env_var", "id": "openai-api-key", "name": "Use OPENAI_API_KEY", "vars": [{"name": "OPENAI_API_KEY"}]}]});
        let m = auth_methods(&codex);
        assert_eq!(m[0].kind, AuthKind::Agent);
        // Codex 2.x offers its key login as a method the client would feed: shown as a variable.
        let codex2 = json!({"authMethods": [{"id": "api-key", "name": "API Key", "_meta": {"api-key": {"provider": "openai"}}}]});
        assert_eq!(auth_methods(&codex2)[0].kind, AuthKind::EnvVar { vars: vec!["OPENAI_API_KEY".into()] });
        assert_eq!(m[1].kind, AuthKind::EnvVar { vars: vec!["OPENAI_API_KEY".into()] });
    }

    #[test]
    fn updates_permissions_and_replies_are_told_apart() {
        let text = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hi"}}}}"#;
        assert_eq!(parse(text), Some(Incoming::Text { session: "s".into(), thought: false, text: "Hi".into() }));
        let tool = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"ok"}}]}}}"#;
        assert_eq!(
            parse(tool),
            Some(Incoming::Tool {
                session: "s".into(),
                id: "t1".into(),
                title: None,
                kind: None,
                status: Some(ToolStatus::Completed),
                input: None,
                output: Some("ok".into()),
                picture: None,
            })
        );
        let perm = r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"sessionId":"s","toolCall":{"toolCallId":"t1","title":"batch"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"},{"optionId":"reject","name":"Reject","kind":"reject_once"}]}}"#;
        let Some(Incoming::Permission { session, rpc, title, kind, options }) = parse(perm) else { panic!() };
        assert_eq!((session.as_str(), title.as_str(), options.len(), options[1].kind.as_str()), ("s", "batch", 2, "reject_once"));
        assert_eq!(kind, None);
        assert_eq!(permission_reply(rpc.clone(), Some("allow"))["result"]["outcome"], json!({"outcome": "selected", "optionId": "allow"}));
        assert_eq!(permission_reply(rpc, None)["result"]["outcome"]["outcome"], "cancelled");
        let fs = r#"{"jsonrpc":"2.0","id":8,"method":"fs/read_text_file","params":{}}"#;
        assert!(matches!(parse(fs), Some(Incoming::Unsupported { .. })));
        let err = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32000,"message":"Authentication required"}}"#;
        assert_eq!(parse(err), Some(Incoming::Reply { id: 3, result: Err((AUTH_REQUIRED, "Authentication required".into())) }));
        assert_eq!(parse("not json"), None);
        let mode = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"current_mode_update","currentModeId":"plan"}}}"#;
        assert_eq!(parse(mode), Some(Incoming::ModeChanged("plan".into())));
        let new = json!({"sessionId": "s", "modes": {"currentModeId": "default", "availableModes": [{"id": "default", "name": "Default"}, {"id": "acceptEdits", "name": "Accept Edits"}]}});
        assert_eq!(modes(&new), Some(("default".into(), vec![("default".into(), "Default".into()), ("acceptEdits".into(), "Accept Edits".into())])));
    }

    /// Mastra Code's agent sends a tool's arguments as JSON text and its result as `rawOutput`,
    /// with no `content`: the row's details were empty for every one of its tools.
    #[test]
    fn a_tool_call_whose_arguments_are_json_text_and_whose_result_is_raw_output_keeps_both() {
        let started = json!({"sessionUpdate": "tool_call", "toolCallId": "t", "title": "execute_command", "status": "in_progress", "rawInput": "{\"command\":\"ls\"}"});
        let Incoming::Tool { input, output, .. } = update(&started, "s") else { panic!() };
        assert_eq!((input, output), (Some(json!({"command": "ls"})), None));
        let ended = json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "status": "completed", "rawOutput": "a.txt\nb.txt"});
        let Incoming::Tool { input, output, .. } = update(&ended, "s") else { panic!() };
        assert_eq!((input, output.as_deref()), (None, Some("a.txt\nb.txt")));
        // A result that is not text is told as its JSON; what the tool shows as content comes first.
        let object = json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "rawOutput": {"ok": true}});
        let Incoming::Tool { output, .. } = update(&object, "s") else { panic!() };
        assert_eq!(output.as_deref(), Some(r#"{"ok":true}"#));
        let both = json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "rawOutput": "raw", "content": [{"type": "content", "content": {"type": "text", "text": "shown"}}]});
        let Incoming::Tool { output, .. } = update(&both, "s") else { panic!() };
        assert_eq!(output.as_deref(), Some("shown"));
        // Arguments that are not an object, text that is not JSON, and an empty result are nothing.
        for raw in [json!("not json"), json!("[1,2]"), json!(7), json!("x".repeat(MAX_RAW_INPUT + 1))] {
            let Incoming::Tool { input, .. } = update(&json!({"sessionUpdate": "tool_call", "toolCallId": "t", "rawInput": raw}), "s") else {
                panic!()
            };
            assert_eq!(input, None);
        }
        for raw in [json!(""), Value::Null] {
            let Incoming::Tool { output, .. } = update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "rawOutput": raw}), "s")
            else {
                panic!()
            };
            assert_eq!(output, None);
        }
        let long = json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "rawOutput": "é".repeat(3000)});
        let Incoming::Tool { output: Some(cut), .. } = update(&long, "s") else { panic!() };
        assert!(cut.len() <= MAX_OUTPUT + 3 && cut.ends_with('…'));
    }

    /// Mastra Code hands back a file it read as media: the JSON text of an object with the file's
    /// bytes in base64. It is a line and a picture, never two thousand characters of base64.
    /// The app's own screenshot comes back as an MCP result with the picture in it: it is a line
    /// that names the picture, what the tool said, and the picture, never base64 as the result.
    #[test]
    fn a_picture_in_an_mcp_result_is_a_line_and_a_picture() {
        let png = b"\x89PNG\r\n\x1a\nthe rest";
        let base64 = super::super::backend::base64(png);
        let returned = |result: Value| {
            let Incoming::Tool { output, picture, .. } =
                update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "rawOutput": result.to_string()}), "s")
            else {
                panic!()
            };
            (output, picture)
        };
        let shot = json!({"content": [{"type": "image", "data": base64, "mimeType": "image/png"}, {"type": "text", "text": "{\"width\":8}"}]});
        assert_eq!(returned(shot), (Some("image.png (16 bytes, image/png)\n{\"width\":8}".into()), Some(png.to_vec())));
        // The picture alone, and a text block before it.
        let alone = json!({"content": [{"type": "image", "data": base64, "mimeType": "image/jpeg"}]});
        assert_eq!(returned(alone), (Some("image.jpeg (16 bytes, image/jpeg)".into()), Some(png.to_vec())));
        let after = json!({"content": [{"type": "text", "text": "numbers"}, {"type": "image", "data": base64, "mimeType": "image/png"}]});
        assert_eq!(returned(after), (Some("image.png (16 bytes, image/png)\nnumbers".into()), Some(png.to_vec())));
        // The type under the other name, and under no name: the bytes say what the picture is.
        let other_name = json!({"content": [{"type": "image", "data": base64, "mediaType": "image/webp"}]});
        assert_eq!(returned(other_name), (Some("image.webp (16 bytes, image/webp)".into()), Some(png.to_vec())));
        let unnamed = json!({"content": [{"type": "image", "data": base64}]});
        assert_eq!(returned(unnamed), (Some("image.png (16 bytes, image/png)".into()), Some(png.to_vec())));
        let unknown = json!({"content": [{"type": "image", "data": super::super::backend::base64(b"neither of them")}]});
        assert_eq!(returned(unknown.clone()), (Some(unknown.to_string()), None));
        // No picture that decodes, or only the word: read as any other result.
        for other in [
            json!({"content": [{"type": "image", "data": "not base64!", "mimeType": "image/png"}]}),
            json!({"content": [{"type": "image", "data": base64, "mimeType": "application/pdf"}]}),
            json!({"content": [{"type": "text", "text": "an \"image\" is named"}]}),
            json!({"note": "\"image\""}),
        ] {
            assert_eq!(returned(other.clone()), (Some(other.to_string()), None));
        }
    }

    #[test]
    fn a_file_handed_back_as_media_is_a_line_and_a_picture() {
        let png = b"\x89PNG\r\n\x1a\nthe rest";
        let handed = |kind: &str, data: &str| {
            let media = json!({"__workspaceMedia": true, "text": format!("shots/a.png (16 bytes, {kind})"), "mediaType": kind, "data": data});
            let Incoming::Tool { output, picture, .. } =
                update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "rawOutput": media.to_string()}), "s")
            else {
                panic!()
            };
            (output, picture)
        };
        let base64 = super::super::backend::base64(png);
        assert_eq!(handed("image/png", &base64), (Some("shots/a.png (16 bytes, image/png)".into()), Some(png.to_vec())));
        // A PDF comes the same way: its line, and no picture. So does a picture whose bytes do not decode.
        assert_eq!(handed("application/pdf", &base64), (Some("shots/a.png (16 bytes, application/pdf)".into()), None));
        assert_eq!(handed("image/png", "not base64!"), (Some("shots/a.png (16 bytes, image/png)".into()), None));
        // Anything that only looks like it is read as any other result.
        for raw in [json!({"__workspaceMedia": false, "text": "x"}).to_string(), json!({"note": "\"__workspaceMedia\""}).to_string()] {
            let Incoming::Tool { output, picture, .. } =
                update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "t", "rawOutput": raw.clone()}), "s")
            else {
                panic!()
            };
            assert_eq!((output, picture), (Some(raw), None));
        }
    }

    #[test]
    fn long_tool_output_is_cut_on_a_character_boundary() {
        let long = "é".repeat(3000);
        let u = json!({"sessionUpdate": "tool_call", "toolCallId": "t", "content": [{"type": "content", "content": {"type": "text", "text": long}}]});
        let Incoming::Tool { output: Some(o), .. } = update(&u, "s") else { panic!() };
        assert!(o.len() <= MAX_OUTPUT + 3 && o.ends_with('…'));
    }

    #[test]
    fn the_models_of_a_session_reply_are_read_like_its_modes() {
        let v = json!({"sessionId": "s", "models": {"currentModelId": "sonnet", "availableModels": [
            {"modelId": "opus", "name": "Opus", "description": "Most capable"},
            {"modelId": "sonnet", "name": "Sonnet"},
            {"modelId": "haiku"},
            {"name": "no id"}]}});
        let (current, list) = models(&v).unwrap();
        assert_eq!(current, "sonnet");
        let names: Vec<_> = list.iter().map(|m| (m.id.as_str(), m.name.as_str(), m.description.as_str())).collect();
        assert_eq!(names, [("opus", "Opus", "Most capable"), ("sonnet", "Sonnet", ""), ("haiku", "haiku", "")]);
        // An agent without models, or with an empty list, reports none.
        assert!(models(&json!({"sessionId": "s"})).is_none());
        assert!(models(&json!({"models": {"currentModelId": "x", "availableModels": []}})).is_none());
        let changed = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"current_model_update","currentModelId":"haiku"}}}"#;
        assert_eq!(parse(changed), Some(Incoming::ModelChanged("haiku".into())));
        assert_eq!(set_model_params("s", "opus"), json!({"sessionId": "s", "modelId": "opus"}));
        assert_eq!(set_config_option_params("s1", "vendor.model", "smart"), json!({"sessionId":"s1","configId":"vendor.model","value":"smart"}));
    }

    #[test]
    fn config_options_parse_flat_and_grouped_selects_and_ignore_unsupported() {
        let v = json!({"configOptions":[
            {"id":"arbitrary","name":"Model","category":"model","type":"select","currentValue":"a","options":[{"value":"a","name":"A"}]},
            {"id":"effort","category":"thought_level","type":"select","currentValue":"high","options":[{"group":"Reasoning","options":[{"value":"high","name":"High"}]}]},
            {"id":"bad","type":"boolean"}]});
        let ConfigParse::Snapshot(options) = config_options(&v) else { panic!() };
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].id, "arbitrary");
        assert_eq!(options[1].options[0].group.as_deref(), Some("Reasoning"));
        assert!(matches!(
            config_options(&json!({"configOptions":[{"type":"select","id":"broken","currentValue":"x","options":[]}]})),
            ConfigParse::Invalid
        ));
        assert_eq!(config_options(&json!({"configOptions":[]})), ConfigParse::Snapshot(vec![]));
        let notification = json!({"sessionUpdate":"config_option_update","configOptions":[]});
        assert_eq!(update(&notification, "s"), Incoming::ConfigOptions { session: "s".into(), options: vec![] });
    }

    /// The built-in agent advertises its slash commands in one update after the session opens
    /// (`agent-smoke.mjs` reads `skill/<name>` from it). It replaces the list whole; hostile
    /// entries are dropped, the list is bounded, and an empty list is a real clear.
    #[test]
    fn advertised_slash_commands_parse_without_panicking_on_hostile_entries() {
        let real = json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "skill/solidcraft-modeling", "description": "Build a part"},
            {"name": "  skill/solidcraft-editing  ", "description": 7},
            {"name": "\t", "description": "blank name"},
            {"description": "no name at all"},
            {"name": "skill/plain"},
            "not an object",
            {"name": 3},
        ]});
        assert_eq!(
            update(&real, "s"),
            Incoming::Commands {
                session: "s".into(),
                commands: vec![
                    AvailableCommand { name: "skill/solidcraft-modeling".into(), description: "Build a part".into() },
                    AvailableCommand { name: "skill/solidcraft-editing".into(), description: String::new() },
                    AvailableCommand { name: "skill/plain".into(), description: String::new() },
                ],
            }
        );
        let many: Vec<Value> = (0..300).map(|n| json!({"name": format!("skill/{n}")})).collect();
        let Incoming::Commands { commands, .. } = update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": many}), "s")
        else {
            panic!()
        };
        assert_eq!(commands.len(), 256, "an update is bounded");
        assert_eq!(
            update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": []}), "s"),
            Incoming::Commands { session: "s".into(), commands: vec![] },
            "an empty list clears the menu"
        );
        for hostile in [
            json!({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "x"}]}),
            json!({"sessionUpdate": "available_commands_update", "availableCommands": []}),
            json!({"sessionUpdate": "available_commands_update", "availableCommands": "skill/x"}),
            json!({"sessionUpdate": "available_commands_update", "availableCommands": {}}),
            json!({"sessionUpdate": "available_commands_update"}),
        ] {
            assert_eq!(update(&hostile, ""), Incoming::Ignored, "a list without a session belongs to no chat");
        }
        // With a session, a malformed or absent list is not an empty clear: the menu stays as it was.
        for malformed in [
            json!({"sessionUpdate": "available_commands_update", "availableCommands": "skill/x"}),
            json!({"sessionUpdate": "available_commands_update", "availableCommands": {}}),
            json!({"sessionUpdate": "available_commands_update"}),
        ] {
            assert_eq!(update(&malformed, "s"), Incoming::Ignored, "only an array is a command list");
        }
        // A name longer than a message could be sent is dropped rather than offered as a draft
        // the send path would refuse.
        let too_long: String = "x".repeat(MAX_COMMAND_NAME + 1);
        let Incoming::Commands { commands, .. } =
            update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": too_long}]}), "s")
        else {
            panic!()
        };
        assert!(commands.is_empty(), "an unsendable command is no command");
        // Four-byte code points pass a character-only cap but can overflow the page's byte
        // budget: such a name would make the send action too large, so it is dropped.
        let emoji = "😀".repeat(MAX_COMMAND_NAME);
        let Incoming::Commands { commands, .. } =
            update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": emoji}]}), "s")
        else {
            panic!()
        };
        assert!(commands.is_empty(), "a name over the IPC byte budget is dropped");
        // A four-byte name that fits both budgets is kept.
        let emoji_ok = "😀".repeat(MAX_COMMAND_NAME / 4);
        let Incoming::Commands { commands, .. } =
            update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": emoji_ok.clone()}]}), "s")
        else {
            panic!()
        };
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, emoji_ok);
    }

    #[test]
    fn config_snapshots_bound_choices_and_preserve_unknown_categories() {
        let choices: Vec<_> = (0..300).map(|n| json!({"value":n.to_string(),"name":"Choice"})).collect();
        let option = json!({"id":"custom","name":"Custom","type":"select","currentValue":"0","options":[
            {"group":"internal-id","name":"Readable group","options":choices},
            {"value":"extra","name":"Extra"}
        ]});
        let ConfigParse::Snapshot(options) = config_options(&json!({"configOptions":[option.clone(),option]})) else { panic!() };
        assert_eq!(options.len(), 1, "duplicate IDs do not produce duplicate controls");
        assert_eq!(options[0].options.len(), 128, "the choice cap spans groups and flat entries");
        assert_eq!(options[0].options[0].group.as_deref(), Some("Readable group"));
        assert_eq!(options[0].category, "", "categories are optional metadata");
        assert_eq!(config_options(&json!({})), ConfigParse::Absent);
        assert_eq!(config_options(&json!({"configOptions":null})), ConfigParse::Invalid);
        assert_eq!(config_options(&json!({"configOptions":[{"type":"select","id":"broken"}]})), ConfigParse::Invalid);
    }
}
