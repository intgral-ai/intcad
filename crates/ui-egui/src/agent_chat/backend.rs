//! The seam between the chat panel and whatever answers it. The panel (conversation, sign-in,
//! approvals, turns, undo, the tool gate) only talks to a [`ChatBackend`]; the first backend is
//! [`crate::agent_chat::acp_client::AcpBackend`], which drives an agent the user already has over
//! the Agent Client Protocol. Another backend (one that calls a model API itself, say) slots in
//! by implementing the trait and sending its tool calls into [`StartContext::tools`]: they then
//! pass the same gate, count against the same per-turn limit and undo as one step, exactly like
//! the ACP agent's.

use serde_json::Value;

pub use super::acp::{AuthMethod, PermissionOption, ToolStatus};
pub use super::acp::{ConfigChoice, ConfigOption};
pub use super::skill::SkillSupport;

/// A connection test keeps authentication failures distinct from transport or protocol errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestError {
    AuthRequired,
    Other(String),
}

impl TestError {
    pub fn message(&self) -> &str {
        match self {
            Self::AuthRequired => crate::i18n::tr("ui.agent_chat_state_auth"),
            Self::Other(message) => message,
        }
    }
}

/// One part of a message to the agent.
#[derive(Clone, Debug, PartialEq)]
pub enum PromptPart {
    Text(String),
    /// A picture of the design (PNG), sent only to agents that take images.
    Image {
        mime: String,
        base64: String,
    },
    /// A file the agent reads as a document (a PDF), sent only to agents that take documents.
    Document {
        name: String,
        /// Where it is on this machine (a `file://` link).
        uri: String,
        mime: String,
        base64: String,
    },
}

/// What a backend reports to the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Whether the agent takes documents (PDFs) in a message; sent before [`Event::Ready`].
    Documents(bool),
    /// Whether SolidCraft tools were offered by the backend.
    ToolsAvailable(bool),
    /// Ready for a message. `agent`: its name and version; `images`: it takes pictures.
    Ready {
        agent: String,
        images: bool,
    },
    /// A standalone Mastra runtime process is ready. This says nothing about any AI connection's
    /// authentication, which arrives separately in [`Event::ConnectionAuth`].
    RuntimeReady {
        version: String,
    },
    /// The selected AI connection's authentication, checked independently of runtime readiness.
    ConnectionAuth {
        available: bool,
    },
    /// Text it streams (`thought`: its reasoning, shown dimmed).
    Text {
        thought: bool,
        text: String,
    },
    /// A new tool call, or news about one (`input`: its arguments, when the agent says).
    Tool {
        id: String,
        title: Option<String>,
        kind: Option<String>,
        status: Option<ToolStatus>,
        input: Option<Value>,
        output: Option<String>,
    },
    /// Its plan, replaced whole: each entry with its status.
    Plan(Vec<(String, String)>),
    /// It asks before running something of its own; answered with [`ChatBackend::answer_permission`].
    Permission {
        key: u64,
        title: String,
        /// The tool call's kind, as the agent gives it.
        kind: Option<String>,
        options: Vec<PermissionOption>,
    },
    /// It needs the user to sign in; `methods` are its own ways to do so.
    AuthRequired {
        methods: Vec<AuthMethod>,
    },
    /// What it says about its login.
    AuthStatus(String),
    /// Its own modes (an ACP agent's `modes`: ask, accept edits, plan, …) and the current one.
    Modes {
        current: String,
        available: Vec<(String, String)>,
    },
    /// Its models (empty `available`: only the current one changed).
    Models {
        current: String,
        available: Vec<super::acp::ModelInfo>,
    },
    /// Why an endpoint's models could not be listed (`None`: they could). It comes before the
    /// [`Event::Models`] of that listing, which then holds only the configured model.
    ModelsNote(Option<String>),
    /// The agent refused the model it was just sent, and says why.
    ModelRejected(String),
    ConfigOptions(Vec<ConfigOption>),
    ConfigOptionFinished {
        id: String,
        result: Result<Vec<ConfigOption>, String>,
    },
    /// The message is answered (`reason`: why it stopped, empty for a normal end).
    TurnEnded {
        reason: String,
    },
    /// Something failed; the backend is still running unless [`Event::Exited`] follows.
    Error(String),
    /// It ended; its last error output.
    Exited {
        tail: Vec<String>,
    },
}

