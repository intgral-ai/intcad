//! Private, bounded Assistant transport and document authority. No native types.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

pub(super) type Pictures = Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>;

/// Remove capability-shaped credentials even when embedded in a native error.
pub(super) fn sanitized_diagnostic(text: &str) -> String {
    if text.len() > MAX_HEAD {
        return "Assistant diagnostic refused (oversize)".into();
    }
    let mut output = String::new();
    if output.try_reserve(text.len().min(MAX_HEAD)).is_err() {
        return "Assistant diagnostic unavailable".into();
    }
    let mut at = 0;
    while let Some(byte) = text.as_bytes().get(at) {
        if output.len() >= MAX_HEAD {
            return "Assistant diagnostic refused (oversize)".into();
        }
        let tail = text.get(at..).unwrap_or_default();
        if tail.starts_with("http://") || tail.starts_with("https://") {
            let end = tail.find(|ch: char| ch.is_whitespace() || "\"'<>,;)]}".contains(ch)).unwrap_or(tail.len());
            at += end;
            output.push_str("[private]");
        } else if byte.is_ascii_hexdigit() {
            let start = at;
            while text.as_bytes().get(at).is_some_and(u8::is_ascii_hexdigit) {
                at += 1;
            }
            if at - start >= 64 {
                output.push_str("[private]");
            } else if let Some(part) = text.get(start..at) {
                output.push_str(part);
            }
        } else {
            let Some(ch) = text.get(at..).and_then(|s| s.chars().next()) else { break };
            output.push(ch);
            at += ch.len_utf8();
        }
    }
    output
}
const MAX_BODY: usize = 400_000;
const MAX_HEAD: usize = 16_384;
const MAX_LINE: usize = 2_048;
const MAX_HEADER: usize = 8_192;
const MAX_HEADERS: usize = 64;
const MAX_ENTRIES: usize = 32;
const MAX_INBOX_BYTES: usize = 3_200_000;
const MAX_BATCH_ENTRIES: usize = 8;
const MAX_BATCH_BYTES: usize = 800_000;
const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DocumentId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BrowserOwnerId(pub u64);
pub(super) struct NavigationIdentity {
    pub document: DocumentId,
    pub owner: BrowserOwnerId,
    pub browser_id: i32,
    pub url: String,
}
pub(super) struct IpcEnvelope {
    pub document: DocumentId,
    pub body: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransportError {
    BadHead,
    BadFraming,
    Unauthorized,
    TooLarge,
    InvalidUtf8,
    QueueFull,
    StaleDocument,
    BrowserMismatch,
    PermitNotArmed,
    PermitConsumed,
    Deadline,
    Allocation,
    Stopped,
    Io,
}
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Categories only: never include request URLs, capabilities or bodies.
        write!(f, "Assistant transport: {self:?}")
    }
}
impl std::error::Error for TransportError {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Post,
}
pub(super) struct RequestHead {
    method: Method,
    target: String,
    host: String,
    origin: Option<String>,
    length: usize,
}
struct BrowserOwner {
    id: BrowserOwnerId,
    browser_id: Option<i32>,
    created: Instant,
    initializing: bool,
    revoked: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Permit {
    Pending,
    Armed,
    Claimed,
}
struct Document {
    id: DocumentId,
    owner: BrowserOwnerId,
    browser_id: i32,
    random: [u8; 32],
    url: Option<String>,
    namespace: Option<String>,
    permit: Permit,
}
#[derive(Default)]
struct AuthorityState {
    next_owner: u64,
    next_document: u64,
    owner: Option<BrowserOwner>,
    current: Option<Document>,
    last_random: Option<[u8; 32]>,
    host: Option<String>,
    inbox: VecDeque<IpcEnvelope>,
    inbox_bytes: usize,
    stopped: bool,
}
pub(super) struct DocumentAuthority {
    state: RwLock<AuthorityState>,
}
pub(super) struct DispatchLease<'a> {
    _held: RwLockReadGuard<'a, AuthorityState>,
}

