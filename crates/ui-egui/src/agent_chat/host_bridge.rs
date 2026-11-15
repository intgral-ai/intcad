//! The host half of #9: one MCP lane from the Mastra runtime to this window's chat document.
//!
//! The runtime opens `--bridge-fd <fd>` as a duplex stream and starts one MCP session per turn.
//! This side owns the other end, serves the existing `solidcraft_mcp::Server` over it, and sends
//! every tool call through the chat's existing control queue. The turn's cancellation flag is
//! attached here, not read from tool arguments, so a stopped or replaced turn cannot inherit the
//! next turn's permission.

use std::collections::VecDeque;
use std::io::{BufReader, BufWriter};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use solidcraft_engine::schema::ParamCheck;
use solidcraft_mcp::{Backend, GuardedCallError, GuardedReply, Server};

use super::backend::{Event, ToolStatus};
use crate::control::ControlRequest;

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

/// Longest a single MCP tool call may wait for the app's answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(3_600);

/// The turn currently allowed to use the lane.
#[derive(Clone)]
pub(crate) struct BridgeTurn {
    pub(crate) active: Arc<AtomicBool>,
    pub(crate) generation: u64,
}

struct BridgeState {
    turn: Mutex<Option<BridgeTurn>>,
    generation: AtomicU64,
}

impl BridgeState {
    fn current(&self) -> Result<BridgeTurn, String> {
        let turn = self.turn.lock().map_err(|_| "the bridge turn state is unavailable".to_string())?.clone();
        let Some(turn) = turn else {
            return Err("not run: the agent chat turn ended or was replaced".into());
        };
        if !turn.active.load(Ordering::Relaxed) {
            return Err("not run: the agent chat turn was stopped".into());
        }
        Ok(turn)
    }

    fn begin(&self, active: Arc<AtomicBool>) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if let Ok(mut turn) = self.turn.lock() {
            *turn = Some(BridgeTurn { active, generation });
        }
        generation
    }

    fn end(&self, generation: u64) {
        if let Ok(mut turn) = self.turn.lock()
            && turn.as_ref().is_some_and(|turn| turn.generation == generation)
            && let Some(turn) = turn.take()
        {
            turn.active.store(false, Ordering::Relaxed);
        }
    }

    fn cancel(&self) {
        if let Ok(mut turn) = self.turn.lock()
            && let Some(turn) = turn.take()
        {
            turn.active.store(false, Ordering::Relaxed);
        }
    }
}

/// A backend over the chat's existing control queue. Every request carries the turn's trusted
/// cancellation flag; the control gate repeats the check at actual application.
struct ChatBackend {
    tools: Sender<ControlRequest>,
    state: Arc<BridgeState>,
    events: Arc<Mutex<VecDeque<Event>>>,
    next_call: Arc<AtomicU64>,
}

impl ChatBackend {
    fn raw_call(&self, method: &str, params: Value) -> Result<Value, String> {
        let turn = self.state.current()?;
        let call = self.next_call.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        let id = format!("bridge:{call}");
        push_event(
            &self.events,
            Event::Tool {
                id: id.clone(),
                title: Some(format!("mcp.{method}")),
                kind: Some("bridge".into()),
                status: Some(ToolStatus::InProgress),
                input: None,
                output: None,
            },
        );
        let (mut request, reply) = ControlRequest::new(method, params);
        request.chat_turn = Some(turn.active);
        if self.tools.send(request).is_err() {
            push_event(&self.events, bridge_result(&id, method, false, "the agent chat bridge is closed"));
            return Err("not run: the agent chat bridge is closed".into());
        }
        match reply.recv_timeout(CALL_TIMEOUT) {
            Ok(value) => {
                let ok = value.get("ok").and_then(Value::as_bool) == Some(true);
                let text = bounded_json(&value);
                push_event(&self.events, bridge_result(&id, method, ok, &text));
                Ok(value)
            }
            Err(_) => {
                push_event(&self.events, bridge_result(&id, method, false, "the Intcad chat bridge did not answer"));
                Err("the Intcad chat bridge did not answer".into())
            }
        }
    }

    fn legacy(&self, raw: &Value) -> Result<Value, String> {
        if raw.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(raw.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(raw.get("error").and_then(Value::as_str).unwrap_or("the chat bridge refused the call").to_string())
        }
    }
}

