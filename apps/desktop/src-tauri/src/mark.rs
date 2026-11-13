//! The tray's mark (brand/BRAND.md, "The tray"): the bare arrow in one
//! colour like the system's own tray icons, white on a dark taskbar and ink
//! on a light one. The pictures are the brand kit's own
//! (`brand/png/tray-{white,ink}-{16,20,24,32}.png`, resolution-reduced
//! arrows with real 1 px gaps), decoded at compile time. Their rows carry
//! the signature motion, lighting up top to bottom (BRAND.md §6: a 2.6 s
//! loop, 0.12 s stagger per row, opacity 0.15 → 1 → 1 → 0.15 at 0, 12, 85
//! and 100%, linear). The taskbar button and the window keep the app icon,
//! Onyx or its white twin (`platform::update_app_icon`).

use std::time::Duration;

use tauri::image::Image;

/// One loop of the motion.
pub(crate) const LOOP: Duration = Duration::from_millis(2600);
/// How often the tray shows the next frame: 10 per second is smooth for
/// opacity steps this small, and cheap.
pub(crate) const FRAME: Duration = Duration::from_millis(100);

const STAGGER_S: f64 = 0.12;
const REST: f64 = 0.15;
/// The canonical arrow's last row: a reduced arrow's rows spread over
/// rows 0 to 8 for their timing.
const LAST_ROW: f64 = 8.0;

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
}

/// The brand's tray glyphs, white then ink, by size.
static GLYPHS: [(u32, Image<'static>, Image<'static>); 4] = [
    (
        16,
        tauri::include_image!("../../../brand/png/tray-white-16.png"),
        tauri::include_image!("../../../brand/png/tray-ink-16.png"),
    ),
    (
        20,
        tauri::include_image!("../../../brand/png/tray-white-20.png"),
        tauri::include_image!("../../../brand/png/tray-ink-20.png"),
    ),
    (
        24,
        tauri::include_image!("../../../brand/png/tray-white-24.png"),
        tauri::include_image!("../../../brand/png/tray-ink-24.png"),
    ),
    (
        32,
        tauri::include_image!("../../../brand/png/tray-white-32.png"),
        tauri::include_image!("../../../brand/png/tray-ink-32.png"),
    ),
];

/// The glyph drawn for a small icon `size` (what Windows asks for): that
/// size when the kit has it, else the largest smaller one (Windows centres
/// it rather than blurring a scaled one), else the smallest.
fn glyph(size: u32, taskbar: Taskbar) -> &'static Image<'static> {
    let (_, white, ink) = GLYPHS
        .iter()
        .rev()
        .find(|(s, ..)| *s <= size)
        .unwrap_or(&GLYPHS[0]);
    match taskbar {
        Taskbar::Dark => white,
        Taskbar::Light => ink,
    }
}

/// The still mark for a small icon `size`: RGBA and its side in pixels.
pub(crate) fn still(size: u32, taskbar: Taskbar) -> (Vec<u8>, u32) {
    let image = glyph(size, taskbar);
    (image.rgba().to_vec(), image.width())
}

/// For each pixel row of a glyph, the canonical row whose timing it follows,
/// or `None` for a gap. The arrow's rows are the runs of rows with any lit
/// pixel; a reduced arrow's `n` rows spread evenly over the canonical 9.
fn row_timing(image: &Image<'_>) -> Vec<Option<u8>> {
    let width = image.width() as usize * 4;
    let lit: Vec<bool> = image
        .rgba()
        .chunks_exact(width)
        .map(|line| line.as_chunks::<4>().0.iter().any(|px| px[3] > 0))
        .collect();
    let mut band = Vec::with_capacity(lit.len());
    let mut count = 0u32;
    for (y, &on) in lit.iter().enumerate() {
        if on && (y == 0 || !lit[y - 1]) {
            count += 1;
        }
        band.push(on.then(|| count - 1));
    }
    let span = f64::from(count.saturating_sub(1).max(1));
    band.into_iter()
        .map(|b| {
            b.map(|b| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "0 to 8"
                )]
                let row = (f64::from(b) * LAST_ROW / span).round() as u8;
                row
            })
        })
        .collect()
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