/// What a backend gets to start.
pub struct StartContext {
    /// Its working folder: an empty folder of SolidCraft's own, not the user's files.
    pub cwd: std::path::PathBuf,
    /// The MCP server that reaches this window (`solidcraft-cli mcp --connect` to the chat's own
    /// endpoint), for backends that start MCP servers; `None` when it could not be set up.
    pub bridge: Option<super::acp::McpBridge>,
    /// Where tool calls run in process go, for backends that run their own tool loop.
    pub tools: Option<std::sync::mpsc::Sender<crate::ControlRequest>>,
    /// Wakes the window when something arrives.
    pub wake: Box<dyn Fn() + Send + Sync>,
}

/// A chat backend. Methods never block the frame; replies arrive through [`ChatBackend::poll`].
pub trait ChatBackend {
    /// What arrived since the last call.
    fn poll(&mut self) -> Vec<Event>;
    /// Send a message (the panel only sends one after [`Event::Ready`] and while no turn runs).
    fn prompt(&mut self, parts: &[PromptPart]);
    /// Stop the message being answered; open permission questions count as cancelled.
    fn cancel(&mut self);
    /// Start a fresh conversation on the running backend (no restart); [`Event::Ready`] follows.
    /// A message being answered is cancelled first.
    fn new_session(&mut self);
    /// The user's custom instructions (#8220), added to the system note of every session that
    /// starts after this call. The panel sets them before the first session and before New chat.
    fn set_instructions(&mut self, _text: &str) {}
    /// The open document a turn is about to bind to: its identity and actual revision, read when
    /// the turn starts. Backends that pin a document use it; the others ignore it.
    fn set_document(&mut self, _document_id: &str, _revision: u64) {}
    /// The trusted cancellation flag of the turn about to start. Backends that bridge tool calls
    /// attach it at the host boundary; model/runtime arguments never supply it.
    fn set_turn_control(&mut self, _active: std::sync::Arc<std::sync::atomic::AtomicBool>) {}
    /// What this backend can prove for a built-in skill. Unknown backends stay unverified; a
    /// backend that cannot run the skill refuses before the model call.
    fn skill_support(&self, _skill: super::skill::Skill) -> SkillSupport {
        SkillSupport::Unverified
    }
    /// Answer an [`Event::Permission`] with one of its options (`None`: cancelled).
    fn answer_permission(&mut self, key: u64, option: Option<&str>);
    /// Ask the backend to sign in by itself with one of its methods (it opens the browser).
    fn authenticate(&mut self, method: &str);
    /// Use one of the models it reported in [`Event::Models`].
    fn set_model(&mut self, _model: &str) {}
    /// Switch to one of the modes it reported in [`Event::Modes`].
    fn set_mode(&mut self, _mode: &str) {}
    /// An endpoint's new request timeout, in seconds, from the next request on.
    fn set_request_timeout(&mut self, _secs: u64) {}
    /// An endpoint's new reasoning effort (empty: send nothing), from the next request on.
    fn set_reasoning_effort(&mut self, _effort: &str) {}
    fn set_config_option(&mut self, _id: &str, _value: &str) {}
    /// Its process, for status and tests.
    fn pid(&self) -> Option<u32> {
        None
    }
    /// End it and everything it started. `wait`: block until done (quitting).
    fn shutdown(&mut self, wait: bool);
}

/// One HTTP request of the API backend (the desktop host sends it; nothing else in the chat
/// reaches the network).
#[derive(Clone, PartialEq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout_s: u64,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Even non-auth headers and URL paths/queries may contain credentials or prompt text.
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &"[redacted]")
            .field("header_count", &self.headers.len())
            .field("body_bytes", &self.body.as_ref().map(Vec::len))
            .field("timeout_s", &self.timeout_s)
            .finish()
    }
}

/// Sends an [`HttpRequest`]: the status and the body, or why it failed.
pub type HttpFn = std::sync::Arc<dyn Fn(&HttpRequest) -> Result<(u16, Vec<u8>), String> + Send + Sync>;

/// An open HTTP response body. Status and headers are available before any body byte is read,
/// so a streaming caller can reject a non-2xx or non-SSE answer before it produces output.
pub struct HttpStreamResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub reader: Box<dyn std::io::Read + Send>,
}

/// One request's cancellation handle. The host registers its transport shutdown before the
/// request blocks, so Stop can interrupt TLS/headers/body reads through `ureq`'s agent shutdown.
#[derive(Default)]
pub struct StreamCancellation {
    shutdown: std::sync::Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
    cancelled: std::sync::atomic::AtomicBool,
}

