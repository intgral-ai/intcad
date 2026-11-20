// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! View geometry: physical size plus the display scale factor.
//!
//! CEF's windowless model separates the two, and getting it wrong is quiet
//! rather than loud:
//!
//! - `GetViewRect` is answered in **DIP** (device-independent pixels, i.e. CSS
//!   pixels), not physical pixels.
//! - `GetScreenInfo::device_scale_factor` is the multiplier CEF applies to
//!   produce the physical texture it paints.
//! - Mouse coordinates handed to CEF are in DIP as well.
//!
//! Answer `GetViewRect` in physical pixels with a scale of 1, as welding did
//! before, and a 2x display gets a page laid out at twice the CSS width, so
//! everything renders at half its intended size. Feed it physical mouse
//! coordinates under a real scale and clicks land at the wrong place. Both
//! failure modes look like a working browser, which is why this lives in one
//! place with tests rather than being open-coded per platform.
//!
//! Hosts talk to welding in **physical** pixels throughout, because that is
//! what a window system hands them; the DIP conversion happens here.

use std::sync::Mutex;

use dpi::PhysicalSize;

use crate::requested_metrics::{RequestedViewMetrics, sane_scale};
use crate::surface::PopupRect;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewMetrics {
    size: PhysicalSize<u32>,
    scale: f32,
}

impl ViewMetrics {
    pub(crate) fn new(size: PhysicalSize<u32>, scale: f32) -> Self {
        Self {
            size,
            scale: sane_scale(scale),
        }
    }

    pub(crate) fn set_size(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
    }

    pub(crate) fn set_scale(&mut self, scale: f32) {
        self.scale = sane_scale(scale);
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    /// What the host asked CEF to lay out: acquisition-independent requested metrics.
    pub(crate) fn requested(&self) -> RequestedViewMetrics {
        RequestedViewMetrics::new([self.size.width, self.size.height], self.scale)
    }

    /// What `GetViewRect` should report: the view in DIP.
    pub(crate) fn logical(&self) -> (i32, i32) {
        // One rounding contract with the requested-metrics receipt.
        let dip = self.requested().dip;
        (dip[0], dip[1])
    }

    /// Host physical coordinates to the DIP that CEF expects for input.
    pub(crate) fn point_to_dip(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (x as f32 / self.scale).round() as i32,
            (y as f32 / self.scale).round() as i32,
        )
    }

    /// CEF hands popup geometry back in DIP; hosts draw in physical pixels.
    pub(crate) fn rect_to_physical(&self, rect: PopupRect) -> PopupRect {
        PopupRect {
            x: (rect.x as f32 * self.scale).round() as i32,
            y: (rect.y as f32 * self.scale).round() as i32,
            width: (rect.width as f32 * self.scale).round() as u32,
            height: (rect.height as f32 * self.scale).round() as u32,
        }
    }
}

/// Lock the published metrics and convert CEF DIP bounds to physical pixels.
///
/// `on_ime_composition_range_changed` runs inside native CEF. A poisoned lock
/// means a thread panicked while holding the metrics, so `None` lets the
/// callback publish no candidate geometry instead of panicking across FFI.
pub(crate) fn rect_to_physical_locked(metrics: &Mutex<ViewMetrics>, rect: PopupRect) -> Option<PopupRect> {
    metrics.lock().ok().map(|metrics| metrics.rect_to_physical(rect))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unscaled_view_is_reported_as_is() {
        let m = ViewMetrics::new(PhysicalSize::new(1280, 800), 1.0);
        assert_eq!(m.logical(), (1280, 800));
        assert_eq!(m.point_to_dip(640, 400), (640, 400));
    }

    #[test]
    fn hidpi_view_is_reported_in_dip() {
        let m = ViewMetrics::new(PhysicalSize::new(2560, 1600), 2.0);
        // CEF lays out 1280x800 CSS pixels and paints a 2560x1600 texture.
        assert_eq!(m.logical(), (1280, 800));
        // A click at the physical centre is the DIP centre.
        assert_eq!(m.point_to_dip(1280, 800), (640, 400));
        // A popup CEF places at DIP 40,80 is drawn at physical 80,160.
        assert_eq!(
            m.rect_to_physical(PopupRect {
                x: 40,
                y: 80,
                width: 200,
                height: 95
            }),
            PopupRect {
                x: 80,
                y: 160,
                width: 400,
                height: 190
            }
        );
    }

    #[test]
    fn fractional_scale_rounds_without_collapsing() {
        let m = ViewMetrics::new(PhysicalSize::new(1920, 1080), 1.5);
        assert_eq!(m.logical(), (1280, 720));
    }

    fn dip(x: i32, y: i32, width: u32, height: u32) -> PopupRect {
        PopupRect { x, y, width, height }
    }

    /// `on_ime_composition_range_changed` runs inside native CEF. If another
    /// thread poisoned the metrics mutex, the callback must publish no
    /// candidate geometry instead of panicking across the FFI boundary.
    #[test]
    fn poisoned_metrics_publish_no_candidate_bounds() {
        let metrics = Mutex::new(ViewMetrics::new(PhysicalSize::new(2560, 1600), 2.0));
        let _ = std::panic::catch_unwind(|| {
            let _guard = metrics.lock().expect("initial metrics lock");
            panic!("poison the metrics for the test");
        });
        assert!(metrics.is_poisoned());
        assert_eq!(rect_to_physical_locked(&metrics, dip(40, 80, 200, 95)), None);
    }

    /// A healthy lock keeps exactly the one DIP -> physical conversion.
    #[test]
    fn healthy_metrics_convert_ime_bounds_once() {
        let metrics = Mutex::new(ViewMetrics::new(PhysicalSize::new(2560, 1600), 2.0));
        assert_eq!(rect_to_physical_locked(&metrics, dip(40, 80, 200, 95)), Some(dip(80, 160, 400, 190)));
    }

    #[test]
    fn nonsense_scales_are_clamped_not_fatal() {
        assert_eq!(
            ViewMetrics::new(PhysicalSize::new(800, 600), 0.0).scale(),
            0.25
        );
        assert_eq!(
            ViewMetrics::new(PhysicalSize::new(800, 600), -3.0).scale(),
            0.25
        );
        assert_eq!(
            ViewMetrics::new(PhysicalSize::new(800, 600), f32::NAN).scale(),
            1.0
        );
        // A degenerate view still reports at least one DIP.
        let m = ViewMetrics::new(PhysicalSize::new(1, 1), 8.0);
        assert_eq!(m.logical(), (1, 1));
    }
}
