//! Intcad application mark: Intgral's V22 compact-integrated I on a light card.
//! Shared by desktop, packaging and CLI. The source art is
//! `assets/branding/logo/open-skyfall-mark.svg`; these are its seven filled faces.
type P = (f32, f32);

/// One filled face in the source SVG's units.
struct Face {
    outline: &'static [P],
    colour: [u8; 3],
}

const INK: [u8; 3] = [30, 40, 44];

const FACES: [Face; 7] = [
    Face { outline: &[(547.1514, 436.0), (573.1514, 436.0), (581.6514, 431.0925), (555.6514, 431.0925)], colour: INK },
    Face { outline: &[(617.1514, 436.0), (643.1514, 436.0), (651.6514, 431.0925), (625.6514, 431.0925)], colour: INK },
    Face { outline: &[(643.1514, 436.0), (643.1514, 467.0), (651.6514, 462.0925), (651.6514, 431.0925)], colour: INK },
    Face { outline: &[(617.1514, 258.4075), (617.1514, 436.0), (625.6514, 431.0925), (625.6514, 253.5)], colour: INK },
    Face { outline: &[(547.1514, 227.4075), (643.1514, 227.4075), (651.6514, 222.5), (555.6514, 222.5)], colour: INK },
    Face { outline: &[(643.1514, 227.4075), (643.1514, 258.4075), (651.6514, 253.5), (651.6514, 222.5)], colour: INK },
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
        colour: INK,
    },
];

/// The mark's drawing box in SVG units: width and height.
const MARK: (f32, f32) = (104.5, 244.5);
/// The mark's centre in SVG units.
const CENTRE: (f32, f32) = (599.4, 344.75);
/// The light card behind the mark.
const CARD: [u8; 3] = [252, 252, 252];

/// This block is the icon image data.
pub fn rgba(size: usize) -> Result<Vec<u8>, String> {
    if !(1..=2048).contains(&size) {
        return Err("icon size must be 1..=2048".into());
    }
    // Coverage buffer at 4x4 samples per pixel (2x2 for big sizes), averaged down at the end.
    let ss = if size > 512 { 2 } else { 4 };
    let n = size * ss;
    let mut buf = vec![[0u8; 4]; n * n];

    // A rounded light card, so the dark mark stays visible on light and dark desktops.
    let radius = n as f32 * 0.22;
    for y in 0..n {
        for x in 0..n {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = if cx < radius {
                radius - cx
            } else if cx > n as f32 - radius {
                cx - (n as f32 - radius)
            } else {
                0.0
            };
            let dy = if cy < radius {
                radius - cy
            } else if cy > n as f32 - radius {
                cy - (n as f32 - radius)
            } else {
                0.0
            };
            if dx * dx + dy * dy <= radius * radius {
                buf[y * n + x] = [CARD[0], CARD[1], CARD[2], 255];
            }
        }
    }

    // Fit the mark to 70% of the card's height and centre it.
    let s = n as f32 * 0.70 / MARK.1;
    let (ox, oy) = (n as f32 / 2.0 - CENTRE.0 * s, n as f32 / 2.0 - CENTRE.1 * s);
    for face in &FACES {
        let v: Vec<P> = face.outline.iter().map(|p| (ox + p.0 * s, oy + p.1 * s)).collect();
        let (top, bottom) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        let mut xs = Vec::with_capacity(v.len());
        for y in ((top - 0.5).ceil() as i32).max(0)..=((bottom - 0.5).floor() as i32).min(n as i32 - 1) {
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
                for x in ((from - 0.5).ceil() as i32).max(0)..=((to - 0.5).floor() as i32).min(n as i32 - 1) {
                    buf[y as usize * n + x as usize] = [face.colour[0], face.colour[1], face.colour[2], 255];
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
            assert!(px.iter().any(|p| p[3] == 255 && p[0] < 80), "the mark is dark");
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
        // A dark mark sits between the light card edges at the centre row.
        assert!(px(size / 2, size / 2)[0] < 80);
        assert!(px(size / 2, 8)[0] > 240 || px(size / 2, 8)[3] == 0);
    }
}
