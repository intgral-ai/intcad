//! Convert the CSS border boxes of the centred page's painted surfaces into a GDK region.

/// WebKit reports integer CSS viewport dimensions. Allow one CSS pixel for rounding, not a
/// further GTK pixel: a mask for the preceding allocation can clip the new controls.
pub(super) fn matches_allocation(css_width: f64, css_height: f64, zoom: f64, width: i32, height: i32) -> bool {
    zoom > 0.0 && zoom.is_finite() && (css_width * zoom - f64::from(width)).abs() <= zoom && (css_height * zoom - f64::from(height)).abs() <= zoom
}

/// `None` means keep the whole native allocation (e.g. for shadows and unmasked portals).
pub(super) fn mask(surfaces: &[[f64; 12]], zoom: f64, width: i32, height: i32) -> Option<Vec<[i32; 4]>> {
    let spans = spans(surfaces, zoom, width, height);
    (!spans.is_empty()).then_some(spans)
}

/// Border box (left, top, right, bottom), then the horizontal/vertical radii of its corners
/// clockwise from top left. Each rectangle returned is in GTK logical pixels.
pub(super) fn spans(surfaces: &[[f64; 12]], zoom: f64, width: i32, height: i32) -> Vec<[i32; 4]> {
    // Bound the work even if IPC or the requested allocation is malformed.
    if !(0.0..=16.0).contains(&zoom) || zoom == 0.0 || !(1..=32767).contains(&width) || !(1..=32767).contains(&height) {
        return Vec::new();
    }
    let mut out: Vec<[i32; 4]> = Vec::new();
    for surface in surfaces.iter().take(8) {
        if !surface.iter().all(|v| (-65536.0..=65536.0).contains(v)) {
            continue;
        }
        let [l, t, r, b, tlx, tly, trx, try_, brx, bry, blx, bly] = surface.map(|v| v * zoom);
        if r <= l || b <= t {
            continue;
        }
        let (tlx, tly, trx, try_, brx, bry, blx, bly) =
            (tlx.max(0.), tly.max(0.), trx.max(0.), try_.max(0.), brx.max(0.), bry.max(0.), blx.max(0.), bly.max(0.));
        // CSS scales all radii together when the curves would overlap.
        let factor = 1.0_f64.min((r - l) / (tlx + trx)).min((r - l) / (blx + brx)).min((b - t) / (tly + bly)).min((b - t) / (try_ + bry));
        let inset = |rx: f64, ry: f64, dy: f64| {
            let ry = ry * factor;
            if ry <= 0.0 || dy >= ry {
                0.0
            } else {
                let normalized = 1.0 - (dy / ry).max(0.0);
                rx * factor * (1.0 - (1.0 - normalized * normalized).sqrt())
            }
        };
        let start = out.len();
        // A row starting above a fractional top can be entirely unpainted by WebKit.
        // Don't expose this opaque native row by rounding the top outwards.
        for y in (t.ceil() as i32).clamp(0, height)..(b.ceil() as i32).clamp(0, height) {
            // Use the narrowest point of the curve across the whole row, then include only
            // complete pixels on that side. Partial arc pixels can expose opaque backing.
            // Preserve the old rounding on a straight side, including opposite a curve.
            let top = f64::from(y) - t;
            let bottom = b - f64::from(y + 1);
            let left_inset = inset(tlx, tly, top).max(inset(blx, bly, bottom));
            let right_inset = inset(trx, try_, top).max(inset(brx, bry, bottom));
            let left = if left_inset > 0.0 { (l + left_inset).ceil() } else { l.floor() } as i32;
            let right = if right_inset > 0.0 { (r - right_inset).floor() } else { r.ceil() } as i32;
            let (left, right) = (left.clamp(0, width), right.clamp(0, width));
            if right <= left {
                continue;
            }
            let span_width = right - left;
            if out.len() > start
                && let Some(last) = out.last_mut()
                && last[0] == left
                && last[2] == span_width
                && last[1] + last[3] == y
            {
                last[3] += 1;
            } else {
                out.push([left, y, span_width, 1]);
            }
        }
    }
    out
}
