//! Pure close/startup ownership policy for the Assistant's CEF runtime.
//!
//! This module owns no native object and performs no CEF call. It decides only
//! from explicit transitions and observed owners, so the app can require a real
//! matching completion before shutdown instead of inferring it from a request.
//!
//! `Initializing → StartupCancelled` is the DRAFT native `cef_initialize == false`
//! branch; it is distinct from a running runtime's quarantine. Native truth of
//! either branch remains a proof gate, not a result of these tests.

use std::time::{Duration, Instant};

/// Author-selected finite close budget, measured from close start and never
/// reset by callbacks.
pub(crate) const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Unavailable,
    Initializing,
    Running,
    Closing,
    Closed,
    Shutdown,
    Quarantined,
    StartupCancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseProgress {
    Pending,
    Complete,
    Quarantined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleError {
    MismatchedBrowser,
    InvalidTransition,
    IdentityExhausted,
}

impl std::fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MismatchedBrowser => "lifecycle: completion does not match the requested browser",
            Self::InvalidTransition => "lifecycle: transition not allowed in the current phase",
            Self::IdentityExhausted => "lifecycle: browser identity space exhausted",
        })
    }
}

#[derive(Debug)]
pub(crate) struct Lifecycle {
    phase: Phase,
    pending_creation: usize,
    live: Vec<i32>,
    close_started: Option<Instant>,
}

impl Lifecycle {
    pub(crate) fn new() -> Self {
        Self { phase: Phase::Unavailable, pending_creation: 0, live: Vec::new(), close_started: None }
    }

    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    pub(crate) fn begin_initialization(&mut self) -> Result<(), LifecycleError> {
        match self.phase {
            Phase::Unavailable => {
                self.phase = Phase::Initializing;
                Ok(())
            }
            _ => Err(LifecycleError::InvalidTransition),
        }
    }

    pub(crate) fn browser_requested(&mut self) -> Result<(), LifecycleError> {
        if !matches!(self.phase, Phase::Initializing | Phase::Running) {
            return Err(LifecycleError::InvalidTransition);
        }
        if self.pending_creation != 0 {
            return Err(LifecycleError::InvalidTransition);
        }
        self.pending_creation = self.pending_creation.checked_add(1).ok_or(LifecycleError::IdentityExhausted)?;
        Ok(())
    }

    /// A browser that actually exists. While closing or quarantined it is still
    /// owned and must be closed, never leaked or treated as a fresh start.
    pub(crate) fn observe_created(&mut self, browser_id: i32) -> Result<(), LifecycleError> {
        if browser_id <= 0 {
            return Err(LifecycleError::IdentityExhausted);
        }
        if !matches!(self.phase, Phase::Initializing | Phase::Running | Phase::Closing | Phase::Quarantined) {
            return Err(LifecycleError::InvalidTransition);
        }
        if self.pending_creation == 0 || self.live.contains(&browser_id) {
            return Err(LifecycleError::InvalidTransition);
        }
        self.pending_creation -= 1;
        self.live.push(browser_id);
        if self.phase == Phase::Initializing {
            self.phase = Phase::Running;
        }
        Ok(())
    }

    pub(crate) fn observe_creation_cancelled(&mut self) -> Result<(), LifecycleError> {
        if self.pending_creation == 0 || !matches!(self.phase, Phase::Initializing | Phase::Running | Phase::Closing | Phase::Quarantined) {
            return Err(LifecycleError::InvalidTransition);
        }
        self.pending_creation -= 1;
        Ok(())
    }

    /// The DRAFT native `cef_initialize == false` branch: terminal for this
    /// launch, with no live runtime and no browser to close.
    pub(crate) fn native_initialization_returned_false(&mut self) -> Result<(), LifecycleError> {
        match self.phase {
            Phase::Initializing => {
                self.phase = Phase::StartupCancelled;
                Ok(())
            }
            _ => Err(LifecycleError::InvalidTransition),
        }
    }

