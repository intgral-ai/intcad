//! Text and IME geometry shared by the Assistant's native input path.
//!
//! Std-only: no CEF types, no GPU, so these conversions can be tested for real.
//! Character ranges here are Unicode scalar values; CEF's IME selection and
//! keyboard character are UTF-16 code units.

/// The UTF-16 units of one character: one for BMP, a surrogate pair otherwise.
pub fn utf16_units(ch: char) -> ([u16; 2], usize) {
    let mut units = [0u16; 2];
    let len = ch.encode_utf16(&mut units).len();
    (units, len)
}

/// The UTF-16 units of one character after the first, in send order.
///
/// Empty when there is no character (`len == 0`, as for a normal key-down or
/// key-up) or when the character is a single BMP unit. A non-BMP character
/// yields its trailing low surrogate, so `units[0]` plus this tail is the whole
/// character. `len` is clamped to the two-unit buffer.
pub fn utf16_tail(units: &[u16; 2], len: usize) -> &[u16] {
    let end = len.min(units.len());
    if end <= 1 {
        &[]
    } else {
        &units[1..end]
    }
}

/// Convert a character range within `text` to a UTF-16 code-unit range.
///
/// `None` when the range is reversed or outside the text, or when the offsets do
/// not fit CEF's 32-bit range.
pub fn utf16_selection(text: &str, range: std::ops::Range<usize>) -> Option<(u32, u32)> {
    if range.start > range.end {
        return None;
    }
    let mut units = 0u32;
    let mut start = None;
    let mut end = None;
    for (index, ch) in text.chars().enumerate() {
        if index == range.start {
            start = Some(units);
        }
        if index == range.end {
            end = Some(units);
            break;
        }
        if index >= range.end {
            return None;
        }
        units = units.checked_add(ch.len_utf16() as u32)?;
    }
    let chars = text.chars().count();
    if start.is_none() && range.start == chars {
        start = Some(units);
    }
    if end.is_none() && range.end == chars {
        end = Some(units);
    }
    Some((start?, end?))
}

/// A physical page-relative IME bounds rectangle to window points.
///
/// Returns `[x, y, width, height]` in the same units as `origin`, or `None` for a
/// non-finite/zero scale or origin.
pub fn ime_window_rect(x: i32, y: i32, width: u32, height: u32, ppp: f32, origin: [f32; 2]) -> Option<[f32; 4]> {
    if !ppp.is_finite() || ppp <= 0.0 || !origin[0].is_finite() || !origin[1].is_finite() {
        return None;
    }
    if width == 0 || height == 0 {
        return None;
    }
    let rect = [
        origin[0] + x as f32 / ppp,
        origin[1] + y as f32 / ppp,
        width as f32 / ppp,
        height as f32 / ppp,
    ];
    rect.iter().all(|value| value.is_finite()).then_some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_range_converts_to_utf16_offsets() {
        assert_eq!(utf16_selection("a😀中", 1..2), Some((1, 3)));
        assert_eq!(utf16_selection("a😀中", 0..3), Some((0, 4)));
        assert_eq!(utf16_selection("a😀中", 0..0), Some((0, 0)));
        assert_eq!(utf16_selection("a😀中", 3..3), Some((4, 4)));
        assert_eq!(utf16_selection("a😀中", 3..2), None);
        assert_eq!(utf16_selection("a😀中", 0..4), None);
        assert_eq!(utf16_selection("", 0..0), Some((0, 0)));
    }

    #[test]
    fn non_bmp_character_is_a_surrogate_pair() {
        assert_eq!(utf16_units('😀'), ([0xD83D, 0xDE00], 2));
        assert_eq!(utf16_units('中'), ([0x4E2D, 0], 1));
        assert_eq!(utf16_units('a'), ([0x61, 0], 1));
    }

    #[test]
    fn no_character_has_no_utf16_tail_for_down_or_up() {
        // Normal key-down/key-up events carry no character: the production
        // composition yields `([0, 0], 0)` and must not slice `units[1..0]`.
        let (units, len) = Option::<char>::None.map(utf16_units).unwrap_or(([0, 0], 0));
        assert_eq!(utf16_tail(&units, len), &[] as &[u16]);
    }

    #[test]
    fn bmp_character_has_no_utf16_tail() {
        let (units, len) = utf16_units('中');
        assert_eq!(utf16_tail(&units, len), &[] as &[u16]);
    }

    #[test]
    fn non_bmp_character_tail_is_its_low_surrogate() {
        let (units, len) = utf16_units('😀');
        assert_eq!(utf16_tail(&units, len), &[0xDE00u16][..]);
        // The helper is a host-input boundary: never index past the two units.
        assert_eq!(utf16_tail(&units, usize::MAX), &[0xDE00u16][..]);
    }

    #[test]
    fn lead_and_tail_rebuild_the_character_in_order() {
        for ch in ['a', '中', '😀'] {
            let mut expected = [0u16; 2];
            let expected_len = ch.encode_utf16(&mut expected).len();
            let (units, len) = utf16_units(ch);
            let tail = utf16_tail(&units, len);
            let mut sent = [0u16; 2];
            sent[0] = units[0];
            sent[1..1 + tail.len()].copy_from_slice(tail);
            assert_eq!(&sent[..1 + tail.len()], &expected[..expected_len], "{ch:?}");
        }
    }

    #[test]
    fn ime_bounds_use_current_window_points() {
        assert_eq!(ime_window_rect(20, 40, 10, 20, 2.0, [100.0, 200.0]), Some([110.0, 220.0, 5.0, 10.0]));
        assert_eq!(ime_window_rect(20, 40, 10, 20, 1.25, [0.0, 0.0]), Some([16.0, 32.0, 8.0, 16.0]));
    }

    #[test]
    fn unusable_scale_or_origin_clears_the_output() {
        for ppp in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(ime_window_rect(20, 40, 10, 20, ppp, [0.0, 0.0]), None, "{ppp}");
        }
        for origin in [[f32::NAN, 0.0], [0.0, f32::INFINITY]] {
            assert_eq!(ime_window_rect(20, 40, 10, 20, 2.0, origin), None, "{origin:?}");
        }
        assert_eq!(ime_window_rect(0, 0, 0, 0, 2.0, [0.0, 0.0]), None);
        assert_eq!(ime_window_rect(20, 40, 10, 0, 2.0, [0.0, 0.0]), None);
    }
}
