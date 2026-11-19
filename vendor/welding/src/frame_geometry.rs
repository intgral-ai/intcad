// Copyright 2026 Intcad contributors
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! SolidCraft: owned capture scalars, independent of CEF and GPU types.
//! A capture generation identifies a mailbox paint, not a renderer commit.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameGeometry {
    pub coded_size: [i32; 2],
    pub visible_rect: FrameRect,
    pub content_rect: FrameRect,
    pub source_size: Option<[i32; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintElement {
    View,
    Popup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureMetadata {
    pub element: PaintElement,
    pub geometry: FrameGeometry,
    /// CEF microseconds since capture start; zero is a valid timestamp.
    pub timestamp_micros: u64,
    pub capture_counter: Option<u64>,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWatermarks {
    pub view: u64,
    pub popup: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError {
    InvalidSize,
    InvalidRect,
    MissingSource,
    NotFullSurface,
}

impl FrameGeometry {
    /// Admit only a bounded, uncropped allocation that exactly covers its source.
    pub fn validate_full_surface(&self, max_edge: u32) -> Result<[u32; 2], GeometryError> {
        let [width, height] = self.coded_size;
        let bounded = |edge| u32::try_from(edge).ok().filter(|&v| v > 0 && v <= max_edge);
        let size = [bounded(width).ok_or(GeometryError::InvalidSize)?, bounded(height).ok_or(GeometryError::InvalidSize)?];
        for rect in [self.visible_rect, self.content_rect] {
            if rect.x < 0
                || rect.y < 0
                || rect.width <= 0
                || rect.height <= 0
                || rect.x.checked_add(rect.width).is_none_or(|right| right > width)
                || rect.y.checked_add(rect.height).is_none_or(|bottom| bottom > height)
            {
                return Err(GeometryError::InvalidRect);
            }
        }
        let source = self.source_size.ok_or(GeometryError::MissingSource)?;
        if source.into_iter().any(|edge| bounded(edge).is_none()) {
            return Err(GeometryError::InvalidSize);
        }
        let full = FrameRect { x: 0, y: 0, width, height };
        if source != self.coded_size || self.visible_rect != full || self.content_rect != full {
            return Err(GeometryError::NotFullSurface);
        }
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full(width: i32, height: i32) -> FrameGeometry {
        let rect = FrameRect { x: 0, y: 0, width, height };
        FrameGeometry { coded_size: [width, height], visible_rect: rect, content_rect: rect, source_size: Some([width, height]) }
    }

    // Catch refusal of the rounded-up 1.25-density full allocation.
    #[test]
    fn full_capture_is_valid() {
        assert_eq!(full(463, 1123).validate_full_surface(4096), Ok([463, 1123]));
        assert_eq!(full(463, 1123).validate_full_surface(1123), Ok([463, 1123]));
        assert_eq!(full(1, 1).validate_full_surface(1), Ok([1, 1]));
    }

    // Catch accepting letterboxing or a larger allocation merely because it fits.
    #[test]
    fn letterboxed_and_oversized_coded_frames_are_refused() {
        let mut letterboxed = full(900, 600);
        letterboxed.content_rect = FrameRect { x: 326, y: 0, width: 248, height: 600 };
        letterboxed.source_size = Some([463, 1122]);
        assert!(letterboxed.validate_full_surface(4096).is_err());
        let mut oversized = full(579, 1403);
        oversized.source_size = Some([463, 1123]);
        assert!(oversized.validate_full_surface(4096).is_err());
    }

    // Catch unchecked signed arithmetic, missing-source admission and either
    // rectangle being ignored. No native dependency is needed for this boundary.
    #[test]
    fn hostile_geometry_never_panics() {
        let mut missing = full(463, 1123);
        missing.source_size = None;
        assert!(missing.validate_full_surface(4096).is_err());
        for size in [[-1, 1], [1, -1], [0, 1], [1, 0], [4097, 1], [1, 4097], [i32::MIN, i32::MAX], [i32::MAX, i32::MAX]] {
            assert!(full(size[0], size[1]).validate_full_surface(4096).is_err());
            let mut source = full(463, 1123);
            source.source_size = Some(size);
            assert!(source.validate_full_surface(4096).is_err());
        }
        assert!(full(1, 1).validate_full_surface(0).is_err());
        let extremes = [i32::MIN, -1, 0, 1, 463, 1123, i32::MAX];
        for x in extremes {
            for y in extremes {
                for width in extremes {
                    for height in extremes {
                        let rect = FrameRect { x, y, width, height };
                        for visible in [false, true] {
                            let mut geometry = full(463, 1123);
                            if visible {
                                geometry.visible_rect = rect;
                            } else {
                                geometry.content_rect = rect;
                            }
                            let result = geometry.validate_full_surface(4096);
                            if rect == (FrameRect { x: 0, y: 0, width: 463, height: 1123 }) {
                                assert_eq!(result, Ok([463, 1123]));
                            } else {
                                assert!(result.is_err(), "admitted {geometry:?}");
                            }
                        }
                    }
                }
            }
        }
        // Overflow must also be refused when the coded edge itself is allowed.
        let mut overflow = full(i32::MAX, 1);
        overflow.visible_rect.x = 1;
        assert!(overflow.validate_full_surface(u32::MAX).is_err());
    }

    // Catch validating a popup against page dimensions rather than its surface.
    #[test]
    fn popup_is_its_own_surface() {
        let popup =
            CaptureMetadata { element: PaintElement::Popup, geometry: full(248, 600), timestamp_micros: 0, capture_counter: Some(0), generation: 7 };
        assert_eq!(popup.geometry.validate_full_surface(4096), Ok([248, 600]));
    }
}
