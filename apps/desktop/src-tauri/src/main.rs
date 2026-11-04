//! The Boltget desktop app: a Tauri 2 shell around `boltget-engine`.
//!
//! The v0.2 app: one window with a custom title bar, the theme plumbing and
//! the native details that make it feel like a Windows app
//! (docs/APP-DESIGN.md), and the engine bridge (`bridge`): the app owns the
//! engine and its database and streams it to the page. Around the window:
//! one instance per session (`shell`), the tray and taskbar progress
//! (`tray`), notifications (`notify`), the named pipe the CLI talks to
//! (`pipe`), and the end of the session (`RunEvent::Exit` below).
// No console window behind the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod catalog;
mod commands;
mod error;
mod ipc;
mod mark;
mod notify;
mod pipe;
mod platform;
mod shell;
mod theme;
mod tray;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use boltget_engine::EngineConfig;
use tauri::{Manager, RunEvent, WebviewWindow, WindowEvent};
use tauri_plugin_window_state::StateFlags;

use crate::bridge::Bridge;
use crate::shell::{Launch, PendingLinks, StartHidden};

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
    init_logging();
    let ipc = ipc::builder();
    let (bridge, hub) = Bridge::new(bridge::TICK);
    let launch = Launch::parse(std::env::args().skip(1));
    let app = tauri::Builder::default()
        // First, so a second launch exits before it creates anything; its
        // links come here (docs/ARCHITECTURE.md, "The desktop app").
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            shell::second_instance(app, args);
        }))
        // Size, position and maximized state, restored while the window is
        // still hidden. Visibility is ours: the UI shows it after its first
        // frame (APP-DESIGN #30).
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        // The folder picker. The capability grants the page none of its
        // commands; `choose_folder` uses it from Rust.
        .plugin(tauri_plugin_dialog::init())
        .manage(WindowShown::default())
        .manage(StartHidden(AtomicBool::new(launch.background)))
        .manage(PendingLinks::new(launch.links))
        .manage(bridge)
        .invoke_handler(ipc.invoke_handler())
        .setup(move |app| {
            ipc.mount_events(app);
            tauri::async_runtime::spawn(hub);
            let dir = data_dir();
            if let Some(dir) = &dir {
                pipe::start(app.handle(), dir.clone());
            }
            start_engine(app.handle(), dir);
            let window = app
                .get_webview_window(MAIN_WINDOW)
                .ok_or("the main window is missing from tauri.conf.json")?;
            prepare_main_window(&window);
            tray::install(app.handle())?;
            notify::start(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|app, event| {
            if let RunEvent::Exit = event {
                // Every running download writes its final checkpoint. Quitting
                // from the tray leaves them paused. Any other exit is the end
                // of the session (Tauri exits on `WM_ENDSESSION`: logoff,
                // restart, shutdown, Restart Manager), since closing the
                // window only hides it: what runs continues on the next start.
                let bridge = app.state::<Bridge>();
                if shell::quitting() {
                    tauri::async_runtime::block_on(bridge.shutdown());
                } else {
                    tauri::async_runtime::block_on(bridge.shutdown_for_session_end());
                }
            }
        }),
        Err(error) => {
            eprintln!("Boltget could not start: {error}");
            std::process::exit(1);
        }
    }
}

/// Logs go to stderr in debug builds (`RUST_LOG` filters them); release
/// builds log nothing until log files arrive (docs/ARCHITECTURE.md,
/// "Persistence").
fn init_logging() {
    if cfg!(debug_assertions) {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
    }
}

/// Where the database lives: the engine's default; debug builds accept
/// `BOLTGET_DATA_DIR`, like the CLI, so development never touches real
/// downloads.
fn data_dir() -> Option<PathBuf> {
    cfg!(debug_assertions)
        .then(|| std::env::var_os("BOLTGET_DATA_DIR"))
        .flatten()
        .map(PathBuf::from)
        .or_else(boltget_engine::default_data_dir)
}

/// Opens the engine in the background: the window does not wait for the
/// database, and shows the engine's status when it arrives.
fn start_engine(app: &tauri::AppHandle, dir: Option<PathBuf>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let bridge = app.state::<Bridge>();
        match dir {
            Some(dir) => {
                bridge
                    .open(dir, EngineConfig::default(), bridge::LOCK_RETRY)
                    .await;
            }
            None => bridge.no_data_dir(),
        }
    });
}

/// Everything that must happen before the hidden main window is first shown.
fn prepare_main_window(window: &WebviewWindow) {
    // The background behind the page matches the theme, so a resize or a slow
    // first frame never shows white on dark (or the reverse).
    let theme = window.theme().unwrap_or(tauri::Theme::Light);
    let _ = window.set_background_color(Some(theme::content(theme)));

    platform::install(window, theme == tauri::Theme::Dark);

    let events = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            platform::update_caption_layout(&events);
        }
        // The window closes to the tray (APP-DESIGN #15); quitting is in the
        // tray's menu.
        WindowEvent::CloseRequested { api, .. } if !shell::quitting() => {
            api.prevent_close();
            shell::hide_to_tray(&events);
        }
        WindowEvent::Focused(true) => events.state::<Bridge>().acknowledge(),
        _ => {}
    });

    let fallback = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_FALLBACK);
        if fallback.state::<WindowShown>().mark() && !commands::stays_hidden(&fallback) {
            commands::reveal(&fallback);
        }
    });
}
