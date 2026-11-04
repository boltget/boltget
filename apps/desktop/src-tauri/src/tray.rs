//! The tray and the taskbar button (docs/APP-DESIGN.md #13, #14): progress
//! outside the window.
//!
//! - **Tray icon.** The still mark while idle; the pixel arrow's motion while
//!   bytes are flowing (it freezes when everything is paused or stalled, and
//!   never moves when Windows' animation effects are off). Left click opens
//!   the window; right click opens the native menu.
//! - **Tooltip.** `3 active · 48.2 MiB/s · about 2 min left`, at most once a
//!   second so the digits do not churn.
//! - **Taskbar button.** The batch's progress (see `bridge/activity.rs`):
//!   green while downloading, yellow when paused, red when something needs
//!   attention.

use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager, Wry};

use crate::bridge::Bridge;
use crate::bridge::activity::{Activity, Taskbar};
use crate::catalog::{self, Arg};
use crate::shell::{self, ShellRequest};
use crate::{MAIN_WINDOW, mark, platform};

const MIB: u64 = 1024 * 1024;
/// The speed limits the menu offers, in MiB/s (0: none).
const LIMITS: [u64; 6] = [0, 1, 5, 10, 25, 50];
/// How often the tooltip's numbers may change.
const TOOLTIP_EVERY: Duration = Duration::from_secs(1);
/// How often the menu's limit is compared with the engine's (the window or
/// the CLI may change it).
const LIMIT_EVERY: Duration = Duration::from_secs(2);

/// The speed limit items, checked to match the engine's global limit.
struct Limits(Vec<(u64, CheckMenuItem<Wry>)>);

impl Limits {
    fn show(&self, limit: Option<u64>) {
        let exact = limit.filter(|b| b % MIB == 0).map(|b| b / MIB);
        for (mib, item) in &self.0 {
            let checked = match limit {
                None => *mib == 0,
                Some(_) => exact == Some(*mib),
            };
            let _ = item.set_checked(checked);
        }
    }
}

/// The tooltip for `a`.
pub(crate) fn tooltip(a: &Activity) -> String {
    if a.running == 0 {
        return catalog::t("tray.tooltip");
    }
    let join = |first: &str, second: &str| {
        catalog::tr(
            "status.join",
            &[("first", Arg::Text(first)), ("second", Arg::Text(second))],
        )
    };
    let active = catalog::tr("tray.active", &[("count", Arg::Number(a.running as u64))]);
    if a.bps == 0 {
        // Starting, or stalled: no speed to show yet.
        return active;
    }
    let text = join(&active, &catalog::speed(a.bps));
    match a.eta_secs {
        Some(eta) => join(&text, &catalog::eta(eta)),
        None => text,
    }
}

fn progress_bar(t: Taskbar) -> ProgressBarState {
    let (status, progress) = match t {
        Taskbar::None => (ProgressBarStatus::None, None),
        Taskbar::Indeterminate => (ProgressBarStatus::Indeterminate, None),
        Taskbar::Normal(p) => (ProgressBarStatus::Normal, Some(p)),
        Taskbar::Paused(p) => (ProgressBarStatus::Paused, Some(p)),
        Taskbar::Error(p) => (ProgressBarStatus::Error, Some(p)),
    };
    ProgressBarState {
        status: Some(status),
        progress: progress.map(u64::from),
    }
}

fn limit_label(mib: u64) -> String {
    if mib == 0 {
        catalog::t("tray.no_limit")
    } else {
        catalog::tr("speed.mib", &[("value", Arg::Number(mib))])
    }
}