/// The glyph at `t` seconds into the loop: each row's alpha scaled by its
/// opacity.
fn render(image: &Image<'_>, timing: &[Option<u8>], t: f64) -> Vec<u8> {
    let width = image.width() as usize * 4;
    let mut out = image.rgba().to_vec();
    for (line, row) in out.chunks_exact_mut(width).zip(timing) {
        let Some(row) = row else { continue };
        let a = opacity(*row, t);
        for px in line.as_chunks_mut::<4>().0 {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "0 to 255"
            )]
            let alpha = (f64::from(px[3]) * a).round() as u8;
            px[3] = alpha;
        }
    }
    out
}

/// Every frame of one loop, [`FRAME`] apart, for a small icon `size`.
pub(crate) fn frames(size: u32, taskbar: Taskbar) -> Vec<Vec<u8>> {
    let image = glyph(size, taskbar);
    let timing = row_timing(image);
    let count = (LOOP.as_millis() / FRAME.as_millis()) as u32;
    (0..count)
        .map(|i| render(image, &timing, f64::from(i) * FRAME.as_secs_f64()))
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
    fn the_still_mark_is_the_brand_glyph_per_taskbar() {
        let (dark, side) = still(16, Taskbar::Dark);
        assert_eq!((side, dark.len()), (16, 16 * 16 * 4));
        // The 16 px glyph: a 5 × 5 arrow in 2 px cells with 1 px gaps, the
        // shaft at x 7..9 from y 1, the head 1..15 at y 7..9.
        assert_eq!(pixel(&dark, 16, 0, 0)[3], 0);
        assert_eq!(pixel(&dark, 16, 7, 3)[3], 0, "a real gap");
        assert_eq!(pixel(&dark, 16, 7, 1), [255, 255, 255, 255]);
        assert_eq!(pixel(&dark, 16, 1, 7), [255, 255, 255, 255]);
        let (light, _) = still(16, Taskbar::Light);
        assert_eq!(pixel(&light, 16, 7, 1), [0x0A, 0x0A, 0x0B, 255]);
        assert_eq!(Taskbar::from_light(true), Taskbar::Light);
    }

    #[test]
    fn each_size_gets_the_kits_glyph() {
        for (size, side) in [
            (12, 16),
            (16, 16),
            (20, 20),
            (24, 24),
            (28, 24),
            (32, 32),
            (48, 32),
        ] {
            assert_eq!(still(size, Taskbar::Dark).1, side, "{size} px");
        }
    }

    #[test]
    fn reduced_rows_spread_over_the_canonical_timing() {
        let rows = |size| {
            let mut r: Vec<u8> = row_timing(glyph(size, Taskbar::Dark))
                .into_iter()
                .flatten()
                .collect();
            r.dedup();
            r
        };
        assert_eq!(rows(16), [0, 2, 4, 6, 8]);
        assert_eq!(rows(24), [0, 1, 3, 4, 5, 7, 8]);
        assert_eq!(rows(32), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn rows_light_up_top_to_bottom() {
        // Rows rise one after another; each holds until the next loop, so
        // just before the tip's turn the top is lit and the tip is dim.
        let image = glyph(16, Taskbar::Dark);
        let timing = row_timing(image);
        let early = render(image, &timing, 0.9);
        let top = pixel(&early, 16, 7, 1);
        let tip = pixel(&early, 16, 7, 13);
        assert!(top[3] > tip[3], "top {top:?} tip {tip:?}");
        // Mid-loop every row is lit, like the still mark.
        assert_eq!(render(image, &timing, 1.5), still(16, Taskbar::Dark).0);
        // Rows never fade out entirely.
        assert!((opacity(0, 2.599) - REST).abs() < 0.01);
        assert!(opacity(8, 0.0) >= REST);
        assert!(tip[3] >= (REST * 255.0) as u8);
    }

    #[test]
    fn one_loop_of_frames() {
        let frames = frames(16, Taskbar::Light);
        assert_eq!(frames.len(), 26);
        assert_ne!(frames[0], frames[5]);
    }
}
