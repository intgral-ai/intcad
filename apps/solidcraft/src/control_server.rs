//! Loopback JSON-lines control server: one request per line, one reply per line.
//! Agents and tests drive the app through it (adapted from CADCraft, MIT OR Apache-2.0).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, channel, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use solidcraft_engine::MAX_LINE;
use solidcraft_engine::control_auth;
use solidcraft_engine::control_pending;
use solidcraft_ui_egui::ControlRequest;
use solidcraft_ui_egui::control::{DESIGN_WATCH, DesignWatch, Subscriber};

/// Connections served at once, so a flood must not be able to spawn threads without end.
const MAX_CONNECTIONS: usize = 16;
/// How long a refused connection has for its reply and cleanup, from the moment it is refused.
/// One absolute deadline covers the write and the drain; nothing extends it.
const REFUSAL_BUDGET: Duration = Duration::from_millis(500);
/// The smallest socket timeout a refusal uses. Some platforms round a smaller positive timeout
/// to zero, which can mean no timeout at all, so a sub-millisecond remainder fails closed.
const REFUSAL_TIMEOUT_FLOOR: Duration = Duration::from_millis(1);
/// Bytes of what a refused client already sent that are read and dropped before closing on the
/// unauthorized path, so a reset cannot overtake the refusal.
const UNAUTHORIZED_DRAIN_BYTES: u64 = 64 * 1024;
/// Bytes a connection-limit refusal drains: one request line and its framing LF, the most a
/// client can have sent before the limit rejects it. The rest is closed, not read.
const LIMIT_REFUSAL_DRAIN_BYTES: u64 = MAX_LINE as u64 + 1;
/// Lines that are not JSON before the connection is closed, so garbage cannot loop forever.
const MAX_BAD_LINES: usize = 8;
/// How long a subscribed connection waits after a `design.changed` line before the next: what
/// changes meanwhile (a parameter dragged) goes out as one line, the latest (#6985).
const EVENT_GAP: Duration = Duration::from_millis(100);
/// How long a new connection has to show the token. A client connects when it has a request to
/// send, so this is plenty; one that doesn't send it loses the connection, and its seat among
/// [`MAX_CONNECTIONS`] goes to someone else.
const TOKEN_WAIT: Duration = Duration::from_secs(5);

static OPEN: AtomicUsize = AtomicUsize::new(0);
/// The process's one refusal worker and waiting slot, started before any listener accepts.
static REFUSALS: Refusals = Refusals::new();
/// The token files this launch wrote (its control port, the agent chat's endpoint) and the
/// token in each, removed when the app ends unless another launch has written the file since.
static TOKEN_FILES: std::sync::Mutex<Vec<(std::path::PathBuf, String)>> = std::sync::Mutex::new(Vec::new());

/// A new random token: 32 bytes from the operating system, as hex.
fn new_token() -> Option<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Remove this launch's token files and discovery file (at exit), leaving any that another
/// launch has written since (#8794).
pub fn remove_token_file() {
    if let Ok(files) = TOKEN_FILES.lock() {
        for (path, token) in files.iter() {
            control_auth::remove_token_file(path, token);
        }
    }
    if let Some((port, token)) = APP_PORT.lock().ok().and_then(|p| p.clone()) {
        control_auth::remove_current_port(port, &token);
    }
}

/// The port the app's own channel listens on and its token, for removing the discovery file at exit.
static APP_PORT: std::sync::Mutex<Option<(u16, String)>> = std::sync::Mutex::new(None);

pub fn start(listen_on: control_auth::Listen, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    // Every connection must show the token before anything runs (docs/control-protocol.md).
    match control_auth::env_token().or_else(new_token) {
        Some(token) => {
            let listener = match listen_on {
                control_auth::Listen::Exact(port) => TcpListener::bind(("127.0.0.1", port)),
                control_auth::Listen::Default => control_auth::bind_with_fallback(control_auth::DEFAULT_PORT, control_auth::PORT_TRIES),
            };
            match listener {
                Ok(l) => {
                    REFUSALS.start();
                    if let Some(port) = listen_on_listener(l, token.clone(), tx, ctx, false) {
                        if matches!(listen_on, control_auth::Listen::Default) {
                            solidcraft_engine::note!(
                                "solidcraft: control channel on the default port 127.0.0.1:{port} (Preferences > Assistant turns it off)"
                            );
                        }
                        // The discovery file: where `intcad-cli mcp` finds this app with no arguments.
                        match control_auth::write_current_port(port) {
                            Ok(_) => {
                                if let Ok(mut p) = APP_PORT.lock() {
                                    *p = Some((port, token));
                                }
                            }
                            Err(e) => solidcraft_engine::note!("solidcraft: could not write the control discovery file ({e})"),
                        }
                    }
                }
                Err(e) => solidcraft_engine::note!("solidcraft: control server failed to bind 127.0.0.1: {e}"),
            }
        }
        None => solidcraft_engine::note!("solidcraft: control server not started: no random token available"),
    }
    rx
}

/// The agent chat's own endpoint for its agent's MCP server (docs/agent-chat.md): a free loopback
/// port with a fresh token of its own (never `SOLIDCRAFT_CONTROL_TOKEN`), written to the usual
/// token file, where the MCP server finds it, so the token never passes through the agent. Its
/// requests go to `tx`, where the chat's gate reads them; one may wait long for the user's Allow.
pub fn listen_agent(tx: Sender<ControlRequest>, ctx: egui::Context) -> Option<u16> {
    let Some(token) = new_token() else {
        solidcraft_engine::note!("solidcraft: agent chat endpoint not started: no random token available");
        return None;
    };
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            solidcraft_engine::note!("solidcraft: control server failed to bind 127.0.0.1: {e}");
            return None;
        }
    };
    REFUSALS.start();
    listen_on_listener(listener, token, tx, ctx, true)
}

/// Serve `listener` (loopback) with `token`; the port it listens on.
/// `patient`: every request may wait as long as a batch (a person decides on it).
fn listen_on_listener(listener: TcpListener, token: String, tx: Sender<ControlRequest>, ctx: egui::Context, patient: bool) -> Option<u16> {
    let port = listener.local_addr().ok()?.port();
    match control_auth::token_path(port) {
        Some(path) => match control_auth::write_token_file(&path, &token) {
            Ok(()) => {
                solidcraft_engine::note!("solidcraft: control server listening on 127.0.0.1:{port} (token in {})", path.display());
                if let Ok(mut files) = TOKEN_FILES.lock() {
                    files.push((path, token.clone()));
                }
            }
            Err(e) => solidcraft_engine::note!(
                "solidcraft: control server listening on 127.0.0.1:{port}, but its token file {} could not be written ({e}); \
                 clients need SOLIDCRAFT_CONTROL_TOKEN",
                path.display()
            ),
        },
        None => {
            solidcraft_engine::note!(
                "solidcraft: control server listening on 127.0.0.1:{port}; no data folder for its token, clients need SOLIDCRAFT_CONTROL_TOKEN"
            )
        }
    }
    let token = std::sync::Arc::new(token);
    let refusals = REFUSALS.owner();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            route(
                stream,
                &OPEN,
                MAX_CONNECTIONS,
                |stream| {
                    let tx = tx.clone();
                    let ctx = ctx.clone();
                    let token = token.clone();
                    std::thread::spawn(move || {
                        serve(stream, &token, tx, ctx, patient);
                        OPEN.fetch_sub(1, Ordering::Relaxed);
                    });
                },
                |stream| refusals.hand_off(stream),
            );
        }
    });
    Some(port)
}

/// One accepted connection's place: a free served seat, or the limit's refusal.
enum Admission {
    Serve,
    Refuse,
}

/// Take one of `max` served seats for an accepted connection. A refusal never holds a seat: it
/// gives its increment back before it returns, so disposal cannot keep a new admission waiting.
fn take_seat(open: &AtomicUsize, max: usize) -> Admission {
    if open.fetch_add(1, Ordering::Relaxed) >= max {
        open.fetch_sub(1, Ordering::Relaxed);
        Admission::Refuse
    } else {
        Admission::Serve
    }
}

/// Route one accepted connection: a served seat dispatches it, and a refusal goes to disposal.
/// A refused connection is never dispatched, and neither route waits.
fn route<C>(stream: C, open: &AtomicUsize, max: usize, serve: impl FnOnce(C), refuse: impl FnOnce(C)) {
    match take_seat(open, max) {
        Admission::Serve => serve(stream),
        Admission::Refuse => refuse(stream),
    }
}

/// The reply a connection past [`MAX_CONNECTIONS`] gets.
fn too_many_connections() -> Value {
    json!({"ok": false, "error": "too many connections"})
}

/// What is left of an absolute deadline at `now`, when it is at least [`REFUSAL_TIMEOUT_FLOOR`]:
/// `None` once it has passed or is too small for a safe socket timeout.
fn refusal_time_left(deadline: Instant, now: Instant) -> Option<Duration> {
    deadline.checked_duration_since(now).filter(|left| *left >= REFUSAL_TIMEOUT_FLOOR)
}

/// The socket work of one refusal, apart from admission, so a test can script it in process.
trait RefusalIo: Send {
    /// Write the refusal line inside `deadline`.
    fn write_reply(&mut self, reply: &Value, deadline: Instant) -> std::io::Result<()>;
    /// End what this side sends, before any request still arriving is read.
    fn close_write(&mut self);
    /// Read and drop up to `max` bytes of what the client already sent, inside `deadline`.
    fn drain(&mut self, deadline: Instant, max: u64);
}

/// Reply to and close one refused connection inside `deadline`. The deadline is absolute and set
/// before the refusal is handed off, so waiting in the worker's slot does not reset it: the write
/// and the drain share the same instant. A failed or incomplete write closes the write side and
/// drains what is left of the budget; an orderly close is expected on the normal path only.
fn refuse_within(io: &mut impl RefusalIo, reply: &Value, deadline: Instant) {
    let mut now = Instant::now;
    refuse_within_with(io, reply, deadline, &mut now);
}

/// [`refuse_within`] with the clock given as `now`, so a test can show a successful reply spending
/// the budget before cleanup is considered.
fn refuse_within_with(io: &mut impl RefusalIo, reply: &Value, deadline: Instant, now: &mut impl FnMut() -> Instant) {
    if refusal_time_left(deadline, now()).is_none() {
        return;
    }
    let _ = io.write_reply(reply, deadline);
    io.close_write();
    if refusal_time_left(deadline, now()).is_some() {
        io.drain(deadline, LIMIT_REFUSAL_DRAIN_BYTES);
    }
}

/// A writer whose next write can be given only what is left of an absolute deadline.
trait DeadlineWrite {
    /// Set the timeout for the next write. The write must not be attempted when this fails.
    fn set_write_timeout(&mut self, left: Option<Duration>) -> std::io::Result<()>;
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize>;
}