/// Creates the tray icon and keeps it, and the taskbar button, current.
pub(crate) fn install(app: &AppHandle) -> tauri::Result<()> {
    let limits: Vec<(u64, CheckMenuItem<Wry>)> = LIMITS
        .iter()
        .map(|&mib| {
            CheckMenuItem::with_id(
                app,
                format!("limit:{mib}"),
                limit_label(mib),
                true,
                mib == 0,
                None::<&str>,
            )
            .map(|item| (mib, item))
        })
        .collect::<tauri::Result<_>>()?;
    let limit_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = limits
        .iter()
        .map(|(_, i)| i as &dyn tauri::menu::IsMenuItem<Wry>)
        .collect();
    let speed = Submenu::with_items(app, catalog::t("tray.speed_limit"), true, &limit_refs)?;
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", catalog::t("tray.open"), true, None::<&str>)?,
            &MenuItem::with_id(
                app,
                "add",
                catalog::t("tray.add_clipboard"),
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "pause_all",
                catalog::t("tray.pause_all"),
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "resume_all",
                catalog::t("tray.resume_all"),
                true,
                None::<&str>,
            )?,
            &speed,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "folder",
                catalog::t("tray.open_folder"),
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", catalog::t("tray.quit"), true, None::<&str>)?,
        ],
    )?;

    let size = platform::small_icon_size();
    let still = mark::render(size, None);
    let tray = TrayIconBuilder::with_id("main")
        .icon(Image::new_owned(still.clone(), size, size))
        .tooltip(catalog::t("tray.tooltip"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                shell::show(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(Limits(limits));
    tauri::async_runtime::spawn(run(app.clone(), tray, size, still));
    Ok(())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the signature Tauri's menu handler has"
)]
fn on_menu(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    match id {
        "open" => shell::show(app),
        "add" => shell::request(app, &ShellRequest::AddFromClipboard),
        "pause_all" | "resume_all" => {
            let app = app.clone();
            let pause = id == "pause_all";
            tauri::async_runtime::spawn(async move {
                let bridge = app.state::<Bridge>();
                if pause {
                    bridge.pause_all().await;
                } else {
                    bridge.resume_all().await;
                }
            });
        }
        "folder" => match app.path().download_dir() {
            Ok(dir) => {
                if let Err(e) = tauri_plugin_opener::open_path(&dir, None::<&str>) {
                    tracing::warn!(error = %e, "could not open the downloads folder");
                }
            }
            Err(e) => tracing::warn!(error = %e, "no downloads folder"),
        },
        "quit" => shell::quit(app),
        other => {
            if let Some(mib) = other
                .strip_prefix("limit:")
                .and_then(|m| m.parse::<u64>().ok())
            {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let limit = Some(mib * MIB).filter(|&b| b > 0);
                    app.state::<Bridge>().set_global_limit(limit).await;
                    // A click toggles the item; the engine's limit decides.
                    app.state::<Limits>().show(limit);
                });
            }
        }
    }
}

/// Keeps the icon, tooltip, limit checks and taskbar button current.
async fn run(app: AppHandle, tray: TrayIcon, size: u32, still: Vec<u8>) {
    let bridge = app.state::<Bridge>();
    let limits = app.state::<Limits>();
    let mut limit_check = tokio::time::interval(LIMIT_EVERY);
    let mut activity = bridge.activity();
    let mut frames: Option<Vec<Vec<u8>>> = None;
    let mut frame = 0usize;
    let mut animating = false;
    let mut ticker = tokio::time::interval(mark::FRAME);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut tip: (String, usize, Option<Instant>) = (String::new(), 0, None);
    let mut taskbar = None;
    let mut limit = None;
    loop {
        tokio::select! {
            changed = activity.changed() => {
                if changed.is_err() {
                    return;
                }
                let a = activity.borrow_and_update().clone();
                // The tooltip: new numbers once a second, a new count at once.
                if tip.2.is_none_or(|t| t.elapsed() >= TOOLTIP_EVERY) || tip.1 != a.running {
                    let text = tooltip(&a);
                    if text != tip.0 {
                        let _ = tray.set_tooltip(Some(&text));
                    }
                    tip = (text, a.running, Some(Instant::now()));
                }
                if taskbar != Some(a.taskbar) {
                    taskbar = Some(a.taskbar);
                    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
                        let _ = w.set_progress_bar(progress_bar(a.taskbar));
                    }
                }
                let animate = a.flowing() && platform::animations_enabled();
                if animate != animating {
                    animating = animate;
                    if !animating {
                        let _ = tray.set_icon(Some(Image::new_owned(still.clone(), size, size)));
                    }
                }
            }
            _ = limit_check.tick() => {
                if let Ok(engine) = bridge.engine() {
                    let now = Some(engine.limit(boltget_engine::LimitScope::Global));
                    if now != limit {
                        limit = now;
                        limits.show(now.flatten());
                    }
                }
            }
            _ = ticker.tick(), if animating => {
                let frames = frames.get_or_insert_with(|| mark::frames(size));
                frame = (frame + 1) % frames.len();
                let _ = tray.set_icon(Some(Image::new_owned(frames[frame].clone(), size, size)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_reads_like_the_design() {
        assert_eq!(tooltip(&Activity::default()), "Boltget");
        let a = Activity {
            running: 3,
            bps: 50_541_363,
            eta_secs: Some(130),
            taskbar: Taskbar::Normal(40),
        };
        assert_eq!(tooltip(&a), "3 active · 48.2 MiB/s · about 2 min left");
        let stalled = Activity { bps: 0, ..a };
        assert_eq!(tooltip(&stalled), "3 active");
    }

    #[test]
    fn the_taskbar_states_map_to_tauri() {
        let s = progress_bar(Taskbar::Paused(30));
        assert!(matches!(
            (s.status, s.progress),
            (Some(ProgressBarStatus::Paused), Some(30))
        ));
        let s = progress_bar(Taskbar::None);
        assert!(matches!(
            (s.status, s.progress),
            (Some(ProgressBarStatus::None), None)
        ));
    }
}
