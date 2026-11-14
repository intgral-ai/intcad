//! Intcad application mark: Intgral's V22 compact-integrated I on a light card.
//! Shared by desktop, packaging and CLI. The source art is
//! `assets/branding/logo/open-skyfall-mark.svg`; these are its seven faces, painted as the
//! file paints them: each face filled, then its outline stroked in ink with round joins. The
//! six top and side faces are ink; the front face is white inside a wider ink outline.
type P = (f32, f32);

/// One face in the source SVG's units, with the fill and the stroke width the file gives it.
struct Face {
    outline: &'static [P],
    fill: [u8; 3],
    stroke: f32,
}

/// `#1e282c`: the top and side faces and every outline.
const INK: [u8; 3] = [30, 40, 44];
/// `#fcfcfc`: the front face.
const FRONT: [u8; 3] = [252, 252, 252];
/// Stroke width of the top and side faces, and of the front face.
const EDGE: f32 = 3.0;
const FRONT_EDGE: f32 = 10.0;

/// In the file's order, which is the paint order: the front face goes on last.
const FACES: [Face; 7] = [
    Face { outline: &[(547.1514, 436.0), (573.1514, 436.0), (581.6514, 431.0925), (555.6514, 431.0925)], fill: INK, stroke: EDGE },
    Face { outline: &[(617.1514, 436.0), (643.1514, 436.0), (651.6514, 431.0925), (625.6514, 431.0925)], fill: INK, stroke: EDGE },
    Face { outline: &[(643.1514, 436.0), (643.1514, 467.0), (651.6514, 462.0925), (651.6514, 431.0925)], fill: INK, stroke: EDGE },
    Face { outline: &[(617.1514, 258.4075), (617.1514, 436.0), (625.6514, 431.0925), (625.6514, 253.5)], fill: INK, stroke: EDGE },
    Face { outline: &[(547.1514, 227.4075), (643.1514, 227.4075), (651.6514, 222.5), (555.6514, 222.5)], fill: INK, stroke: EDGE },
    Face { outline: &[(643.1514, 227.4075), (643.1514, 258.4075), (651.6514, 253.5), (651.6514, 222.5)], fill: INK, stroke: EDGE },
    Face {
        outline: &[
            (547.1514, 227.4075),
            (643.1514, 227.4075),
            (643.1514, 258.4075),
            (617.1514, 258.4075),
            (617.1514, 436.0),
            (643.1514, 436.0),
            (643.1514, 467.0),
            (547.1514, 467.0),
            (547.1514, 436.0),
            (573.1514, 436.0),
            (573.1514, 258.4075),
            (547.1514, 258.4075),
        ],
        fill: FRONT,
        stroke: FRONT_EDGE,
    },
];

/// The file's own placement of the mark on its 1024 square: `matrix(a 0 0 a e f)`. It centres
/// the mark with its outlines on the square, 720 of the 1024 tall.
const PLACE: (f32, f32, f32) = (2.868_526, -1_202.378_5, -481.944_2);
const SHEET: f32 = 1024.0;
/// The light card behind the mark.
const CARD: [u8; 3] = [252, 252, 252];
/// The share of a macOS icon's square that its shape takes (824 of 1024 points, Apple's icon
/// grid since macOS 11); the rest is the clear margin every Mac icon has.
const MACOS_SHAPE: f32 = 824.0 / 1024.0;

/// This block is the icon image data: the card fills the square.
pub fn rgba(size: usize) -> Result<Vec<u8>, String> {
    draw(size, 1.0)
}

/// The icon for a macOS icon file: the same card and mark with the margin of Apple's icon grid.
/// The Windows and Linux callers have no use for it.
#[allow(dead_code)]
pub fn rgba_macos(size: usize) -> Result<Vec<u8>, String> {
    draw(size, MACOS_SHAPE)
}

