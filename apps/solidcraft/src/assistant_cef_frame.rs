//! App-local frame/input admission. Capture identities are not renderer epochs.
use std::time::{Duration, Instant};

use welding::frame_geometry::{CaptureMetadata, FrameRect, FrameWatermarks, GeometryError, PaintElement};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    pub origin_px: [i32; 2],
    pub physical: [u32; 2],
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InputTicket {
    pub lifetime: u64,
    pub placement_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ThemeTicket {
    pub lifetime: u64,
    pub revision: u64,
}

impl ThemeTicket {
    pub(super) fn parse(text: &str) -> Option<Self> {
        if text.len() > 41 || !text.is_ascii() {
            return None;
        }
        let (lifetime, revision) = text.split_once(':')?;
        let number = |part: &str| {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            part.parse::<u64>().ok()
        };
        Some(Self { lifetime: number(lifetime)?, revision: number(revision)? })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameReject {
    InvalidTarget,
    MissingMetadata,
    Geometry(GeometryError),
    WrongElement,
    WrongSize,
    OldGeneration,
    ImportMismatch,
    ThemePending,
    IdentityExhausted,
    Timeout,
}

/// Reset transport admission; return whether the caller must explicitly ask
/// the native producer for a new frame. B has its bounded admission retries.
pub(super) fn reset_transport(
    admission: Option<&mut FrameAdmission>,
    lifetime: u64,
    placement: Option<Placement>,
    floors: FrameWatermarks,
    now: Instant,
) -> Result<bool, FrameReject> {
    if let Some(admission) = admission {
        admission.reset(lifetime, placement, floors, now)?;
        return Ok(false);
    }
    Ok(true)
}

/// Legacy transports have no capture metadata contract. B never bypasses it.
pub(super) fn check_transport_view(admission: Option<&FrameAdmission>, metadata: Option<&CaptureMetadata>) -> Result<(), FrameReject> {
    admission.map_or(Ok(()), |admission| admission.check_view(metadata.ok_or(FrameReject::MissingMetadata)?))
}

pub(super) struct FrameAdmission {
    max_edge: u32,
    revision: u64,
    lifetime: u64,
    expected: Option<[u32; 2]>,
    floors: FrameWatermarks,
    theme: Option<ThemeTicket>,
    themed: bool,
    admitted: bool,
    error: Option<FrameReject>,
    since: Instant,
    last_retry: Option<Instant>,
    retries: u8,
}

impl FrameAdmission {
    pub(super) fn new(max_edge: u32) -> Self {
        Self {
            max_edge,
            revision: 0,
            lifetime: 0,
            expected: None,
            floors: FrameWatermarks::default(),
            theme: None,
            themed: false,
            admitted: false,
            error: None,
            since: Instant::now(),
            last_retry: None,
            retries: 0,
        }
    }

    pub(super) fn reset(&mut self, lifetime: u64, placement: Option<Placement>, floors: FrameWatermarks, now: Instant) -> Result<(), FrameReject> {
        self.expected = None;
        self.theme = None;
        self.themed = false;
        self.invalidate(now)?;
        self.lifetime = lifetime;
        self.raise_floors(floors);
        if let Some(placement) = placement {
            match native_extent(placement, self.max_edge) {
                Ok(size) => self.expected = Some(size),
                Err(error) => {
                    self.error = Some(error);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(super) fn theme_pending(&mut self, ticket: ThemeTicket, now: Instant) -> Result<(), FrameReject> {
        self.invalidate(now)?;
        self.themed = false;
        if ticket.lifetime != self.lifetime || self.theme.is_some_and(|old| ticket.revision <= old.revision) {
            self.theme = None;
            return Err(FrameReject::ThemePending);
        }
        self.theme = Some(ticket);
        Ok(())
    }

    pub(super) fn theme_applied(&mut self, ticket: ThemeTicket, floors: FrameWatermarks, now: Instant) -> bool {
        if self.error.is_some() || self.themed || self.theme != Some(ticket) || ticket.lifetime != self.lifetime {
            return false;
        }
        self.themed = true;
        self.raise_floors(floors);
        // Start the frame wait, not a new input identity. In particular, do not
        // reset away the acknowledgement we just received.
        self.start_wait(now);
        true
    }

    pub(super) fn check_view(&self, metadata: &CaptureMetadata) -> Result<(), FrameReject> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let expected = self.expected.ok_or(FrameReject::InvalidTarget)?;
        if !self.themed {
            return Err(FrameReject::ThemePending);
        }
        if metadata.element != PaintElement::View {
            return Err(FrameReject::WrongElement);
        }
        let size = metadata.geometry.validate_full_surface(self.max_edge).map_err(FrameReject::Geometry)?;
        if size != expected {
            return Err(FrameReject::WrongSize);
        }
        if metadata.generation <= self.floors.view {
            return Err(FrameReject::OldGeneration);
        }
        Ok(())
    }

    pub(super) fn admit_view(
        &mut self,
        metadata: &CaptureMetadata,
        imported_size: [u32; 2],
        imported_generation: u64,
    ) -> Result<InputTicket, FrameReject> {
        self.check_view(metadata)?;
        if Some(imported_size) != self.expected || imported_generation != metadata.generation {
            return Err(FrameReject::ImportMismatch);
        }
        self.floors.view = metadata.generation;
        self.admitted = true;
        Ok(InputTicket { lifetime: self.lifetime, placement_revision: self.revision })
    }

    pub(super) fn check_popup(
        &self,
        metadata: &CaptureMetadata,
        physical_rect: FrameRect,
        imported_size: [u32; 2],
        imported_generation: u64,
    ) -> Result<(), FrameReject> {
        if self.input_ticket().is_none() {
            return Err(FrameReject::ThemePending);
        }
        if metadata.element != PaintElement::Popup {
            return Err(FrameReject::WrongElement);
        }
        let size = metadata.geometry.validate_full_surface(self.max_edge).map_err(FrameReject::Geometry)?;
        if size != imported_size || imported_generation != metadata.generation {
            return Err(FrameReject::ImportMismatch);
        }
        if metadata.generation <= self.floors.popup {
            return Err(FrameReject::OldGeneration);
        }
        let rect_size = [physical_rect.width, physical_rect.height];
        for (edge, rect_edge) in size.into_iter().zip(rect_size) {
            let rect_edge = u32::try_from(rect_edge).ok().filter(|&v| v > 0 && v <= self.max_edge).ok_or(FrameReject::WrongSize)?;
            if edge.abs_diff(rect_edge) > 1 {
                return Err(FrameReject::WrongSize);
            }
        }
        if physical_rect.x.checked_add(physical_rect.width).is_none() || physical_rect.y.checked_add(physical_rect.height).is_none() {
            return Err(FrameReject::WrongSize);
        }
        Ok(())
    }

    pub(super) fn input_ticket(&self) -> Option<InputTicket> {
        (self.admitted && self.themed && self.expected.is_some() && self.error.is_none())
            .then_some(InputTicket { lifetime: self.lifetime, placement_revision: self.revision })
    }

    pub(super) fn permits_input(&self, ticket: InputTicket) -> bool {
        self.input_ticket() == Some(ticket)
    }

    pub(super) fn retry_due(&mut self, now: Instant) -> bool {
        if self.admitted || self.expected.is_none() || self.error.is_some() || self.retries >= 4 || self.timed_out(now) {
            return false;
        }
        if self.last_retry.is_some_and(|last| now.saturating_duration_since(last) < Duration::from_millis(500)) {
            return false;
        }
        self.last_retry = Some(now);
        self.retries += 1; // bounded above by four
        true
    }

    pub(super) fn pending_error(&self, now: Instant) -> Option<FrameReject> {
        self.error.or_else(|| (!self.admitted && self.expected.is_some() && self.timed_out(now)).then_some(FrameReject::Timeout))
    }

    fn timed_out(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= Duration::from_secs(2)
    }

    fn raise_floors(&mut self, floors: FrameWatermarks) {
        self.floors.view = self.floors.view.max(floors.view);
        self.floors.popup = self.floors.popup.max(floors.popup);
    }

    fn start_wait(&mut self, now: Instant) {
        self.since = now;
        self.last_retry = None;
        self.retries = 0;
    }

    fn invalidate(&mut self, now: Instant) -> Result<(), FrameReject> {
        self.admitted = false;
        self.start_wait(now);
        if self.error == Some(FrameReject::IdentityExhausted) {
            return Err(FrameReject::IdentityExhausted);
        }
        let Some(revision) = self.revision.checked_add(1) else {
            self.error = Some(FrameReject::IdentityExhausted);
            return Err(FrameReject::IdentityExhausted);
        };
        self.revision = revision;
        self.error = None;
        Ok(())
    }
}

/// Mirror the native GetViewRect round-to-DIP policy, then CEF's ceiling to
/// pixels. This is an expected extent, never a resampling factor.
fn native_extent(placement: Placement, max_edge: u32) -> Result<[u32; 2], FrameReject> {
    let scale = placement.scale;
    if !scale.is_finite() || !(0.25..=8.0).contains(&scale) {
        return Err(FrameReject::InvalidTarget);
    }
    let mut size = [0; 2];
    for (result, physical) in size.iter_mut().zip(placement.physical) {
        // f32 represents every integer through 2^24. Refuse larger targets
        // rather than silently losing native pixel identity before conversion.
        if physical == 0 || physical > max_edge.min(1 << 24) {
            return Err(FrameReject::InvalidTarget);
        }
        let dip = (physical as f32 / scale).round().max(1.0);
        let pixels = (f64::from(dip) * f64::from(scale)).ceil();
        if !pixels.is_finite() || pixels < 1.0 || pixels > f64::from(max_edge.min(i32::MAX as u32)) {
            return Err(FrameReject::InvalidTarget);
        }
        *result = pixels as u32;
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(physical: [u32; 2], scale: f32) -> Placement {
        Placement { origin_px: [10, 20], physical, scale }
    }

    fn capture(size: [i32; 2], generation: u64) -> CaptureMetadata {
        let rect = FrameRect { x: 0, y: 0, width: size[0], height: size[1] };
        CaptureMetadata {
            element: PaintElement::View,
            geometry: welding::frame_geometry::FrameGeometry { coded_size: size, visible_rect: rect, content_rect: rect, source_size: Some(size) },
            timestamp_micros: 0,
            capture_counter: Some(0),
            generation,
        }
    }

    fn prepared(target: Placement, now: Instant) -> FrameAdmission {
        let mut state = FrameAdmission::new(4096);
        assert_eq!(state.reset(1, Some(target), FrameWatermarks::default(), now), Ok(()));
        let theme = ThemeTicket { lifetime: 1, revision: 1 };
        assert_eq!(state.theme_pending(theme, now), Ok(()));
        assert!(state.theme_applied(theme, FrameWatermarks::default(), now));
        state
    }

    // Catch non-B resets waiting forever for an unrelated native paint when
    // neither dimensions nor scale changed. The returned action is consumed
    // directly by the real reset path, not by a mock producer.
    #[test]
    fn non_b_same_size_move_and_retry_request_native_repaint() {
        let now = Instant::now();
        let a = placement([463, 1122], 1.25);
        let moved = Placement { origin_px: [a.origin_px[0] + 10, a.origin_px[1]], ..a };
        for target in [moved, moved] {
            // origin-only move, then Retry at the identical target
            assert_eq!(reset_transport(None, 1, Some(target), FrameWatermarks::default(), now), Ok(true));
            assert_eq!(check_transport_view(None, None), Ok(()), "legacy None metadata remains valid");
        }
    }

    #[test]
    fn b_reset_keeps_missing_metadata_theme_and_input_gates() {
        let now = Instant::now();
        let target = placement([463, 1122], 1.25);
        let mut state = prepared(target, now);
        let old = state.admit_view(&capture([463, 1123], 1), [463, 1123], 1).unwrap();
        let moved = Placement { origin_px: [20, 20], ..target };
        assert_eq!(reset_transport(Some(&mut state), 1, Some(moved), FrameWatermarks { view: 1, popup: 0 }, now), Ok(false));
        assert!(!state.permits_input(old));
        assert_eq!(check_transport_view(Some(&state), None), Err(FrameReject::MissingMetadata));
        assert_eq!(check_transport_view(Some(&state), Some(&capture([463, 1123], 2))), Err(FrameReject::ThemePending));
        assert!(state.retry_due(now), "B retains its existing bounded repaint path");
        assert!(!state.retry_due(now));
        let theme = ThemeTicket { lifetime: 1, revision: 2 };
        state.theme_pending(theme, now).unwrap();
        assert!(state.theme_applied(theme, FrameWatermarks { view: 2, popup: 0 }, now));
        assert_eq!(check_transport_view(Some(&state), Some(&capture([463, 1123], 2))), Err(FrameReject::OldGeneration));
        assert!(state.admit_view(&capture([463, 1123], 3), [463, 1123], 3).is_ok());
    }

    // Catch accepting a DIP raster or resampling a rounded-up native allocation.
    #[test]
    fn native_ceiling_extent_is_not_resampling() {
        let now = Instant::now();
        for (physical, scale, native) in
            [([370, 898], 1.0, [370, 898]), ([463, 1122], 1.25, [463, 1123]), ([555, 1347], 1.5, [555, 1347]), ([740, 1796], 2.0, [740, 1796])]
        {
            let mut state = prepared(placement(physical, scale), now);
            let metadata = capture(native, 1);
            assert_eq!(state.check_view(&metadata), Ok(()));
            let size = [native[0] as u32, native[1] as u32];
            let ticket = state.admit_view(&metadata, size, 1).unwrap();
            assert!(state.permits_input(ticket));
        }
        let state = prepared(placement([463, 1122], 1.25), now);
        assert_eq!(state.check_view(&capture([370, 898], 1)), Err(FrameReject::WrongSize));
        assert_eq!(state.check_view(&capture([463, 1122], 1)), Err(FrameReject::WrongSize));
        let mut letterbox = capture([463, 1123], 1);
        letterbox.geometry.content_rect.height = 1100;
        assert_eq!(state.check_view(&letterbox), Err(FrameReject::Geometry(GeometryError::NotFullSurface)));
    }

    // Catch A→B→A reuse, origin-only reuse, queued identity reuse and post-import mismatch.
    #[test]
    fn aba_clears_known_frames_and_input_tickets() {
        let now = Instant::now();
        let a = placement([463, 1122], 1.25);
        let mut state = prepared(a, now);
        let old = state.admit_view(&capture([463, 1123], 1), [463, 1123], 1).unwrap();
        for target in [placement([740, 1796], 2.0), a, Placement { origin_px: [11, 20], ..a }] {
            assert_eq!(state.reset(1, Some(target), FrameWatermarks { view: 5, popup: 7 }, now), Ok(()));
            assert!(!state.permits_input(old));
            assert_eq!(state.input_ticket(), None);
        }
        let theme = ThemeTicket { lifetime: 1, revision: 2 };
        state.theme_pending(theme, now).unwrap();
        assert!(state.theme_applied(theme, FrameWatermarks { view: 8, popup: 9 }, now));
        assert_eq!(state.check_view(&capture([463, 1123], 8)), Err(FrameReject::OldGeneration));
        let next = capture([463, 1123], 10);
        assert_eq!(state.admit_view(&next, [463, 1123], 11), Err(FrameReject::ImportMismatch));
        assert_eq!(state.admit_view(&next, [370, 898], 10), Err(FrameReject::ImportMismatch));
        assert_eq!(state.input_ticket(), None);
        let current = state.admit_view(&next, [463, 1123], 10).unwrap();
        assert_ne!(old, current);
        assert_eq!(state.admit_view(&next, [463, 1123], 10), Err(FrameReject::OldGeneration));
        assert!(state.permits_input(current));
    }

    // Catch a palette/reload acknowledgement initializing the wrong theme or page.
    #[test]
    fn stale_theme_ack_cannot_initialize_new_page() {
        let now = Instant::now();
        let a = ThemeTicket { lifetime: 1, revision: 1 };
        let b = ThemeTicket { lifetime: 1, revision: 2 };
        let c = ThemeTicket { lifetime: 2, revision: 3 };
        let mut state = prepared(placement([463, 1122], 1.25), now);
        let old = state.admit_view(&capture([463, 1123], 1), [463, 1123], 1).unwrap();
        state.theme_pending(b, now).unwrap();
        assert!(!state.permits_input(old));
        assert!(!state.theme_applied(a, FrameWatermarks::default(), now));
        assert_eq!(state.check_view(&capture([463, 1123], 2)), Err(FrameReject::ThemePending));
        assert!(state.theme_applied(b, FrameWatermarks { view: 2, popup: 3 }, now));
        let ticket = state.admit_view(&capture([463, 1123], 3), [463, 1123], 3).unwrap();
        assert_ne!(old, ticket);
        assert!(!state.theme_applied(b, FrameWatermarks { view: 100, popup: 100 }, now));
        assert!(state.permits_input(ticket));
        state.reset(2, Some(placement([463, 1122], 1.25)), FrameWatermarks { view: 3, popup: 3 }, now).unwrap();
        state.theme_pending(c, now).unwrap();
        assert!(!state.theme_applied(b, FrameWatermarks::default(), now));
        assert_eq!(state.input_ticket(), None);
        assert!(state.theme_applied(c, FrameWatermarks { view: 4, popup: 4 }, now));
        assert_eq!(state.check_view(&capture([463, 1123], 4)), Err(FrameReject::OldGeneration));
    }

    // Catch per-tick deadline restart, repaint storms and invisible-page input.
    #[test]
    fn pending_has_bounded_retry_and_no_input() {
        let now = Instant::now();
        let mut state = prepared(placement([463, 1122], 1.25), now);
        for ms in 0..2500 {
            let due = state.retry_due(now + Duration::from_millis(ms));
            assert_eq!(due, [0, 500, 1000, 1500].contains(&ms), "{ms}");
            assert_eq!(state.input_ticket(), None);
        }
        assert_eq!(state.pending_error(now + Duration::from_secs(2)), Some(FrameReject::Timeout));
        // Timeout is recoverable, not a permanent refusal.
        assert!(state.admit_view(&capture([463, 1123], 1), [463, 1123], 1).is_ok());
        assert_eq!(state.pending_error(now + Duration::from_secs(3)), None);
        assert!(!state.retry_due(now + Duration::from_secs(3)));
    }

    #[test]
    fn current_theme_ack_starts_frame_wait_without_restarting_identity() {
        let now = Instant::now();
        let mut state = FrameAdmission::new(4096);
        state.reset(1, Some(placement([463, 1122], 1.25)), FrameWatermarks::default(), now).unwrap();
        let theme = ThemeTicket { lifetime: 1, revision: 1 };
        state.theme_pending(theme, now).unwrap();
        let revision = state.revision;
        let later = now + Duration::from_secs(3);
        assert!(state.theme_applied(theme, FrameWatermarks { view: 5, popup: 7 }, later));
        assert_eq!(state.revision, revision);
        assert_eq!(state.pending_error(later), None);
        assert!(state.retry_due(later));
        assert_eq!(state.pending_error(later + Duration::from_secs(2)), Some(FrameReject::Timeout));
        assert!(state.admit_view(&capture([463, 1123], 6), [463, 1123], 6).is_ok());
    }

    #[test]
    fn theme_ticket_parser_is_bounded_and_checked() {
        assert_eq!(ThemeTicket::parse("0:0"), Some(ThemeTicket { lifetime: 0, revision: 0 }));
        assert_eq!(ThemeTicket::parse("18446744073709551615:18446744073709551615"), Some(ThemeTicket { lifetime: u64::MAX, revision: u64::MAX }));
        for text in [
            "",
            "1",
            "1:",
            ":1",
            "1:2:3",
            "-1:2",
            "+1:2",
            "1: 2",
            "1:2\n",
            "１:2",
            "18446744073709551616:2",
            "1:18446744073709551616",
            "0000000000000000000001:0000000000000000000002",
        ] {
            assert_eq!(ThemeTicket::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn hostile_targets_and_exhausted_identity_stay_noninteractive() {
        let now = Instant::now();
        let mut state = prepared(placement([463, 1122], 1.25), now);
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0, 0.249, 8.01, f32::MAX] {
            assert_eq!(state.reset(1, Some(placement([463, 1122], scale)), FrameWatermarks::default(), now), Err(FrameReject::InvalidTarget));
            assert_eq!(state.input_ticket(), None);
        }
        for size in [[0, 1], [1, 0], [4097, 1], [1, u32::MAX]] {
            assert_eq!(state.reset(1, Some(placement(size, 1.0)), FrameWatermarks::default(), now), Err(FrameReject::InvalidTarget));
        }
        assert_eq!(state.reset(1, None, FrameWatermarks::default(), now), Ok(()));
        assert_eq!(state.input_ticket(), None);
        state.revision = u64::MAX;
        assert_eq!(state.reset(1, Some(placement([463, 1122], 1.25)), FrameWatermarks::default(), now), Err(FrameReject::IdentityExhausted));
        assert_eq!(state.input_ticket(), None);
        assert_eq!(state.theme_pending(ThemeTicket { lifetime: 1, revision: 8 }, now), Err(FrameReject::IdentityExhausted));
    }

    #[test]
    fn popup_requires_current_view_full_surface_and_own_identity() {
        let now = Instant::now();
        let mut state = prepared(placement([463, 1122], 1.25), now);
        let rect = FrameRect { x: 3, y: 4, width: 248, height: 600 };
        let mut popup = capture([248, 601], 1);
        popup.element = PaintElement::Popup;
        assert_eq!(state.check_popup(&popup, rect, [248, 601], 1), Err(FrameReject::ThemePending));
        state.admit_view(&capture([463, 1123], 1), [463, 1123], 1).unwrap();
        assert_eq!(state.check_popup(&popup, rect, [248, 601], 1), Ok(()));
        assert_eq!(state.check_popup(&popup, rect, [248, 601], 2), Err(FrameReject::ImportMismatch));
        assert_eq!(state.check_popup(&popup, FrameRect { width: 245, ..rect }, [248, 601], 1), Err(FrameReject::WrongSize));
        popup.element = PaintElement::View;
        assert_eq!(state.check_popup(&popup, rect, [248, 601], 1), Err(FrameReject::WrongElement));
        popup.element = PaintElement::Popup;
        popup.geometry.source_size = None;
        assert_eq!(state.check_popup(&popup, rect, [248, 601], 1), Err(FrameReject::Geometry(GeometryError::MissingSource)));
    }

    // This limitation is deliberate: no capture scalar proves renderer-commit freshness.
    #[test]
    fn late_same_size_a_with_new_capture_identity_is_indistinguishable() {
        let now = Instant::now();
        let mut state = prepared(placement([463, 1122], 1.25), now);
        state.admit_view(&capture([463, 1123], 1), [463, 1123], 1).unwrap();
        state.reset(1, Some(placement([740, 1796], 2.0)), FrameWatermarks { view: 1, popup: 0 }, now).unwrap();
        state.reset(1, Some(placement([463, 1122], 1.25)), FrameWatermarks { view: 5, popup: 0 }, now).unwrap();
        let theme = ThemeTicket { lifetime: 1, revision: 2 };
        state.theme_pending(theme, now).unwrap();
        assert!(state.theme_applied(theme, FrameWatermarks { view: 6, popup: 0 }, now));
        assert!(state.admit_view(&capture([463, 1123], 7), [463, 1123], 7).is_ok());
    }
}
