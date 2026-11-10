//! Host ports. These are interfaces only: Phase 0 does not bind a memory store or a persistent
//! task store. The native Chat adapter in `ui-egui` implements the CAD ones over the existing
//! engine, snapshot, validation and control paths.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contract::{CheckResult, DocumentRef, SnapshotPurpose, SnapshotRef, TaskEvent, ValidationProfileRef, ValidationReport};

/// A port failure. It is always reported, never turned into success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortError(pub String);

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PortError {}

impl PortError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Monotonic-enough time, injectable for deterministic budget tests.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
    }
}

/// A clock tests move by hand.
#[derive(Clone, Debug)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn new(now_ms: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now_ms)))
    }

    pub fn set(&self, now_ms: u64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }

    pub fn advance(&self, milliseconds: u64) {
        self.0.fetch_add(milliseconds, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The CAD side of the host: inspect, snapshot, exact checks. Every write itself still goes
/// through the host's existing gate/control queue; this port never opens a second write path.
pub trait ToolPort {
    fn inspect(&mut self, document: &DocumentRef) -> Result<String, PortError>;
    fn current_document(&mut self) -> Result<DocumentRef, PortError>;
    fn capture_snapshot(&mut self, document: &DocumentRef, purpose: SnapshotPurpose) -> Result<SnapshotRef, PortError>;
    fn run_checks(&mut self, document: &DocumentRef, required: &[String]) -> Result<Vec<CheckResult>, PortError>;
}

/// The trusted, headless validation side. A local implementation runs the engine; no LLM is
/// involved and the model cannot widen the profile.
pub trait ValidationPort {
    fn validate(&mut self, snapshot: &SnapshotRef, profile: &ValidationProfileRef) -> Result<ValidationReport, PortError>;
}

/// An explicit event/state store. Phase 0's implementation is in memory.
pub trait TaskStore: Send {
    fn record(&mut self, event: TaskEvent);
    fn events(&self) -> Vec<TaskEvent>;
}

/// Phase 0's explicit in-memory store. A later host can persist the same event stream.
#[derive(Default)]
pub struct InMemoryStore {
    events: Vec<TaskEvent>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TaskStore for InMemoryStore {
    fn record(&mut self, event: TaskEvent) {
        self.events.push(event);
    }

    fn events(&self) -> Vec<TaskEvent> {
        self.events.clone()
    }
}

/// Context only. Memory may help the model choose words; it is never CAD or check authority.
pub trait MemoryPort {
    fn recall(&self, scope: &str, query: &str) -> Vec<String>;
}

/// The Phase 0 default: no memory is bound.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoMemory;

impl MemoryPort for NoMemory {
    fn recall(&self, _scope: &str, _query: &str) -> Vec<String> {
        Vec::new()
    }
}