/// The card, `share` of the square wide and centred in it, with the mark on it.
fn draw(size: usize, share: f32) -> Result<Vec<u8>, String> {
    if !(1..=2048).contains(&size) {
        return Err("icon size must be 1..=2048".into());
    }
    // Coverage buffer at ss x ss samples per pixel, averaged down at the end: more samples at
    // the small sizes, where an outline is a fraction of a pixel wide.
    let ss = if size <= 64 {
        8
    } else if size <= 512 {
        4
    } else {
        2
    };
    let n = size * ss;
    let mut buf = vec![[0u8; 4]; n * n];

    // A rounded light card, so the dark mark stays visible on light and dark desktops.
    let card = n as f32 * share;
    let margin = (n as f32 - card) / 2.0;
    let radius = card * 0.22;
    for y in 0..n {
        for x in 0..n {
            let (cx, cy) = (x as f32 + 0.5 - margin, y as f32 + 0.5 - margin);
            if cx < 0.0 || cy < 0.0 || cx > card || cy > card {
                continue;
            }
            let dx = if cx < radius {
                radius - cx
            } else if cx > card - radius {
                cx - (card - radius)
            } else {
                0.0
            };
            let dy = if cy < radius {
                radius - cy
            } else if cy > card - radius {
                cy - (card - radius)
            } else {
                0.0
            };
            if dx * dx + dy * dy <= radius * radius {
                buf[y * n + x] = [CARD[0], CARD[1], CARD[2], 255];
            }
        }
    }

    // The mark where the file puts it on its square, the square being the card.
    let s = card * PLACE.0 / SHEET;
    let (ox, oy) = (margin + card * PLACE.1 / SHEET, margin + card * PLACE.2 / SHEET);
    let last = n as i32 - 1;
    // Sample rows or columns whose centres lie in `from..=to`.
    let span = |from: f32, to: f32| ((from - 0.5).ceil() as i32).max(0)..=((to - 0.5).floor() as i32).min(last);
    for face in &FACES {
        let v: Vec<P> = face.outline.iter().map(|p| (ox + p.0 * s, oy + p.1 * s)).collect();

        // The fill: an even-odd scanline over the outline.
        let (top, bottom) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        let mut xs = Vec::with_capacity(v.len());
        for y in span(top, bottom) {
            let cy = y as f32 + 0.5;
            xs.clear();
            for i in 0..v.len() {
                let (a, b) = (v[i], v[(i + 1) % v.len()]);
                if (a.1 <= cy) != (b.1 <= cy) {
                    xs.push(a.0 + (cy - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
            xs.sort_by(f32::total_cmp);
            for &[from, to] in xs.as_chunks::<2>().0 {
                for x in span(from, to) {
                    buf[y as usize * n + x as usize] = [face.fill[0], face.fill[1], face.fill[2], 255];
                }
            }
        }

        // The stroke: every sample within half the width of an edge. Measuring to the edge as a
        // segment rounds the joins, as the file asks.
        let half = face.stroke * s / 2.0;
        for i in 0..v.len() {
            let (a, b) = (v[i], v[(i + 1) % v.len()]);
            let (ex, ey) = (b.0 - a.0, b.1 - a.1);
            let length2 = ex * ex + ey * ey;
            for y in span(a.1.min(b.1) - half, a.1.max(b.1) + half) {
                for x in span(a.0.min(b.0) - half, a.0.max(b.0) + half) {
                    let (px, py) = (x as f32 + 0.5 - a.0, y as f32 + 0.5 - a.1);
                    let t = if length2 > 0.0 { ((px * ex + py * ey) / length2).clamp(0.0, 1.0) } else { 0.0 };
                    let (qx, qy) = (px - t * ex, py - t * ey);
                    if qx * qx + qy * qy <= half * half {
                        buf[y as usize * n + x as usize] = [INK[0], INK[1], INK[2], 255];
                    }
                }
            }
        }
    }

    // Average each ss x ss block down to one pixel (straight alpha = covered share).
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let (mut sum, mut cnt) = ([0u32; 3], 0u32);
            for j in 0..ss {
                for i in 0..ss {
                    let p = buf[(y * ss + j) * n + x * ss + i];
                    if p[3] != 0 {
                        cnt += 1;
                        for k in 0..3 {
                            sum[k] += u32::from(p[k]);
                        }
                    }
                }
            }
            if cnt == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let a = 255 * cnt / (ss * ss) as u32;
                rgba.extend_from_slice(&[(sum[0] / cnt) as u8, (sum[1] / cnt) as u8, (sum[2] / cnt) as u8, a as u8]);
            }
        }
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    #[test]
    fn mark_has_a_light_card_and_a_dark_isometric_i() {
        for size in [16, 24, 32, 48, 64, 128, 256] {
            let pixels = super::rgba(size).expect("valid size");
            assert_eq!(pixels.len(), size * size * 4);
            let px = pixels.as_chunks::<4>().0;
            assert!(px.iter().any(|p| p[3] == 0), "the rounded corners are transparent");
            assert!(px.iter().any(|p| p[3] == 255 && p[0] > 240), "the card is light");
            // From 24 px up the outline reaches full ink. At 16 px it is under half a pixel wide,
            // so the mark is a mid grey there (the darkest pixel measures 134): legible, but
            // fainter than the solid mark this icon drew before.
            let ink = if size >= 24 { 80 } else { 150 };
            assert!(px.iter().any(|p| p[3] == 255 && p[0] < ink), "the mark is dark at {size} px");
        }
        assert!(super::rgba(0).is_err());
        assert!(super::rgba(2049).is_err());
    }

    #[test]
    fn mark_is_centred_and_clear_of_the_card_edge() {
        let size = 256;
        let pixels = super::rgba(size).expect("valid size");
        let px = |x: usize, y: usize| pixels.as_chunks::<4>().0[y * size + x];
        // The card fills the centre, and the transparent corners leave the rounded silhouette.
        assert_eq!(px(size / 2, size / 2)[3], 255);
        assert_eq!(px(0, 0)[3], 0);
        assert_eq!(px(size - 1, size - 1)[3], 0);
        // The stem's front is white between its two ink outlines, as in the source art.
        let row = size / 2;
        assert!(px(126, row)[0] > 240, "the front of the stem is white");
        assert!(px(110, row)[0] < 80 && px(142, row)[0] < 80, "an ink outline on each side of the stem");
        assert!(px(100, row)[0] > 240 && px(156, row)[0] > 240, "the card shows on both sides of the mark");
        assert!(px(size / 2, 8)[0] > 240 || px(size / 2, 8)[3] == 0);
    }

    #[test]
    fn macos_icon_keeps_the_grid_margin() {
        let size = 1024;
        let pixels = super::rgba_macos(size).expect("valid size");
        let px = |x: usize, y: usize| pixels.as_chunks::<4>().0[y * size + x];
        // 100 clear points on every side of the 824-point shape, and the card inside it.
        for at in [0, 50, 99] {
            assert_eq!(px(at, size / 2)[3], 0);
            assert_eq!(px(size - 1 - at, size / 2)[3], 0);
            assert_eq!(px(size / 2, at)[3], 0);
            assert_eq!(px(size / 2, size - 1 - at)[3], 0);
        }
        assert_eq!(px(101, size / 2), [252, 252, 252, 255]);
        assert_eq!(px(size - 102, size / 2), [252, 252, 252, 255]);
        // The mark is the same drawing, smaller: white stem between two ink outlines.
        assert!(px(size / 2 - 2, size / 2)[0] > 240);
        assert!((0..size).any(|x| px(x, size / 2)[3] == 255 && px(x, size / 2)[0] < 80));
    }

    #[test]
    fn mark_sits_where_the_source_art_puts_it() {
        // The file centres the mark with its outlines on its square and makes it 720 of 1024
        // tall; at 1024 px a pixel is a unit of the file.
        let size = 1024;
        let pixels = super::rgba(size).expect("valid size");
        let ink = |x: usize, y: usize| {
            let p = pixels.as_chunks::<4>().0[y * size + x];
            p[3] == 255 && p[0] < 80
        };
        let rows: Vec<usize> = (0..size).filter(|&y| (0..size).any(|x| ink(x, y))).collect();
        let columns: Vec<usize> = (0..size).filter(|&x| (0..size).any(|y| ink(x, y))).collect();
        let (top, bottom) = (rows[0], rows[rows.len() - 1]);
        let (left, right) = (columns[0], columns[columns.len() - 1]);
        assert!((718..=722).contains(&(bottom - top + 1)), "{top}..{bottom}");
        assert!((top + bottom).abs_diff(size - 1) <= 2 && (left + right).abs_diff(size - 1) <= 2, "{left}..{right}, {top}..{bottom}");
    }
}