    /// Start the one close budget. Repeated requests keep the original instant;
    /// an unavailable/cancelled runtime has no native owner and is already done.
    pub(crate) fn begin_close(&mut self, now: Instant) {
        match self.phase {
            Phase::Initializing | Phase::Running => {
                self.phase = Phase::Closing;
                self.close_started = Some(now);
            }
            Phase::Unavailable | Phase::StartupCancelled => {
                self.phase = Phase::Closed;
            }
            Phase::Closing | Phase::Quarantined | Phase::Closed | Phase::Shutdown => {}
        }
    }

    pub(crate) fn observe_closed(&mut self, browser_id: i32) -> Result<(), LifecycleError> {
        if !matches!(self.phase, Phase::Closing | Phase::Quarantined) {
            return Err(LifecycleError::InvalidTransition);
        }
        let index = self.live.iter().position(|id| *id == browser_id).ok_or(LifecycleError::MismatchedBrowser)?;
        self.live.remove(index);
        if self.live.is_empty() && self.pending_creation == 0 {
            self.phase = Phase::Closed;
        }
        Ok(())
    }

    pub(crate) fn poll(&mut self, now: Instant) -> CloseProgress {
        match self.phase {
            Phase::Closing => match self.close_started {
                Some(start) if now.saturating_duration_since(start) >= CLOSE_TIMEOUT => {
                    self.phase = Phase::Quarantined;
                    CloseProgress::Quarantined
                }
                _ => CloseProgress::Pending,
            },
            Phase::Quarantined => CloseProgress::Quarantined,
            Phase::Closed | Phase::Shutdown => CloseProgress::Complete,
            Phase::Unavailable | Phase::Initializing | Phase::Running | Phase::StartupCancelled => CloseProgress::Pending,
        }
    }

    pub(crate) fn mark_shutdown(&mut self) -> Result<(), LifecycleError> {
        match self.phase {
            Phase::Closed => {
                self.phase = Phase::Shutdown;
                Ok(())
            }
            _ => Err(LifecycleError::InvalidTransition),
        }
    }

    /// Retry means navigating the live browser in the same runtime. A close,
    /// quarantine or terminal startup cancellation never reconstructs one.
    pub(crate) fn permits_retry(&self) -> bool {
        self.phase == Phase::Running && !self.live.is_empty()
    }
}

