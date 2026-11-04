//! The tray's mark, drawn from the brand geometry (brand/BRAND.md): the
//! pixel-snapped small icons (`svg/icon-16.svg` at 16 and 32 px,
//! `svg/icon-24.svg` at 24 and 48 px), and the signature motion, the arrow's
//! rows lighting up top to bottom (BRAND.md §6: a 2.6 s loop, 0.12 s stagger
//! per row, opacity 0.15 → 1 → 1 → 0.15 at 0, 12, 85 and 100%, linear).
//!
//! Frames are rendered once, as RGBA, with 4 × 4 samples per pixel for the
//! tile's rounded corners.
#![expect(
    clippy::many_single_char_names,
    reason = "geometry reads best as x, y, t"
)]

use std::time::Duration;

/// One loop of the motion.
pub(crate) const LOOP: Duration = Duration::from_millis(2600);
/// How often the tray shows the next frame: 10 per second is smooth for
/// opacity steps this small, and cheap.
pub(crate) const FRAME: Duration = Duration::from_millis(100);

const STAGGER_S: f64 = 0.12;
const REST: f64 = 0.15;
const TILE: [f64; 3] = [11.0, 11.0, 15.0];
const STOPS: [(f64, [f64; 3]); 3] = [
    (0.0, [167.0, 243.0, 208.0]),
    (0.5, [52.0, 211.0, 153.0]),
    (1.0, [5.0, 150.0, 105.0]),
];

/// A small-size design: its grid, tile corner, gradient line and the
/// arrow's rows as `(y0, y1, x0, x1, canonical row)`.
struct Design {
    grid: f64,
    radius: f64,
    gradient: (f64, f64),
    rows: &'static [(f64, f64, f64, f64, u8)],
}

/// `svg/icon-16.svg`: one pixel per row; the shaft's five rows share the
/// canonical shaft's four.
static SMALL: Design = Design {
    grid: 16.0,
    radius: 3.5,
    gradient: (3.0, 13.0),
    rows: &[
        (3.0, 4.0, 6.0, 10.0, 0),
        (4.0, 5.0, 6.0, 10.0, 0),
        (5.0, 6.0, 6.0, 10.0, 1),
        (6.0, 7.0, 6.0, 10.0, 2),
        (7.0, 8.0, 6.0, 10.0, 3),
        (8.0, 9.0, 3.0, 13.0, 4),
        (9.0, 10.0, 4.0, 12.0, 5),
        (10.0, 11.0, 5.0, 11.0, 6),
        (11.0, 12.0, 6.0, 10.0, 7),
        (12.0, 13.0, 7.0, 9.0, 8),
    ],
};

/// `svg/icon-24.svg`: 2 px chunks.
static MEDIUM: Design = Design {
    grid: 24.0,
    radius: 5.0,
    gradient: (5.0, 19.0),
    rows: &[
        (5.0, 7.0, 9.0, 15.0, 0),
        (7.0, 9.0, 9.0, 15.0, 1),
        (9.0, 11.0, 9.0, 15.0, 2),
        (11.0, 13.0, 5.0, 19.0, 4),
        (13.0, 15.0, 7.0, 17.0, 5),
        (15.0, 17.0, 9.0, 15.0, 7),
        (17.0, 19.0, 11.0, 13.0, 8),
    ],
};

/// The design BRAND.md prescribes for an icon of `size` pixels.
fn design(size: u32) -> &'static Design {
    if size.is_multiple_of(24) || (size > 20 && !size.is_multiple_of(16)) {
        &MEDIUM
    } else {
        &SMALL
    }
}

/// A canonical row's opacity at `t` seconds into the loop.
fn opacity(row: u8, t: f64) -> f64 {
    let period = LOOP.as_secs_f64();
    let local = (t - f64::from(row) * STAGGER_S).rem_euclid(period) / period;
    if local < 0.12 {
        REST + (1.0 - REST) * local / 0.12
    } else if local < 0.85 {
        1.0
    } else {
        1.0 - (1.0 - REST) * (local - 0.85) / 0.15
    }
}

fn gradient(d: &Design, x: f64, y: f64) -> [f64; 3] {
    let (a, b) = d.gradient;
    // Along the diagonal from (a, a) to (b, b).
    let t = (((x - a) + (y - a)) / (2.0 * (b - a))).clamp(0.0, 1.0);
    let [(t0, c0), (t1, c1), (t2, c2)] = STOPS;
    let (from, to, f) = if t <= t1 {
        (c0, c1, (t - t0) / (t1 - t0))
    } else {
        (c1, c2, (t - t1) / (t2 - t1))
    };
    [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * f)
}

