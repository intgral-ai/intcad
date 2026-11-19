//! One bounded control lane independent of runner execution. No book/queue mutex is held
//! while acquiring a storage transaction. Dropping the lane requests shutdown without joining
//! a disk-blocked thread on the window thread.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::{CancellationStatus, ValidationBook, ValidationRunRecord, validation_port};

pub(super) const CAPACITY: usize = 64;
const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(super) struct Request {
    pub run: ValidationRunRecord,
    deadline: Instant,
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<Request>,
    active: HashSet<String>,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    stop: AtomicBool,
}

pub(super) struct Control {
    shared: Arc<Shared>,
}

impl Control {
    pub fn start(port: Arc<validation_port::ValidationPort>, book: Arc<Mutex<ValidationBook>>) -> Result<Self, String> {
        Self::start_with(port, book, |work| std::thread::Builder::new().name("validation-cancel-control".into()).spawn(work).map(|_| ()))
    }

    fn start_with(
        port: Arc<validation_port::ValidationPort>,
        book: Arc<Mutex<ValidationBook>>,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
    ) -> Result<Self, String> {
        let control = Self { shared: Arc::new(Shared::default()) };
        let shared = Arc::clone(&control.shared);
        spawn(Box::new(move || run(shared, port, book))).map_err(|error| format!("validation cancellation control could not start: {error}"))?;
        Ok(control)
    }

    pub fn enqueue(&self, run: ValidationRunRecord) -> Result<(), String> {
        let mut queue = self.shared.queue.lock().map_err(|_| "validation cancellation queue is unavailable".to_string())?;
        if self.shared.stop.load(Ordering::Acquire) {
            return Err("validation cancellation control is shut down; cancellation was not persisted".into());
        }
        if queue.active.contains(&run.run_id) {
            return Ok(());
        }
        if queue.active.len() >= CAPACITY {
            return Err("validation cancellation queue is full; cancellation was not persisted".into());
        }
        queue.active.insert(run.run_id.clone());
        queue.pending.push_back(Request { run, deadline: Instant::now() + DEADLINE });
        self.shared.ready.notify_all();
        Ok(())
    }

    pub fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.ready.notify_all();
    }

    #[cfg(test)]
    pub fn wait_idle(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut queue = self.shared.queue.lock().map_err(|_| "cancellation queue poisoned".to_string())?;
        while !queue.active.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("cancellation control did not settle".into());
            }
            queue = self.shared.ready.wait_timeout(queue, remaining).map_err(|_| "cancellation queue poisoned".to_string())?.0;
        }
        Ok(())
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn record(book: &Mutex<ValidationBook>, run: &ValidationRunRecord, result: Result<validation_port::ValidationState, String>) {
    let Ok(mut book) = book.lock() else {
        solidcraft_engine::note!("validation cancellation result could not be recorded");
        return;
    };
    let Some(current) = book.runs.get_mut(&run.run_id) else { return };
    if current.session_id != run.session_id || current.turn_id != run.turn_id || current.workflow_run_id != run.workflow_run_id {
        return;
    }
    current.cancellation = Some(match result {
        Ok(state) => {
            current.state = state;
            CancellationStatus::Persisted
        }
        Err(error) => CancellationStatus::Failed(error),
    });
    current.cancellation_dirty = true;
}

fn run(shared: Arc<Shared>, port: Arc<validation_port::ValidationPort>, book: Arc<Mutex<ValidationBook>>) {
    loop {
        let request = {
            let Ok(mut queue) = shared.queue.lock() else {
                solidcraft_engine::note!("validation cancellation queue is unavailable");
                return;
            };
            while queue.pending.is_empty() && !shared.stop.load(Ordering::Acquire) {
                #[cfg(test)]
                prewait_barrier();
                let Ok(next) = shared.ready.wait(queue) else {
                    solidcraft_engine::note!("validation cancellation queue is unavailable");
                    return;
                };
                queue = next;
            }
            let next = queue.pending.pop_front();
            if next.is_none() && shared.stop.load(Ordering::Acquire) {
                return;
            }
            next
        };
        let Some(request) = request else { continue };
        let result = port
            .cancel_until(&request.run.run_id, request.deadline, &shared.stop)
            .map(|result| result.state)
            .map_err(|error| format!("validation cancellation was not confirmed: {error}"));
        record(&book, &request.run, result);
        #[cfg(test)]
        completion_barrier();
        if let Ok(mut queue) = shared.queue.lock() {
            queue.active.remove(&request.run.run_id);
            shared.ready.notify_all();
        }
    }
}