/// Production close barrier for one owned Assistant: the matching platform
/// completion may release the browser owner only after every owned HTTP worker
/// has stopped. Callers poll both sides without blocking, then apply the one
/// existing close deadline in `Lifecycle::poll`.
pub(crate) fn advance_owned_close(
    lifecycle: &mut Lifecycle,
    browser_id: i32,
    platform_complete: bool,
    workers_stopped: bool,
    now: Instant,
) -> CloseProgress {
    if platform_complete && workers_stopped {
        let _ = lifecycle.observe_closed(browser_id);
    }
    lifecycle.poll(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_cef_http::{DocumentAuthority, PageAssets, PageServer, Pictures, ServerProgress};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn running() -> Lifecycle {
        let mut lifecycle = Lifecycle::new();
        lifecycle.begin_initialization().unwrap();
        lifecycle.browser_requested().unwrap();
        lifecycle.observe_created(7).unwrap();
        assert_eq!(lifecycle.phase(), Phase::Running);
        lifecycle
    }

    #[test]
    fn close_requires_matching_completion_not_request() {
        let now = Instant::now();
        let mut lifecycle = running();
        lifecycle.begin_close(now);
        assert_eq!(lifecycle.phase(), Phase::Closing);
        // The request alone is never completion, and a foreign id cannot close.
        assert_eq!(lifecycle.poll(now + Duration::from_millis(10)), CloseProgress::Pending);
        assert_eq!(lifecycle.observe_closed(8), Err(LifecycleError::MismatchedBrowser));
        assert_eq!(lifecycle.phase(), Phase::Closing);
        // A second close request cannot reset the original deadline.
        lifecycle.begin_close(now + Duration::from_millis(4999));
        lifecycle.observe_closed(7).unwrap();
        assert_eq!(lifecycle.phase(), Phase::Closed);
        assert_eq!(lifecycle.poll(now + Duration::from_millis(4999)), CloseProgress::Complete);
        lifecycle.mark_shutdown().unwrap();
        assert_eq!(lifecycle.phase(), Phase::Shutdown);
    }

    #[test]
    fn close_timeout_cancels_quit_and_never_allows_retry() {
        let now = Instant::now();
        let mut lifecycle = running();
        lifecycle.begin_close(now);
        assert_eq!(lifecycle.poll(now + Duration::from_millis(4999)), CloseProgress::Pending);
        assert_eq!(lifecycle.poll(now + CLOSE_TIMEOUT), CloseProgress::Quarantined);
        assert_eq!(lifecycle.phase(), Phase::Quarantined);
        // Repeated polls neither extend the budget nor turn timeout into completion.
        assert_eq!(lifecycle.poll(now + CLOSE_TIMEOUT * 4), CloseProgress::Quarantined);
        assert!(!lifecycle.permits_retry());
        assert_eq!(lifecycle.mark_shutdown(), Err(LifecycleError::InvalidTransition));
        // The unresolved owner is retained: a wrong completion still mismatches.
        assert_eq!(lifecycle.observe_closed(8), Err(LifecycleError::MismatchedBrowser));
        assert_eq!(lifecycle.phase(), Phase::Quarantined);
    }

    #[test]
    fn late_close_finishes_cleanup_without_resuming_quit() {
        let now = Instant::now();
        let mut lifecycle = running();
        lifecycle.begin_close(now);
        assert_eq!(lifecycle.poll(now + CLOSE_TIMEOUT), CloseProgress::Quarantined);
        // Late completion may finish cleanup, but it cannot restore Retry or quit.
        lifecycle.observe_closed(7).unwrap();
        assert_eq!(lifecycle.phase(), Phase::Closed);
        assert_eq!(lifecycle.poll(now + CLOSE_TIMEOUT), CloseProgress::Complete);
        assert!(!lifecycle.permits_retry());
        lifecycle.mark_shutdown().unwrap();
        assert_eq!(lifecycle.phase(), Phase::Shutdown);
    }

    #[test]
    fn partial_initialization_cannot_create_overlapping_owner() {
        let mut lifecycle = Lifecycle::new();
        assert_eq!(lifecycle.browser_requested(), Err(LifecycleError::InvalidTransition));
        lifecycle.begin_initialization().unwrap();
        lifecycle.browser_requested().unwrap();
        assert_eq!(lifecycle.browser_requested(), Err(LifecycleError::InvalidTransition));
        // Failure before completion cannot leave a phantom owner or allow a second one.
        lifecycle.observe_creation_cancelled().unwrap();
        lifecycle.browser_requested().unwrap();
        lifecycle.observe_created(11).unwrap();
        assert_eq!(lifecycle.phase(), Phase::Running);
        // Closing with a pending creation keeps it owned; it cannot be forgotten.
        lifecycle.browser_requested().unwrap();
        lifecycle.begin_close(Instant::now());
        assert_eq!(lifecycle.poll(Instant::now()), CloseProgress::Pending);
    }

    #[test]
    fn late_creation_after_close_is_still_owned() {
        let now = Instant::now();
        let mut lifecycle = running();
        lifecycle.browser_requested().unwrap();
        lifecycle.begin_close(now);
        // Creation that lands after the close request is still closed, not leaked.
        lifecycle.observe_created(9).unwrap();
        lifecycle.observe_closed(7).unwrap();
        assert_eq!(lifecycle.phase(), Phase::Closing);
        assert_eq!(lifecycle.poll(now), CloseProgress::Pending);
        lifecycle.observe_closed(9).unwrap();
        assert_eq!(lifecycle.phase(), Phase::Closed);
    }

    #[test]
    fn native_false_is_terminal_startup_cancelled_not_quarantine() {
        let mut lifecycle = Lifecycle::new();
        lifecycle.begin_initialization().unwrap();
        lifecycle.native_initialization_returned_false().unwrap();
        assert_eq!(lifecycle.phase(), Phase::StartupCancelled);
        assert!(!lifecycle.permits_retry());
        // No normal-close path may reinterpret the terminal startup cancellation.
        assert_eq!(lifecycle.mark_shutdown(), Err(LifecycleError::InvalidTransition));
        assert_eq!(lifecycle.observe_closed(7), Err(LifecycleError::InvalidTransition));
        assert_eq!(lifecycle.begin_initialization(), Err(LifecycleError::InvalidTransition));
    }

    #[test]
    fn unavailable_runtime_closes_without_native_ownership() {
        let mut lifecycle = Lifecycle::new();
        assert!(!lifecycle.permits_retry());
        lifecycle.begin_close(Instant::now());
        assert_eq!(lifecycle.phase(), Phase::Closed);
        assert_eq!(lifecycle.poll(Instant::now()), CloseProgress::Complete);
        lifecycle.mark_shutdown().unwrap();
        assert_eq!(lifecycle.phase(), Phase::Shutdown);
    }

    #[test]
    fn owned_close_requires_real_http_workers_to_stop_before_completion() {
        let now = Instant::now();
        let mut lifecycle = running();
        lifecycle.begin_close(now);
        let authority = Arc::new(DocumentAuthority::new());
        let pictures: Pictures = Arc::new(Mutex::new(HashMap::new()));
        let mut server = PageServer::start(PageAssets { resolve: crate::assistant_page::page_file, font: b"font" }, pictures, authority).unwrap();
        // The platform reported its matching completion, but the four owned HTTP
        // workers still run: the close barrier must stay pending.
        assert_ne!(server.poll_stop(), ServerProgress::Stopped);
        assert_eq!(
            advance_owned_close(&mut lifecycle, 7, true, server.poll_stop() == ServerProgress::Stopped, now + Duration::from_millis(10)),
            CloseProgress::Pending
        );
        assert_eq!(lifecycle.phase(), Phase::Closing);
        // Drive the real owned stop without joining from the decision path.
        server.request_stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.poll_stop() == ServerProgress::Stopping {
            assert!(Instant::now() < deadline, "owned workers did not stop");
            std::thread::yield_now();
        }
        let stopped = server.poll_stop() == ServerProgress::Stopped;
        assert!(stopped, "owned workers must reach Stopped");
        // The worker barrier alone is not completion: without the matching
        // platform completion the close stays pending and the owner is retained.
        assert_eq!(
            (advance_owned_close(&mut lifecycle, 7, false, stopped, now + Duration::from_millis(100)), lifecycle.phase()),
            (CloseProgress::Pending, Phase::Closing)
        );
        assert_eq!(advance_owned_close(&mut lifecycle, 7, true, stopped, now + Duration::from_millis(100)), CloseProgress::Complete);
        assert_eq!(lifecycle.phase(), Phase::Closed);
    }

    #[test]
    fn stopping_workers_keep_close_pending_until_existing_deadline_quarantines() {
        let now = Instant::now();
        let mut lifecycle = running();
        lifecycle.begin_close(now);
        // The same finite close budget covers worker reaping; a still-running
        // worker cannot be reported as a completed close.
        assert_eq!(advance_owned_close(&mut lifecycle, 7, true, false, now + CLOSE_TIMEOUT - Duration::from_millis(1)), CloseProgress::Pending);
        assert_eq!(advance_owned_close(&mut lifecycle, 7, true, false, now + CLOSE_TIMEOUT), CloseProgress::Quarantined);
        // A later real worker stop releases the retained owner without resuming quit.
        assert_eq!(advance_owned_close(&mut lifecycle, 7, true, true, now + CLOSE_TIMEOUT), CloseProgress::Complete);
        assert_eq!(lifecycle.phase(), Phase::Closed);
        assert!(!lifecycle.permits_retry());
    }

    #[test]
    fn invalid_browser_identities_are_refused() {
        let mut lifecycle = Lifecycle::new();
        lifecycle.begin_initialization().unwrap();
        lifecycle.browser_requested().unwrap();
        assert_eq!(lifecycle.observe_created(0), Err(LifecycleError::IdentityExhausted));
        assert_eq!(lifecycle.observe_created(-1), Err(LifecycleError::IdentityExhausted));
        lifecycle.observe_created(3).unwrap();
        lifecycle.browser_requested().unwrap();
        assert_eq!(lifecycle.observe_created(3), Err(LifecycleError::InvalidTransition));
    }
}