impl DocumentAuthority {
    pub fn new() -> Self {
        Self { state: RwLock::new(AuthorityState::default()) }
    }
    pub fn reserve_browser_owner(&self) -> Result<BrowserOwnerId, TransportError> {
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        if state.stopped || state.owner.is_some() {
            return Err(TransportError::Stopped);
        }
        state.next_owner = state.next_owner.checked_add(1).ok_or(TransportError::Stopped)?;
        let id = BrowserOwnerId(state.next_owner);
        state.owner = Some(BrowserOwner { id, browser_id: None, created: Instant::now(), initializing: true, revoked: false });
        Ok(id)
    }
    pub fn bind_browser(&self, owner: BrowserOwnerId, id: i32) -> Result<(), TransportError> {
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        if state.stopped {
            return Err(TransportError::Stopped);
        }
        let held = state.owner.as_mut().filter(|o| o.id == owner && !o.revoked).ok_or(TransportError::BrowserMismatch)?;
        if held.initializing {
            remaining(Instant::now(), held.created + DEADLINE)?;
        }
        if id <= 0 || held.browser_id.is_some_and(|before| before != id) {
            return Err(TransportError::BrowserMismatch);
        }
        held.browser_id = Some(id);
        Ok(())
    }
    pub fn bound_browser(&self, owner: BrowserOwnerId) -> Result<Option<i32>, TransportError> {
        let state = self.state.read().map_err(|_| TransportError::Stopped)?;
        if state.stopped {
            return Err(TransportError::Stopped);
        }
        let held = state.owner.as_ref().filter(|o| o.id == owner && !o.revoked).ok_or(TransportError::BrowserMismatch)?;
        if held.initializing {
            remaining(Instant::now(), held.created + DEADLINE)?;
        }
        Ok(held.browser_id)
    }
    pub fn check_inert_navigation(&self, owner: BrowserOwnerId, id: i32, url: &str, main: bool, redirect: bool) -> Result<(), TransportError> {
        let state = self.state.read().map_err(|_| TransportError::Stopped)?;
        let held = state.owner.as_ref().filter(|o| o.id == owner && !o.revoked && o.initializing).ok_or(TransportError::BrowserMismatch)?;
        if state.stopped || id <= 0 || held.browser_id.is_some_and(|bound| bound != id) || url != "about:blank" || !main || redirect {
            return Err(TransportError::Unauthorized);
        }
        remaining(Instant::now(), held.created + DEADLINE)?;
        Ok(())
    }
    pub fn begin_document(&self, owner: BrowserOwnerId, random: [u8; 32]) -> Result<DocumentId, TransportError> {
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        if state.stopped || state.current.is_some() {
            return Err(TransportError::PermitNotArmed);
        }
        if state.last_random == Some(random) {
            return Err(TransportError::Unauthorized);
        }
        let held = state.owner.as_ref().filter(|o| o.id == owner && !o.revoked).ok_or(TransportError::BrowserMismatch)?;
        let browser_id = held.browser_id.ok_or(TransportError::BrowserMismatch)?;
        if held.initializing {
            remaining(Instant::now(), held.created + DEADLINE)?;
        }
        state.next_document = state.next_document.checked_add(1).ok_or(TransportError::Stopped)?;
        let id = DocumentId(state.next_document);
        state.current = Some(Document { id, owner, browser_id, random, url: None, namespace: None, permit: Permit::Pending });
        state.last_random = Some(random);
        Ok(id)
    }
    pub fn bootstrap_url(&self, document: DocumentId, port: u16) -> Result<String, TransportError> {
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        let host = format!("127.0.0.1:{port}");
        if port == 0 || state.stopped || state.host.as_ref().is_some_and(|old| old != &host) {
            return Err(TransportError::Unauthorized);
        }
        let current = state.current.as_mut().filter(|d| d.id == document).ok_or(TransportError::StaleDocument)?;
        if current.permit != Permit::Pending {
            return Err(TransportError::PermitConsumed);
        }
        let capability: String = current.random.iter().map(|byte| format!("{byte:02x}")).collect();
        let namespace = format!("/__assistant/{capability}/");
        let url = format!("http://{host}{namespace}index.html");
        current.namespace = Some(namespace);
        current.url = Some(url.clone());
        state.host = Some(host);
        Ok(url)
    }
    pub fn arm_main_navigation(&self, identity: NavigationIdentity) -> Result<(), TransportError> {
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        if state.stopped {
            return Err(TransportError::Stopped);
        }
        let held = state.owner.as_ref().filter(|o| o.id == identity.owner && !o.revoked).ok_or(TransportError::BrowserMismatch)?;
        if held.initializing {
            remaining(Instant::now(), held.created + DEADLINE)?;
        }
        let current = state.current.as_mut().ok_or(TransportError::StaleDocument)?;
        check_identity(current, &identity)?;
        if current.permit != Permit::Pending {
            return Err(TransportError::PermitConsumed);
        }
        current.permit = Permit::Armed;
        if let Some(owner) = &mut state.owner {
            owner.initializing = false;
        }
        Ok(())
    }
    pub fn navigation_identity(&self, owner: BrowserOwnerId, id: i32, url: &str) -> Result<NavigationIdentity, TransportError> {
        if url.len() > MAX_LINE {
            return Err(TransportError::TooLarge);
        }
        let state = self.state.read().map_err(|_| TransportError::Stopped)?;
        if state.stopped {
            return Err(TransportError::Stopped);
        }
        let current = state.current.as_ref().ok_or(TransportError::StaleDocument)?;
        let identity = NavigationIdentity { document: current.id, owner, browser_id: id, url: owned(url)? };
        check_identity(current, &identity)?;
        Ok(identity)
    }
    pub fn claim_main_navigation(&self, identity: &NavigationIdentity, main: bool, redirect: bool) -> Result<DocumentId, TransportError> {
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        if state.stopped || !main || redirect {
            return Err(TransportError::Unauthorized);
        }
        let current = state.current.as_mut().ok_or(TransportError::StaleDocument)?;
        check_identity(current, identity)?;
        match current.permit {
            Permit::Pending => return Err(TransportError::PermitNotArmed),
            Permit::Claimed => return Err(TransportError::PermitConsumed),
            Permit::Armed => current.permit = Permit::Claimed,
        }
        Ok(current.id)
    }
    pub fn check_resource(&self, owner: BrowserOwnerId, id: i32, url: &str, image: bool) -> Result<DocumentId, TransportError> {
        let state = self.state.read().map_err(|_| TransportError::Stopped)?;
        let current = live_document(&state)?;
        if current.owner != owner || current.browser_id != id {
            return Err(TransportError::BrowserMismatch);
        }
        let host = state.host.as_deref().ok_or(TransportError::Unauthorized)?;
        let origin = format!("http://{host}");
        if image && (url.starts_with("data:image/") || url.starts_with(&format!("blob:{origin}/"))) {
            return Ok(current.id);
        }
        let target = url.strip_prefix(&origin).ok_or(TransportError::Unauthorized)?;
        validate_target(target)?;
        private_route(current, target)?;
        Ok(current.id)
    }
    pub fn authorize_http(&self, head: &RequestHead) -> Result<DocumentId, TransportError> {
        let state = self.state.read().map_err(|_| TransportError::Stopped)?;
        let current = live_document(&state)?;
        let host = state.host.as_deref().ok_or(TransportError::Unauthorized)?;
        let origin = format!("http://{host}");
        if head.host != host || head.origin.as_deref().is_some_and(|o| o != origin) {
            return Err(TransportError::Unauthorized);
        }
        let route = private_route(current, &head.target)?;
        match head.method {
            Method::Post if route == "ipc" && head.origin.as_deref() == Some(origin.as_str()) => Ok(current.id),
            Method::Get if route != "ipc" => Ok(current.id),
            _ => Err(TransportError::Unauthorized),
        }
    }
    pub fn permits(&self, document: DocumentId) -> bool {
        self.dispatch_lease(document).is_ok()
    }
    pub fn dispatch_lease(&self, document: DocumentId) -> Result<DispatchLease<'_>, TransportError> {
        let held = self.state.read().map_err(|_| TransportError::Stopped)?;
        if live_document(&held)?.id != document {
            return Err(TransportError::StaleDocument);
        }
        Ok(DispatchLease { _held: held })
    }
    pub fn revoke(&self) {
        // Poisoned authority is never read by a successful auth check. Clear it
        // here as well so its inbox cannot retain business data indefinitely.
        let mut state = self.state.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.current = None;
        state.inbox.clear();
        state.inbox_bytes = 0;
        if let Some(owner) = &mut state.owner {
            if owner.initializing {
                owner.revoked = true;
            }
        }
    }
    fn enqueue(&self, document: DocumentId, body: String) -> Result<(), TransportError> {
        if body.len() > MAX_BODY {
            return Err(TransportError::TooLarge);
        }
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        if live_document(&state)?.id != document {
            return Err(TransportError::StaleDocument);
        }
        let bytes = state.inbox_bytes.checked_add(body.len()).ok_or(TransportError::QueueFull)?;
        if bytes > MAX_INBOX_BYTES || state.inbox.len() >= MAX_ENTRIES {
            return Err(TransportError::QueueFull);
        }
        state.inbox.try_reserve(1).map_err(|_| TransportError::Allocation)?;
        state.inbox.push_back(IpcEnvelope { document, body });
        state.inbox_bytes = bytes;
        Ok(())
    }
    fn take_batch(&self) -> Result<Vec<IpcEnvelope>, TransportError> {
        let mut state = self.state.write().map_err(|_| TransportError::Stopped)?;
        let mut batch = Vec::new();
        batch.try_reserve(MAX_BATCH_ENTRIES).map_err(|_| TransportError::Allocation)?;
        let mut bytes = 0usize;
        while batch.len() < MAX_BATCH_ENTRIES {
            let Some(entry) = state.inbox.front() else { break };
            let next = bytes.checked_add(entry.body.len()).ok_or(TransportError::Allocation)?;
            if next > MAX_BATCH_BYTES {
                break;
            }
            let Some(entry) = state.inbox.pop_front() else { break };
            state.inbox_bytes = state.inbox_bytes.saturating_sub(entry.body.len());
            bytes = next;
            batch.push(entry);
        }
        Ok(batch)
    }
}
fn live_document(state: &AuthorityState) -> Result<&Document, TransportError> {
    if state.stopped {
        return Err(TransportError::Stopped);
    }
    state.current.as_ref().filter(|d| d.permit == Permit::Claimed).ok_or(TransportError::StaleDocument)
}
fn check_identity(current: &Document, identity: &NavigationIdentity) -> Result<(), TransportError> {
    if current.id != identity.document {
        return Err(TransportError::StaleDocument);
    }
    if current.owner != identity.owner || current.browser_id != identity.browser_id {
        return Err(TransportError::BrowserMismatch);
    }
    if current.url.as_deref() != Some(identity.url.as_str()) {
        return Err(TransportError::Unauthorized);
    }
    Ok(())
}
fn private_route<'a>(current: &Document, target: &'a str) -> Result<&'a str, TransportError> {
    let namespace = current.namespace.as_deref().ok_or(TransportError::Unauthorized)?;
    let route = target.strip_prefix(namespace).filter(|r| !r.is_empty()).ok_or(TransportError::Unauthorized)?;
    if route.contains('/') && !route.strip_prefix("picture/").is_some_and(|k| !k.is_empty() && !k.contains('/')) {
        return Err(TransportError::Unauthorized);
    }
    Ok(route)
}
fn owned(text: &str) -> Result<String, TransportError> {
    let mut out = String::new();
    out.try_reserve(text.len()).map_err(|_| TransportError::Allocation)?;
    out.push_str(text);
    Ok(out)
}
fn decode_path(text: &str) -> Result<String, TransportError> {
    let mut bytes = Vec::new();
    bytes.try_reserve(text.len()).map_err(|_| TransportError::Allocation)?;
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        let value = if byte == b'%' {
            let high = input.next().and_then(|b| char::from(b).to_digit(16)).ok_or(TransportError::BadHead)?;
            let low = input.next().and_then(|b| char::from(b).to_digit(16)).ok_or(TransportError::BadHead)?;
            let decoded = u8::try_from(high * 16 + low).map_err(|_| TransportError::BadHead)?;
            if b"/\\%?#".contains(&decoded) || decoded.is_ascii_control() {
                return Err(TransportError::BadHead);
            }
            decoded
        } else {
            byte
        };
        bytes.push(value);
    }
    String::from_utf8(bytes).map_err(|_| TransportError::InvalidUtf8)
}
fn validate_target(target: &str) -> Result<(), TransportError> {
    if target.len() > MAX_LINE {
        return Err(TransportError::TooLarge);
    }
    if !target.starts_with('/')
        || target.starts_with("//")
        || target.contains("//")
        || target.bytes().any(|b| !b.is_ascii() || b <= b' ' || b == 127 || b"\\?#".contains(&b))
    {
        return Err(TransportError::BadHead);
    }
    let decoded = decode_path(target)?;
    if decoded.split('/').any(|part| matches!(part, "." | "..")) {
        return Err(TransportError::BadHead);
    }
    // Encoded names are needed only for a single picture key. No encoding of a
    // namespace, capability or static resource can acquire an alternate spelling.
    if target.contains('%') && !target.split_once("/picture/").is_some_and(|(base, key)| !base.contains('%') && !key.contains('/')) {
        return Err(TransportError::BadHead);
    }
    Ok(())
}
fn parse_head(bytes: &[u8], expected_host: &str) -> Result<RequestHead, TransportError> {
    if bytes.len() > MAX_HEAD {
        return Err(TransportError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| TransportError::InvalidUtf8)?;
    let text = text.strip_suffix("\r\n\r\n").ok_or(TransportError::BadHead)?;
    if text.contains("\r\n\r\n") {
        return Err(TransportError::BadFraming);
    }
    let mut lines = text.split("\r\n");
    let request = lines.next().ok_or(TransportError::BadHead)?;
    if request.len() + 2 > MAX_LINE {
        return Err(TransportError::TooLarge);
    }
    let mut parts = request.split(' ');
    let method = match parts.next() {
        Some("GET") => Method::Get,
        Some("POST") => Method::Post,
        _ => return Err(TransportError::BadFraming),
    };
    let target = parts.next().ok_or(TransportError::BadHead)?;
    validate_target(target)?;
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        return Err(TransportError::BadHead);
    }
    let (mut host, mut origin, mut length) = (None, None, None);
    for (i, line) in lines.enumerate() {
        if i >= MAX_HEADERS || line.len() + 2 > MAX_HEADER {
            return Err(TransportError::TooLarge);
        }
        let (name, value) = line.split_once(':').ok_or(TransportError::BadHead)?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.bytes().any(|b| b < b' ' || b == 127)
        {
            return Err(TransportError::BadHead);
        }
        let value = value.trim_matches(' ');
        match name.to_ascii_lowercase().as_str() {
            "host" => {
                if host.is_some() || value != expected_host {
                    return Err(TransportError::Unauthorized);
                }
                host = Some(owned(value)?);
            }
            "origin" => {
                if origin.is_some() {
                    return Err(TransportError::BadFraming);
                }
                origin = Some(owned(value)?);
            }
            "content-length" => {
                if length.is_some() || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(TransportError::BadFraming);
                }
                let mut n = 0usize;
                for byte in value.bytes() {
                    n = n.checked_mul(10).and_then(|n| n.checked_add(usize::from(byte - b'0'))).ok_or(TransportError::TooLarge)?;
                }
                if n > MAX_BODY {
                    return Err(TransportError::TooLarge);
                }
                length = Some(n);
            }
            "transfer-encoding" | "upgrade" | "expect" => return Err(TransportError::BadFraming),
            _ => {}
        }
    }
    if (method == Method::Post && length.is_none()) || (method == Method::Get && length.is_some_and(|n| n != 0)) {
        return Err(TransportError::BadFraming);
    }
    Ok(RequestHead { method, target: owned(target)?, host: host.ok_or(TransportError::Unauthorized)?, origin, length: length.unwrap_or(0) })
}
fn remaining(now: Instant, deadline: Instant) -> Result<Duration, TransportError> {
    deadline.checked_duration_since(now).filter(|d| !d.is_zero()).ok_or(TransportError::Deadline)
}