fn in_tile(d: &Design, x: f64, y: f64) -> bool {
    let r = d.radius;
    let (g, cx, cy) = (d.grid, x.clamp(r, d.grid - r), y.clamp(r, d.grid - r));
    (0.0..=g).contains(&x) && (0.0..=g).contains(&y) && (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// The icon at `size` × `size` pixels, RGBA. `t`: seconds into the loop, or
/// `None` for the still mark, every row lit.
pub(crate) fn render(size: u32, t: Option<f64>) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let d = design(size);
    let scale = d.grid / f64::from(size);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut cover, mut rgb) = (0.0, [0.0; 3]);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (f64::from(px) + (f64::from(sx) + 0.5) / f64::from(SAMPLES)) * scale;
                    let y = (f64::from(py) + (f64::from(sy) + 0.5) / f64::from(SAMPLES)) * scale;
                    if !in_tile(d, x, y) {
                        continue;
                    }
                    cover += 1.0;
                    let lit = d
                        .rows
                        .iter()
                        .find(|&&(y0, y1, x0, x1, _)| y >= y0 && y < y1 && x >= x0 && x < x1)
                        .map(|&(.., row)| t.map_or(1.0, |t| opacity(row, t)));
                    let color = match lit {
                        Some(a) => {
                            let g = gradient(d, x, y);
                            [0, 1, 2].map(|i| TILE[i] * (1.0 - a) + g[i] * a)
                        }
                        None => TILE,
                    };
                    for i in 0..3 {
                        rgb[i] += color[i];
                    }
                }
            }
            let n = f64::from(SAMPLES * SAMPLES);
            if cover > 0.0 {
                out.extend(rgb.map(|c| (c / cover).round() as u8));
            } else {
                out.extend([0, 0, 0]);
            }
            out.push((cover / n * 255.0).round() as u8);
        }
    }
    out
}

/// Every frame of one loop, [`FRAME`] apart.
pub(crate) fn frames(size: u32) -> Vec<Vec<u8>> {
    let count = (LOOP.as_millis() / FRAME.as_millis()) as u32;
    (0..count)
        .map(|i| render(size, Some(f64::from(i) * FRAME.as_secs_f64())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    #[test]
    fn the_still_mark_is_the_16_px_icon() {
        let img = render(16, None);
        assert_eq!(img.len(), 16 * 16 * 4);
        // The rounded corner is transparent, the tile ink.
        assert_eq!(pixel(&img, 16, 0, 0)[3], 0);
        assert_eq!(pixel(&img, 16, 1, 8), [11, 11, 15, 255]);
        // The arrow: mint at the top of the shaft, deeper green at its tip.
        let top = pixel(&img, 16, 7, 3);
        let tip = pixel(&img, 16, 8, 12);
        assert!(top[1] > 200 && top[3] == 255, "{top:?}");
        assert!(tip[1] > 150 && tip[0] < 60, "{tip:?}");
    }

    #[test]
    fn rows_light_up_top_to_bottom() {
        // Rows rise one after another; each holds until the next loop, so
        // just before the tip's turn the top is lit and the tip is dim.
        let early = render(16, Some(0.9));
        let top = pixel(&early, 16, 7, 3);
        let tip = pixel(&early, 16, 8, 12);
        assert!(top[1] > tip[1], "top {top:?} tip {tip:?}");
        // Mid-loop every row is lit, like the still mark.
        assert_eq!(render(16, Some(1.5)), render(16, None));
        // Rows never fade out entirely.
        assert!((opacity(0, 2.599) - REST).abs() < 0.01);
        assert!(opacity(8, 0.0) >= REST);
    }

    #[test]
    fn sizes_use_the_brand_designs() {
        assert_eq!(design(16).grid as u32, 16);
        assert_eq!(design(32).grid as u32, 16);
        assert_eq!(design(20).grid as u32, 16);
        assert_eq!(design(24).grid as u32, 24);
        assert_eq!(design(48).grid as u32, 24);
        let img = render(24, None);
        // The 24 px arrow's head is 14 px wide at y = 11..13.
        assert!(pixel(&img, 24, 5, 12)[1] > 100);
        assert_eq!(pixel(&img, 24, 4, 12), [11, 11, 15, 255]);
    }

    #[test]
    fn one_loop_of_frames() {
        let frames = frames(16);
        assert_eq!(frames.len(), 26);
        assert_ne!(frames[0], frames[5]);
    }
}