/// Write `line` inside `deadline`, giving every write only what is left of it. A partial write
/// must not buy the next one another full timeout, so the timeout is recomputed before each
/// write; `now` is a parameter so a test can advance the clock without sleeping. A failed timeout
/// setup, or less than [`REFUSAL_TIMEOUT_FLOOR`] left, stops before the write.
fn write_deadline(w: &mut impl DeadlineWrite, line: &[u8], deadline: Instant, now: &mut impl FnMut() -> Instant) -> std::io::Result<()> {
    let mut written = 0;
    while written < line.len() {
        let Some(left) = refusal_time_left(deadline, now()) else {
            return Err(std::io::ErrorKind::TimedOut.into());
        };
        w.set_write_timeout(Some(left))?;
        match w.write(line.get(written..).unwrap_or(&[])) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => written += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Read and drop up to `max` bytes of an unauthorized client's request: that path's byte ceiling.
fn drain_refused(reader: &mut impl std::io::Read, max: u64) -> u64 {
    let mut bounded = std::io::Read::take(reader, max);
    std::io::copy(&mut bounded, &mut std::io::sink()).unwrap_or(0)
}

/// A reader whose next read can be given only what is left of an absolute deadline.
trait DeadlineRead {
    /// Set the timeout for the next read. The read must not be attempted when this fails.
    fn set_read_timeout(&mut self, left: Duration) -> std::io::Result<()>;
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}

/// Read and drop up to `max` bytes of a refused client's request inside `deadline`, giving every
/// read only what is left of it. The loop stops at the byte ceiling, at
/// [`REFUSAL_TIMEOUT_FLOOR`], when a timeout cannot be configured, at end of stream, or on a read
/// error; `now` is a parameter so a test can script the clock.
fn drain_deadline(r: &mut impl DeadlineRead, max: u64, deadline: Instant, now: &mut impl FnMut() -> Instant) -> u64 {
    const CHUNK: usize = 8 * 1024;
    let mut buf = [0u8; CHUNK];
    let mut dropped = 0u64;
    while dropped < max {
        let Some(left) = refusal_time_left(deadline, now()) else { break };
        if r.set_read_timeout(left).is_err() {
            break;
        }
        let room = (max - dropped).min(CHUNK as u64) as usize;
        let Some(chunk) = buf.get_mut(..room) else { break };
        match r.read(chunk) {
            Ok(0) => break,
            Ok(n) => dropped = dropped.saturating_add(n.min(room) as u64),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    dropped
}

/// One connection past the limit: its socket, to be replied to and closed inside the deadline the
/// refusal job carries.
struct SocketRefusal {
    stream: TcpStream,
}

/// The refusal socket as a [`DeadlineWrite`]: each write is bounded by what is left of the one
/// deadline, including across partial writes.
struct SocketWrite<'a>(&'a mut TcpStream);

impl DeadlineWrite for SocketWrite<'_> {
    fn set_write_timeout(&mut self, left: Option<Duration>) -> std::io::Result<()> {
        self.0.set_write_timeout(left)
    }

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
}

/// The refusal socket as a [`DeadlineRead`]: each read is bounded by what is left of the one
/// deadline, including across partial reads.
struct SocketRead {
    socket: TcpStream,
}

impl DeadlineRead for SocketRead {
    fn set_read_timeout(&mut self, left: Duration) -> std::io::Result<()> {
        self.socket.set_read_timeout(Some(left))
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut self.socket, buf)
    }
}

impl RefusalIo for SocketRefusal {
    fn write_reply(&mut self, reply: &Value, deadline: Instant) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(reply).map_err(std::io::Error::other)?;
        line.push(b'\n');
        let mut socket = SocketWrite(&mut self.stream);
        let mut now = Instant::now;
        write_deadline(&mut socket, &line, deadline, &mut now)
    }

    fn close_write(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Write);
    }

    fn drain(&mut self, deadline: Instant, max: u64) {
        let Ok(read) = self.stream.try_clone() else { return };
        let mut reader = SocketRead { socket: read };
        let mut now = Instant::now;
        drain_deadline(&mut reader, max, deadline, &mut now);
    }
}

/// What the process's one refusal worker does with one refused connection.
type RefusalJob = Box<dyn FnOnce() + Send>;

/// The process's refusal disposal: one worker and one waiting slot, started once outside any
/// accept loop. Every listener and conversation takes a cheap [`RefusalOwner`] handle instead of
/// starting a worker of its own, so extra listeners add no threads or queues.
struct Refusals {
    worker: OnceLock<Option<SyncSender<RefusalJob>>>,
    #[cfg(test)]
    starts: AtomicUsize,
}

impl Refusals {
    const fn new() -> Self {
        Self {
            worker: OnceLock::new(),
            #[cfg(test)]
            starts: AtomicUsize::new(0),
        }
    }

    /// Start the process's one refusal worker. A second call is a no-op. Without a worker, every
    /// refusal fails closed rather than retrying on each offer or listener creation.
    fn start(&self) {
        self.worker.get_or_init(|| {
            #[cfg(test)]
            self.starts.fetch_add(1, Ordering::Relaxed);
            let (worker, jobs) = sync_channel::<RefusalJob>(1);
            std::thread::Builder::new()
                .name("control-refusal".into())
                .spawn(move || {
                    while let Ok(job) = jobs.recv() {
                        job();
                    }
                })
                .ok()
                .map(|_| worker)
        });
    }

    /// A listener's or conversation's disposal handle. Taking or dropping it never starts or
    /// stops the process worker; the shared waiting slot stays the same for every owner.
    fn owner(&self) -> RefusalOwner {
        RefusalOwner { worker: self.worker.get().and_then(|worker| worker.clone()) }
    }
}

/// One listener's or conversation's share of the process refusal worker, cheap to take and drop.
#[derive(Clone)]
struct RefusalOwner {
    worker: Option<SyncSender<RefusalJob>>,
}

impl RefusalOwner {
    /// Offer `job` without waiting. `false` when the process queue's one waiting slot is taken,
    /// the worker is absent, or it has ended: the caller fails closed instead of holding the
    /// listener.
    fn hand_off_job(&self, job: RefusalJob) -> bool {
        match self.worker.as_ref() {
            Some(worker) => worker.try_send(job).is_ok(),
            None => false,
        }
    }

    /// Refuse `stream` on the process worker, with one absolute deadline that starts here, before
    /// the handoff; waiting in the shared slot does not reset it.
    fn hand_off(&self, stream: TcpStream) {
        let deadline = Instant::now() + REFUSAL_BUDGET;
        self.hand_off_job(Box::new(move || {
            let mut io = SocketRefusal { stream };
            refuse_within(&mut io, &too_many_connections(), deadline);
        }));
    }
}

/// One line without its newline, or `None` at the end of the stream. A client that sends
/// gigabytes with no newline would otherwise make this thread allocate until the process is
/// killed, so the line is refused as soon as it passes `max`.
fn read_line(r: &mut impl BufRead, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let (done, used) = {
            let available = r.fill_buf()?;
            if available.is_empty() {
                return Ok((!line.is_empty()).then_some(line));
            }
            let newline = available.iter().position(|b| *b == b'\n');
            let bytes = newline.unwrap_or(available.len());
            // Bound JSON bytes before appending or returning, including the final LF block.
            // The delimiter itself does not consume the request's JSON byte budget.
            if bytes > max.saturating_sub(line.len()) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "request too large"));
            }
            line.extend_from_slice(available.get(..bytes).unwrap_or(&[]));
            (newline.is_some(), bytes + usize::from(newline.is_some()))
        };
        r.consume(used);
        if done {
            return Ok(Some(line));
        }
    }
}

/// A reply line, written to the socket at once. `writeln!` with a JSON value hands the socket each
/// of its pieces apart, a system call for every string, number and comma: half a second for a
/// design's component tree when it has thousands of components.
fn send(out: &mut impl Write, reply: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(reply).map_err(std::io::Error::other)?;
    line.push(b'\n');
    out.write_all(&line)
}

/// The `"token"` of a request line that is a JSON object, read without building the rest of it.
/// Until a connection shows the token, a line costs only its own bytes: parsed whole, a 16 MiB
/// line of `[0,0,…]` is hundreds of MB, and 16 such connections could exhaust memory before any
/// token was checked.
fn token_of(line: &str) -> Option<String> {
    struct Token;
    impl<'de> serde::de::Visitor<'de> for Token {
        type Value = Option<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a request object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut token = None;
            while let Some(key) = map.next_key::<String>()? {
                if key == "token" {
                    token = Some(map.next_value::<String>()?);
                } else {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(token)
        }
    }
    let mut de = serde_json::Deserializer::from_str(line);
    let token = serde::Deserializer::deserialize_map(&mut de, Token).ok()?;
    de.end().ok()?;
    token
}

/// The reply to a connection that didn't show the token; the connection is then closed.
fn unauthorized() -> Value {
    json!({"ok": false, "error": "unauthorized: send the control token as \"token\" (see docs/control-protocol.md)"})
}

/// Refuse the connection: the reply, then the end of what we send, then what the client had
/// already sent read and dropped (a little, briefly). Closing a socket with unread input resets
/// the connection, and a reset can reach the client before it has read the refusal.
fn refuse(out: &mut TcpStream, reader: &mut BufReader<ClientInput>) {
    let _ = send(out, &unauthorized());
    let _ = out.shutdown(std::net::Shutdown::Write);
    reader.get_mut().deadline = Some(Instant::now() + REFUSAL_BUDGET);
    drain_refused(reader, UNAUTHORIZED_DRAIN_BYTES);
}

/// What a client sends. Until it shows the token, all of its reads share one deadline, so neither
/// blank lines nor a line sent a byte at a time buy it more time. After that it may stay idle as
/// long as it likes.
struct ClientInput {
    socket: TcpStream,
    deadline: Option<Instant>,
}

impl ClientInput {
    /// No deadline any more: the client showed the token.
    fn admit(&mut self) -> std::io::Result<()> {
        self.deadline = None;
        self.socket.set_read_timeout(None)
    }
}

impl std::io::Read for ClientInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            self.socket.set_read_timeout(Some(left))?;
        }
        self.socket.read(buf)
    }
}

/// How long a request waits for the app's reply: most requests, and a batch (or every request,
/// on a patient connection); and how long a new connection has to show the token.
#[derive(Clone, Copy)]
struct Waits {
    short: Duration,
    long: Duration,
    token: Duration,
}

const WAITS: Waits = Waits { short: Duration::from_secs(60), long: Duration::from_secs(3600), token: TOKEN_WAIT };

/// A request that timed out but is still running in the app (#1618): its id and method, and where
/// its reply will land.
struct Pending {
    id: Value,
    method: String,
    /// The request in messages ("`engine.execute` (solid.fillet)").
    what: String,
    reply: Receiver<Value>,
}

/// Every request the app has been sent and not answered yet, from every connection, for
/// `control.running` (#3654). Each connection's own thread answers that, so it never waits behind
/// a running request.
struct Running {
    list: Mutex<Vec<InFlight>>,
    connections: AtomicU64,
    keys: AtomicU64,
}

/// What `control.running` says about a request: never its parameters beyond the command id.
struct InFlight {
    key: u64,
    connection: u64,
    id: Value,
    method: String,
    command: Option<&'static str>,
    steps: Option<usize>,
    since: Instant,
    started: Arc<AtomicBool>,
}

/// The app's requests, from both of its listeners.
static RUNNING: Running = Running::new();