#[derive(Clone, Copy)]
pub(super) struct PageAssets {
    pub resolve: fn(&str) -> Option<(&'static str, &'static [u8])>,
    pub font: &'static [u8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ServerProgress {
    Running,
    Stopping,
    Stopped,
    Failed,
}
const WORKERS: usize = 4;
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data: blob:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-src 'none'; object-src 'none'; worker-src 'none'";
pub(super) const IPC_SHIM: &str = r#"window.ipc = { postMessage(message) {
  fetch(new URL('ipc', document.baseURI), { method: 'POST', body: message })
    .then(r => { if (!r.ok) throw new Error('Assistant transport refused'); })
    .catch(() => { window.__intcadTransportFailed = true;
      window.dispatchEvent(new Event('intcad-transport-failed')); });
} };
"#;

struct ServerShared {
    stop: std::sync::atomic::AtomicBool,
    fatal: std::sync::atomic::AtomicBool,
    sockets: Mutex<[Option<std::net::TcpStream>; WORKERS]>,
    failure: Mutex<Option<TransportError>>,
    authority: Arc<DocumentAuthority>,
    assets: PageAssets,
    index: Arc<Vec<u8>>,
    pictures: Pictures,
    host: String,
    #[cfg(test)]
    finish_gate: Option<Arc<WorkerGate>>,
}