impl Backend for ChatBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let raw = self.raw_call(method, params)?;
        self.legacy(&raw)
    }

    fn call_guarded(&mut self, method: &str, params: Value) -> Result<GuardedReply, GuardedCallError> {
        solidcraft_mcp::validate_guarded_request(method, &params)?;
        let raw = self.raw_call(method, params).map_err(GuardedCallError::CommandFailed)?;
        solidcraft_mcp::decode_guarded_reply(&raw)
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        "the Intcad chat bridge (one turn at a time)".into()
    }

    fn param_check(&self) -> ParamCheck {
        ParamCheck::Off
    }
}

/// The host end of the runtime's bridge lane.
pub(crate) struct HostBridge {
    #[cfg(unix)]
    host: UnixStream,
    #[cfg(unix)]
    child: Option<UnixStream>,
    state: Arc<BridgeState>,
    error: Arc<Mutex<Option<String>>>,
    events: Arc<Mutex<VecDeque<Event>>>,
}

impl HostBridge {
    /// Create the lane and start serving MCP on the host end. `tools` is the chat's existing
    /// control queue; no second command catalog is used.
    pub(crate) fn open(tools: Sender<ControlRequest>) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = tools;
            return Err("the Mastra bridge lane is unavailable on this platform".into());
        }
        #[cfg(unix)]
        {
            let (host, child) = UnixStream::pair().map_err(|error| format!("the Mastra bridge lane could not be created: {error}"))?;
            if !inheritable(child.as_raw_fd()) {
                return Err("the Mastra bridge descriptor is below fd 3 and cannot be inherited safely".into());
            }
            clear_cloexec(&child)?;
            host.set_nonblocking(false).map_err(|error| error.to_string())?;
            child.set_nonblocking(false).map_err(|error| error.to_string())?;
            let state = Arc::new(BridgeState { turn: Mutex::new(None), generation: AtomicU64::new(0) });
            let error = Arc::new(Mutex::new(None));
            let reader = host.try_clone().map_err(|error| format!("the Mastra bridge lane could not be cloned: {error}"))?;
            let writer = host.try_clone().map_err(|error| format!("the Mastra bridge lane could not be cloned: {error}"))?;
            let events = Arc::new(Mutex::new(VecDeque::new()));
            let backend = ChatBackend { tools, state: Arc::clone(&state), events: Arc::clone(&events), next_call: Arc::new(AtomicU64::new(0)) };
            let reported = Arc::clone(&error);
            let close_state = Arc::clone(&state);
            std::thread::Builder::new()
                .name("mastra-bridge".into())
                .spawn(move || {
                    let mut server = Server::new(Box::new(backend));
                    let failure = match server.serve(BufReader::new(reader), BufWriter::new(writer)) {
                        // EOF after the runtime normally retired its turn is not an error.
                        Ok(()) if close_state.current().is_err() => None,
                        Ok(()) => Some("the Mastra bridge lane closed during a turn".to_string()),
                        Err(failure) => Some(format!("the Mastra bridge stopped: {failure}")),
                    };
                    if let Some(failure) = failure
                        && let Ok(mut slot) = reported.lock()
                    {
                        *slot = Some(failure);
                    }
                })
                .map_err(|error| format!("the Mastra bridge server could not start: {error}"))?;
            Ok(Self { host, child: Some(child), state, error, events })
        }
    }

    /// The inherited descriptor number the runtime must receive.
    #[cfg(unix)]
    pub(crate) fn child_fd(&self) -> RawFd {
        self.child.as_ref().map(AsRawFd::as_raw_fd).unwrap_or(-1)
    }

    #[cfg(not(unix))]
    pub(crate) fn child_fd(&self) -> i32 {
        -1
    }

    #[cfg(all(test, unix))]
    pub(crate) fn take_child(&mut self) -> Option<UnixStream> {
        self.child.take()
    }

    /// Close the parent's copy after the child spawned; the child owns its inherited copy now.
    pub(crate) fn release_child(&mut self) {
        #[cfg(unix)]
        {
            self.child.take();
        }
    }

    /// Begin the turn the lane may serve. Returns a generation used to end only that turn.
    pub(crate) fn begin_turn(&self, active: Arc<AtomicBool>) -> u64 {
        self.state.begin(active)
    }

    pub(crate) fn end_turn(&self, generation: u64) {
        self.state.end(generation);
    }

    pub(crate) fn cancel_turn(&self) {
        self.state.cancel();
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut error| error.take())
    }

    pub(crate) fn take_events(&self) -> Vec<Event> {
        self.events.lock().map(|mut events| events.drain(..).collect()).unwrap_or_default()
    }
}

