//! Window and app commands the UI invokes (docs/ARCHITECTURE.md, "UI to
//! backend"); the engine's are in `bridge/commands.rs`.
//!
//! Errors reach the UI as [`UiError`]: a string catalog key, never prose.

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{Manager, WebviewWindow, window::Color};

use crate::error::UiError;
use crate::{WindowShown, platform};

/// What the About screen and bug reports need to know about this build.
#[derive(Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppInfo {
    version: &'static str,
    debug: bool,
    /// Debug builds only: `BOLTGET_DEV_THEME=light|dark` forces a theme, so
    /// both themes can be reviewed and captured without changing Windows.
    dev_theme: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub(crate) fn app_info() -> AppInfo {
    let debug = cfg!(debug_assertions);
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        debug,
        dev_theme: debug
            .then(|| std::env::var("BOLTGET_DEV_THEME").ok())
            .flatten(),
    }
}

/// Called by the UI once its first frame is ready: only now is the window
/// shown, already at its restored size and in the right colours.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn show_main_window(window: WebviewWindow) -> Result<(), UiError> {
    if window.state::<WindowShown>().mark() && !stays_hidden(&window) {
        window.show().map_err(|_| UiError::window())?;
        let _ = window.set_focus();
    }
    Ok(())
}

/// Whether the app started with `--background`, in the tray: the first
/// show is skipped, once.
pub(crate) fn stays_hidden(window: &WebviewWindow) -> bool {
    let hidden = window
        .state::<crate::shell::StartHidden>()
        .0
        .swap(false, Ordering::AcqRel);
    if hidden {
        window.state::<crate::bridge::Bridge>().visible(false);
    }
    hidden
}

/// Brings the main window to the front: a second launch, or the fallback.
pub(crate) fn reveal(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// The resolved theme of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeName {
    Light,
    Dark,
}

/// Native colours that follow the page's theme: the background behind the
/// page and the system-drawn window border.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn set_window_chrome(
    window: WebviewWindow,
    theme: ThemeName,
    background: [u8; 3],
    border: [u8; 3],
) -> Result<(), UiError> {
    let [r, g, b] = background;
    window
        .set_background_color(Some(Color(r, g, b, 255)))
        .map_err(|_| UiError::window())?;
    platform::set_chrome(&window, theme == ThemeName::Dark, border);
    Ok(())
}

/// Quits Boltget (the palette's "Quit Boltget"; the tray has its own):
/// every download checkpoints and stays paused.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn quit_app(app: tauri::AppHandle) {
    crate::shell::quit(&app);
}

/// Where the maximize caption button is, in CSS pixels, measured from the
/// window's top-right corner (the title bar is anchored there).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Type)]
pub(crate) struct CaptionRect {
    pub(crate) right: f64,
    pub(crate) top: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// The UI reports the maximize button's place; on Windows a native window
/// over it gives the system the hit test it needs for snap layouts.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn set_caption_layout(window: WebviewWindow, maximize: CaptionRect) {
    platform::set_caption_layout(&window, maximize);
}

/// How the page should draw its maximize button: the native overlay over it
/// (Windows) reports hover and press, which the page cannot see itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CaptionMaximize {
    Rest,
    Hover,
    Press,
}