impl Running {
    const fn new() -> Self {
        Running { list: Mutex::new(Vec::new()), connections: AtomicU64::new(0), keys: AtomicU64::new(0) }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<InFlight>> {
        self.list.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// List `req`, sent on `connection`, until the returned ticket is dropped.
    fn add(&'static self, connection: u64, id: &Value, req: &ControlRequest) -> Ticket {
        let key = self.keys.fetch_add(1, Ordering::Relaxed);
        // Ids and methods come from the client: only short ones are repeated to the others.
        let id = match id {
            Value::Number(_) => id.clone(),
            Value::String(s) if s.len() <= 64 => id.clone(),
            _ => Value::Null,
        };
        let p = &req.params;
        let command = match req.method.as_str() {
            "engine.execute" | "command" => p.get("command").and_then(Value::as_str).and_then(solidcraft_engine::find_command).map(|c| c.id),
            _ => None,
        };
        let steps = match req.method.as_str() {
            "engine.batch" | "engine.script" => p.get("commands").and_then(Value::as_array).map(Vec::len),
            _ => None,
        };
        let method = req.method.chars().take(64).collect();
        let started = req.started.clone();
        self.lock().push(InFlight { key, connection, id, method, command, steps, since: Instant::now(), started });
        Ticket { running: self, key }
    }

    /// The `control.running` result, as `asker`'s connection sees it.
    fn result(&self, asker: u64) -> Value {
        let rows: Vec<Value> = self
            .lock()
            .iter()
            .map(|e| {
                let mut r = json!({
                    "connection": e.connection,
                    "id": e.id,
                    "method": e.method,
                    "running_s": (e.since.elapsed().as_secs_f64() * 10.0).round() / 10.0,
                    "queued": !e.started.load(Ordering::Relaxed),
                });
                if let Some(c) = e.command {
                    r["command"] = json!(c);
                }
                if let Some(n) = e.steps {
                    r["steps"] = json!(n);
                }
                if e.connection == asker {
                    r["self"] = json!(true);
                }
                r
            })
            .collect();
        json!({"running": rows})
    }
}

/// A listed request; dropped when its reply arrives.
struct Ticket {
    running: &'static Running,
    key: u64,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let key = self.key;
        self.running.lock().retain(|e| e.key != key);
    }
}

/// The reply to a timed-out request, received on a thread of its own so the request stays listed
/// until the app answers, also once its connection has closed (#3654).
fn watch(reply: Receiver<Value>, ticket: Ticket) -> Receiver<Value> {
    let (late_tx, late_rx) = channel();
    // A small thread per timed-out request (each connection has at most one). If none can be
    // made, the reply is lost and `control.pending` says the app ended the request without one.
    // The reply is passed on before the request leaves the list: once `control.running` no longer
    // shows it, its connection's `control.pending` has it.
    let _ = std::thread::Builder::new().stack_size(64 * 1024).spawn(move || {
        if let Ok(v) = reply.recv() {
            let _ = late_tx.send(v);
        }
        drop(late_tx);
        drop(ticket);
    });
    late_rx
}

/// The reply text for a request that outlasts its wait. A request that only reads changes
/// nothing when it finishes; an export or a save still writes its file, and any other request
/// still applies, as it was not cancelled.
fn timeout_message(what: &str, effect: control_pending::Effect) -> String {
    match effect {
        control_pending::Effect::ReadsOnly => {
            format!("timeout: {what} is still running in the app ({}); ask `control.pending` for its answer", effect.when_done())
        }
        control_pending::Effect::WritesFile => format!(
            "timeout: {what} is still running in the app and will still write the file when it finishes (it was not cancelled: \
             don't send it again; the design does not change); ask `control.pending` for its result"
        ),
        control_pending::Effect::Changes => format!(
            "timeout: {what} is still running in the app and will still apply when it finishes (it was not cancelled: don't send it \
             again); ask `control.pending` for its result"
        ),
    }
}

/// The reply text for a request refused because an earlier one on the connection still runs.
fn busy_message(p: &Pending, method: &str) -> String {
    let method: String = method.chars().take(128).collect();
    format!(
        "busy: request {} ({}) is still running, so `{method}` was not run; ask `control.pending` for that request's result, then \
         send this one again",
        p.id, p.what
    )
}

/// A subscribed connection's socket, for its replies and its events: each line is written whole
/// and alone, so an event never lands inside a reply (#6985).
struct Lines {
    socket: TcpStream,
    turn: Mutex<()>,
}

impl Lines {
    fn send(&self, line: &Value) -> std::io::Result<()> {
        let _turn = self.turn.lock().unwrap_or_else(PoisonError::into_inner);
        send(&mut &self.socket, line)
    }
}

/// A subscribed connection's seat on the watch: it leaves exactly once, whichever thread sees
/// the connection's end first — the connection's own thread at the end of the stream, or the
/// events thread at the first write a closed client refuses. The seat used to wait for the
/// events thread alone, and that thread only leaves its wait when the design moves, so the
/// count could outlive the connection by a second or more (#8315).
struct Seat(Mutex<Option<Subscriber>>);

impl Seat {
    fn new(counted: Subscriber) -> Self {
        Seat(Mutex::new(Some(counted)))
    }

    /// Leave the watch now: the seat goes once, whoever gets here first.
    fn release(&self) {
        let counted = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        drop(counted);
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.release();
    }
}

/// A connection's `control.subscribe` (#6985): its `design.changed` lines, sent from a thread of
/// their own until the connection ends.
struct Subscription {
    lines: Arc<Lines>,
    stop: Arc<AtomicBool>,
    seat: Arc<Seat>,
}

impl Subscription {
    /// Write `reply`, the window's answer to `control.subscribe` naming the design `seen`, on
    /// `out`, then send `design`'s changes after it: the reply goes out before any event. `None`,
    /// with nothing written, when no thread or socket was left for the events.
    fn start(
        design: &'static DesignWatch,
        counted: Subscriber,
        out: &TcpStream,
        seen: (String, u64),
        reply: &Value,
    ) -> Option<(Self, std::io::Result<()>)> {
        let lines = Arc::new(Lines { socket: out.try_clone().ok()?, turn: Mutex::new(()) });
        let stop = Arc::new(AtomicBool::new(false));
        let seat = Arc::new(Seat::new(counted));
        let first = lines.turn.lock().unwrap_or_else(PoisonError::into_inner);
        let (l, s, watch_seat) = (lines.clone(), stop.clone(), Arc::clone(&seat));
        // The connection's seat on the watch rides the events thread: it leaves when the thread
        // does, so the next failed write to a closed client retires it even while the connection's
        // own thread is slow to see the end of the stream (#7226). The connection's own thread
        // leaves it too, so a closed connection stops counting at once (#8315).
        std::thread::Builder::new().stack_size(64 * 1024).spawn(move || send_events(design, &l, &s, seen, watch_seat)).ok()?;
        let sent = send(&mut &lines.socket, reply);
        drop(first);
        Some((Subscription { lines, stop, seat }, sent))
    }
}

/// The design a `control.subscribe` reply names.
fn design_of(reply: &Value) -> Option<(String, u64)> {
    let result = reply.get("result")?;
    Some((result.get("document_id")?.as_str()?.to_string(), result.get("revision")?.as_u64()?))
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The seat on the watch goes now: the connection is over, and nothing counts it any more
        // (#8315).
        self.seat.release();
        // The socket closes now, not when the thread next looks; a line blocked on a client that
        // stopped reading ends too.
        let _ = self.lines.socket.shutdown(std::net::Shutdown::Both);
    }
}

/// The `design.changed` lines of a subscribed connection, each time the design differs from the
/// last one it was told of (`seen`), until `stop`. Holds the connection's seat on the watch
/// (`seat`), which it leaves when the thread ends: on `stop`, or on the first write a closed
/// client refuses (#7226).
fn send_events(design: &DesignWatch, lines: &Lines, stop: &AtomicBool, mut seen: (String, u64), seat: Arc<Seat>) {
    while !stop.load(Ordering::Relaxed) {
        let Some(now) = design.wait(&seen, stop, Duration::from_secs(1)).filter(|now| *now != seen) else { continue };
        if lines.send(&json!({"event": "design.changed", "params": {"document_id": now.0, "revision": now.1}})).is_err() {
            break;
        }
        seen = now;
        std::thread::sleep(EVENT_GAP);
    }
    // The client is gone: it is not counted on the watch any more, whatever the connection's own
    // thread still has to notice (#8315).
    seat.release();
}

/// What this app's control channel does beyond the methods every version has, for a client to
/// find out before it relies on it (an older app answers `control.capabilities` with "unknown
/// method"). `param_check`: the levels `engine.execute`, `engine.batch` and `engine.script` take
/// as a request's own parameter check (#9184).
fn capabilities() -> Value {
    json!({"param_check": solidcraft_engine::schema::ParamCheck::ALL.map(|l| l.name()),
        "guardedExecution":{"version":"1.0","execute":true,"batch":true}})
}

/// Write `reply` on the connection: through its subscription once it has one.
fn answer(out: &mut TcpStream, events: Option<&Subscription>, reply: &Value) -> std::io::Result<()> {
    match events {
        Some(e) => e.lines.send(reply),
        None => send(out, reply),
    }
}

fn serve(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context, patient: bool) {
    serve_with(stream, token, tx, ctx, patient, WAITS, &RUNNING, &DESIGN_WATCH);
}

