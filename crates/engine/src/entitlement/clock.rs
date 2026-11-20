//! Conservative monotonic time and authorization deadlines.

#[cfg(test)]
use std::cell::Cell;
use std::time::{Duration, SystemTime};

/// A reading from [`MonotonicClock`]. It advances across suspend when the host can account for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonotonicInstant(Duration);

impl MonotonicInstant {
    pub fn from_millis(millis: u64) -> Self {
        Self(Duration::from_millis(millis))
    }

    pub fn as_duration(self) -> Duration {
        self.0
    }

    pub fn saturating_add(self, other: Duration) -> Self {
        Self(self.0.saturating_add(other))
    }

    pub fn saturating_sub(self, other: Duration) -> Self {
        Self(self.0.saturating_sub(other))
    }
}

/// Host time. `now` must include suspend; `wall_now` is for diagnostics only.
pub trait MonotonicClock {
    fn now(&self) -> MonotonicInstant;
    fn wall_now(&self) -> SystemTime;
}

/// A deadline anchored when the request was sent, never when its response arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConservativeDeadline {
    anchored_at_send: MonotonicInstant,
    expires_at: MonotonicInstant,
}

impl ConservativeDeadline {
    pub fn new(sent_at: MonotonicInstant, server_remaining: Duration) -> Self {
        Self { anchored_at_send: sent_at, expires_at: sent_at.saturating_add(server_remaining) }
    }

    /// Remaining time before the half-open deadline, or `None` at/after it or on a clock anomaly.
    pub fn remaining(&self, now: MonotonicInstant) -> Option<Duration> {
        if now < self.anchored_at_send || now >= self.expires_at {
            return None;
        }
        Some(self.expires_at.as_duration().saturating_sub(now.as_duration()))
    }

    /// The half-open deadline instant. For a server-known expiry this is the local upper bound.
    pub fn expires_at(&self) -> MonotonicInstant {
        self.expires_at
    }

    /// Whether a request at `at` is at or after the known deadline. Unlike [`Self::remaining`], a
    /// clock that moved backwards before the anchor does not turn an early request into an
    /// expired one; only the local known expiry decides.
    pub fn expired_at(&self, at: MonotonicInstant) -> bool {
        at >= self.expires_at
    }

    /// A valid renewal re-anchors at send. A late, rejected or anomalous exchange keeps the old bound.
    pub fn renewed(&self, attempt: &RenewalAttempt, server_remaining: Duration) -> Result<ConservativeDeadline, DeadlineError> {
        if !attempt.still_valid {
            return Err(DeadlineError::Rejected);
        }
        if attempt.old_deadline != *self || attempt.sent_at < self.anchored_at_send || attempt.received_at < attempt.sent_at {
            return Err(DeadlineError::ClockAnomaly);
        }
        if attempt.sent_at >= self.expires_at || attempt.received_at >= self.expires_at {
            return Err(DeadlineError::Expired);
        }
        Ok(Self::new(attempt.sent_at, server_remaining))
    }
}

/// One renewal exchange, with both client-side timestamps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenewalAttempt {
    pub sent_at: MonotonicInstant,
    pub received_at: MonotonicInstant,
    pub old_deadline: ConservativeDeadline,
    pub still_valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadlineError {
    Expired,
    Rejected,
    ClockAnomaly,
}

/// A controllable test clock. Suspend advances time; backward movement makes deadlines fail closed.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct FakeMonotonicClock {
    elapsed: Cell<Duration>,
    wall_offset: Cell<Duration>,
}

#[cfg(test)]
impl FakeMonotonicClock {
    pub fn at_millis(millis: u64) -> Self {
        Self { elapsed: Cell::new(Duration::from_millis(millis)), wall_offset: Cell::new(Duration::ZERO) }
    }

    pub fn advance(&self, delta: Duration) {
        self.elapsed.set(self.elapsed.get().saturating_add(delta));
    }

    pub fn backward(&self, delta: Duration) {
        self.elapsed.set(self.elapsed.get().saturating_sub(delta));
    }

    /// Device suspend is counted in monotonic time, so a deadline still expires.
    pub fn suspend(&self, delta: Duration) {
        self.advance(delta);
    }

    /// Wall time has no bearing on deadlines; this exists to show diagnostics-only use.
    pub fn wall_jump(&self, delta: Duration) {
        self.wall_offset.set(self.wall_offset.get().saturating_add(delta));
    }
}

#[cfg(test)]
impl MonotonicClock for FakeMonotonicClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant(self.elapsed.get())
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
            .checked_add(self.elapsed.get())
            .and_then(|time| time.checked_add(self.wall_offset.get()))
            .unwrap_or(SystemTime::UNIX_EPOCH)
    }
}
