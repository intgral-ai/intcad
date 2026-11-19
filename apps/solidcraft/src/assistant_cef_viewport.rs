//! Synchronous native viewport setters; frame readiness is a separate contract.
use std::time::{Duration, Instant};

pub(super) type Metrics = ([u32; 2], f32);

pub(super) struct ViewportState {
    applied: Option<Metrics>,
    pending: Option<Metrics>,
    retry_at: Option<Instant>,
}

impl ViewportState {
    pub(super) fn new(now: Instant) -> Self {
        Self { applied: None, pending: None, retry_at: Some(now) }
    }

    pub(super) fn reset(&mut self, now: Instant) {
        self.applied = None;
        self.pending = None;
        self.retry_at = Some(now);
    }

    pub(super) fn needs_update(&self, target: Metrics, now: Instant) -> bool {
        let pending = self.pending == Some(target);
        self.applied != Some(target) && !pending && self.retry_at.is_some_and(|at| now >= at)
    }

    pub(super) fn begin(&mut self, target: Metrics) {
        // A partial native setter transition invalidates the previous configuration.
        self.applied = None;
        self.pending = Some(target);
    }

    pub(super) fn finish(&mut self, target: Metrics, success: bool, now: Instant) {
        if self.pending != Some(target) {
            return;
        }
        self.pending = None;
        if success {
            self.applied = Some(target);
            self.retry_at = Some(now);
        } else {
            self.retry_at = now.checked_add(Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_transition_cannot_reuse_old_native_state() {
        for failure in ["resize", "scale"] {
            let now = Instant::now();
            let a = ([460, 1100], 1.25);
            let b = ([460, 1100], 2.0);
            let mut state = ViewportState::new(now);
            state.begin(a);
            state.finish(a, true, now);
            assert!(!state.needs_update(a, now));
            state.begin(b);
            state.finish(b, false, now);
            let later = now + Duration::from_secs(3);
            assert!(state.needs_update(a, later), "{failure}: native state may be mutated to B, so A needs reapplication");
        }
    }

    #[test]
    fn scale_only_changes_and_native_failure_retry_are_distinct() {
        let now = Instant::now();
        let a = ([460, 1100], 1.25);
        let b = ([460, 1100], 2.0);
        let mut state = ViewportState::new(now);
        state.begin(a);
        state.finish(a, true, now);
        assert!(state.needs_update(b, now));
        state.begin(b);
        assert!(!state.needs_update(b, now));
        state.finish(b, false, now);
        assert!(!state.needs_update(b, now + Duration::from_millis(499)));
        assert!(state.needs_update(b, now + Duration::from_millis(500)));
        state.begin(b);
        state.finish(a, true, now); // A mismatched completion cannot apply B.
        assert!(!state.needs_update(b, now));
        state.finish(b, true, now);
        assert!(!state.needs_update(b, now));
        state.reset(now);
        assert!(state.needs_update(b, now));
        state.begin(b);
        state.finish(b, false, now);
        assert!(!state.needs_update(b, now));
        assert!(state.needs_update(b, now + Duration::from_millis(501)));
    }
}
