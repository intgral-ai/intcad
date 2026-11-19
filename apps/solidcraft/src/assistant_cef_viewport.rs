//! Acknowledged viewport configuration, independent of CEF and its asynchronous receipts.
use std::time::{Duration, Instant};

pub(super) type Metrics = ([u32; 2], f32);

pub(super) struct ViewportState {
    applied: Option<Metrics>,
    pending: Option<(u32, Metrics, Instant)>,
    next_id: u32,
    retry_at: Instant,
}

impl ViewportState {
    pub(super) fn new(now: Instant) -> Self {
        Self { applied: None, pending: None, next_id: 1, retry_at: now }
    }

    pub(super) fn reset(&mut self, now: Instant) {
        self.applied = None;
        self.pending = None;
        self.retry_at = now;
    }

    pub(super) fn needs_update(&self, target: Metrics, now: Instant) -> bool {
        let pending = self.pending.is_some_and(|(_, metrics, at)| metrics == target && now.duration_since(at) < Duration::from_secs(2));
        self.applied != Some(target) && !pending && now >= self.retry_at
    }

    pub(super) fn begin(&mut self, target: Metrics, now: Instant) -> u32 {
        // Native resize/scale setters run before the protocol receipt. Even a failed request
        // invalidates the previous joint native/Chromium configuration.
        self.applied = None;
        let id = self.next_id;
        self.next_id = id.checked_add(1).filter(|id| *id <= i32::MAX as u32).unwrap_or(1);
        self.pending = Some((id, target, now));
        id
    }

    pub(super) fn failed(&mut self, now: Instant) {
        self.pending = None;
        self.retry_at = now + Duration::from_millis(500);
    }

    pub(super) fn complete(&mut self, id: u32, success: bool, now: Instant) -> bool {
        let Some((pending_id, target, _)) = self.pending else { return false };
        if id != pending_id {
            return false;
        }
        self.pending = None;
        if success {
            self.applied = Some(target);
        } else {
            self.failed(now);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_transition_cannot_reuse_an_old_acknowledgement() {
        for failure in ["send", "protocol", "missing"] {
            let now = Instant::now();
            let a = ([460, 1100], 1.25);
            let b = ([460, 1100], 2.0);
            let mut state = ViewportState::new(now);
            let first = state.begin(a, now);
            assert!(state.complete(first, true, now));
            assert!(!state.needs_update(a, now));
            let second = state.begin(b, now);
            match failure {
                "send" => state.failed(now),
                "protocol" => {
                    assert!(state.complete(second, false, now));
                }
                _ => {}
            }
            let later = now + Duration::from_secs(3);
            assert!(state.needs_update(a, later), "{failure}: native state was mutated to B, so A needs reapplication");
        }
    }

    #[test]
    fn scale_only_changes_retries_and_stale_receipts_are_distinct() {
        let now = Instant::now();
        let a = ([460, 1100], 1.25);
        let b = ([460, 1100], 2.0);
        let mut state = ViewportState::new(now);
        let first = state.begin(a, now);
        assert!(state.complete(first, true, now));
        assert!(state.needs_update(b, now));
        let second = state.begin(b, now);
        assert!(!state.needs_update(b, now));
        assert!(state.needs_update(b, now + Duration::from_secs(3)));
        assert!(!state.complete(first, true, now));
        assert!(state.complete(second, true, now));
        assert!(!state.needs_update(b, now));
        state.reset(now);
        assert!(state.needs_update(b, now));
        let third = state.begin(b, now);
        assert!(state.complete(third, false, now));
        assert!(!state.needs_update(b, now));
        assert!(state.needs_update(b, now + Duration::from_millis(501)));
    }
}
