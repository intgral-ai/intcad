//! The host half of #9: one MCP lane from the Mastra runtime to this window's chat document.
//!
//! The runtime opens `--bridge-fd <fd>` as a duplex stream and starts one MCP session per turn.
//! This side owns the other end, serves the existing `solidcraft_mcp::Server` over it, and sends
//! every tool call through the chat's existing control queue. The turn's cancellation flag is
//! attached here, not read from tool arguments, so a stopped or replaced turn cannot inherit the
//! next turn's permission.

use std::collections::VecDeque;
use std::io::{BufReader, BufWriter};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use solidcraft_engine::entitlement::{
    AuthenticatedHostScope, BridgeScopeCheck, Capability, ConservativeDeadline, EntryCategory, MonotonicInstant, OperationId, TrustedContext,
};
use solidcraft_engine::schema::ParamCheck;
use solidcraft_mcp::{Backend, GuardedCallError, GuardedReply, Server, ToolCallContext};

use super::backend::{Event, ToolStatus};
use crate::control::ControlRequest;

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

/// Longest a single MCP tool call may wait for the app's answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(3_600);

/// The host-owned provenance a bridge may bind forwarded calls to.
///
/// It is created only by the host composition root from an authenticated
/// [`AuthenticatedHostScope`]. Each Intcad logical tool call gets a fresh operation; Mastra's own
/// prompt/plan tools never reach the bridge and therefore never mint one.
#[derive(Clone)]
pub(crate) struct HostProvenance {
    scope: AuthenticatedHostScope,
    deadline: Option<ConservativeDeadline>,
    now: Option<MonotonicInstant>,
    minted: Arc<AtomicUsize>,
}

impl HostProvenance {
    /// Bind a bridge to the host's authenticated account, installation and session.
    pub(crate) fn new(scope: AuthenticatedHostScope) -> Self {
        Self { scope, deadline: None, now: None, minted: Arc::new(AtomicUsize::new(0)) }
    }

    /// The host's conservative authorization window, when the composition root has one. A
    /// forwarded call with a window that has already passed is refused before dispatch.
    #[cfg(test)]
    pub(crate) fn with_deadline(mut self, deadline: ConservativeDeadline, now: MonotonicInstant) -> Self {
        self.deadline = Some(deadline);
        self.now = Some(now);
        self
    }

    pub(crate) fn scope(&self) -> &AuthenticatedHostScope {
        &self.scope
    }

    #[cfg(test)]
    pub(crate) fn minted(&self) -> usize {
        self.minted.load(Ordering::Relaxed)
    }

    /// Mint one logical operation for one Intcad tool call. The operation comes from host
    /// entropy; the caller cannot supply it.
    pub(crate) fn begin_logical_call(&self, entry: EntryCategory, tool: &str) -> Result<LogicalCall, String> {
        if !tool.is_empty() && solidcraft_mcp::tool_is_free(tool) {
            return Err("a free helper does not create a billed logical operation".into());
        }
        let capability: Capability = crate::control::entry_capability(entry).ok_or_else(|| "this host entry has no billed capability".to_string())?;
        let mut entropy = [0_u8; 16];
        getrandom::fill(&mut entropy).map_err(|error| format!("the host could not mint an operation: {error}"))?;
        let operation = OperationId::mint_from_entropy(&entropy).map_err(|_| "the host could not mint a valid operation id".to_string())?;
        let trusted = TrustedContext::mint(&self.scope, operation.as_str(), capability, entry)
            .map_err(|_| "the host could not mint a trusted operation context".to_string())?;
        self.minted.fetch_add(1, Ordering::Relaxed);
        Ok(LogicalCall { trusted, remaining: None, deadline: self.deadline.clone(), now: self.now })
    }
}

/// One logical Intcad tool call. Its internal substeps share this operation id.
#[derive(Clone)]
pub(crate) struct LogicalCall {
    trusted: TrustedContext,
    remaining: Option<Duration>,
    deadline: Option<ConservativeDeadline>,
    now: Option<MonotonicInstant>,
}

impl LogicalCall {
    #[cfg(test)]
    pub(crate) fn operation_id(&self) -> &OperationId {
        self.trusted.operation()
    }

    #[cfg(test)]
    pub(crate) fn trusted(&self) -> &TrustedContext {
        &self.trusted
    }

    /// A substep of the same logical call, for example `validate` then `apply` inside one bridge
    /// tool call. It keeps the operation, capability, entry and host binding.
    pub(crate) fn substep(&self, _step: &str) -> ToolCallContext {
        let mut context = ToolCallContext::from_trusted(self.trusted.clone());
        if let Some(remaining) = self.remaining {
            context = context.with_safe_stop(remaining);
        }
        if let Some(deadline) = &self.deadline {
            context = context.with_deadline(deadline.clone());
        }
        if let Some(now) = self.now {
            context = context.with_now(now);
        }
        context
    }

