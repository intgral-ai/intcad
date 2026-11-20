// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! IME composition feedback from CEF.
//!
//! The host owns the input method under windowless rendering, so it needs to
//! know where the text being composed actually sits on screen; that is where
//! the candidate window goes. CEF reports it through
//! `OnImeCompositionRangeChanged`, in DIP.

use std::sync::Mutex;

use crate::surface::{ImeComposition, PopupRect};

#[derive(Debug, Default)]
pub(crate) struct LatestComposition(Mutex<Option<ImeComposition>>);

impl LatestComposition {
    pub(crate) fn set(&self, composition: ImeComposition) {
        *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(composition);
    }

    pub(crate) fn take(&self) -> Option<ImeComposition> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}

/// The union of the character bounds CEF reports, which is the rectangle a
/// host wants to sit its candidate window under. CEF hands over one rect per
/// character; a host almost never wants them individually.
pub(crate) const MAX_BOUNDS: usize = 200_000;

pub(crate) fn bounds_union_iter<I>(rects: I) -> Option<PopupRect>
where
    I: ExactSizeIterator<Item = PopupRect>,
{
    // Check the exact length before `filter` or the caller's lazy map pulls any
    // element: hostile bounds are refused before allocation or work.
    if rects.len() > MAX_BOUNDS {
        return None;
    }
    let mut it = rects.filter(|r| r.width > 0 && r.height > 0);
    let first = it.next()?;
    let (mut x0, mut y0) = (i64::from(first.x), i64::from(first.y));
    let (mut x1, mut y1) = (x0 + i64::from(first.width), y0 + i64::from(first.height));
    for r in it {
        let (rx, ry) = (i64::from(r.x), i64::from(r.y));
        x0 = x0.min(rx);
        y0 = y0.min(ry);
        x1 = x1.max(rx + i64::from(r.width));
        y1 = y1.max(ry + i64::from(r.height));
    }
    // A rectangle CEF could not express in its own 32-bit coordinate space is
    // refused rather than clamped into a misleading candidate position.
    if x1 > i64::from(i32::MAX) || y1 > i64::from(i32::MAX) {
        return None;
    }
    Some(PopupRect {
        x: i32::try_from(x0).ok()?,
        y: i32::try_from(y0).ok()?,
        width: u32::try_from(x1 - x0).ok()?,
        height: u32::try_from(y1 - y0).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: u32, h: u32) -> PopupRect {
        PopupRect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn union_spans_every_character() {
        let got = bounds_union_iter([r(10, 20, 8, 16), r(18, 20, 8, 16), r(26, 20, 8, 16)].into_iter());
        assert_eq!(got, Some(r(10, 20, 24, 16)));
    }

    #[test]
    fn empty_rects_are_ignored_not_counted() {
        // CEF pads with zero-size rects; letting one in would drag the union
        // to the origin and put the candidate window in the corner.
        assert_eq!(bounds_union_iter([r(0, 0, 0, 0), r(40, 60, 10, 12)].into_iter()), Some(r(40, 60, 10, 12)));
        assert_eq!(bounds_union_iter(std::iter::empty::<PopupRect>()), None);
        assert_eq!(bounds_union_iter([r(0, 0, 0, 0)].into_iter()), None);
    }
}

#[cfg(test)]
mod bounded_tests {
    use super::*;

    fn r(x: i32, y: i32, w: u32, h: u32) -> PopupRect {
        PopupRect { x, y, width: w, height: h }
    }

    #[test]
    fn extrema_never_overflow_the_union() {
        assert_eq!(bounds_union_iter([r(i32::MIN, i32::MIN, 0, 0)].into_iter()), None);
        assert_eq!(bounds_union_iter([r(i32::MAX, 0, 1, 1)].into_iter()), None);
        assert_eq!(bounds_union_iter([r(0, 0, u32::MAX, 1)].into_iter()), None);
        assert_eq!(bounds_union_iter([r(i32::MIN, 0, 1, 1), r(i32::MAX, 0, 2, 1)].into_iter()), None);
    }

    #[test]
    fn excessive_character_count_is_refused_without_allocation() {
        let rects = vec![r(0, 0, 1, 1); MAX_BOUNDS + 1];
        assert_eq!(bounds_union_iter(rects.into_iter()), None);
    }

    /// Reports an exact length like CEF's `&[cef::Rect]` iterator does, and
    /// records every element actually pulled, so the cap can be proven to
    /// refuse work before any bound is read or mapped.
    struct CountedBounds {
        len: usize,
        read: usize,
    }

    impl Iterator for CountedBounds {
        type Item = PopupRect;

        fn next(&mut self) -> Option<PopupRect> {
            if self.read == self.len {
                return None;
            }
            self.read += 1;
            Some(r(0, 0, 1, 1))
        }
    }

    impl ExactSizeIterator for CountedBounds {
        fn len(&self) -> usize {
            self.len
        }
    }

    #[test]
    fn excessive_reported_bounds_are_refused_before_they_are_read() {
        let mut excessive = CountedBounds { len: MAX_BOUNDS + 1, read: 0 };
        assert_eq!(bounds_union_iter(excessive.by_ref()), None);
        assert_eq!(excessive.read, 0, "the cap is checked before any bound is read");

        let mut normal = CountedBounds { len: 3, read: 0 };
        assert_eq!(bounds_union_iter(normal.by_ref()), Some(r(0, 0, 1, 1)));
        assert_eq!(normal.read, 3);
    }

    #[test]
    fn oversized_bounds_are_refused_before_any_bound_is_mapped() {
        // The call sites hand `bounds_union_iter` a lazy `slice::iter().map()`.
        // This proves the length check happens before that closure runs.
        let rects = vec![r(0, 0, 1, 1); MAX_BOUNDS + 1];
        let mapped = std::cell::Cell::new(0usize);
        let got = bounds_union_iter(rects.iter().map(|rect| {
            mapped.set(mapped.get() + 1);
            *rect
        }));
        assert_eq!(got, None);
        assert_eq!(mapped.get(), 0, "the cap is checked before the lazy map runs");
    }

    #[test]
    fn a_poisoned_mailbox_is_still_usable() {
        let mailbox = std::sync::Arc::new(LatestComposition::default());
        let other = mailbox.clone();
        let _ = std::thread::spawn(move || {
            let _guard = other.0.lock().unwrap();
            panic!("poison the mailbox");
        })
        .join();
        let composition = ImeComposition { bounds: r(10, 20, 8, 16), selection_start: 0, selection_end: 1 };
        mailbox.set(composition);
        assert_eq!(mailbox.take().map(|c| c.bounds), Some(r(10, 20, 8, 16)));
    }
}
