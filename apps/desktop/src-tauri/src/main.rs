//! The BoltGet desktop app: a Tauri 2 shell around `boltget-engine`.
//!
//! This is the v0.2 app shell: one window with a custom title bar, the theme
//! plumbing and the native details that make it feel like a Windows app
//! (docs/APP-DESIGN.md). The engine is not wired yet.
// No console window behind the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod platform;
mod theme;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{Manager, WebviewWindow, WindowEvent};
use tauri_plugin_window_state::StateFlags;

/// The label of the main window in `tauri.conf.json`.
pub(crate) const MAIN_WINDOW: &str = "main";

/// If the UI never reports its first frame (a crash in the page, say), the
/// window is shown anyway after this long, so the app never runs invisibly.
const SHOW_FALLBACK: Duration = Duration::from_secs(3);

/// Whether the main window has been shown since it was created.
#[derive(Debug, Default)]
pub(crate) struct WindowShown(AtomicBool);

impl WindowShown {
    /// Marks the window as shown; returns `true` the first time only.
    pub(crate) fn mark(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
}

fn main() {
    let app = tauri::Builder::default()
        // First, so a second launch exits before it creates anything.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                commands::reveal(&window);
            }
        }))
        // Size, position and maximized state, restored while the window is
        // still hidden. Visibility is ours: the UI shows it after its first
        // frame (APP-DESIGN #30).
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .manage(WindowShown::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::show_main_window,
            commands::set_window_chrome,
            commands::set_caption_layout,
        ])
        .setup(|app| {
            let window = app
                .get_webview_window(MAIN_WINDOW)
                .ok_or("the main window is missing from tauri.conf.json")?;
            prepare_main_window(&window);
            Ok(())
        })
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|_, _| {}),
        Err(error) => {
            eprintln!("BoltGet could not start: {error}");
            std::process::exit(1);
        }
    }
}

/// Everything that must happen before the hidden main window is first shown.
fn prepare_main_window(window: &WebviewWindow) {
    // The background behind the page matches the theme, so a resize or a slow
    // first frame never shows white on dark (or the reverse).
    let theme = window.theme().unwrap_or(tauri::Theme::Light);
    let _ = window.set_background_color(Some(theme::content(theme)));

    platform::install(window, theme == tauri::Theme::Dark);

    let events = window.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            platform::update_caption_layout(&events);
        }
    });

    let fallback = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_FALLBACK);
        if fallback.state::<WindowShown>().mark() {
            commands::reveal(&fallback);
        }
    });
}
