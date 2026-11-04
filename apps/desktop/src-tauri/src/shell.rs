//! The app as one process in the user's session: one instance, links handed
//! to it on its command line, the window closing to the tray, and requests
//! for the page from outside it (the tray, notifications, a second launch).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_specta::Event as _;

use crate::bridge::Bridge;
use crate::bridge::dto::DownloadId;
use crate::{MAIN_WINDOW, commands};

/// What the page is asked to do from outside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum ShellRequest {
    /// Open the add sheet with these links (one per line): `Boltget.exe
    /// <url>`, or a second launch with links.
    Add { links: String },
    /// Open the add sheet with the clipboard's links (the tray's "Add from
    /// clipboard").
    AddFromClipboard,
    /// Select a download and show it (a notification was clicked); with
    /// `fix`, also offer what fixes it (a new link, say).
    Select { id: DownloadId, fix: bool },
}

/// The command line: links to add, and `--background` (start in the tray,
/// docs/ARCHITECTURE.md, "The desktop app").
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Launch {
    pub(crate) links: Vec<String>,
    pub(crate) background: bool,
}

impl Launch {
    /// Reads arguments after the program name.
    pub(crate) fn parse<I: IntoIterator<Item = String>>(args: I) -> Self {
        let mut launch = Self::default();
        for arg in args {
            let lower = arg.to_ascii_lowercase();
            if lower == "--background" {
                launch.background = true;
            } else if lower.starts_with("http://") || lower.starts_with("https://") {
                launch.links.push(arg);
            }
        }
        launch
    }
}

/// Links from this process's own command line, until the page takes them.
#[derive(Debug, Default)]
pub(crate) struct PendingLinks(Mutex<Vec<String>>);

impl PendingLinks {
    pub(crate) fn new(links: Vec<String>) -> Self {
        Self(Mutex::new(links))
    }
}

/// The links `Boltget.exe` was started with, once; `null` when there are
/// none. The page opens the add sheet with them when the engine is ready.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn take_launch_links(pending: tauri::State<'_, PendingLinks>) -> Option<String> {
    let links = std::mem::take(&mut *pending.0.lock().unwrap_or_else(PoisonError::into_inner));
    (!links.is_empty()).then(|| links.join("\n"))
}

/// Whether `--background` asked for the window to stay hidden at start.
#[derive(Debug, Default)]
pub(crate) struct StartHidden(pub(crate) AtomicBool);

/// Set once the app is quitting, so closing the window no longer hides it.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Quits: every download checkpoints (`RunEvent::Exit`).
pub(crate) fn quit(app: &AppHandle) {
    QUITTING.store(true, Ordering::Release);
    app.exit(0);
}

pub(crate) fn quitting() -> bool {
    QUITTING.load(Ordering::Acquire)
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW)
}

/// Brings the window back, from the tray or a notification.
pub(crate) fn show(app: &AppHandle) {
    if let Some(window) = main_window(app) {
        commands::reveal(&window);
        app.state::<Bridge>().visible(true);
    }
}

/// Hides the window to the tray (APP-DESIGN #15). The first time, a
/// notification says where Boltget went.
pub(crate) fn hide_to_tray(window: &WebviewWindow) {
    let _ = window.hide();
    let app = window.app_handle();
    app.state::<Bridge>().visible(false);
    crate::notify::explain_background(app);
}

/// Sends the page a request, showing the window first.
pub(crate) fn request(app: &AppHandle, req: &ShellRequest) {
    show(app);
    if let Err(e) = req.emit(app) {
        tracing::warn!(error = %e, "could not reach the page");
    }
}

/// A second `Boltget.exe` started: it exits, and this one comes forward
/// with its links (docs/ARCHITECTURE.md, "Single instance").
pub(crate) fn second_instance(app: &AppHandle, args: Vec<String>) {
    let launch = Launch::parse(args.into_iter().skip(1));
    if launch.links.is_empty() {
        if !launch.background {
            show(app);
        }
    } else {
        request(
            app,
            &ShellRequest::Add {
                links: launch.links.join("\n"),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Launch;

    fn parse(args: &[&str]) -> Launch {
        Launch::parse(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn reads_links_and_background() {
        assert_eq!(parse(&[]), Launch::default());
        let l = parse(&[
            "https://x.test/a.iso",
            "--background",
            "HTTP://y.test/b",
            "notes.txt",
        ]);
        assert_eq!(l.links, ["https://x.test/a.iso", "HTTP://y.test/b"]);
        assert!(l.background);
        let l = parse(&["ftp://x.test/a", "--BACKGROUND"]);
        assert_eq!(l.links.len(), 0);
        assert!(l.background);
    }
}