/// Test-only gate: keeps a worker's join handle unfinished until the test has
/// observed the retained owner, so "still stopping" is deterministic.
#[cfg(test)]
struct WorkerGate {
    held: Mutex<bool>,
    released: std::sync::Condvar,
}
#[cfg(test)]
impl WorkerGate {
    fn new() -> Arc<Self> {
        Arc::new(Self { held: Mutex::new(true), released: std::sync::Condvar::new() })
    }
    fn wait(&self) {
        let mut held = self.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *held {
            held = self.released.wait(held).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
    fn release(&self) {
        *self.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        self.released.notify_all();
    }
}
struct StartOptions {
    worker_limit: usize,
    #[cfg(test)]
    finish_gate: Option<Arc<WorkerGate>>,
}
impl StartOptions {
    fn standard() -> Self {
        Self { worker_limit: WORKERS, #[cfg(test)] finish_gate: None }
    }
    #[cfg(test)]
    fn partial(worker_limit: usize, gate: Arc<WorkerGate>) -> Self {
        Self { worker_limit, finish_gate: Some(gate) }
    }
}
impl ServerShared {
    fn failure(&self, error: TransportError) {
        if let Ok(mut held) = self.failure.lock() {
            if held.is_none() {
                *held = Some(error);
            }
        }
    }
}
pub(super) struct PageServer {
    port: u16,
    listener: Option<std::net::TcpListener>,
    workers: Vec<std::thread::JoinHandle<()>>,
    shared: Arc<ServerShared>,
}
impl PageServer {
    pub fn start(assets: PageAssets, pictures: Pictures, authority: Arc<DocumentAuthority>) -> std::io::Result<Self> {
        Self::start_with(assets, pictures, authority, StartOptions::standard())
    }
    fn start_with(assets: PageAssets, pictures: Pictures, authority: Arc<DocumentAuthority>, options: StartOptions) -> std::io::Result<Self> {
        let StartOptions { worker_limit, #[cfg(test)] finish_gate } = options;
        let (_, raw) = (assets.resolve)("index.html").ok_or_else(|| std::io::Error::other("Assistant index unavailable"))?;
        let index = Arc::new(index_with_bridge(raw).map_err(std::io::Error::other)?);
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let mut workers = Vec::new();
        workers.try_reserve(WORKERS).map_err(std::io::Error::other)?;
        let shared = Arc::new(ServerShared {
            stop: false.into(),
            fatal: false.into(),
            sockets: Mutex::new(std::array::from_fn(|_| None)),
            failure: Mutex::new(None),
            authority,
            assets,
            index,
            pictures,
            host: format!("127.0.0.1:{port}"),
            #[cfg(test)]
            finish_gate,
        });
        let mut server = Self { port, listener: Some(listener), workers, shared };
        for slot in 0..WORKERS {
            if slot >= worker_limit {
                // Partial pool: the same stopping-owner handoff a failed spawn
                // performs below. The caller must retain and poll this server.
                server.shared.failure(TransportError::Io);
                server.shared.fatal.store(true, std::sync::atomic::Ordering::Release);
                server.request_stop();
                break;
            }
            let listener = match server.listener.as_ref().map(std::net::TcpListener::try_clone) {
                Some(Ok(listener)) => listener,
                _ => {
                    server.shared.failure(TransportError::Io);
                    server.shared.fatal.store(true, std::sync::atomic::Ordering::Release);
                    server.request_stop();
                    break;
                }
            };
            let shared = server.shared.clone();
            match std::thread::Builder::new().name(format!("assistant-http-{slot}")).spawn(move || worker(listener, shared, slot)) {
                Ok(worker) => server.workers.push(worker),
                Err(_) => {
                    // Returning Err after spawning would detach the partial pool.
                    // Return this stopping owner instead; caller must poll/reap.
                    server.shared.failure(TransportError::Io);
                    server.shared.fatal.store(true, std::sync::atomic::Ordering::Release);
                    server.request_stop();
                    break;
                }
            }
        }
        Ok(server)
    }
    #[cfg(test)]
    fn start_partial(assets: PageAssets, pictures: Pictures, authority: Arc<DocumentAuthority>, worker_limit: usize, gate: Arc<WorkerGate>) -> std::io::Result<Self> {
        Self::start_with(assets, pictures, authority, StartOptions::partial(worker_limit, gate))
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn take_batch(&mut self) -> Result<Vec<IpcEnvelope>, TransportError> {
        self.shared.authority.take_batch()
    }
    pub fn request_stop(&mut self) {
        self.shared.stop.store(true, std::sync::atomic::Ordering::Release);
        self.shared.authority.revoke();
        {
            let mut held = self.shared.authority.state.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            held.stopped = true;
        }
        self.listener = None;
        {
            let held = self.shared.sockets.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            for stream in held.iter().flatten() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
        // Reap handles that already completed before this stop. A worker that is
        // still running is left to the asynchronous poll_stop progression; it is
        // never joined here.
        self.reap_finished();
    }
    /// Join only workers that have already finished. Running workers are left
    /// owned by `workers` for a later `poll_stop`; this never blocks on one.
    fn reap_finished(&mut self) {
        let mut at = 0;
        while at < self.workers.len() {
            if self.workers.get(at).is_some_and(std::thread::JoinHandle::is_finished) {
                let worker = self.workers.swap_remove(at);
                if worker.join().is_err() {
                    self.shared.failure(TransportError::Io);
                    self.shared.fatal.store(true, std::sync::atomic::Ordering::Release);
                }
            } else {
                at += 1;
            }
        }
    }
    pub fn poll_stop(&mut self) -> ServerProgress {
        self.reap_finished();
        if self.shared.stop.load(std::sync::atomic::Ordering::Acquire) {
            return if self.workers.is_empty() { ServerProgress::Stopped } else { ServerProgress::Stopping };
        }
        if self.shared.fatal.load(std::sync::atomic::Ordering::Acquire) { ServerProgress::Failed } else { ServerProgress::Running }
    }
    pub fn take_failure(&mut self) -> Option<TransportError> {
        self.shared.failure.lock().ok().and_then(|mut held| held.take())
    }
}
impl Drop for PageServer {
    fn drop(&mut self) {
        self.request_stop();
        // Never join a running worker on the UI thread. Normal/partial-init
        // ownership must keep this server until poll_stop completes (W3).
        if self.workers.iter().any(|worker| !worker.is_finished()) {
            eprintln!("Assistant page server dropped before owned stop completion");
        }
    }
}
/// A failed-start server the app keeps until its owned workers have stopped.
/// Dropping this before `Stopped` would detach the partial pool, so only the
/// stopped server is released; the native owners stay retained elsewhere.
pub(super) struct FailedStartServer(Option<PageServer>);
impl FailedStartServer {
    pub fn retain(mut server: PageServer) -> Self {
        server.request_stop();
        Self(Some(server))
    }
    pub fn advance(&mut self) -> ServerProgress {
        let Some(server) = self.0.as_mut() else {
            return ServerProgress::Stopped;
        };
        let progress = server.poll_stop();
        if progress == ServerProgress::Stopped {
            // All owned handles are joined now, so releasing the Rust server
            // cannot detach a worker. Native owners stay retained elsewhere.
            self.0 = None;
        }
        progress
    }
}
fn worker(listener: std::net::TcpListener, shared: Arc<ServerShared>, slot: usize) {
    use std::sync::atomic::Ordering;
    while !shared.stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let deadline = Instant::now() + DEADLINE;
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let Ok(cancellation) = stream.try_clone() else { continue };
                {
                    let Ok(mut held) = shared.sockets.lock() else { break };
                    let Some(place) = held.get_mut(slot) else { break };
                    *place = Some(cancellation);
                }
                if shared.stop.load(Ordering::Acquire) {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                } else {
                    let _ = serve_connection(&mut stream, &shared, deadline);
                }
                if let Ok(mut held) = shared.sockets.lock() {
                    if let Some(place) = held.get_mut(slot) {
                        *place = None;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                if !shared.stop.load(Ordering::Acquire) {
                    shared.failure(TransportError::Io);
                    shared.fatal.store(true, Ordering::Release);
                }
                break;
            }
        }
    }
    #[cfg(test)]
    if let Some(gate) = shared.finish_gate.as_ref() {
        gate.wait();
    }
}
fn read_head(stream: &mut std::net::TcpStream, deadline: Instant) -> Result<Vec<u8>, TransportError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    bytes.try_reserve(MAX_HEAD).map_err(|_| TransportError::Allocation)?;
    let mut line = 0usize;
    let mut first = true;
    loop {
        if bytes.len() >= MAX_HEAD {
            return Err(TransportError::TooLarge);
        }
        stream.set_read_timeout(Some(remaining(Instant::now(), deadline)?)).map_err(|_| TransportError::Io)?;
        let mut byte = [0];
        if stream.read(&mut byte).map_err(|_| TransportError::Io)? != 1 {
            return Err(TransportError::BadHead);
        }
        bytes.push(byte[0]);
        line += 1;
        if line > if first { MAX_LINE } else { MAX_HEADER } {
            return Err(TransportError::TooLarge);
        }
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
        if bytes.ends_with(b"\r\n") {
            line = 0;
            first = false;
        }
    }
}
fn read_body(stream: &mut std::net::TcpStream, length: usize, deadline: Instant) -> Result<String, TransportError> {
    use std::io::Read;
    if length > MAX_BODY {
        return Err(TransportError::TooLarge);
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| TransportError::Allocation)?;
    bytes.resize(length, 0);
    let mut at = 0;
    while at < length {
        stream.set_read_timeout(Some(remaining(Instant::now(), deadline)?)).map_err(|_| TransportError::Io)?;
        let read = stream.read(bytes.get_mut(at..).ok_or(TransportError::TooLarge)?).map_err(|_| TransportError::Io)?;
        if read == 0 {
            return Err(TransportError::BadFraming);
        }
        at = at.checked_add(read).ok_or(TransportError::TooLarge)?;
    }
    String::from_utf8(bytes).map_err(|_| TransportError::InvalidUtf8)
}
fn reject_pipelining(stream: &std::net::TcpStream) -> Result<(), TransportError> {
    stream.set_nonblocking(true).map_err(|_| TransportError::Io)?;
    let mut byte = [0];
    let peeked = stream.peek(&mut byte);
    stream.set_nonblocking(false).map_err(|_| TransportError::Io)?;
    match peeked {
        Ok(0) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
        Ok(_) => Err(TransportError::BadFraming),
        Err(_) => Err(TransportError::Io),
    }
}
enum Payload {
    Static(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}
impl Payload {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Static(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}
fn response(stream: &mut std::net::TcpStream, status: u16, kind: &str, payload: Payload) -> Result<(), TransportError> {
    let deadline = Instant::now() + DEADLINE;
    let header = format!(
        "HTTP/1.1 {status} Response\r\nContent-Type: {kind}\r\nContent-Security-Policy: {CSP}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        payload.bytes().len()
    );
    write_bytes(stream, header.as_bytes(), deadline)?;
    write_bytes(stream, payload.bytes(), deadline)
}
fn write_bytes(stream: &mut std::net::TcpStream, mut bytes: &[u8], deadline: Instant) -> Result<(), TransportError> {
    use std::io::Write;
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(Instant::now(), deadline)?)).map_err(|_| TransportError::Io)?;
        let wrote = stream.write(bytes).map_err(|_| TransportError::Io)?;
        if wrote == 0 {
            return Err(TransportError::Io);
        }
        bytes = bytes.get(wrote..).ok_or(TransportError::Io)?;
    }
    Ok(())
}
fn refusal(stream: &mut std::net::TcpStream, error: TransportError) -> Result<(), TransportError> {
    let status = match error {
        TransportError::Unauthorized | TransportError::StaleDocument | TransportError::PermitNotArmed => 404,
        TransportError::TooLarge => 413,
        TransportError::QueueFull | TransportError::Stopped | TransportError::Allocation => 503,
        _ => 400,
    };
    response(stream, status, "text/plain; charset=utf-8", Payload::Static(b"request refused"))
}
fn serve_connection(stream: &mut std::net::TcpStream, shared: &ServerShared, deadline: Instant) -> Result<(), TransportError> {
    let head = match read_head(stream, deadline).and_then(|bytes| parse_head(&bytes, &shared.host)) {
        Ok(head) => head,
        Err(error) => return refusal(stream, error),
    };
    // Authority/framing checks precede any allocation based on Content-Length.
    let document = match shared.authority.authorize_http(&head) {
        Ok(document) => document,
        Err(error) => return refusal(stream, error),
    };
    let body = match read_body(stream, head.length, deadline) {
        Ok(body) => body,
        Err(error) => return refusal(stream, error),
    };
    if let Err(error) = reject_pipelining(stream) {
        return refusal(stream, error);
    }
    if head.method == Method::Post {
        return match shared.authority.enqueue(document, body) {
            Ok(()) => response(stream, 200, "text/plain; charset=utf-8", Payload::Static(b"ok")),
            Err(error) => {
                shared.failure(error);
                refusal(stream, error)
            }
        };
    }
    let route = (|| {
        let state = shared.authority.state.read().map_err(|_| TransportError::Stopped)?;
        let current = live_document(&state)?;
        if current.id != document {
            return Err(TransportError::StaleDocument);
        }
        owned(private_route(current, &head.target)?)
    })();
    let route = match route {
        Ok(route) => route,
        Err(error) => return refusal(stream, error),
    };
    let (kind, payload) = match route.as_str() {
        "index.html" => ("text/html; charset=utf-8", Payload::Shared(shared.index.clone())),
        "ipc.js" => ("text/javascript; charset=utf-8", Payload::Static(IPC_SHIM.as_bytes())),
        "intcad-plex.ttf" => ("font/ttf", Payload::Static(shared.assets.font)),
        name if name.starts_with("picture/") => {
            let key = decode_path(name.strip_prefix("picture/").ok_or(TransportError::BadHead)?)?;
            let bytes = shared.pictures.lock().ok().and_then(|held| held.get(&key).cloned());
            let Some(bytes) = bytes else { return refusal(stream, TransportError::Unauthorized) };
            let kind = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                "image/png"
            } else if bytes.starts_with(&[255, 216, 255]) {
                "image/jpeg"
            } else if bytes.starts_with(b"RIFF") {
                "image/webp"
            } else {
                "application/octet-stream"
            };
            (kind, Payload::Shared(bytes))
        }
        name => match (shared.assets.resolve)(name) {
            Some((kind, bytes)) => (kind, Payload::Static(bytes)),
            None => return refusal(stream, TransportError::Unauthorized),
        },
    };
    response(stream, 200, kind, payload)
}
pub(super) fn index_with_bridge(html: &[u8]) -> Result<Vec<u8>, TransportError> {
    let text = std::str::from_utf8(html).map_err(|_| TransportError::InvalidUtf8)?;
    let text = text.replace("connect-src 'none'", "connect-src 'self'");
    let shim = r#"<script src="ipc.js"></script>"#;
    let at = text.find("<script type=\"module\"").or_else(|| text.find("</head>")).ok_or(TransportError::BadHead)?;
    let mut output = Vec::new();
    output.try_reserve(text.len().checked_add(shim.len()).ok_or(TransportError::Allocation)?).map_err(|_| TransportError::Allocation)?;
    output.extend_from_slice(text.get(..at).ok_or(TransportError::BadHead)?.as_bytes());
    output.extend_from_slice(shim.as_bytes());
    output.extend_from_slice(text.get(at..).ok_or(TransportError::BadHead)?.as_bytes());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_errors_never_log_capability_or_business_text() {
        let secret = "abcd".repeat(16);
        let text = format!("failure http://127.0.0.1:43123/__assistant/{secret}/index.html; secret={secret}; {}", secret.to_uppercase());
        let clean = sanitized_diagnostic(&text);
        assert!(!clean.contains(&secret));
        assert!(!clean.contains(&secret.to_uppercase()));
        assert!(clean.contains("[private]"));
        assert!(!clean.contains("http://127.0.0.1:43123/"));
        assert!(!clean.contains("/__assistant/"));
        assert_eq!(sanitized_diagnostic("error https://outside.invalid/navigation"), "error [private]");
        assert_eq!(sanitized_diagnostic(&"x".repeat(MAX_HEAD + 1)), "Assistant diagnostic refused (oversize)");
        assert_eq!(sanitized_diagnostic("CEF_VIEWPORT {\"scale\":1.25}"), "CEF_VIEWPORT {\"scale\":1.25}");
    }

    fn bound(a: &DocumentAuthority) -> BrowserOwnerId {
        let owner = a.reserve_browser_owner().unwrap();
        a.bind_browser(owner, 7).unwrap();
        owner
    }
    fn arm(a: &DocumentAuthority, owner: BrowserOwnerId, random: u8) -> (DocumentId, String) {
        let doc = a.begin_document(owner, [random; 32]).unwrap();
        let url = a.bootstrap_url(doc, 43123).unwrap();
        a.arm_main_navigation(NavigationIdentity { document: doc, owner, browser_id: 7, url: url.clone() }).unwrap();
        (doc, url)
    }
    fn claimed(a: &DocumentAuthority) -> (BrowserOwnerId, DocumentId, String) {
        let owner = bound(a);
        let (doc, url) = arm(a, owner, 1);
        let identity = a.navigation_identity(owner, 7, &url).unwrap();
        assert_eq!(a.claim_main_navigation(&identity, true, false), Ok(doc));
        (owner, doc, url)
    }
    fn head(method: &str, target: &str, extra: &str) -> Vec<u8> {
        format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:43123\r\n{extra}\r\n").into_bytes()
    }
    #[test]
    fn main_navigation_permit_is_single_use_for_exact_identity() {
        let a = DocumentAuthority::new();
        let owner = bound(&a);
        let (doc, url) = arm(&a, owner, 1);
        let identity = a.navigation_identity(owner, 7, &url).unwrap();
        assert!(!a.permits(doc));
        assert!(a.claim_main_navigation(&identity, false, false).is_err());
        assert!(a.claim_main_navigation(&identity, true, true).is_err());
        assert!(a.navigation_identity(owner, 8, &url).is_err());
        assert!(a.navigation_identity(BrowserOwnerId(999), 7, &url).is_err());
        assert!(a.navigation_identity(owner, 7, &(url.clone() + "#replay")).is_err());
        assert_eq!(a.claim_main_navigation(&identity, true, false), Ok(doc));
        assert_eq!(a.claim_main_navigation(&identity, true, false), Err(TransportError::PermitConsumed));
        assert!(a.arm_main_navigation(identity).is_err());
        assert!(a.permits(doc));
    }
    #[test]
    fn inert_blank_before_or_after_bind_has_no_authority() {
        let a = DocumentAuthority::new();
        let owner = a.reserve_browser_owner().unwrap();
        assert!(a.check_inert_navigation(owner, 7, "about:blank", true, false).is_ok());
        assert!(a.begin_document(owner, [1; 32]).is_err());
        assert!(a.check_inert_navigation(owner, 7, "about:blank#x", true, false).is_err());
        assert!(a.check_inert_navigation(owner, 7, "about:blank", false, false).is_err());
        a.bind_browser(owner, 7).unwrap();
        a.bind_browser(owner, 7).unwrap();
        assert_eq!(a.bound_browser(owner).unwrap(), Some(7));
        assert!(a.bind_browser(owner, 8).is_err());
        assert!(a.check_resource(owner, 7, "about:blank", false).is_err());
        let (doc, _) = arm(&a, owner, 1);
        assert!(!a.permits(doc));
        assert!(a.check_inert_navigation(owner, 7, "about:blank", true, false).is_err());
        a.revoke();
        assert!(a.bind_browser(BrowserOwnerId(999), 7).is_err());
    }
    #[test]
    fn resource_or_ipc_first_does_not_consume_navigation_permit() {
        let a = DocumentAuthority::new();
        let owner = bound(&a);
        let (doc, url) = arm(&a, owner, 1);
        let bytes = head(
            "POST",
            &url["http://127.0.0.1:43123".len()..].replace("index.html", "ipc"),
            "Origin: http://127.0.0.1:43123\r\nContent-Length: 2\r\n",
        );
        let h = parse_head(&bytes, "127.0.0.1:43123").unwrap();
        assert!(a.authorize_http(&h).is_err());
        assert!(a.check_resource(owner, 7, &url.replace("index.html", "app.js"), false).is_err());
        assert!(a.enqueue(doc, "{}".into()).is_err());
        let identity = a.navigation_identity(owner, 7, &url).unwrap();
        assert_eq!(a.claim_main_navigation(&identity, true, false), Ok(doc));
        assert_eq!(a.authorize_http(&h), Ok(doc));
        for _ in 0..3 {
            assert_eq!(a.check_resource(owner, 7, &url.replace("index.html", "app.js"), false), Ok(doc));
        }
        assert!(a.check_resource(owner, 7, "https://outside.invalid/app.js", false).is_err());
        assert!(a.check_resource(owner, 7, "data:text/html,evil", false).is_err());
        assert_eq!(a.check_resource(owner, 7, "data:image/png;base64,AA==", true), Ok(doc));
    }
    #[test]
    fn private_bootstrap_and_every_asset_require_current_capability() {
        let a = DocumentAuthority::new();
        let (_, doc, url) = claimed(&a);
        for route in ["index.html", "app.js", "app.css", "ipc.js", "intcad-plex.ttf", "picture/ab-cd"] {
            let target = url["http://127.0.0.1:43123".len()..].replace("index.html", route);
            assert_eq!(a.authorize_http(&parse_head(&head("GET", &target, ""), "127.0.0.1:43123").unwrap()), Ok(doc));
            for bad in [target.replace(&"01".repeat(32), &"02".repeat(32)), format!("/{route}")] {
                assert!(a.authorize_http(&parse_head(&head("GET", &bad, ""), "127.0.0.1:43123").unwrap()).is_err());
            }
        }
        a.revoke();
        assert!(!a.permits(doc));
        assert!(a.authorize_http(&parse_head(&head("GET", &url["http://127.0.0.1:43123".len()..], ""), "127.0.0.1:43123").unwrap()).is_err());
    }
    #[test]
    fn ipc_requires_capability_exact_host_and_origin() {
        let a = DocumentAuthority::new();
        let (_, doc, url) = claimed(&a);
        let target = url["http://127.0.0.1:43123".len()..].replace("index.html", "ipc");
        for origin in ["", "Origin: http://outside.invalid\r\n", "Origin: http://127.0.0.1:1\r\n"] {
            let bytes = head("POST", &target, &(origin.to_owned() + "Content-Length: 2\r\n"));
            assert!(a.authorize_http(&parse_head(&bytes, "127.0.0.1:43123").unwrap()).is_err());
        }
        let bytes = head("POST", &target, "Origin: http://127.0.0.1:43123\r\nContent-Length: 2\r\n");
        assert_eq!(a.authorize_http(&parse_head(&bytes, "127.0.0.1:43123").unwrap()), Ok(doc));
        assert!(
            parse_head(&String::from_utf8(bytes).unwrap().replace("Host: 127.0.0.1:43123", "Host: localhost:43123").into_bytes(), "127.0.0.1:43123")
                .is_err()
        );
    }
    #[test]
    fn hostile_http_head_never_allocates_body() {
        assert!(parse_head(&head("GET", "/index.html", ""), "127.0.0.1:43123").is_ok());
        for (method, target, extra) in [
            ("GET", "/", "Content-Length: 1\r\n"),
            ("POST", "/ipc", ""),
            ("POST", "/ipc", "Content-Length: +1\r\n"),
            ("POST", "/ipc", "Content-Length: -1\r\n"),
            ("POST", "/ipc", "Content-Length: 184467440737095516160\r\n"),
            ("POST", "/ipc", "Content-Length: 400001\r\n"),
            ("POST", "/ipc", "Content-Length: 1\r\nContent-Length: 1\r\n"),
            ("POST", "/ipc", "Content-Length: 1\r\nTransfer-Encoding: chunked\r\n"),
            ("GET", "/", "Upgrade: websocket\r\n"),
            ("GET", "/", "Expect: 100-continue\r\n"),
            ("GET", "/", "Origin: x\r\nOrigin: y\r\n"),
            ("GET", "/", " Content-Length: 0\r\n"),
            ("GET", "http://127.0.0.1:43123/", ""),
            ("GET", "/../private", ""),
            ("GET", "/picture/%2fsecret", ""),
            ("GET", "/picture/%252f", ""),
            ("GET", "/picture/%00", ""),
            ("GET", "/index.html?token=secret", ""),
        ] {
            assert!(parse_head(&head(method, target, extra), "127.0.0.1:43123").is_err(), "{method} {target}");
        }
        for extra in ["X: a\r\n".repeat(65), format!("X: {}\r\n", "a".repeat(8192)), "X: a\r\n".repeat(4096)] {
            assert!(parse_head(&head("GET", "/", &extra), "127.0.0.1:43123").is_err());
        }
        assert!(parse_head(&head("GET", &format!("/{}", "a".repeat(2049)), ""), "127.0.0.1:43123").is_err());
        assert!(parse_head(b"GET / HTTP/1.1\r\nHost: \xff\r\n\r\n", "127.0.0.1:43123").is_err());
        assert!(parse_head(b"GET / HTTP/1.1\r\nHost: 127.0.0.1:43123\r\n\r\nGET /", "127.0.0.1:43123").is_err());
    }
    #[test]
    fn deadlines_are_absolute_across_progress() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(5);
        assert_eq!(remaining(start + Duration::from_secs(4), deadline), Ok(Duration::from_secs(1)));
        assert_eq!(remaining(deadline, deadline), Err(TransportError::Deadline));
        assert_eq!(remaining(deadline + Duration::from_secs(1), deadline), Err(TransportError::Deadline));
    }
    #[test]
    fn queue_limits_count_bytes_and_drain() {
        let a = DocumentAuthority::new();
        let (_, doc, _) = claimed(&a);
        for _ in 0..8 {
            a.enqueue(doc, "x".repeat(400_000)).unwrap();
        }
        assert_eq!(a.enqueue(doc, "x".into()), Err(TransportError::QueueFull));
        let batch = a.take_batch().unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.iter().map(|e| e.body.len()).sum::<usize>(), 800_000);
        for _ in 0..3 {
            assert_eq!(a.take_batch().unwrap().len(), 2);
        }
        for i in 0..32 {
            a.enqueue(doc, i.to_string()).unwrap();
        }
        assert_eq!(a.enqueue(doc, "full".into()), Err(TransportError::QueueFull));
        let batch = a.take_batch().unwrap();
        assert_eq!(batch.len(), 8);
        assert_eq!(batch[7].body, "7");
        assert_eq!(a.enqueue(doc, "x".repeat(400_001)), Err(TransportError::TooLarge));
    }
    #[test]
    fn revocation_discards_queued_ready_and_approval() {
        let a = DocumentAuthority::new();
        let (owner, doc, _) = claimed(&a);
        a.enqueue(doc, "ready".into()).unwrap();
        a.enqueue(doc, "approval".into()).unwrap();
        a.revoke();
        assert!(a.take_batch().unwrap().is_empty());
        assert!(a.enqueue(doc, "racing old message".into()).is_err());
        let (next, url) = arm(&a, owner, 2);
        assert_ne!(next, doc);
        let identity = a.navigation_identity(owner, 7, &url).unwrap();
        assert_eq!(a.claim_main_navigation(&identity, true, false), Ok(next));
        assert!(a.dispatch_lease(doc).is_err());
        assert!(a.dispatch_lease(next).is_ok());
    }
    #[test]
    fn two_concurrent_main_claims_have_one_winner() {
        let a = Arc::new(DocumentAuthority::new());
        let owner = bound(&a);
        let (_, url) = arm(&a, owner, 1);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let joins: Vec<_> = (0..2)
            .map(|_| {
                let (a, barrier, url) = (a.clone(), barrier.clone(), url.clone());
                std::thread::spawn(move || {
                    let identity = a.navigation_identity(owner, 7, &url).unwrap();
                    barrier.wait();
                    a.claim_main_navigation(&identity, true, false).is_ok()
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(joins.into_iter().map(|j| usize::from(j.join().unwrap())).sum::<usize>(), 1);
    }
    #[test]
    fn native_private_urls_are_bounded_before_resolution() {
        let a = DocumentAuthority::new();
        let (owner, _, url) = claimed(&a);
        let large = url.replace("index.html", &"x".repeat(MAX_LINE + 1));
        assert_eq!(a.check_resource(owner, 7, &large, false), Err(TransportError::TooLarge));
        assert!(matches!(a.navigation_identity(owner, 7, &large), Err(TransportError::TooLarge)));
    }
    #[test]
    fn revocation_linearizes_against_navigation_claim() {
        let a = Arc::new(DocumentAuthority::new());
        let owner = bound(&a);
        let (doc, url) = arm(&a, owner, 1);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let other = {
            let (a, barrier) = (a.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                a.navigation_identity(owner, 7, &url).and_then(|identity| a.claim_main_navigation(&identity, true, false))
            })
        };
        barrier.wait();
        a.revoke();
        let _ = other.join().unwrap();
        assert!(!a.permits(doc));
        assert!(a.enqueue(doc, "stale approval".into()).is_err());
    }
    #[test]
    fn dispatch_lease_excludes_revocation_until_action_finishes() {
        let a = DocumentAuthority::new();
        let (_, doc, _) = claimed(&a);
        let lease = a.dispatch_lease(doc).unwrap();
        assert!(matches!(a.state.try_write(), Err(std::sync::TryLockError::WouldBlock)));
        drop(lease);
        a.revoke();
        assert!(a.dispatch_lease(doc).is_err());
    }
    #[test]
    fn inert_creation_deadline_replay_and_late_binding_fail_closed() {
        let a = DocumentAuthority::new();
        let owner = a.reserve_browser_owner().unwrap();
        a.state.write().unwrap().owner.as_mut().unwrap().created = Instant::now() - DEADLINE;
        assert_eq!(a.check_inert_navigation(owner, 7, "about:blank", true, false), Err(TransportError::Deadline));
        assert_eq!(a.bind_browser(owner, 7), Err(TransportError::Deadline));
        a.revoke();
        assert!(a.bind_browser(owner, 7).is_err());
        assert!(a.begin_document(owner, [1; 32]).is_err());
    }
    #[test]
    fn reload_cannot_reuse_claimed_document() {
        let a = DocumentAuthority::new();
        let (owner, old, url) = claimed(&a);
        let identity = a.navigation_identity(owner, 7, &url).unwrap();
        assert_eq!(a.claim_main_navigation(&identity, true, false), Err(TransportError::PermitConsumed));
        a.revoke();
        assert!(a.begin_document(owner, [1; 32]).is_err());
        let (fresh, fresh_url) = arm(&a, owner, 2);
        assert_ne!(old, fresh);
        assert_ne!(url, fresh_url);
        assert!(a.claim_main_navigation(&identity, true, false).is_err());
        let fresh_identity = a.navigation_identity(owner, 7, &fresh_url).unwrap();
        assert_eq!(a.claim_main_navigation(&fresh_identity, true, false), Ok(fresh));
    }

    fn assets(name: &str) -> Option<(&'static str, &'static [u8])> {
        match name {
            "index.html" => Some(("text/html", b"<head><script type=\"module\" src=\"./app.js\"></script></head><body></body>")),
            "app.js" => Some(("text/javascript", b"export const example = 1;")),
            "app.css" => Some(("text/css", b"body { color: black; }")),
            _ => None,
        }
    }
    fn server() -> (PageServer, Arc<DocumentAuthority>, DocumentId, String) {
        let authority = Arc::new(DocumentAuthority::new());
        let pictures = Arc::new(Mutex::new(HashMap::from([("large".into(), Arc::new(vec![7; 3_750_000]))])));
        let server = PageServer::start(PageAssets { resolve: assets, font: b"font" }, pictures, authority.clone()).unwrap();
        let owner = bound(&authority);
        let document = authority.begin_document(owner, [1; 32]).unwrap();
        let url = authority.bootstrap_url(document, server.port()).unwrap();
        let identity = NavigationIdentity { document, owner, browser_id: 7, url: url.clone() };
        authority.arm_main_navigation(identity).unwrap();
        let identity = authority.navigation_identity(owner, 7, &url).unwrap();
        authority.claim_main_navigation(&identity, true, false).unwrap();
        let path = url.split_once("/__assistant/").unwrap().1;
        let prefix = format!("/__assistant/{}", path.strip_suffix("index.html").unwrap());
        (server, authority, document, prefix)
    }
    fn request(port: u16, method: &str, target: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
        write!(stream, "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}\r\n").unwrap();
        stream.write_all(body).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        response
    }
    fn stop(server: &mut PageServer) {
        server.request_stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.poll_stop() == ServerProgress::Stopping {
            assert!(Instant::now() < deadline, "owned workers did not stop");
            std::thread::yield_now();
        }
        assert_eq!(server.poll_stop(), ServerProgress::Stopped);
    }
    #[test]
    fn actual_embedded_files_are_private_and_bootstrap_shim_precedes_module() {
        // Actual production resolver and shipped bytes, not an ABI/native test.
        let authority = Arc::new(DocumentAuthority::new());
        let mut server = PageServer::start(
            PageAssets { resolve: crate::assistant_page::page_file, font: b"font" },
            Arc::new(Mutex::new(HashMap::new())),
            authority.clone(),
        )
        .unwrap();
        let owner = bound(&authority);
        let document = authority.begin_document(owner, [1; 32]).unwrap();
        let url = authority.bootstrap_url(document, server.port()).unwrap();
        authority.arm_main_navigation(NavigationIdentity { document, owner, browser_id: 7, url: url.clone() }).unwrap();
        let prefix = url.strip_prefix(&format!("http://127.0.0.1:{}", server.port())).unwrap().strip_suffix("index.html").unwrap();
        assert!(request(server.port(), "GET", &(prefix.to_owned() + "index.html"), "", b"").starts_with(b"HTTP/1.1 404 "));
        let identity = authority.navigation_identity(owner, 7, &url).unwrap();
        authority.claim_main_navigation(&identity, true, false).unwrap();
        let names = [
            "index.html",
            "assistant.js",
            "style.css",
            "CommitMono-400-Regular.woff2",
            "CommitMono-700-Regular.woff2",
            "MonaSans-VariableFont_wdth-wght.ttf",
        ];
        for name in names {
            assert!(request(server.port(), "GET", &format!("/{name}"), "", b"").starts_with(b"HTTP/1.1 404 "));
            let response = request(server.port(), "GET", &(prefix.to_owned() + name), "", b"");
            assert!(response.starts_with(b"HTTP/1.1 200 "), "{name}");
            let payload_at = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let payload = &response[payload_at..];
            if name == "index.html" {
                let html = std::str::from_utf8(payload).unwrap();
                assert!(html.find("<script src=\"ipc.js\">").unwrap() < html.find("<script type=\"module\"").unwrap());
            } else {
                assert_eq!(payload, crate::assistant_page::page_file(name).unwrap().1);
            }
        }
        authority.revoke();
        for name in names {
            assert!(request(server.port(), "GET", &(prefix.to_owned() + name), "", b"").starts_with(b"HTTP/1.1 404 "));
        }
        stop(&mut server);
    }

    #[test]
    fn server_authenticates_before_body_read_and_serves_only_current_assets() {
        let (mut server, authority, _, prefix) = server();
        let port = server.port();
        let response = request(port, "POST", "/ipc", "Content-Length: 400000\r\n", b"");
        assert!(response.starts_with(b"HTTP/1.1 404 "));
        for name in ["index.html", "app.js", "app.css", "ipc.js", "intcad-plex.ttf", "picture/large"] {
            let response = request(port, "GET", &(prefix.clone() + name), "", b"");
            assert!(response.starts_with(b"HTTP/1.1 200 "), "{name}");
            let text = String::from_utf8_lossy(&response);
            assert!(text.contains("Referrer-Policy: no-referrer\r\n"));
            assert!(text.contains("X-Content-Type-Options: nosniff\r\n"));
            if name == "index.html" {
                let shim = text.find("<script src=\"ipc.js\">").unwrap();
                assert!(shim < text.find("<script type=\"module\"").unwrap());
            }
            if name == "picture/large" {
                assert!(response.len() > 3_750_000);
            }
        }
        authority.revoke();
        assert!(request(port, "GET", &(prefix + "app.js"), "", b"").starts_with(b"HTTP/1.1 404 "));
        stop(&mut server);
    }
    #[test]
    fn server_strict_utf8_business_boundary_and_visible_backpressure() {
        let (mut server, authority, document, prefix) = server();
        let port = server.port();
        let extra = format!("Origin: http://127.0.0.1:{port}\r\nContent-Length: 400000\r\n");
        assert!(request(port, "POST", &(prefix.clone() + "ipc"), &extra, &vec![b'x'; 400_000]).starts_with(b"HTTP/1.1 200 "));
        let batch = server.take_batch().unwrap();
        assert_eq!((batch.len(), batch[0].document, batch[0].body.len()), (1, document, 400_000));
        let extra = format!("Origin: http://127.0.0.1:{port}\r\nContent-Length: 400001\r\n");
        assert!(request(port, "POST", &(prefix.clone() + "ipc"), &extra, b"").starts_with(b"HTTP/1.1 413 "));
        let extra = format!("Origin: http://127.0.0.1:{port}\r\nContent-Length: 1\r\n");
        assert!(request(port, "POST", &(prefix.clone() + "ipc"), &extra, &[255]).starts_with(b"HTTP/1.1 400 "));
        assert!(server.take_batch().unwrap().is_empty());
        for _ in 0..32 {
            authority.enqueue(document, "old".into()).unwrap();
        }
        assert!(request(port, "POST", &(prefix + "ipc"), &extra, b"x").starts_with(b"HTTP/1.1 503 "));
        assert_eq!(server.take_failure(), Some(TransportError::QueueFull));
        stop(&mut server);
    }
    #[test]
    fn server_stop_reaps_owned_workers_and_rejects_enqueue() {
        use std::io::{Read, Write};
        // Idle and active readers must both be interrupted, and the owned pool
        // must be reaped without joining a worker that is still running.
        let authority = Arc::new(DocumentAuthority::new());
        // Larger than any loopback socket buffer, so the serving worker really
        // holds the response lease while the stop interrupts it.
        let inner = Arc::new(vec![7u8; 16 * 1024 * 1024]);
        let pictures: Pictures = Arc::new(Mutex::new(HashMap::from([("huge".to_string(), inner.clone())])));
        let mut server = PageServer::start(PageAssets { resolve: assets, font: b"font" }, pictures, authority.clone()).unwrap();
        let owner = bound(&authority);
        let document = authority.begin_document(owner, [1; 32]).unwrap();
        let url = authority.bootstrap_url(document, server.port()).unwrap();
        authority.arm_main_navigation(NavigationIdentity { document, owner, browser_id: 7, url: url.clone() }).unwrap();
        let identity = authority.navigation_identity(owner, 7, &url).unwrap();
        authority.claim_main_navigation(&identity, true, false).unwrap();
        let prefix = format!("/__assistant/{}", url.split_once("/__assistant/").unwrap().1.strip_suffix("index.html").unwrap());
        let port = server.port();
        let baseline = Arc::strong_count(&inner);
        assert_eq!(server.workers.len(), WORKERS, "owned pool did not start");
        // Idle reader: connected, no request bytes yet.
        let _idle = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        // Active reader: holding a live response lease on the large picture.
        let mut slow = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        slow.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        write!(slow, "GET {prefix}picture/huge HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
        let mut head = [0u8; 256];
        assert!(slow.read(&mut head).unwrap() > 0);
        let wait = Instant::now() + Duration::from_secs(2);
        while Arc::strong_count(&inner) <= baseline {
            assert!(Instant::now() < wait, "worker never leased the picture");
            std::thread::yield_now();
        }
        server.request_stop();
        assert!(authority.enqueue(document, "approval".into()).is_err(), "stop accepted a new enqueue");
        let start = Instant::now();
        let mut slowest = Duration::ZERO;
        loop {
            let poll = Instant::now();
            let progress = server.poll_stop();
            slowest = slowest.max(poll.elapsed());
            if progress != ServerProgress::Stopping {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2), "owned workers did not stop");
            std::thread::yield_now();
        }
        assert!(slowest < Duration::from_millis(500), "poll joined an unfinished worker: {slowest:?}");
        assert_eq!(server.poll_stop(), ServerProgress::Stopped);
        assert!(server.workers.is_empty(), "owned handles were not reaped");
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "owned port stayed open");
        assert_eq!(Arc::strong_count(&inner), baseline, "picture lease survived stop");
        assert_eq!(server.take_failure(), None);
        assert!(server.take_batch().unwrap().is_empty());
        drop((slow, _idle));

        // A repeated stop (what shutdown and Drop perform) must reap workers
        // that have already finished instead of detaching them.
        let (mut again, _, _, _) = self::server();
        again.request_stop();
        let wait = Instant::now() + Duration::from_secs(2);
        while !again.workers.iter().all(std::thread::JoinHandle::is_finished) {
            assert!(Instant::now() < wait, "workers did not finish after stop");
            std::thread::yield_now();
        }
        again.request_stop();
        assert!(again.workers.is_empty(), "repeated stop did not reap finished workers");
        assert_eq!(again.poll_stop(), ServerProgress::Stopped);
    }
    #[test]
    fn retained_failed_start_releases_its_server_only_after_owned_workers_stop() {
        // A partial pool returned as Ok(stopping_server) by production start,
        // retained through the same wrapper the failed-owner handoff uses.
        let authority = Arc::new(DocumentAuthority::new());
        let owner = bound(&authority);
        let document = authority.begin_document(owner, [1; 32]).unwrap();
        let pictures: Pictures = Arc::new(Mutex::new(HashMap::new()));
        let witness = Arc::downgrade(&pictures);
        let gate = WorkerGate::new();
        let server = PageServer::start_partial(PageAssets { resolve: assets, font: b"font" }, pictures, authority.clone(), 1, gate.clone()).unwrap();
        let mut retained = FailedStartServer::retain(server);
        assert!(authority.enqueue(document, "approval".into()).is_err(), "failed start did not revoke its queue");
        assert_eq!(retained.advance(), ServerProgress::Stopping);
        assert!(witness.upgrade().is_some(), "picture ownership was released while an owned worker was unfinished");
        gate.release();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let progress = retained.advance();
            if progress == ServerProgress::Stopped {
                break;
            }
            assert_eq!(progress, ServerProgress::Stopping, "retained server left the stopping progression");
            assert!(Instant::now() < deadline, "owned worker did not stop");
            std::thread::yield_now();
        }
        assert!(witness.upgrade().is_none(), "stopped Rust server still retained its pictures");
        assert_eq!(retained.advance(), ServerProgress::Stopped, "released server did not stay released");
    }
}