impl Drop for HostBridge {
    fn drop(&mut self) {
        self.state.cancel();
        #[cfg(unix)]
        {
            self.child.take();
            // Shut down the socket, not just this handle: the server thread has its own clone
            // blocked on read and must see EOF so this thread does not outlive the backend.
            let _ = self.host.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn push_event(events: &Mutex<VecDeque<Event>>, event: Event) {
    if let Ok(mut events) = events.lock() {
        if events.len() >= 64 {
            events.pop_front();
        }
        events.push_back(event);
    }
}

fn bridge_result(id: &str, method: &str, ok: bool, output: &str) -> Event {
    Event::Tool {
        id: id.to_owned(),
        title: Some(format!("mcp.{method}")),
        kind: Some("bridge".into()),
        status: Some(if ok { ToolStatus::Completed } else { ToolStatus::Failed }),
        input: None,
        output: Some(bounded(output)),
    }
}

fn bounded_json(value: &Value) -> String {
    bounded(&value.to_string())
}

fn bounded(text: &str) -> String {
    let mut out: String = text.chars().take(64 << 10).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

#[cfg(unix)]
fn inheritable(fd: RawFd) -> bool {
    fd >= 3
}

#[cfg(unix)]
fn clear_cloexec(stream: &UnixStream) -> Result<(), String> {
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    fcntl(stream, FcntlArg::F_SETFD(FdFlag::empty()))
        .map(|_| ())
        .map_err(|error| format!("the Mastra bridge descriptor could not be inherited: {error}"))
}

/// Whether the actual MCP tool schemas advertise the guarded `expectedDocument` path.
pub(crate) fn guarded_capabilities() -> (bool, bool) {
    let tools = solidcraft_mcp::tool_definitions();
    let guarded = |name: &str| {
        tools.iter().any(|tool| {
            tool.get("name").and_then(Value::as_str) == Some(name)
                && tool
                    .get("inputSchema")
                    .and_then(|schema| schema.get("properties"))
                    .and_then(|properties| properties.get("expectedDocument"))
                    .is_some()
        })
    };
    (guarded("execute"), guarded("batch"))
}

/// The launch argument the runtime uses to report the same guarded capabilities in `hello`.
pub(crate) fn capability_argument() -> Option<String> {
    let (execute, batch) = guarded_capabilities();
    if !execute && !batch {
        return None;
    }
    let list = match (execute, batch) {
        (true, true) => "execute,batch",
        (true, false) => "execute",
        (false, true) => "batch",
        (false, false) => return None,
    };
    Some(format!("--bridge-capabilities={list}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn the_host_end_serves_mcp_initialize_over_the_inherited_lane() {
        use std::io::{BufRead, BufReader, Write};
        let (tools, _rx) = std::sync::mpsc::channel();
        let mut bridge = HostBridge::open(tools).expect("the bridge opens");
        let child = bridge.take_child().expect("the child end");
        let active = Arc::new(AtomicBool::new(true));
        bridge.begin_turn(Arc::clone(&active));
        let mut writer = child.try_clone().expect("clone");
        let mut reader = BufReader::new(child);
        writer
            .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#)
            .expect("write");
        writer.write_all(b"\n").expect("newline");
        writer.flush().expect("flush");
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        assert!(line.contains("\"serverInfo\""), "{line}");
    }

    #[test]
    #[cfg(unix)]
    fn a_closed_bridge_lane_reports_an_actionable_error() {
        let (tools, _rx) = std::sync::mpsc::channel();
        let mut bridge = HostBridge::open(tools).expect("the bridge opens");
        bridge.begin_turn(Arc::new(AtomicBool::new(true)));
        drop(bridge.take_child());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while bridge.take_error().is_none() {
            assert!(std::time::Instant::now() < deadline, "the closed lane never reported");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    #[cfg(unix)]
    fn stdio_descriptors_are_not_accepted_as_the_bridge_fd() {
        assert!(!inheritable(0));
        assert!(!inheritable(1));
        assert!(!inheritable(2));
        assert!(inheritable(3));
    }

    #[test]
    fn guarded_capabilities_match_the_mcp_schemas() {
        let (execute, batch) = guarded_capabilities();
        assert!(execute && batch, "the existing MCP execute/batch tools carry expectedDocument");
        assert_eq!(capability_argument().as_deref(), Some("--bridge-capabilities=execute,batch"));
    }
}