impl StreamCancellation {
    /// Stop may arrive before the host has a transport to shut down. The latch makes that
    /// cancellation stick: a later registration is invoked immediately.
    pub fn register(&self, shutdown: std::sync::Arc<dyn Fn() + Send + Sync>) {
        use std::sync::atomic::Ordering;
        let cancel_now = {
            let Ok(mut slot) = self.shutdown.lock() else { return };
            if self.cancelled.load(Ordering::Acquire) {
                true
            } else {
                *slot = Some(shutdown.clone());
                false
            }
        };
        if cancel_now {
            shutdown();
        }
    }

    pub fn cancel(&self) {
        use std::sync::atomic::Ordering;
        self.cancelled.store(true, Ordering::Release);
        let shutdown = self.shutdown.lock().ok().and_then(|mut slot| slot.take());
        if let Some(shutdown) = shutdown {
            shutdown();
        }
    }
}

/// Sends an [`HttpRequest`] and hands back the open body without buffering it. The caller's
/// cancellation handle is registered before any blocking work starts.
pub type HttpStreamFn = std::sync::Arc<dyn Fn(&HttpRequest, &std::sync::Arc<StreamCancellation>) -> Result<HttpStreamResponse, String> + Send + Sync>;

/// The system keychain (the desktop host's), for API keys: never in the preferences.
pub trait Secrets: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>, String>;
    fn set(&self, id: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// Standard base64 with padding (RFC 4648), for pictures in messages.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(T.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decode standard padded base64 (RFC 4648), as the composer stores an attached picture or PDF.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(4) {
        return Err("base64 length is not a multiple of four".into());
    }
    let byte = |c: u8| -> Option<u8> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for (index, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let a = byte(chunk[0]).ok_or_else(|| "base64 contains an invalid character".to_string())?;
        let b = byte(chunk[1]).ok_or_else(|| "base64 contains an invalid character".to_string())?;
        let c = if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' {
                return Err("base64 padding is invalid".into());
            }
            0
        } else {
            byte(chunk[2]).ok_or_else(|| "base64 contains an invalid character".to_string())?
        };
        let d = if chunk[3] == b'=' {
            if !last {
                return Err("base64 padding is invalid".into());
            }
            0
        } else {
            byte(chunk[3]).ok_or_else(|| "base64 contains an invalid character".to_string())?
        };
        let value = (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c) << 6) | u32::from(d);
        out.push((value >> 16) as u8);
        if chunk[2] != b'=' {
            out.push((value >> 8) as u8);
        } else if chunk[3] != b'=' {
            return Err("base64 padding is invalid".into());
        }
        if chunk[3] != b'=' {
            out.push(value as u8);
        }
    }
    Ok(out)
}

/// A message's parts as ACP content blocks.
pub fn content_blocks(parts: &[PromptPart]) -> Vec<Value> {
    parts
        .iter()
        .map(|p| match p {
            PromptPart::Text(t) => serde_json::json!({"type": "text", "text": t}),
            PromptPart::Image { mime, base64 } => serde_json::json!({"type": "image", "mimeType": mime, "data": base64}),
            PromptPart::Document { uri, mime, base64, .. } => {
                serde_json::json!({"type": "resource", "resource": {"uri": uri, "mimeType": mime, "blob": base64}})
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_examples() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn base64_decodes_the_encoder_output_and_refuses_bad_padding() {
        for bytes in [b"".as_slice(), b"f", b"fo", b"foo", b"foobar", &[0u8, 255, 1, 2, 3]] {
            assert_eq!(base64_decode(&base64(bytes)).as_deref(), Ok(bytes));
        }
        assert!(base64_decode("abc").is_err());
        assert!(base64_decode("ab=c").is_err());
        assert!(base64_decode("!!!!").is_err());
    }

    #[test]
    fn pictures_go_as_image_blocks() {
        let b = content_blocks(&[PromptPart::Text("hi".into()), PromptPart::Image { mime: "image/png".into(), base64: "AA==".into() }]);
        assert_eq!(b[1]["type"], "image");
        assert_eq!(b[1]["mimeType"], "image/png");
    }

    #[test]
    fn documents_go_as_embedded_resources() {
        let doc = PromptPart::Document {
            name: "brief.pdf".into(),
            uri: "file:///a/brief.pdf".into(),
            mime: "application/pdf".into(),
            base64: "AA==".into(),
        };
        let b = content_blocks(&[doc]);
        assert_eq!(b[0]["type"], "resource");
        assert_eq!(b[0]["resource"]["uri"], "file:///a/brief.pdf");
        assert_eq!(b[0]["resource"]["mimeType"], "application/pdf");
        assert_eq!(b[0]["resource"]["blob"], "AA==");
    }
}