#[allow(clippy::too_many_arguments)]
fn serve_with(
    stream: TcpStream,
    token: &str,
    tx: Sender<ControlRequest>,
    ctx: egui::Context,
    patient: bool,
    waits: Waits,
    running: &'static Running,
    design: &'static DesignWatch,
) {
    let connection = running.connections.fetch_add(1, Ordering::Relaxed) + 1;
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let mut reader = BufReader::new(ClientInput { socket: read, deadline: Some(Instant::now() + waits.token) });
    let mut bad = 0usize;
    let mut authenticated = false;
    let mut pending: Option<Pending> = None;
    let mut events: Option<Subscription> = None;
    loop {
        // A first `control.subscribe` on its way to the window (#6985).
        let mut joining = None;
        let line = match read_line(&mut reader, MAX_LINE) {
            Ok(Some(l)) => String::from_utf8_lossy(&l).into_owned(),
            Ok(None) => break,
            // Closed without a word: a client that connected early and only now sends its first
            // request would read a line here as the reply to it. It finds the connection closed
            // and opens another.
            Err(e) if !authenticated && matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => break,
            Err(e) => {
                let _ = answer(&mut out, events.as_ref(), &json!({"ok": false, "error": e.to_string()}));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        // The first request must carry the token; later ones on the same connection may.
        if !authenticated {
            match token_of(&line) {
                Some(t) if control_auth::tokens_match(&t, token) => {
                    if reader.get_mut().admit().is_err() {
                        break;
                    }
                    authenticated = true;
                }
                _ => {
                    refuse(&mut out, &mut reader);
                    break;
                }
            }
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                // A request that timed out may have finished since (#1618). `control.running` and
                // `control.capabilities` only look, so they leave that reply where it is.
                let late = pending.as_ref().filter(|_| !matches!(method.as_str(), "control.running" | "control.capabilities")).and_then(|p| match p
                    .reply
                    .try_recv()
                {
                    Ok(reply) => Some(reply),
                    Err(TryRecvError::Disconnected) => Some(json!({"ok": false, "error": "the app ended the request without a reply"})),
                    Err(TryRecvError::Empty) => None,
                });
                let mut r = if method == "control.running" {
                    json!({"ok": true, "result": running.result(connection)})
                } else if method == "control.capabilities" {
                    json!({"ok": true, "result": capabilities()})
                } else if method == "control.pending" {
                    match (pending.take(), late) {
                        (None, _) => json!({"ok": true, "result": {"pending": null}}),
                        (Some(p), Some(reply)) => json!({"ok": true, "result": {"pending": p.id, "method": p.method, "done": true, "reply": reply}}),
                        (Some(p), None) => {
                            let r = json!({"ok": true, "result": {"pending": p.id, "method": p.method, "done": false}});
                            pending = Some(p);
                            r
                        }
                    }
                } else if let Some(p) = pending.as_ref().filter(|_| late.is_none()) {
                    // Nothing more is queued behind it, to run unseen when it ends.
                    json!({"ok": false, "busy": true, "error": busy_message(p, &method)})
                } else if method == "control.subscribe" && events.is_some() {
                    if late.is_some() {
                        // As below: finished, and the client moved on without asking.
                        pending = None;
                    }
                    // Its events already say how the design moves; a second answer could only
                    // arrive behind a newer one.
                    json!({"ok": false, "error": "already subscribed: this connection is sent design.changed lines; ask document.revision for the design now"})
                } else {
                    if late.is_some() {
                        // Finished, and the client moved on without asking: it is dropped.
                        pending = None;
                    }
                    // A batch replaces many short requests: the whole of it may take long (#743).
                    let wait = if patient || method == "engine.batch" { waits.long } else { waits.short };
                    let what = control_pending::describe(&method, &params);
                    let effect = control_pending::effect(&method, &params);
                    // Counted before the window answers, so it publishes every change from its
                    // answer on.
                    joining = (method == "control.subscribe").then(|| design.join());
                    let (req, rrx) = ControlRequest::trusted_control(method.clone(), params);
                    let ticket = running.add(connection, &id, &req);
                    if tx.send(req).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                    match rrx.recv_timeout(wait) {
                        Ok(r) => r,
                        Err(_) => {
                            let r = json!({"ok": false, "running": true, "error": timeout_message(&what, effect)});
                            pending = Some(Pending { id: id.clone(), method, what, reply: watch(rrx, ticket) });
                            r
                        }
                    }
                };
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => {
                bad += 1;
                json!({"ok": false, "error": format!("bad JSON: {e}")})
            }
        };
        // Subscribed once the window named the design (not when it refused or timed out).
        let sent = match joining.and_then(|counted| design_of(&reply).map(|seen| (counted, seen))) {
            Some((counted, seen)) => match Subscription::start(design, counted, &out, seen, &reply) {
                Some((subscription, sent)) => {
                    events = Some(subscription);
                    sent
                }
                None => {
                    send(&mut out, &json!({"id": reply.get("id"), "ok": false, "error": "not subscribed: no thread or socket left for the events"}))
                }
            },
            None => answer(&mut out, events.as_ref(), &reply),
        };
        if sent.is_err() {
            break;
        }
        if bad >= MAX_BAD_LINES {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local test double at the billing boundary for the external control path. It grants only
    /// inside this test module; no production path installs it.
    #[derive(Default)]
    struct ControlTestBilling;

    /// Host-side account/clock authority for this external-control harness. It never ships.
    struct ControlTestAccount;

    impl solidcraft_engine::entitlement::AccountService for ControlTestAccount {
        fn status(&self) -> Result<solidcraft_engine::entitlement::Availability, solidcraft_engine::entitlement::Refusal> {
            Ok(solidcraft_engine::entitlement::Availability {
                online: true,
                version_supported: true,
                model_available: true,
                balance: 100,
                held: 0,
                reasons: Vec::new(),
            })
        }

        fn authorize(
            &mut self,
            _scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
            _capability: solidcraft_engine::entitlement::Capability,
        ) -> Result<solidcraft_engine::entitlement::Grant, solidcraft_engine::entitlement::Refusal> {
            Err(solidcraft_engine::entitlement::Refusal::new(solidcraft_engine::entitlement::RefusalCode::AuthUnavailable, "test authority"))
        }

        fn begin(
            &mut self,
            _scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
            _operation: &solidcraft_engine::entitlement::OperationId,
            _capability: solidcraft_engine::entitlement::Capability,
            _entry: solidcraft_engine::entitlement::EntryCategory,
        ) -> Result<solidcraft_engine::entitlement::Admission, solidcraft_engine::entitlement::Refusal> {
            Err(solidcraft_engine::entitlement::Refusal::new(solidcraft_engine::entitlement::RefusalCode::AuthUnavailable, "test authority"))
        }

        fn finalize(
            &mut self,
            _scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
            _operation: &solidcraft_engine::entitlement::OperationId,
            _result: solidcraft_engine::entitlement::BillingResult,
        ) -> Result<solidcraft_engine::entitlement::OperationStatus, solidcraft_engine::entitlement::Refusal> {
            Err(solidcraft_engine::entitlement::Refusal::new(solidcraft_engine::entitlement::RefusalCode::AuthUnavailable, "test authority"))
        }

        fn operation(
            &self,
            _scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
            _operation: &solidcraft_engine::entitlement::OperationId,
        ) -> Result<solidcraft_engine::entitlement::OperationStatus, solidcraft_engine::entitlement::Refusal> {
            Ok(solidcraft_engine::entitlement::OperationStatus::Held)
        }
    }

    struct ControlTestClock;

    impl solidcraft_engine::entitlement::MonotonicClock for ControlTestClock {
        fn now(&self) -> solidcraft_engine::entitlement::MonotonicInstant {
            solidcraft_engine::entitlement::MonotonicInstant::from_millis(0)
        }

        fn wall_now(&self) -> std::time::SystemTime {
            std::time::SystemTime::UNIX_EPOCH
        }
    }

    impl solidcraft_engine::entitlement::BillingPort for ControlTestBilling {
        fn admit(
            &mut self,
            operation: &solidcraft_engine::entitlement::OperationId,
            capability: solidcraft_engine::entitlement::Capability,
            _entry: solidcraft_engine::entitlement::EntryCategory,
        ) -> Result<solidcraft_engine::entitlement::Grant, solidcraft_engine::entitlement::Refusal> {
            Ok(solidcraft_engine::entitlement::Grant::from_server(
                solidcraft_engine::entitlement::GrantId::new(format!("grant_test_{}", operation.as_str())),
                capability,
                solidcraft_engine::entitlement::MonotonicInstant::from_millis(60_000),
            ))
        }

        fn commit(
            &mut self,
            _operation: &solidcraft_engine::entitlement::OperationId,
            result: solidcraft_engine::entitlement::BillingResult,
        ) -> Result<solidcraft_engine::entitlement::BillingResult, solidcraft_engine::entitlement::Refusal> {
            Ok(result)
        }

        fn release(
            &mut self,
            _operation: &solidcraft_engine::entitlement::OperationId,
            _reason: solidcraft_engine::entitlement::RefusalCode,
        ) -> Result<(), solidcraft_engine::entitlement::Refusal> {
            Ok(())
        }
    }

    fn control_test_budget() -> solidcraft_engine::entitlement::BudgetTable {
        let mut budget = solidcraft_engine::entitlement::BudgetTable::unverified_default();
        budget.set_verified(
            solidcraft_engine::entitlement::Capability::Control,
            solidcraft_engine::entitlement::EntryCategory::DirectControl,
            Duration::from_secs(30),
        );
        budget
    }

    #[test]
    fn control_server_sets_external_category() {
        use solidcraft_ui_egui::control;
        let (control, _) = ControlRequest::trusted_control("engine.execute", json!({"command": "document.inspect"}));
        assert_eq!(control.entry_category(), solidcraft_engine::entitlement::EntryCategory::DirectControl);
        assert!(!control::free_entry(control.entry_category()));
        let mut billing = ControlTestBilling;
        assert!(matches!(
            control::admit_control_request(&control, &solidcraft_engine::entitlement::BudgetTable::unverified_default(), &mut billing),
            solidcraft_engine::entitlement::Admission::Refused(solidcraft_engine::entitlement::Refusal {
                code: solidcraft_engine::entitlement::RefusalCode::UnsafeToStart,
                ..
            })
        ));
    }

    #[test]
    fn gui_paths_remain_free() {
        use solidcraft_ui_egui::control;
        let (gui, _) = ControlRequest::trusted_gui("engine.execute", json!({"command": "document.inspect"}));
        assert_eq!(gui.entry_category(), solidcraft_engine::entitlement::EntryCategory::HumanGuiFree);
        assert!(control::free_entry(gui.entry_category()));
        let mut billing = ControlTestBilling;
        assert!(matches!(
            control::admit_control_request(&gui, &solidcraft_engine::entitlement::BudgetTable::unverified_default(), &mut billing),
            solidcraft_engine::entitlement::Admission::Allowed
        ));
    }

    // Uses the real server and UI control handler; the owner gate makes the stale result late.
    #[test]
    fn guarded_control_capability_is_additive_and_pending_preserves_metadata() {
        use solidcraft_ui_egui::{Services, SolveApp, control};
        static RUNNING: Running = Running::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let (base_tx, base_rx) = channel();
        let (gate_tx, gate_rx) = channel();
        let owner = std::thread::spawn(move || {
            let mut app = SolveApp::new(solidcraft_engine::Session::default(), Services::default());
            app.install_billing(Box::<ControlTestBilling>::default(), control_test_budget());
            app.install_commit_authority(
                solidcraft_engine::entitlement::AuthenticatedHostScope::new(
                    solidcraft_engine::entitlement::AccountId::new("control-test-account"),
                    solidcraft_engine::entitlement::InstallationId::new("control-test-installation"),
                    solidcraft_engine::entitlement::HostSessionBinding::new("control-test-session"),
                ),
                Box::new(ControlTestAccount),
                Box::new(ControlTestClock),
            );
            let ctx = egui::Context::default();
            let stamp = app.session.document_stamp();
            base_tx.send(json!({"documentId":stamp.document_id,"revision":stamp.revision.to_string()})).unwrap();
            for mut req in rx {
                if req.params["command"] == "parameters.list" {
                    gate_rx.recv().unwrap();
                }
                let control::Outcome::Done(reply) = control::handle(&mut app, &ctx, &mut req) else { panic!("synchronous test command") };
                req.reply.send(reply).unwrap();
            }
            assert_eq!(app.session.doc.param("width").unwrap().expr, "30 mm", "the controlled batch applied its retained step");
        });
        let waits = Waits { short: Duration::from_millis(100), long: Duration::from_secs(10), token: TOKEN_WAIT };
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_with(stream, "test-guard-token", tx, egui::Context::default(), false, waits, &RUNNING, &DESIGN_WATCH);
        });
        let mut out = TcpStream::connect(addr).unwrap();
        out.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let mut lines = BufReader::new(out.try_clone().unwrap());
        let mut ask = |method: &str, params: Value| -> Value {
            send(&mut out, &json!({"id":1,"method":method,"params":params,"token":"test-guard-token"})).unwrap();
            let mut line = String::new();
            lines.read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        };
        let base = base_rx.recv().unwrap();
        let capability = ask("control.capabilities", json!({}));
        assert_eq!(capability["result"]["guardedExecution"], json!({"version":"1.0","execute":true,"batch":true}));
        assert_eq!(capability["result"]["param_check"], json!(["off", "warn", "strict"]));
        let write =
            ask("engine.execute", json!({"command":"parameters.change","params":{"name":"width","expression":"20 mm"},"expectedDocument":base}));
        assert_eq!(write["ok"], true);
        assert_eq!(write["result"]["value"], 20.0);
        assert_ne!(write["documentStamp"], base);
        let batch = ask(
            "engine.batch",
            json!({"expectedDocument":write["documentStamp"],"commands":[{"command":"parameters.change","params":{"name":"width","expression":"30 mm"}}]}),
        );
        assert_eq!(batch["result"]["completed"], 1, "{batch}");
        assert_ne!(batch["documentStamp"], write["documentStamp"]);
        let late = ask("engine.execute", json!({"command":"parameters.list","expectedDocument":base}));
        assert_eq!(late["running"], true);
        assert_eq!(ask("control.capabilities", json!({}))["result"], capability["result"], "query while pending is not busy");
        gate_tx.send(()).unwrap();
        // The watch removes the ticket only after putting the original reply in its receiver.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ask("control.running", json!({}))["result"]["running"].as_array().unwrap().is_empty() {
            assert!(Instant::now() < deadline);
        }
        assert_eq!(ask("control.capabilities", json!({}))["result"], capability["result"], "query must not consume a completed pending reply");
        let pending = ask("control.pending", json!({}));
        assert_eq!(pending["result"]["method"], "engine.execute");
        assert_eq!(pending["result"]["done"], true);
        let stale = &pending["result"]["reply"];
        assert_eq!(stale["ok"], false);
        assert_eq!(stale["errorDetails"]["code"], "STALE_DOCUMENT");
        assert_eq!(stale["errorDetails"]["details"], json!({"expected":base,"current":batch["documentStamp"]}));
        drop(lines);
        drop(out);
        server.join().unwrap();
        owner.join().unwrap();
    }

    /// A connection that doesn't show the token in time is closed without a word, however it fills
    /// the time. One that does keeps working when it idles for longer than that.
    #[test]
    fn the_token_has_a_time_limit_and_an_authenticated_client_may_idle() {
        use std::io::Read;
        let token = "0123456789abcdef0123456789abcdef";
        let limit = Duration::from_millis(500);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        std::thread::spawn(move || {
            for req in rx {
                let _ = req.reply.send(json!({"ok": true, "result": req.method}));
            }
        });
        static RUNNING: Running = Running::new();
        let waits = Waits { short: Duration::from_secs(10), long: Duration::from_secs(10), token: limit };
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve_with(stream, token, tx, egui::Context::default(), false, waits, &RUNNING, &DESIGN_WATCH));
            }
        });
        let open = || {
            let s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
            s
        };
        // Sends `drip` every 50 ms until the server closes; what it read, and when it closed.
        let closed_after = |drip: &[u8]| {
            let mut s = open();
            let start = Instant::now();
            s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
            let mut got = Vec::new();
            loop {
                let mut buf = [0u8; 256];
                match s.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => got.extend_from_slice(buf.get(..n).unwrap()),
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                        assert!(start.elapsed() < Duration::from_secs(20), "still open");
                        let _ = s.write_all(drip);
                    }
                    Err(_) => break,
                }
            }
            (got, start.elapsed())
        };
        for drip in [&b""[..], b"\n", b"\r\n \n", b" ", b"{"] {
            let (got, took) = closed_after(drip);
            assert!(got.is_empty(), "{drip:?} was answered: {}", String::from_utf8_lossy(&got));
            assert!(took >= limit.saturating_sub(Duration::from_millis(50)), "{drip:?} closed after {took:?}");
        }

        let mut s = open();
        let mut lines = BufReader::new(s.try_clone().unwrap());
        let mut ask = |line: Value| -> Value {
            writeln!(s, "{line}").unwrap();
            let mut l = String::new();
            lines.read_line(&mut l).unwrap();
            serde_json::from_str(&l).unwrap()
        };
        assert_eq!(ask(json!({"id": 1, "method": "ui.view", "token": token}))["ok"], true);
        std::thread::sleep(limit * 3);
        assert_eq!(ask(json!({"id": 2, "method": "ui.view"}))["ok"], true, "idle for longer than the limit");

        // A wrong token is still answered, at once.
        let mut wrong = open();
        writeln!(wrong, "{}", json!({"id": 1, "method": "ui.view", "token": "nope"})).unwrap();
        let mut l = String::new();
        BufReader::new(wrong).read_line(&mut l).unwrap();
        assert!(l.contains("unauthorized"), "{l}");
    }

    /// #1618: a request that outlasts its wait says it is still running; the connection keeps its
    /// reply for `control.pending`, and refuses other requests as `busy` until it lands, so none
    /// is queued to run unseen behind it.
    #[test]
    fn a_timed_out_request_can_be_asked_for_later() {
        let token = "0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        // The slow request is held until the test has seen it time out, so the test does not
        // race the clock on a loaded machine.
        let (gate_tx, gate_rx) = channel::<()>();
        let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = ran.clone();
        std::thread::spawn(move || {
            for req in rx {
                seen.lock().unwrap().push(req.method.clone());
                if req.method == "slow" {
                    let _ = gate_rx.recv();
                }
                let _ = req.reply.send(json!({"ok": true, "result": req.method}));
            }
        });
        let waits = Waits { short: Duration::from_millis(100), long: Duration::from_millis(100), token: TOKEN_WAIT };
        static RUNNING: Running = Running::new();
        std::thread::spawn(move || {
            if let Some(stream) = listener.incoming().flatten().next() {
                serve_with(stream, token, tx, egui::Context::default(), false, waits, &RUNNING, &DESIGN_WATCH);
            }
        });
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut out = s.try_clone().unwrap();
        let mut lines = BufReader::new(s);
        let mut ask = |v: Value| -> Value {
            writeln!(out, "{v}").unwrap();
            let mut l = String::new();
            lines.read_line(&mut l).unwrap();
            serde_json::from_str(&l).unwrap()
        };
        let r = ask(json!({"id": 1, "method": "slow", "token": token}));
        assert!(r["ok"] == false && r["running"] == true && r["error"].as_str().is_some_and(|e| e.starts_with("timeout")), "{r}");
        let r = ask(json!({"id": 2, "method": "ui.view"}));
        assert!(r["busy"] == true && r["id"] == 2, "{r}");
        let r = ask(json!({"id": 3, "method": "control.pending"}));
        assert_eq!((&r["result"]["pending"], &r["result"]["done"]), (&json!(1), &json!(false)), "{r}");
        gate_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let r = loop {
            let r = ask(json!({"id": 4, "method": "control.pending"}));
            if r["result"]["done"] == true || std::time::Instant::now() > deadline {
                break r;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            (&r["result"]["pending"], &r["result"]["done"], &r["result"]["reply"]["result"]),
            (&json!(1), &json!(true), &json!("slow")),
            "{r}"
        );
        let r = ask(json!({"id": 5, "method": "control.pending"}));
        assert_eq!(r["result"]["pending"], Value::Null, "{r}");
        let r = ask(json!({"id": 6, "method": "ui.view"}));
        assert_eq!((&r["ok"], &r["result"]), (&json!(true), &json!("ui.view")), "{r}");
        assert_eq!(*ran.lock().unwrap(), ["slow", "ui.view"], "the busy request never reached the app");
    }

    /// #3654: `control.running` answers on every connection, at once, with what any connection
    /// has the app running or queued (command ids, never parameters); a timed-out request stays
    /// listed until the app answers it, and an idle app lists nothing.
    #[test]
    fn every_connection_sees_what_the_app_is_running() {
        static RUNNING: Running = Running::new();
        let token = "0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        // The window: one request at a time, as `drain_control` takes them; the export is held
        // until the test lets it go.
        let (gate_tx, gate_rx) = channel::<()>();
        std::thread::spawn(move || {
            for req in rx {
                req.started.store(true, Ordering::Relaxed);
                if req.method == "engine.execute" {
                    let _ = gate_rx.recv();
                }
                let _ = req.reply.send(json!({"ok": true, "result": req.method}));
            }
        });
        let waits = Waits { short: Duration::from_millis(100), long: Duration::from_secs(60), token: TOKEN_WAIT };
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve_with(stream, token, tx, egui::Context::default(), false, waits, &RUNNING, &DESIGN_WATCH));
            }
        });
        let open = || {
            let s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut out = s.try_clone().unwrap();
            let mut lines = BufReader::new(s);
            move |v: Value| -> Value {
                writeln!(out, "{v}").unwrap();
                let mut l = String::new();
                lines.read_line(&mut l).unwrap();
                serde_json::from_str(&l).unwrap()
            }
        };
        let running = |ask: &mut dyn FnMut(Value) -> Value, until: &dyn Fn(&[Value]) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let r = ask(json!({"id": 0, "method": "control.running", "token": token}));
                let rows = r["result"]["running"].as_array().cloned().unwrap_or_default();
                if until(&rows) || Instant::now() > deadline {
                    return (r, rows);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let mut watcher = open();
        let (r, rows) = running(&mut watcher, &|_| true);
        assert!(r["ok"] == true && rows.is_empty(), "idle: {r}");
        // What the app's control channel does beyond the methods every version has (#9184).
        let r = watcher(json!({"id": 0, "method": "control.capabilities", "token": token}));
        assert_eq!((&r["ok"], &r["result"]["param_check"]), (&json!(true), &json!(["off", "warn", "strict"])), "{r}");

        let mut export = open();
        let path = "/private/part.step";
        let params = json!({"command": "file.export", "params": {"path": path}});
        let r = export(json!({"id": 7, "method": "engine.execute", "params": params, "token": token}));
        assert_eq!(r["running"], true, "{r}");
        // A batch from another connection waits behind the export.
        let mut batch = open();
        let queued = std::thread::spawn(move || {
            let steps = json!([{"command": "solid.box"}, {"command": "solid.box"}]);
            batch(json!({"id": "b", "method": "engine.batch", "params": {"commands": steps}, "token": token}))
        });
        // The window may not have taken the export yet, even though it timed out.
        let (r, rows) = running(&mut watcher, &|rows| rows.len() == 2 && rows[0]["queued"] == false);
        assert_eq!(rows.len(), 2, "{r}");
        let (e, b) = (&rows[0], &rows[1]);
        assert_eq!(
            (&e["id"], &e["method"], &e["command"], &e["queued"]),
            (&json!(7), &json!("engine.execute"), &json!("file.export"), &json!(false)),
            "{r}"
        );
        assert!(e["running_s"].as_f64().is_some_and(|s| s >= 0.1), "{r}");
        assert_eq!((&b["id"], &b["method"], &b["steps"], &b["queued"]), (&json!("b"), &json!("engine.batch"), &json!(2), &json!(true)), "{r}");
        assert_ne!(e["connection"], b["connection"], "{r}");
        assert!(rows.iter().all(|row| row.get("self").is_none()), "{r}");
        assert!(!r.to_string().contains(path), "parameters stay with their connection: {r}");
        // The connection whose request timed out may ask too, and sees its own marked.
        let r = export(json!({"id": 8, "method": "control.running"}));
        let own: Vec<&Value> = r["result"]["running"].as_array().unwrap().iter().filter(|row| row["self"] == true).collect();
        assert_eq!(own.len(), 1, "{r}");
        assert_eq!(own[0]["command"], "file.export", "{r}");
        // So may it ask what the app can do: answered at once, not refused as busy.
        let r = export(json!({"id": 8, "method": "control.capabilities"}));
        assert_eq!(r["result"]["param_check"], json!(["off", "warn", "strict"]), "{r}");

        gate_tx.send(()).unwrap();
        assert_eq!(queued.join().unwrap()["result"], "engine.batch");
        let (r, rows) = running(&mut watcher, &|rows| rows.is_empty());
        assert!(rows.is_empty(), "done: {r}");
        // Asking `control.running` did not take the export's reply from its connection.
        let r = export(json!({"id": 9, "method": "control.pending"}));
        assert_eq!((&r["result"]["pending"], &r["result"]["done"]), (&json!(7), &json!(true)), "{r}");
    }

    /// #6985: a connection that sends `control.subscribe` gets the design in the reply, then a
    /// `design.changed` line each time the window publishes another: never before the reply,
    /// none for the same design, the latest after a burst, each line whole between its replies.
    /// A second subscribe is refused without reaching the window. A connection that did not
    /// subscribe gets no events, and one that closed is no longer counted.
    #[test]
    fn subscribed_connections_are_told_of_design_changes() {
        static RUNNING: Running = Running::new();
        static WATCH: DesignWatch = DesignWatch::new();
        let token = "0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        // The window: it answers `control.subscribe` as the app does, publishing the design first;
        // then the design changes at once, before the reply may have been written.
        std::thread::spawn(move || {
            for req in rx {
                if req.method == "control.subscribe" {
                    WATCH.publish("d-1", 3);
                    let _ = req.reply.send(json!({"ok": true, "result": {"document_id": "d-1", "revision": 3}}));
                    WATCH.publish("d-1", 4);
                } else {
                    let _ = req.reply.send(json!({"ok": true, "result": req.method}));
                }
            }
        });
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve_with(stream, token, tx, egui::Context::default(), false, WAITS, &RUNNING, &WATCH));
            }
        });
        let open = || {
            let s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            (s.try_clone().unwrap(), BufReader::new(s))
        };
        let next = |lines: &mut BufReader<TcpStream>| -> Value {
            let mut l = String::new();
            lines.read_line(&mut l).unwrap();
            serde_json::from_str(&l).unwrap()
        };
        let changed = |id: &str, revision: u64| json!({"event": "design.changed", "params": {"document_id": id, "revision": revision}});

        let (mut quiet_out, mut quiet) = open();
        writeln!(quiet_out, "{}", json!({"id": 1, "method": "ui.view", "token": token})).unwrap();
        assert_eq!(next(&mut quiet)["id"], 1);

        let (mut out, mut lines) = open();
        writeln!(out, "{}", json!({"id": 1, "method": "control.subscribe", "token": token})).unwrap();
        assert_eq!(next(&mut lines), json!({"id": 1, "ok": true, "result": {"document_id": "d-1", "revision": 3}}));
        assert_eq!(next(&mut lines), changed("d-1", 4), "the change after the window's answer comes after the reply");
        assert!(WATCH.subscribed());
        // The design it was told of is no news; another revision is.
        WATCH.publish("d-1", 4);
        WATCH.publish("d-1", 5);
        assert_eq!(next(&mut lines), changed("d-1", 5));
        // A burst ends with its latest.
        for revision in 6..=10 {
            WATCH.publish("d-1", revision);
        }
        let mut last = 5;
        while last < 10 {
            let e = next(&mut lines);
            let revision = e["params"]["revision"].as_u64().unwrap();
            assert!(e["event"] == "design.changed" && revision > last, "{e} after {last}");
            last = revision;
        }
        // Another tab: another id, whatever its revision.
        WATCH.publish("d-2", 2);
        assert_eq!(next(&mut lines), changed("d-2", 2));
        // Subscribed already: refused here (the window would have published d-1 again).
        writeln!(out, "{}", json!({"id": 2, "method": "control.subscribe"})).unwrap();
        let r = next(&mut lines);
        assert!(r["id"] == 2 && r["ok"] == false && r["error"].as_str().is_some_and(|e| e.starts_with("already subscribed")), "{r}");
        // Replies still come, whole, and nothing else was sent meanwhile.
        writeln!(out, "{}", json!({"id": 3, "method": "ui.view"})).unwrap();
        assert_eq!(next(&mut lines), json!({"id": 3, "ok": true, "result": "ui.view"}));
        writeln!(quiet_out, "{}", json!({"id": 2, "method": "ui.view"})).unwrap();
        assert_eq!(next(&mut quiet), json!({"id": 2, "ok": true, "result": "ui.view"}), "no events without subscribing");
        // Closed, it is no longer counted. Each publish asks the events thread for one more line,
        // and its write to the closed connection is what retires the seat; that keeps the wait off
        // the connection's own thread, which anything else this process is doing may hold up past
        // the deadline (#7226).
        drop((out, lines));
        let mut revision = 11;
        let deadline = Instant::now() + Duration::from_secs(10);
        while WATCH.subscribed() && Instant::now() < deadline {
            WATCH.publish("d-1", revision);
            revision += 1;
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!WATCH.subscribed());
    }

    /// #8315: a closed connection leaves the watch at once. The seat of a subscribed connection
    /// goes when the connection's own thread sees the end of the stream, not only when the
    /// events thread notices: that thread waits for the design to move, so a connection that
    /// closed while the design stood still kept the watch counting it for a second or more.
    #[test]
    fn a_closed_connection_leaves_the_watch_at_once() {
        static RUNNING: Running = Running::new();
        static WATCH: DesignWatch = DesignWatch::new();
        let token = "0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        // The window names the design once and then leaves it alone: nothing moves it, so the
        // events thread has nothing to write and only the end of the connection can retire it.
        std::thread::spawn(move || {
            for req in rx {
                if req.method == "control.subscribe" {
                    WATCH.publish("d-1", 3);
                    let _ = req.reply.send(json!({"ok": true, "result": {"document_id": "d-1", "revision": 3}}));
                } else {
                    let _ = req.reply.send(json!({"ok": true, "result": req.method}));
                }
            }
        });
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve_with(stream, token, tx, egui::Context::default(), false, WAITS, &RUNNING, &WATCH));
            }
        });
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut out = s.try_clone().unwrap();
        let mut lines = BufReader::new(s);
        writeln!(out, "{}", json!({"id": 1, "method": "control.subscribe", "token": token})).unwrap();
        let mut l = String::new();
        lines.read_line(&mut l).unwrap();
        let reply: Value = serde_json::from_str(&l).unwrap();
        assert!(reply["ok"] == true, "{reply}");
        assert!(WATCH.subscribed(), "the connection is subscribed");
        // Let the events thread settle into its wait: the design stands still, so it has nothing
        // to write, and only the end of the connection can retire it.
        std::thread::sleep(Duration::from_millis(200));
        // The client goes away without asking anything else.
        drop((out, lines));
        let deadline = Instant::now() + Duration::from_millis(400);
        while WATCH.subscribed() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!WATCH.subscribed(), "a closed connection is not counted on the watch");
    }

    /// A timed-out request that only reads is not said to "still apply", nor is one that only
    /// writes a file (an export, #1989); a change is, by name.
    #[test]
    fn timeout_text_tells_reads_from_changes() {
        let says = |method: &str, p: Value| timeout_message(&control_pending::describe(method, &p), control_pending::effect(method, &p));
        let read = says("ui.inspect", json!({}));
        assert!(read.starts_with("timeout") && read.contains("only reads") && !read.contains("will still apply"), "{read}");
        let read = says("engine.execute", json!({"command": "sketch.inspect"}));
        assert!(read.contains("only reads") && read.contains("sketch.inspect"), "{read}");
        let change = says("engine.execute", json!({"command": "solid.fillet"}));
        assert!(change.starts_with("timeout") && change.contains("will still apply") && change.contains("solid.fillet"), "{change}");
        let export = says("engine.execute", json!({"command": "file.export", "params": {"path": "/tmp/wall.step"}}));
        assert!(export.contains("will still write the file") && !export.contains("apply") && export.contains("file.export"), "{export}");
        let save = says("engine.execute", json!({"command": "file.save"}));
        assert!(save.contains("will still write the file") && !save.contains("apply"), "{save}");
        let render = says("ui.render", json!({"path": "/tmp/a.png"}));
        assert!(render.contains("will still write the file"), "{render}");
    }

    /// The token is read from a request object without building the rest of it: other fields,
    /// however large or nested, are only skipped. Anything but an object with a string token is
    /// no token.
    #[test]
    fn token_is_read_without_parsing_the_request() {
        assert_eq!(token_of(r#"{"id":1,"token":"abc","params":{"a":[1,[2,{"b":null}]]}}"#).as_deref(), Some("abc"));
        let flood = format!(r#"{{"params":[{}0],"token":"t"}}"#, "0,".repeat(1 << 20));
        assert_eq!(token_of(&flood).as_deref(), Some("t"));
        for no in [r#"["abc"]"#, r#"{"token":5}"#, r#"{"token":"a"} x"#, r#"{"id":1}"#, "GET / HTTP/1.1", r#""token""#] {
            assert_eq!(token_of(no), None, "{no}");
        }
    }

    /// #205: a connection runs nothing until a request carries the launch's token; a wrong or
    /// missing token, or garbage, is refused and the connection closed. Once shown, the token
    /// need not be repeated on that connection.
    #[test]
    fn requests_run_only_after_the_token() {
        use std::io::Read;
        let token = "0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = ran.clone();
        std::thread::spawn(move || {
            for req in rx {
                seen.lock().unwrap().push(req.method.clone());
                let _ = req.reply.send(json!({"ok": true, "result": null}));
            }
        });
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve(stream, token, tx, egui::Context::default(), false));
            }
        });
        let session = |lines: &[String]| {
            let mut s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            for l in lines {
                let _ = writeln!(s, "{l}");
            }
            let _ = s.shutdown(std::net::Shutdown::Write);
            let mut text = String::new();
            let _ = s.read_to_string(&mut text);
            text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).collect::<Vec<_>>()
        };
        for first in [
            json!({"id": 1, "method": "app.quit", "params": {"force": true}}).to_string(),
            json!({"id": 1, "method": "app.quit", "token": "0123456789abcdef0123456789abcdee"}).to_string(),
            json!({"id": 1, "method": "app.quit", "token": "0123"}).to_string(),
            json!({"id": 1, "method": "app.quit", "token": 7}).to_string(),
            "not json".to_string(),
        ] {
            let replies = session(&[first.clone(), json!({"id": 2, "method": "app.quit", "token": token}).to_string()]);
            assert_eq!(replies.len(), 1, "{first}: closed after refusing: {replies:?}");
            assert!(replies[0]["error"].as_str().is_some_and(|e| e.starts_with("unauthorized")), "{first}: {replies:?}");
        }
        assert!(ran.lock().unwrap().is_empty(), "nothing ran without the token");
        let replies = session(&[
            json!({"id": 1, "method": "ui.inspect", "token": token}).to_string(),
            json!({"id": 2, "method": "engine.commands"}).to_string(),
        ]);
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert!(replies.iter().all(|r| r["ok"] == true), "{replies:?}");
        assert_eq!(*ran.lock().unwrap(), ["ui.inspect", "engine.commands"]);
    }

    /// A request longer than the cap is refused instead of allocated.
    #[test]
    fn lines_past_the_cap_are_refused() {
        let flood = vec![b'A'; MAX_LINE + 2];
        let mut r = BufReader::new(flood.as_slice());
        let e = read_line(&mut r, MAX_LINE).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        assert!(e.to_string().contains("request too large"), "{e}");
    }

    #[test]
    fn lines_cap_excludes_lf_in_every_final_buffer() {
        let max = 1000;
        for capacity in [64, 2048] {
            for len in [max - 1, max, max + 1] {
                for newline in [true, false] {
                    let mut bytes = vec![b' '; len];
                    if newline {
                        bytes.push(b'\n');
                    }
                    let mut reader = BufReader::with_capacity(capacity, bytes.as_slice());
                    let got = read_line(&mut reader, max);
                    if len <= max {
                        assert_eq!(got.unwrap(), Some(vec![b' '; len]));
                        assert!(read_line(&mut reader, max).unwrap().is_none());
                    } else {
                        assert!(got.is_err(), "{len} bytes, capacity={capacity}, LF={newline}: {got:?}");
                        let error = got.unwrap_err();
                        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                        assert_eq!(error.to_string(), "request too large");
                    }
                }
            }
            let mut reader = BufReader::with_capacity(capacity, b"\n".as_slice());
            assert_eq!(read_line(&mut reader, 0).unwrap(), Some(vec![]));
            assert_eq!(read_line(&mut reader, 0).unwrap(), None);
            let mut reader = BufReader::with_capacity(capacity, b"".as_slice());
            assert_eq!(read_line(&mut reader, max).unwrap(), None);
        }
    }

    // Valid authenticated JSON with an overlong whitespace tail must be rejected by the real
    // reader before auth/dispatch, not just by the no-LF flood or invalid-token branches.
    #[test]
    fn serve_refuses_overlong_valid_json_with_lf_before_ui_queue() {
        use std::io::Read;
        static RUNNING: Running = Running::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let waits = Waits { short: Duration::from_millis(100), long: Duration::from_secs(10), token: TOKEN_WAIT };
            serve_with(stream, "test-line-token", tx, egui::Context::default(), false, waits, &RUNNING, &DESIGN_WATCH);
        });
        let mut bytes = serde_json::to_vec(&json!({"id":1,"method":"engine.execute","token":"test-line-token",
            "params":{"command":"parameters.change","params":{"name":"width","expression":"30 mm"}}}))
        .unwrap();
        bytes.resize(MAX_LINE + 1, b' ');
        assert!(serde_json::from_slice::<Value>(&bytes).is_ok(), "not an auth or bad-JSON fixture");
        bytes.push(b'\n');
        let mut socket = TcpStream::connect(addr).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        socket.write_all(&bytes).unwrap();
        socket.shutdown(std::net::Shutdown::Write).unwrap();
        let mut replies = String::new();
        socket.read_to_string(&mut replies).unwrap();
        server.join().unwrap();
        assert_eq!(rx.try_iter().count(), 0, "no request entered the UI queue");
        let replies: Vec<Value> = replies.lines().map(|s| serde_json::from_str(s).unwrap()).collect();
        assert_eq!(replies, vec![json!({"ok":false,"error":"request too large"})], "one refusal, then EOF");
    }

    /// Ordinary framing: lines split on newlines, blank lines kept, no trailing newline needed.
    #[test]
    fn lines_split_on_newlines() {
        let text = b"{\"a\":1}\n\n{\"b\":2}\nlast".to_vec();
        let mut r = BufReader::new(text.as_slice());
        let mut got = Vec::new();
        while let Some(line) = read_line(&mut r, MAX_LINE).unwrap() {
            got.push(String::from_utf8(line).unwrap());
        }
        assert_eq!(got, vec!["{\"a\":1}", "", "{\"b\":2}", "last"]);
        assert!(read_line(&mut r, MAX_LINE).unwrap().is_none());
    }

    /// A line that spans several reads of the underlying stream still arrives whole.
    #[test]
    fn lines_longer_than_one_buffer_arrive_whole() {
        let long = "x".repeat(200_000);
        let text = format!("{long}\n{long}\n").into_bytes();
        let mut r = BufReader::with_capacity(64, text.as_slice());
        assert_eq!(read_line(&mut r, MAX_LINE).unwrap().unwrap().len(), long.len());
        assert_eq!(read_line(&mut r, MAX_LINE).unwrap().unwrap().len(), long.len());
        assert!(read_line(&mut r, MAX_LINE).unwrap().is_none());
        // The same line against a smaller cap is refused.
        let mut r = BufReader::with_capacity(64, text.as_slice());
        assert!(read_line(&mut r, 1000).is_err());
    }

    /// A large reply reaches the socket in one write, not one per JSON piece (a component tree of
    /// thousands of components took half a second to send), and reads back as it was.
    #[test]
    fn a_large_reply_is_written_at_once() {
        struct Counted {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for Counted {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let components: Vec<Value> = (0..2000).map(|i| json!({"id": i, "name": format!("Part {i}"), "bodies": ["a", "b"], "parent": 0})).collect();
        let reply = json!({"ok": true, "id": 7, "result": {"components": components}});
        let mut out = Counted { writes: 0, bytes: Vec::new() };
        send(&mut out, &reply).unwrap();
        assert_eq!(out.writes, 1);
        assert_eq!(out.bytes.last(), Some(&b'\n'));
        assert_eq!(serde_json::from_slice::<Value>(&out.bytes).unwrap(), reply);
    }

    /// A refused connection gives its served-seat increment back at once: it holds no seat, and
    /// the route that refuses it never dispatches it.
    #[test]
    fn a_refused_connection_is_disposed_without_a_served_seat_or_dispatch() {
        let open = AtomicUsize::new(1);
        let served = Arc::new(AtomicUsize::new(0));
        let disposed = Arc::new(AtomicUsize::new(0));
        let (s, d) = (served.clone(), disposed.clone());
        route(
            7u8,
            &open,
            1,
            move |_| {
                s.fetch_add(1, Ordering::Relaxed);
            },
            move |n| d.store(usize::from(n), Ordering::Relaxed),
        );
        assert_eq!(served.load(Ordering::Relaxed), 0, "a refused connection is never dispatched");
        assert_eq!(disposed.load(Ordering::Relaxed), 7, "it goes to disposal");
        assert_eq!(open.load(Ordering::Relaxed), 1, "the refused seat is released at once");
    }

    /// A scripted refusal: records the reply, the close, the drain and the deadline it received.
    #[derive(Default)]
    struct Scripted {
        order: Vec<&'static str>,
        reply: Option<Value>,
        write_deadline: Option<Instant>,
        drain: Option<(Instant, u64)>,
        fail_write: bool,
        /// The clock the reply advances by `consume`, for a test that spends the budget.
        clock: Option<Arc<Mutex<Instant>>>,
        consume: Duration,
    }

    impl RefusalIo for Scripted {
        fn write_reply(&mut self, reply: &Value, deadline: Instant) -> std::io::Result<()> {
            self.order.push("reply");
            self.reply = Some(reply.clone());
            self.write_deadline = Some(deadline);
            if let Some(clock) = &self.clock
                && let Ok(mut at) = clock.lock()
            {
                *at += self.consume;
            }
            if self.fail_write { Err(std::io::Error::other("scripted write failure")) } else { Ok(()) }
        }
        fn close_write(&mut self) {
            self.order.push("close");
        }
        fn drain(&mut self, deadline: Instant, max: u64) {
            self.order.push("drain");
            self.drain = Some((deadline, max));
        }
    }

    /// A writer that returns a scripted number of bytes per call and records each timeout.
    struct PartialWriter {
        steps: Vec<usize>,
        writes: usize,
        timeouts: Vec<Duration>,
        /// 1-based timeout call that fails; 0 means no failure.
        fail_at: usize,
    }

    impl DeadlineWrite for PartialWriter {
        fn set_write_timeout(&mut self, left: Option<Duration>) -> std::io::Result<()> {
            self.timeouts.push(left.unwrap_or_default());
            if self.fail_at != 0 && self.timeouts.len() == self.fail_at { Err(std::io::Error::other("scripted timeout failure")) } else { Ok(()) }
        }
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            let step = if self.steps.is_empty() { 0 } else { self.steps.remove(0) };
            Ok(step.min(buf.len()))
        }
    }

    /// A reader that returns a scripted number of bytes per call and records each timeout; its
    /// scripted clock advances after each read.
    struct ScriptedRead {
        steps: Vec<usize>,
        reads: usize,
        timeouts: Vec<Duration>,
        /// 1-based timeout call that fails; 0 means no failure.
        fail_at: usize,
        clock: Arc<Mutex<Instant>>,
        advance: Duration,
    }

    impl DeadlineRead for ScriptedRead {
        fn set_read_timeout(&mut self, left: Duration) -> std::io::Result<()> {
            self.timeouts.push(left);
            if self.fail_at != 0 && self.timeouts.len() == self.fail_at {
                Err(std::io::Error::other("scripted read timeout failure"))
            } else {
                Ok(())
            }
        }
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if let Ok(mut at) = self.clock.lock() {
                *at += self.advance;
            }
            let step = if self.steps.is_empty() { 0 } else { self.steps.remove(0) };
            Ok(step.min(buf.len()))
        }
    }

    /// Sets when its captured flag is dropped, so a refused job that must be closed can be seen.
    struct Mark(Arc<AtomicBool>);

    impl Drop for Mark {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    /// The reply and cleanup of a refusal share one absolute deadline: the exact existing reply
    /// first, then the end of what is sent, then the connection-limit drain. A scripted write
    /// failure still closes and drains, and a deadline that has passed writes nothing.
    #[test]
    fn a_refusal_writes_then_closes_before_a_bounded_limit_drain() {
        let now = Instant::now();
        let deadline = now + REFUSAL_BUDGET;
        let mut clock = || now;
        let mut io = Scripted::default();
        refuse_within_with(&mut io, &too_many_connections(), deadline, &mut clock);
        assert_eq!(io.order, ["reply", "close", "drain"]);
        assert_eq!(io.reply, Some(json!({"ok": false, "error": "too many connections"})), "the existing reply is unchanged");
        assert_eq!(io.write_deadline, Some(deadline), "the write sees the deadline set before the handoff");
        assert_eq!(io.drain.map(|(at, _)| at), Some(deadline), "the drain shares the absolute deadline");
        assert_eq!(io.drain.map(|(_, max)| max), Some(LIMIT_REFUSAL_DRAIN_BYTES));
        assert_eq!(LIMIT_REFUSAL_DRAIN_BYTES, MAX_LINE as u64 + 1, "a limit refusal drains one request line and its LF");
        assert_eq!(UNAUTHORIZED_DRAIN_BYTES, 64 * 1024, "the unauthorized path keeps its own 64 KiB ceiling");

        let mut io = Scripted { fail_write: true, ..Default::default() };
        refuse_within_with(&mut io, &too_many_connections(), deadline, &mut clock);
        assert_eq!(io.order, ["reply", "close", "drain"], "a failed reply still closes the write side and drains");

        let mut io = Scripted::default();
        refuse_within_with(&mut io, &too_many_connections(), now, &mut clock);
        assert!(io.order.is_empty(), "a refusal past its deadline writes nothing");
    }

    /// The unauthorized drain helper reads at most its own 64 KiB ceiling. The connection-limit
    /// path uses the separate `drain_deadline` helper, covered below.
    #[test]
    fn unauthorized_drain_stops_at_its_64_kib_ceiling() {
        for len in [0u64, 1, UNAUTHORIZED_DRAIN_BYTES, UNAUTHORIZED_DRAIN_BYTES + 100] {
            let mut reader = std::io::Cursor::new(vec![b'x'; len as usize]);
            assert_eq!(drain_refused(&mut reader, UNAUTHORIZED_DRAIN_BYTES), UNAUTHORIZED_DRAIN_BYTES.min(len));
        }
    }

    /// The connection-limit drain helper stops exactly at the byte ceiling: when the scripted
    /// chunk is larger than what remains, it reads only the remainder and makes no extra read.
    #[test]
    fn limit_drain_stops_exactly_at_its_byte_ceiling_with_no_extra_read() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(10);
        let clock = Arc::new(Mutex::new(start));
        let mut reader = ScriptedRead {
            steps: vec![8, 8, 8],
            reads: 0,
            timeouts: Vec::new(),
            fail_at: 0,
            clock: clock.clone(),
            advance: Duration::from_micros(100),
        };
        let mut now = {
            let clock = clock.clone();
            move || *clock.lock().unwrap()
        };
        let dropped = drain_deadline(&mut reader, 10, deadline, &mut now);
        assert_eq!(dropped, 10, "the helper drops exactly the ceiling");
        assert_eq!(reader.reads, 2, "the second read takes only the 2-byte remainder; no third read");
        assert_eq!(reader.timeouts.len(), 2);
    }

    /// The production limit-refusal path hands the limit ceiling, not the unauthorized ceiling,
    /// to its drain through the existing `RefusalIo` seam.
    #[test]
    fn the_limit_refusal_drain_uses_the_max_line_ceiling() {
        let start = Instant::now();
        let deadline = start + REFUSAL_BUDGET;
        let mut io = Scripted::default();
        let mut now = || start;
        refuse_within_with(&mut io, &too_many_connections(), deadline, &mut now);
        assert_eq!(io.drain, Some((deadline, LIMIT_REFUSAL_DRAIN_BYTES)), "the limit path passes its own ceiling");
        assert_eq!(LIMIT_REFUSAL_DRAIN_BYTES, MAX_LINE as u64 + 1);
        assert_ne!(LIMIT_REFUSAL_DRAIN_BYTES, UNAUTHORIZED_DRAIN_BYTES);
    }

    /// A partial write is followed by only what is left of the one absolute deadline, not another
    /// full timeout; the clock is scripted, so no sleep or scheduling threshold decides it.
    #[test]
    fn a_partial_refusal_write_recomputes_the_absolute_deadline() {
        let before = Instant::now();
        let deadline = before + REFUSAL_BUDGET;
        let mut clock = before;
        let mut now = move || {
            let at = clock;
            clock += Duration::from_millis(200);
            at
        };
        let mut writer = PartialWriter { steps: vec![2, 2, 2], writes: 0, timeouts: Vec::new(), fail_at: 0 };
        write_deadline(&mut writer, &[b'x'; 6], deadline, &mut now).unwrap();
        assert_eq!(writer.writes, 3);
        assert_eq!(
            writer.timeouts,
            [Duration::from_millis(500), Duration::from_millis(300), Duration::from_millis(100)],
            "each write gets what is left of the same deadline"
        );

        // Once the deadline is past, an incomplete write stops instead of extending it.
        let before = Instant::now();
        let deadline = before + Duration::from_millis(100);
        let mut clock = before;
        let mut now = move || {
            let at = clock;
            clock += Duration::from_millis(150);
            at
        };
        let mut writer = PartialWriter { steps: vec![1, 1], writes: 0, timeouts: Vec::new(), fail_at: 0 };
        let error = write_deadline(&mut writer, &[b'x'; 4], deadline, &mut now).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(writer.writes, 1);
        assert_eq!(writer.timeouts, [Duration::from_millis(100)]);
    }

    /// A timeout the platform refuses is not ignored: the write is not attempted, whether the
    /// setup fails on the first write or after a partial one.
    #[test]
    fn a_refusal_timeout_setup_failure_stops_before_the_write() {
        let now = Instant::now();
        let deadline = now + REFUSAL_BUDGET;
        let clock = now;
        let mut clock_now = move || clock;

        let mut writer = PartialWriter { steps: vec![6], writes: 0, timeouts: Vec::new(), fail_at: 1 };
        let error = write_deadline(&mut writer, &[b'x'; 6], deadline, &mut clock_now).unwrap_err();
        assert_eq!(writer.writes, 0, "no write after the first timeout setup failed");
        assert_eq!(writer.timeouts, [REFUSAL_BUDGET]);
        assert!(error.to_string().contains("scripted timeout failure"), "{error}");

        let clock = now;
        let mut clock_now = move || clock;
        let mut writer = PartialWriter { steps: vec![2, 4], writes: 0, timeouts: Vec::new(), fail_at: 2 };
        let error = write_deadline(&mut writer, &[b'x'; 6], deadline, &mut clock_now).unwrap_err();
        assert_eq!(writer.writes, 1, "the second write is not attempted after its timeout setup failed");
        assert_eq!(writer.timeouts, [REFUSAL_BUDGET, Duration::from_millis(500)]);
        assert!(error.to_string().contains("scripted timeout failure"), "{error}");
    }

    /// A sub-millisecond remainder fails closed before a socket timeout is configured: a platform
    /// may round it to zero, which can mean no timeout at all.
    #[test]
    fn a_sub_millisecond_refusal_fails_closed() {
        let now = Instant::now();
        let deadline = now + Duration::from_micros(500);
        assert!(refusal_time_left(deadline, now).is_none(), "sub-ms is not a safe socket timeout");
        assert_eq!(refusal_time_left(now + Duration::from_millis(2), now), Some(Duration::from_millis(2)));

        let mut clock = move || now;
        let mut io = Scripted::default();
        refuse_within_with(&mut io, &too_many_connections(), deadline, &mut clock);
        assert!(io.order.is_empty(), "no reply or drain starts with a sub-ms remainder");
    }

    /// The write path applies the same sub-millisecond floor: no timeout is configured and no
    /// bytes are written.
    #[test]
    fn a_sub_millisecond_write_fails_closed() {
        let now = Instant::now();
        let deadline = now + Duration::from_micros(500);
        let clock = now;
        let mut clock_now = move || clock;
        let mut writer = PartialWriter { steps: vec![6], writes: 0, timeouts: Vec::new(), fail_at: 0 };
        let error = write_deadline(&mut writer, &[b'x'; 6], deadline, &mut clock_now).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(writer.writes, 0);
        assert!(writer.timeouts.is_empty(), "no platform timeout is configured with a sub-ms remainder");
    }

    /// A successful reply that spends the whole budget leaves nothing for cleanup: no drain
    /// starts and no socket timeout or read is attempted.
    #[test]
    fn a_reply_that_consumes_the_budget_leaves_no_drain() {
        let start = Instant::now();
        let deadline = start + REFUSAL_BUDGET;
        let clock = Arc::new(Mutex::new(start));
        let mut io = Scripted { clock: Some(clock.clone()), consume: REFUSAL_BUDGET, ..Default::default() };
        let mut now = {
            let clock = clock.clone();
            move || *clock.lock().unwrap()
        };
        refuse_within_with(&mut io, &too_many_connections(), deadline, &mut now);
        assert_eq!(io.order, ["reply", "close"], "no drain after the reply spent the budget");
        assert!(io.drain.is_none(), "cleanup is not started with no budget left");
    }

    /// In-progress cleanup stops when less than a millisecond is left, before another timeout or
    /// read is attempted.
    #[test]
    fn cleanup_stops_on_a_sub_millisecond_budget_before_another_read() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(2);
        let clock = Arc::new(Mutex::new(start));
        let mut reader = ScriptedRead {
            steps: vec![4, 4],
            reads: 0,
            timeouts: Vec::new(),
            fail_at: 0,
            clock: clock.clone(),
            advance: Duration::from_micros(1500),
        };
        let mut now = {
            let clock = clock.clone();
            move || *clock.lock().unwrap()
        };
        let dropped = drain_deadline(&mut reader, 100, deadline, &mut now);
        assert_eq!(dropped, 4);
        assert_eq!(reader.reads, 1, "no second read with a sub-ms remainder");
        assert_eq!(reader.timeouts, [Duration::from_millis(2)], "the second timeout is not configured");
    }

    /// In-progress cleanup stops when a read timeout cannot be configured, before another read.
    #[test]
    fn cleanup_stops_on_a_read_timeout_error_before_another_read() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(10);
        let clock = Arc::new(Mutex::new(start));
        let mut reader =
            ScriptedRead { steps: vec![4, 4], reads: 0, timeouts: Vec::new(), fail_at: 2, clock: clock.clone(), advance: Duration::from_micros(100) };
        let mut now = {
            let clock = clock.clone();
            move || *clock.lock().unwrap()
        };
        let dropped = drain_deadline(&mut reader, 100, deadline, &mut now);
        assert_eq!(dropped, 4);
        assert_eq!(reader.reads, 1, "no second read after the timeout setup failed");
        assert_eq!(reader.timeouts.len(), 2, "the failing setup was attempted");
    }

    /// The handoff returns before the reply is delivered: a job held on a channel cannot delay the
    /// accepting thread. A watchdog, not an elapsed threshold, decides.
    #[test]
    fn a_refusal_handoff_does_not_wait_for_the_reply() {
        let refusals = Refusals::new();
        refusals.start();
        let owner = refusals.owner();
        let (started_tx, started_rx) = channel::<()>();
        let (release_tx, release_rx) = channel::<()>();
        let job: RefusalJob = Box::new(move || {
            let _ = started_tx.send(());
            let _ = release_rx.recv();
        });
        let (done_tx, done_rx) = channel::<()>();
        let handoff = std::thread::spawn(move || {
            owner.hand_off_job(job);
            let _ = done_tx.send(());
        });
        started_rx.recv_timeout(Duration::from_secs(5)).expect("the worker takes the refusal");
        done_rx.recv_timeout(Duration::from_secs(5)).expect("the handoff returned before the reply");
        release_tx.send(()).unwrap();
        handoff.join().unwrap();
    }

    /// The worker is fixed: while it is busy, one queued refusal waits, the next handoff fails at
    /// once (its connection closed), and both accepted jobs run on that same worker thread.
    #[test]
    fn a_full_refusal_slot_closes_the_next_at_once_without_another_worker() {
        let queue = Refusals::new();
        queue.start();
        assert_eq!(queue.starts.load(Ordering::Relaxed), 1);
        let refusals = queue.owner();
        let (first_tx, first_rx) = channel::<()>();
        let (second_tx, second_rx) = channel::<()>();
        let (first_started_tx, first_started_rx) = channel::<()>();
        let threads = Arc::new(Mutex::new(Vec::new()));
        let ran = Arc::new(AtomicUsize::new(0));
        let first = {
            let (threads, ran) = (threads.clone(), ran.clone());
            Box::new(move || {
                threads.lock().unwrap().push(std::thread::current().id());
                ran.fetch_add(1, Ordering::Relaxed);
                let _ = first_started_tx.send(());
                let _ = first_rx.recv();
            }) as RefusalJob
        };
        assert!(refusals.hand_off_job(first));
        first_started_rx.recv_timeout(Duration::from_secs(5)).expect("the worker takes the first refusal");
        let second = {
            let (threads, ran) = (threads.clone(), ran.clone());
            Box::new(move || {
                threads.lock().unwrap().push(std::thread::current().id());
                ran.fetch_add(1, Ordering::Relaxed);
                let _ = second_rx.recv();
            }) as RefusalJob
        };
        assert!(refusals.hand_off_job(second), "the one slot holds a second refusal while the worker is busy");
        let dropped = Arc::new(AtomicBool::new(false));
        let mark = Mark(dropped.clone());
        let third: RefusalJob = Box::new(move || {
            let _mark = mark;
        });
        let (done_tx, done_rx) = channel::<bool>();
        let handoff = {
            let refusals = refusals.clone();
            std::thread::spawn(move || {
                let accepted = refusals.hand_off_job(third);
                let _ = done_tx.send(accepted);
            })
        };
        assert!(!done_rx.recv_timeout(Duration::from_secs(5)).expect("the full handoff returned"), "the full worker refuses rather than queues");
        handoff.join().unwrap();
        assert!(dropped.load(Ordering::Relaxed), "the refused connection is closed at once");
        assert_eq!(queue.starts.load(Ordering::Relaxed), 1, "a full slot starts no worker");
        let _ = first_tx.send(());
        let _ = second_tx.send(());
        let guard = Instant::now() + Duration::from_secs(5);
        while ran.load(Ordering::Relaxed) < 2 && Instant::now() < guard {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(ran.load(Ordering::Relaxed), 2, "the waiting refusal is delivered after the first");
        let ids = threads.lock().unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ids[1], "one fixed worker delivers both refusals");
    }

    /// A listener with no worker, or one whose worker has ended, drops the refusal at once rather
    /// than queuing it or waiting.
    #[test]
    fn a_refusal_with_no_worker_or_a_closed_worker_fails_closed() {
        let absent = Refusals::new().owner();
        let dropped = Arc::new(AtomicBool::new(false));
        let mark = Mark(dropped.clone());
        let job: RefusalJob = Box::new(move || {
            let _mark = mark;
        });
        assert!(!absent.hand_off_job(job), "no worker means no reply");
        assert!(dropped.load(Ordering::Relaxed), "the refused connection is closed at once");

        let (worker, jobs) = sync_channel::<RefusalJob>(1);
        drop(jobs);
        let closed = RefusalOwner { worker: Some(worker) };
        let dropped = Arc::new(AtomicBool::new(false));
        let mark = Mark(dropped.clone());
        let job: RefusalJob = Box::new(move || {
            let _mark = mark;
        });
        assert!(!closed.hand_off_job(job), "a closed worker refuses rather than queues");
        assert!(dropped.load(Ordering::Relaxed), "the refused connection is closed at once");
    }

    /// Independent listener/conversation owners share the process's one refusal worker and one
    /// waiting slot: taking or dropping a handle starts no worker, a rejection starts no worker,
    /// and a surviving owner still reaches the same worker after another owner is torn down.
    #[test]
    fn refusal_owners_share_one_worker_and_one_bounded_queue() {
        let queue = Refusals::new();
        let early = queue.owner();
        assert_eq!(queue.starts.load(Ordering::Relaxed), 0, "taking an owner does not initialize");
        assert!(!early.hand_off_job(Box::new(|| {})), "an owner before initialization fails closed");
        assert_eq!(queue.starts.load(Ordering::Relaxed), 0, "a refusal does not initialize");

        queue.start();
        assert_eq!(queue.starts.load(Ordering::Relaxed), 1);
        let first_owner = queue.owner();
        let second_owner = queue.owner();
        queue.start();
        let later_owner = queue.owner();
        assert_eq!(queue.starts.load(Ordering::Relaxed), 1, "repeated listener initialization keeps one worker");

        let (first_started_tx, first_started_rx) = channel::<()>();
        let (release_tx, release_rx) = channel::<()>();
        let threads = Arc::new(Mutex::new(Vec::new()));
        let first = {
            let threads = threads.clone();
            Box::new(move || {
                threads.lock().unwrap().push(std::thread::current().id());
                let _ = first_started_tx.send(());
                let _ = release_rx.recv();
            }) as RefusalJob
        };
        assert!(first_owner.hand_off_job(first));
        first_started_rx.recv_timeout(Duration::from_secs(5)).expect("the shared worker takes the first refusal");

        let second = {
            let threads = threads.clone();
            Box::new(move || {
                threads.lock().unwrap().push(std::thread::current().id());
            }) as RefusalJob
        };
        assert!(second_owner.hand_off_job(second), "one shared waiting slot holds the second refusal");
        let dropped = Arc::new(AtomicBool::new(false));
        let mark = Mark(dropped.clone());
        let rejected: RefusalJob = Box::new(move || {
            let _mark = mark;
        });
        assert!(!second_owner.hand_off_job(rejected), "the shared slot rejects the next refusal");
        assert!(dropped.load(Ordering::Relaxed), "the rejected refusal is closed at once");
        assert_eq!(queue.starts.load(Ordering::Relaxed), 1, "a rejection starts no worker");

        drop(first_owner);
        drop(later_owner);
        assert_eq!(queue.starts.load(Ordering::Relaxed), 1, "owner teardown starts or stops no worker");

        let _ = release_tx.send(());
        let guard = Instant::now() + Duration::from_secs(5);
        while threads.lock().unwrap().len() < 2 && Instant::now() < guard {
            std::thread::sleep(Duration::from_millis(1));
        }
        let ids = threads.lock().unwrap();
        assert_eq!(ids.len(), 2, "the queued refusal is delivered after the first");
        assert_eq!(ids[0], ids[1], "one worker delivers both owners' refusals");
        let worker_id = ids[0];
        drop(ids);

        let (later_tx, later_rx) = channel::<std::thread::ThreadId>();
        let later: RefusalJob = Box::new(move || {
            let _ = later_tx.send(std::thread::current().id());
        });
        assert!(second_owner.hand_off_job(later), "a surviving owner still reaches the shared worker");
        let later_id = later_rx.recv_timeout(Duration::from_secs(5)).expect("the surviving owner's refusal runs");
        assert_eq!(later_id, worker_id, "the surviving owner stays on the one worker");
        assert_eq!(queue.starts.load(Ordering::Relaxed), 1, "owner teardown added no worker");
    }

    /// A refusal queued while the worker is busy keeps the deadline set before the handoff: once
    /// it passes it writes nothing, and the worker delivers it without extending the budget.
    #[test]
    fn a_queued_refusal_expires_without_extending_its_deadline() {
        let queue = Refusals::new();
        queue.start();
        let refusals = queue.owner();
        let (first_started_tx, first_started_rx) = channel::<()>();
        let (release_tx, release_rx) = channel::<()>();
        let first: RefusalJob = Box::new(move || {
            let _ = first_started_tx.send(());
            let _ = release_rx.recv();
        });
        assert!(refusals.hand_off_job(first));
        first_started_rx.recv_timeout(Duration::from_secs(5)).expect("the worker takes the first refusal");

        let deadline = Instant::now() + Duration::from_millis(25);
        let (done_tx, done_rx) = channel::<Vec<&'static str>>();
        let second: RefusalJob = Box::new(move || {
            let mut io = Scripted::default();
            refuse_within(&mut io, &too_many_connections(), deadline);
            let _ = done_tx.send(io.order);
        });
        assert!(refusals.hand_off_job(second), "the one slot holds the queued refusal");
        // Watchdog-wait for the queued deadline to pass; the outcome, not scheduling, is asserted.
        let guard = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            assert!(Instant::now() < guard, "the queued deadline watchdog expired");
            std::thread::sleep(Duration::from_millis(1));
        }
        release_tx.send(()).unwrap();
        assert!(
            done_rx.recv_timeout(Duration::from_secs(5)).expect("the queued refusal is delivered").is_empty(),
            "a queued refusal past its deadline writes nothing"
        );
    }
}
