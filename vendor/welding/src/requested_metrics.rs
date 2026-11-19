//! Requested view geometry, independent of capture or native observations.
//!
//! Physical size and the display scale are what the host asked CEF to lay out.
//! They do not prove the effective native scale CEF used.

pub const MIN_SCALE: f32 = 0.25;
pub const MAX_SCALE: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RequestedViewMetrics {
    pub physical: [u32; 2],
    pub dip: [i32; 2],
    pub scale: f32,
}

pub fn sane_scale(scale: f32) -> f32 {
    if scale.is_finite() { scale.clamp(MIN_SCALE, MAX_SCALE) } else { 1.0 }
}

impl RequestedViewMetrics {
    pub fn new(physical: [u32; 2], scale: f32) -> Self {
        let scale = sane_scale(scale);
        let dip = [
            ((physical[0] as f32 / scale).round().max(1.0)) as i32,
            ((physical[1] as f32 / scale).round().max(1.0)) as i32,
        ];
        Self { physical, dip, scale }
    }

    /// Physical extent implied by the requested DIP size at the requested scale.
    pub fn expected_physical(&self) -> [u32; 2] {
        // Round the DIP first (what GetViewRect reports), then ceil the physical
        // extent: the exact reproduction contract for fractional scales.
        [
            ((self.dip[0] as f32 * self.scale).ceil().max(1.0)) as u32,
            ((self.dip[1] as f32 * self.scale).ceil().max(1.0)) as u32,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_requested_metrics_round_dip_then_ceil_physical() {
        let metrics = RequestedViewMetrics::new([463, 1122], 1.25);
        assert_eq!(metrics.dip, [370, 898]);
        assert_eq!(metrics.expected_physical(), [463, 1123]);
        // The capture's own physical size is one pixel short of the request: that
        // difference must stay visible, never be reconstructed away.
        assert_ne!(metrics.physical, metrics.expected_physical());
    }

    #[test]
    fn scale_is_finite_and_bounded() {
        assert_eq!(sane_scale(1.25), 1.25);
        assert_eq!(sane_scale(0.0), MIN_SCALE);
        assert_eq!(sane_scale(-2.0), MIN_SCALE);
        assert_eq!(sane_scale(f32::NAN), 1.0);
        assert_eq!(sane_scale(99.0), MAX_SCALE);
    }

    #[test]
    fn degenerate_physical_size_still_reports_one_dip() {
        let metrics = RequestedViewMetrics::new([0, 0], 2.0);
        assert_eq!(metrics.dip, [1, 1]);
        assert_eq!(metrics.expected_physical(), [2, 2]);
    }
}