#[cfg(test)]
thread_local! {
    static SUBMISSION_BARRIER: std::cell::RefCell<Option<Arc<std::sync::Barrier>>> = const { std::cell::RefCell::new(None) };
    static REGISTRATION_BARRIER: std::cell::RefCell<Option<Arc<std::sync::Barrier>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
static RUNNER_GATE: Mutex<Option<Arc<std::sync::Barrier>>> = Mutex::new(None);

#[cfg(test)]
pub(super) fn runner_gate() {
    let gate = RUNNER_GATE.lock().ok().and_then(|mut slot| slot.take());
    if let Some(gate) = gate {
        gate.wait();
        gate.wait();
    }
}

#[cfg(test)]
fn prewait_barrier() {
    let barrier = PREWAIT_BARRIER.lock().ok().and_then(|slot| slot.clone());
    if let Some(barrier) = barrier {
        barrier.wait();
        barrier.wait();
    }
}

#[cfg(test)]
fn completion_barrier() {
    let pair = COMPLETION_BARRIERS.lock().ok().and_then(|slot| slot.clone());
    if let Some(pair) = pair {
        let (reached, release) = &*pair;
        reached.wait();
        release.wait();
    }
}

#[cfg(test)]
static PREWAIT_BARRIER: Mutex<Option<Arc<std::sync::Barrier>>> = Mutex::new(None);

#[cfg(test)]
static COMPLETION_BARRIERS: Mutex<Option<Arc<(std::sync::Barrier, std::sync::Barrier)>>> = Mutex::new(None);

/// Pause the control worker immediately before it waits for work, while it still holds the queue
/// mutex. The test proves `shutdown` takes that same mutex instead of racing the wait predicate.
#[cfg(test)]
pub(super) fn install_prewait_barrier(barrier: Arc<std::sync::Barrier>) {
    if let Ok(mut slot) = PREWAIT_BARRIER.lock() {
        *slot = Some(barrier);
    }
}

#[cfg(test)]
pub(super) fn clear_prewait_barrier() {
    if let Ok(mut slot) = PREWAIT_BARRIER.lock() {
        *slot = None;
    }
}

/// Pause the control worker after it recorded a result but before it clears the active entry.
#[cfg(test)]
pub(super) fn install_completion_barriers(reached: Arc<std::sync::Barrier>, release: Arc<std::sync::Barrier>) {
    if let Ok(mut slot) = COMPLETION_BARRIERS.lock() {
        *slot = Some(Arc::new((reached, release)));
    }
}

#[cfg(test)]
pub(super) fn clear_completion_barriers() {
    if let Ok(mut slot) = COMPLETION_BARRIERS.lock() {
        *slot = None;
    }
}

#[cfg(test)]
pub(super) fn submission_barrier() {
    SUBMISSION_BARRIER.with(|barrier| {
        if let Some(barrier) = barrier.borrow_mut().take() {
            barrier.wait();
            barrier.wait();
        }
    });
}

#[cfg(test)]
pub(super) fn registration_barrier() {
    REGISTRATION_BARRIER.with(|barrier| {
        if let Some(barrier) = barrier.borrow_mut().take() {
            barrier.wait();
            barrier.wait();
        }
    });
}

#[cfg(test)]
pub(super) fn install_registration_barrier(barrier: Arc<std::sync::Barrier>) {
    REGISTRATION_BARRIER.with(|slot| *slot.borrow_mut() = Some(barrier));
}

#[cfg(test)]
pub(super) fn install_runner_gate(barrier: Arc<std::sync::Barrier>) {
    if let Ok(mut slot) = RUNNER_GATE.lock() {
        *slot = Some(barrier);
    }
}

#[cfg(test)]
pub(crate) struct Harness {
    control: Control,
    run: ValidationRunRecord,
}

#[cfg(test)]
impl Harness {
    pub(super) fn new(run: ValidationRunRecord) -> Self {
        Self { control: Control { shared: Arc::new(Shared::default()) }, run }
    }

    pub fn enqueue(&self, id: &str) -> Result<(), String> {
        let mut run = self.run.clone();
        run.run_id = id.into();
        self.control.enqueue(run)
    }

    pub fn shutdown(&self) {
        self.control.shutdown();
    }

    pub fn capacity(&self) -> usize {
        CAPACITY
    }
}

#[cfg(test)]
pub(super) fn install_submission_barrier(barrier: Arc<std::sync::Barrier>) {
    SUBMISSION_BARRIER.with(|slot| *slot.borrow_mut() = Some(barrier));
}

#[cfg(test)]
pub(super) fn start_failure(port: Arc<validation_port::ValidationPort>, book: Arc<Mutex<ValidationBook>>) -> String {
    match Control::start_with(port, book, |_| Err(std::io::Error::other("injected thread start refusal"))) {
        Ok(_) => "unexpectedly started".into(),
        Err(error) => error,
    }
}
