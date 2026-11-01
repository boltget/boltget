//! Theme colours the native side needs before the page has rendered,
//! generated at build time from the design tokens (see `build.rs`).

use tauri::{Theme, window::Color};

include!(concat!(env!("OUT_DIR"), "/theme.rs"));

/// The content background of a theme: the colour of the window behind the page.
pub(crate) fn content(theme: Theme) -> Color {
    match theme {
        Theme::Dark => DARK_CONTENT,
        _ => LIGHT_CONTENT,
    }
}