    pub(crate) fn context(&self) -> ToolCallContext {
        self.substep("call")
    }
}

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
    /// Host-minted provenance. Without a verified desktop scope the bridge stays closed to
    /// charged calls. Provenance is never read from the runtime's wire messages.
    provenance: Option<HostProvenance>,
    /// The logical call the server announced for the current request. Replaced on every call
    /// and cleared at [`Backend::end_tool_call`], so protocol/resource paths cannot inherit it.
    forwarded: Option<ToolCallContext>,
    /// This request is a host-selected free helper or protocol read, named so control can admit
    /// only that tool's exact methods. It never mints or bills.
    helper: Option<String>,
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
        let forwarded = self.forwarded.as_ref().map(|context| {
            let trusted = context.trusted().cloned();
            (trusted, context.deadline().cloned(), context.now())
        });
        let (mut request, reply) = if let Some(helper) = self.helper.as_deref() {
            // A published free helper or resource read: no operation, no billing. The tool name
            // lets control accept only that tool's exact fixed methods.
            ControlRequest::trusted_bridge_helper(method, params, helper)
        } else {
            match forwarded {
                Some((Some(trusted), deadline, now)) => ControlRequest::trusted_bridge_forwarded(method, params, trusted, deadline, now),
                // A charged call through an authenticated bridge must have its host context.
                // Nothing else may quietly mint a substitute operation.
                Some((None, ..)) | None if self.provenance.is_some() => {
                    push_event(&self.events, bridge_result(&id, method, false, "not run: this call has no host operation context"));
                    return Err("not run: this call has no host operation context".into());
                }
                _ => ControlRequest::trusted_bridge(method, params),
            }
        };
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

    fn tool_context(&self, tool: &str) -> Option<ToolCallContext> {
        if solidcraft_mcp::tool_is_free(tool) {
            return None;
        }
        let provenance = self.provenance.as_ref()?;
        provenance.begin_logical_call(EntryCategory::GuiBridgeMcp, tool).ok().map(|call| call.context())
    }

    fn begin_tool_call(&mut self, context: Option<&ToolCallContext>, helper: Option<&str>) {
        self.forwarded = context.cloned();
        self.helper = helper.map(str::to_owned);
    }

    fn end_tool_call(&mut self) {
        self.forwarded = None;
        self.helper = None;
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
    provenance: Arc<OnceLock<HostProvenance>>,
}

impl HostBridge {
    /// Create the lane and start serving MCP on the host end. `tools` is the chat's existing
    /// control queue; test fixtures may supply their own provenance.
    #[cfg(test)]
    pub(crate) fn open(tools: Sender<ControlRequest>, provenance: Option<HostProvenance>) -> Result<Self, String> {
        let slot = Arc::new(OnceLock::new());
        if let Some(provenance) = provenance {
            let _ = slot.set(provenance);
        }
        Self::open_lane(tools, slot, None)
    }

    /// Run the host's live credential check on the MCP worker immediately before choosing the
    /// server's authority. A missing or failed check preserves the default closed server.
    pub(crate) fn open_checked(tools: Sender<ControlRequest>, check: Option<BridgeScopeCheck>) -> Result<Self, String> {
        Self::open_lane(tools, Arc::new(OnceLock::new()), check)
    }

    fn open_lane(tools: Sender<ControlRequest>, provenance: Arc<OnceLock<HostProvenance>>, check: Option<BridgeScopeCheck>) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = (tools, provenance, check);
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
            let mut backend = ChatBackend {
                tools,
                state: Arc::clone(&state),
                events: Arc::clone(&events),
                next_call: Arc::new(AtomicU64::new(0)),
                provenance: None,
                forwarded: None,
                helper: None,
            };
            let reported = Arc::clone(&error);
            let close_state = Arc::clone(&state);
            let lane_provenance = Arc::clone(&provenance);
            std::thread::Builder::new()
                .name("mastra-bridge".into())
                .spawn(move || {
                    if let Some(scope) = check.and_then(|check| check()).filter(AuthenticatedHostScope::is_provable) {
                        let _ = lane_provenance.set(HostProvenance::new(scope));
                    }
                    backend.provenance = lane_provenance.get().cloned();
                    let mut server = match lane_provenance.get() {
                        Some(_) => Server::with_forwarded_authority(Box::new(backend)),
                        None => Server::new(Box::new(backend)),
                    };
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
            Ok(Self { host, child: Some(child), state, error, events, provenance })
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

    /// The authenticated host scope this lane can bind. `None` keeps it closed.
    pub(crate) fn scope(&self) -> Option<AuthenticatedHostScope> {
        self.provenance.get().map(|provenance| provenance.scope().clone())
    }

    /// Mint one logical call for the host bridge itself. Tests and the host composition root use
    /// this to prove that sub-steps share an id while separate calls do not.
    #[cfg(test)]
    pub(crate) fn begin_logical_call(&self, entry: EntryCategory, tool: &str) -> Result<LogicalCall, String> {
        self.provenance.get().ok_or_else(|| "the bridge has no authenticated host provenance".to_string())?.begin_logical_call(entry, tool)
    }

    /// How many logical operations this lane has minted (test evidence only).
    #[cfg(test)]
    pub(crate) fn minted_operations(&self) -> usize {
        self.provenance.get().map_or(0, HostProvenance::minted)
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
        let mut bridge = HostBridge::open(tools, None).expect("the bridge opens");
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
        let mut bridge = HostBridge::open(tools, None).expect("the bridge opens");
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
