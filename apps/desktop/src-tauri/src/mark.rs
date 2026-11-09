//! The tray's mark (brand/BRAND.md, "The tray"): the bare arrow, gapless
//! and in one colour like the system's own tray icons, white on a dark
//! taskbar and near-black on a light one. Its rows carry the signature
//! motion, lighting up top to bottom (BRAND.md §6: a 2.6 s loop, 0.12 s
//! stagger per row, opacity 0.15 → 1 → 1 → 0.15 at 0, 12, 85 and 100%,
//! linear). The taskbar button and the window keep the coloured app icon.
//!
//! Every cell is a whole number of pixels, so frames need no antialiasing.

use std::time::Duration;

/// One loop of the motion.
pub(crate) const LOOP: Duration = Duration::from_millis(2600);
/// How often the tray shows the next frame: 10 per second is smooth for
/// opacity steps this small, and cheap.
pub(crate) const FRAME: Duration = Duration::from_millis(100);

const STAGGER_S: f64 = 0.12;
const REST: f64 = 0.15;

/// The taskbar the icon sits on, which decides its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Taskbar {
    Dark,
    Light,
}

impl Taskbar {
    /// The taskbar Windows draws: light when `SystemUsesLightTheme` is set.
    pub(crate) fn from_light(light: bool) -> Self {
        if light { Self::Light } else { Self::Dark }
    }

    /// The arrow's colour: white on dark, `#1B1B1B` on light.
    fn ink(self) -> [u8; 3] {
        match self {
            Self::Dark => [0xFF, 0xFF, 0xFF],
            Self::Light => [0x1B, 0x1B, 0x1B],
        }
    }
}

/// A row of the arrow: first and last column, and the canonical row whose
/// timing it follows.
type Row = (u32, u32, u8);

/// The canonical 9 × 9 arrow.
const FULL: [Row; 9] = [
    (3, 5, 0),
    (3, 5, 1),
    (3, 5, 2),
    (3, 5, 3),
    (0, 8, 4),
    (1, 7, 5),
    (2, 6, 6),
    (3, 5, 7),
    (4, 4, 8),
];

/// A 7 × 7 arrow for sizes where the full grid would come out too small
/// (the favicon's arrow on dark browser chrome at 16 px).
const COMPACT: [Row; 7] = [
    (2, 4, 0),
    (2, 4, 1),
    (2, 4, 3),
    (0, 6, 4),
    (1, 5, 5),
    (2, 4, 6),
    (3, 3, 8),
];

/// How the arrow sits in a square of some size: which shape, its cell size
/// in pixels and its top-left corner.
struct Layout {
    rows: &'static [Row],
    cell: u32,
    origin: u32,
}

/// The largest whole-pixel arrow that leaves at least a pixel free on each
/// side; the full grid when both shapes come out the same size.
fn layout(size: u32) -> Layout {
    let fit = |rows: &'static [Row]| {
        let n = rows.len() as u32;
        let cell = (size.saturating_sub(2) / n).max(1);
        (rows, cell, n * cell)
    };
    let (full, compact) = (fit(&FULL), fit(&COMPACT));
    let (rows, cell, extent) = if compact.2 > full.2 { compact } else { full };
    Layout {
        rows,
        cell,
        origin: size.saturating_sub(extent) / 2,
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

/// The arrow at `size` × `size` pixels, RGBA, for a `taskbar`. `t`: seconds
/// into the loop, or `None` for the still mark, every row lit.
pub(crate) fn render(size: u32, t: Option<f64>, taskbar: Taskbar) -> Vec<u8> {
    let Layout { rows, cell, origin } = layout(size);
    let ink = taskbar.ink();
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let lit = y
                .checked_sub(origin)
                .map(|dy| dy / cell)
                .and_then(|r| rows.get(r as usize))
                .zip(x.checked_sub(origin).map(|dx| dx / cell))
                .filter(|&(&(first, last, _), col)| (first..=last).contains(&col))
                .map(|(&(.., row), _)| t.map_or(1.0, |t| opacity(row, t)));
            match lit {
                Some(a) => {
                    out.extend(ink);
                    out.push((a * 255.0).round() as u8);
                }
                None => out.extend([0, 0, 0, 0]),
            }
        }
    }
    out
}

/// Every frame of one loop, [`FRAME`] apart.
pub(crate) fn frames(size: u32, taskbar: Taskbar) -> Vec<Vec<u8>> {
    let count = (LOOP.as_millis() / FRAME.as_millis()) as u32;
    (0..count)
        .map(|i| render(size, Some(f64::from(i) * FRAME.as_secs_f64()), taskbar))
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
    fn the_still_mark_is_one_colour_per_taskbar() {
        let dark = render(16, None, Taskbar::Dark);
        assert_eq!(dark.len(), 16 * 16 * 4);
        // Transparent around the arrow; the 16 px arrow is the compact one
        // in 2 px cells: shaft x 5..11 from y 1, head 1..15 at y 7..9.
        assert_eq!(pixel(&dark, 16, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&dark, 16, 4, 2), [0, 0, 0, 0]);
        assert_eq!(pixel(&dark, 16, 5, 1), [255, 255, 255, 255]);
        assert_eq!(pixel(&dark, 16, 1, 7), [255, 255, 255, 255]);
        assert_eq!(pixel(&dark, 16, 7, 14), [255, 255, 255, 255]);
        let light = render(16, None, Taskbar::Light);
        assert_eq!(pixel(&light, 16, 5, 1), [0x1B, 0x1B, 0x1B, 255]);
        assert_eq!(Taskbar::from_light(true), Taskbar::Light);
    }

    #[test]
    fn rows_light_up_top_to_bottom() {
        // Rows rise one after another; each holds until the next loop, so
        // just before the tip's turn the top is lit and the tip is dim.
        let early = render(16, Some(0.9), Taskbar::Dark);
        let top = pixel(&early, 16, 7, 1);
        let tip = pixel(&early, 16, 7, 14);
        assert!(top[3] > tip[3], "top {top:?} tip {tip:?}");
        // Mid-loop every row is lit, like the still mark.
        assert_eq!(
            render(16, Some(1.5), Taskbar::Dark),
            render(16, None, Taskbar::Dark)
        );
        // Rows never fade out entirely.
        assert!((opacity(0, 2.599) - REST).abs() < 0.01);
        assert!(opacity(8, 0.0) >= REST);
        assert!(tip[3] >= (REST * 255.0) as u8);
    }

    #[test]
    fn each_size_gets_the_largest_whole_pixel_arrow() {
        for (size, cells, cell) in [
            (16, 7, 2),
            (20, 9, 2),
            (24, 7, 3),
            (28, 7, 3),
            (32, 7, 4),
            (40, 9, 4),
            (48, 9, 5),
        ] {
            let l = layout(size);
            assert_eq!((l.rows.len(), l.cell), (cells, cell), "{size} px");
            assert!(l.origin >= 1 && l.origin + cells as u32 * cell < size);
        }
    }

    #[test]
    fn one_loop_of_frames() {
        let frames = frames(16, Taskbar::Light);
        assert_eq!(frames.len(), 26);
        assert_ne!(frames[0], frames[5]);
    }
}
