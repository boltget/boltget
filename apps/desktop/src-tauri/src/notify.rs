//! Native Windows toasts (docs/APP-DESIGN.md #16): a download completed, a
//! download needs attention, or a batch summary when three or more finish
//! within a minute. Notices that arrive together (within 1.5 s) are shown
//! together; nothing is shown while the window has focus, since the list
//! already says it (and nothing for what the user just did: pausing is not
//! a notice). Windows itself holds toasts back during Focus sessions and
//! Do not disturb.
//!
//! Clicking a toast brings the window forward with the download selected;
//! its buttons open the file, show it in its folder, or offer the fix.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use tokio::sync::broadcast;

use crate::MAIN_WINDOW;
use crate::bridge::Bridge;
use crate::bridge::activity::Notice;
use crate::bridge::dto::{DownloadId, DownloadState};
use crate::catalog::{self, Arg};
use crate::shell::{self, ShellRequest};

/// Notices this close together are one moment.
const GATHER: Duration = Duration::from_millis(1500);
/// The batch window of APP-DESIGN #16.
const BATCH_WINDOW: Duration = Duration::from_secs(60);
/// Completions within the window that make a batch.
const BATCH_SIZE: usize = 3;
/// Names a batch summary lists before "and N more".
const NAMES_SHOWN: usize = 2;
/// The app setting that records that closing to the tray was explained.
const BACKGROUND_EXPLAINED_KEY: &str = "background_explained";

/// What a toast does when clicked, or when one of its buttons is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Bring the window forward.
    Show,
    Select(DownloadId),
    Open(DownloadId),
    Reveal(DownloadId),
    Fix(DownloadId),
}

impl Action {
    fn encode(self) -> String {
        match self {
            Self::Show => "show".into(),
            Self::Select(id) => format!("select:{}", id.0),
            Self::Open(id) => format!("open:{}", id.0),
            Self::Reveal(id) => format!("reveal:{}", id.0),
            Self::Fix(id) => format!("fix:{}", id.0),
        }
    }

    fn decode(s: &str) -> Option<Self> {
        if s == "show" {
            return Some(Self::Show);
        }
        let (verb, id) = s.split_once(':')?;
        let id = DownloadId(id.parse().ok()?);
        Some(match verb {
            "select" => Self::Select(id),
            "open" => Self::Open(id),
            "reveal" => Self::Reveal(id),
            "fix" => Self::Fix(id),
            _ => return None,
        })
    }
}

/// One toast, before it is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    /// What clicking the toast itself does.
    pub(crate) click: Action,
    /// Buttons: label and action.
    pub(crate) buttons: Vec<(String, Action)>,
}

/// Turns notices into toasts, remembering recent completions for batching.
#[derive(Debug, Default)]
pub(crate) struct Planner {
    completed: VecDeque<Instant>,
    summary_at: Option<Instant>,
}

impl Planner {
    /// The toasts for notices that arrived together at `now`.
    pub(crate) fn plan(&mut self, notices: &[Notice], now: Instant) -> Vec<Toast> {
        let mut toasts = Vec::new();
        let completions: Vec<&Notice> = notices
            .iter()
            .filter(|n| matches!(n, Notice::Completed { .. }))
            .collect();
        let attention: Vec<&Notice> = notices
            .iter()
            .filter(|n| matches!(n, Notice::Attention { .. }))
            .collect();

        self.completed.extend(completions.iter().map(|_| now));
        while self
            .completed
            .front()
            .is_some_and(|&t| now.duration_since(t) > BATCH_WINDOW)
        {
            self.completed.pop_front();
        }
        if !completions.is_empty() {
            if self.completed.len() >= BATCH_SIZE {
                // One summary per window: later completions in it are part of
                // the batch the user was already told about.
                let fresh = self
                    .summary_at
                    .is_none_or(|t| now.duration_since(t) > BATCH_WINDOW);
                if fresh {
                    self.summary_at = Some(now);
                    toasts.push(batch(
                        catalog::tr(
                            "notify.batch",
                            &[("count", Arg::Number(self.completed.len() as u64))],
                        ),
                        &completions,
                    ));
                }
            } else {
                toasts.extend(completions.iter().map(|n| one(n)));
            }
        }
        if attention.len() >= BATCH_SIZE {
            toasts.push(batch(
                catalog::tr(
                    "notify.attention.batch",
                    &[("count", Arg::Number(attention.len() as u64))],
                ),
                &attention,
            ));
        } else {
            toasts.extend(attention.iter().map(|n| one(n)));
        }
        toasts
    }
}

fn names(notices: &[&Notice]) -> String {
    let all: Vec<&str> = notices
        .iter()
        .map(|n| match n {
            Notice::Completed { name, .. } | Notice::Attention { name, .. } => name.as_str(),
        })
        .collect();
    let shown = all[..all.len().min(NAMES_SHOWN)].join(", ");
    let more = all.len().saturating_sub(NAMES_SHOWN);
    if more == 0 {
        shown
    } else {
        catalog::tr(
            "notify.batch.more",
            &[
                ("names", Arg::Text(&shown)),
                ("count", Arg::Number(more as u64)),
            ],
        )
    }
}

fn batch(title: String, notices: &[&Notice]) -> Toast {
    Toast {
        title,
        body: Some(names(notices)),
        click: notices
            .first()
            .map_or(Action::Show, |n| Action::Select(n.id())),
        buttons: Vec::new(),
    }
}

/// Why a download needs the user, in the catalog's words.
fn reason(state: DownloadState, key: Option<&str>) -> String {
    match (state, key) {
        (DownloadState::NeedsNewLink, _) => catalog::t("error.link_expired"),
        (DownloadState::Corrupt, _) => catalog::t("error.checksum_mismatch"),
        (_, Some(key)) if catalog::has(key) && !catalog::t(key).contains('{') => catalog::t(key),
        _ => catalog::t("error.unknown"),
    }
}

fn one(notice: &Notice) -> Toast {
    match notice {
        Notice::Completed {
            id,
            name,
            size,
            elapsed,
        } => {
            let body = match (size, elapsed) {
                (Some(size), Some(elapsed)) => Some(catalog::tr(
                    "notify.completed",
                    &[
                        ("size", Arg::Text(&catalog::bytes(*size))),
                        ("time", Arg::Text(&catalog::took(*elapsed))),
                    ],
                )),
                (Some(size), None) => Some(catalog::bytes(*size)),
                (None, _) => None,
            };
            Toast {
                title: name.clone(),
                body,
                click: Action::Select(*id),
                buttons: vec![
                    (catalog::t("action.open"), Action::Open(*id)),
                    (catalog::t("action.show"), Action::Reveal(*id)),
                ],
            }
        }
        Notice::Attention {
            id,
            name,
            state,
            error,
        } => Toast {
            title: name.clone(),
            body: Some(reason(*state, error.as_ref().map(|e| e.key.as_str()))),
            click: Action::Select(*id),
            buttons: vec![(catalog::t("notify.fix"), Action::Fix(*id))],
        },
    }
}

/// Whether the user is looking at the window.
fn window_in_use(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_WINDOW).is_some_and(|w| {
        w.is_visible().unwrap_or(false)
            && !w.is_minimized().unwrap_or(false)
            && w.is_focused().unwrap_or(false)
    })
}

/// Shows notifications for the bridge's notices, for as long as the app
/// runs.
pub(crate) fn start(app: &AppHandle) {
    let mut rx = app.state::<Bridge>().notices();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut planner = Planner::default();
        loop {
            let first = match rx.recv().await {
                Ok(n) => n,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let mut notices = vec![first];
            let until = tokio::time::Instant::now() + GATHER;
            while let Ok(Ok(n)) = tokio::time::timeout_at(until, rx.recv()).await {
                notices.push(n);
            }
            if window_in_use(&app) {
                continue;
            }
            for toast in planner.plan(&notices, Instant::now()) {
                show(&app, toast);
            }
        }
    });
}

/// Runs a toast's action.
fn act(app: &AppHandle, action: Action) {
    match action {
        Action::Show => shell::show(app),
        Action::Select(id) => shell::request(app, &ShellRequest::Select { id, fix: false }),
        Action::Fix(id) => shell::request(app, &ShellRequest::Select { id, fix: true }),
        Action::Open(id) | Action::Reveal(id) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let bridge = app.state::<Bridge>();
                let Ok(engine) = bridge.engine() else { return };
                let done = if matches!(action, Action::Open(_)) {
                    crate::bridge::commands::open_file(engine, id).await
                } else {
                    crate::bridge::commands::reveal_file(engine, id).await
                };
                if let Err(e) = done {
                    tracing::warn!(id = id.0, error = ?e, "a notification's action failed");
                }
            });
        }
    }
}

/// The first time the window closes to the tray, says so (APP-DESIGN #15).
pub(crate) fn explain_background(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let bridge = app.state::<Bridge>();
        let Ok(engine) = bridge.engine() else { return };
        // Only when it is known not to have been said, and is now recorded:
        // a window closing as the app exits finds the database closed.
        if shell::quitting()
            || !matches!(engine.app_setting(BACKGROUND_EXPLAINED_KEY).await, Ok(None))
        {
            return;
        }
        let recorded = engine
            .set_app_setting(
                BACKGROUND_EXPLAINED_KEY,
                Some(serde_json::Value::Bool(true)),
            )
            .await;
        if recorded.is_err() {
            return;
        }
        show(
            &app,
            Toast {
                title: catalog::t("notify.background.title"),
                body: Some(catalog::t("notify.background")),
                click: Action::Show,
                buttons: Vec::new(),
            },
        );
    });
}

#[cfg(windows)]
fn show(app: &AppHandle, toast: Toast) {
    use tauri_winrt_notification::Toast as WinToast;

    // An installed Boltget has a Start menu shortcut with this app id; a
    // development build does not, and Windows shows only toasts of apps it
    // knows, so those borrow PowerShell's (as Tauri's notification plugin
    // does).
    let installed = std::env::current_exe().is_ok_and(|exe| {
        !exe.ancestors()
            .any(|d| d.file_name().is_some_and(|n| n == "target"))
    });
    let app_id = if installed {
        app.config().identifier.clone()
    } else {
        WinToast::POWERSHELL_APP_ID.to_owned()
    };
    let handle = app.clone();
    let click = toast.click;
    tauri::async_runtime::spawn_blocking(move || {
        let mut win = WinToast::new(&app_id).title(&toast.title);
        if let Some(body) = &toast.body {
            win = win.text1(body);
        }
        for (label, action) in &toast.buttons {
            win = win.add_button(label, &action.encode());
        }
        let result = win
            .on_activated(move |arg| {
                let action = arg.as_deref().and_then(Action::decode).unwrap_or(click);
                act(&handle, action);
                Ok(())
            })
            .show();
        if let Err(e) = result {
            tracing::warn!(error = %e, "could not show a notification");
        }
    });
}

#[cfg(not(windows))]
fn show(_app: &AppHandle, toast: Toast) {
    // Notifications on other platforms come after 1.0 (ADR-016). The rest of
    // this module is shared; keep it compiled and warning-free here.
    let _ = (act, Action::decode);
    tracing::info!(
        title = toast.title,
        body = ?toast.body,
        click = toast.click.encode(),
        buttons = ?toast.buttons.iter().map(|(_, a)| a.encode()).collect::<Vec<_>>(),
        "notification"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::dto::{DownloadError, ErrorKind};

    fn done(id: u64, name: &str) -> Notice {
        Notice::Completed {
            id: DownloadId(id),
            name: name.into(),
            size: Some(420 * 1024 * 1024),
            elapsed: Some(Duration::from_millis(7900)),
        }
    }

    #[test]
    fn a_completion_says_how_big_and_how_fast() {
        let toasts = Planner::default().plan(&[done(1, "ubuntu.iso")], Instant::now());
        assert_eq!(
            toasts,
            [Toast {
                title: "ubuntu.iso".into(),
                body: Some("420 MiB in 7.9 s".into()),
                click: Action::Select(DownloadId(1)),
                buttons: vec![
                    ("Open".into(), Action::Open(DownloadId(1))),
                    ("Show in folder".into(), Action::Reveal(DownloadId(1))),
                ],
            }]
        );
    }

    #[test]
    fn three_within_a_minute_make_one_summary() {
        let mut p = Planner::default();
        let t0 = Instant::now();
        assert_eq!(p.plan(&[done(1, "a.iso")], t0).len(), 1);
        assert_eq!(
            p.plan(&[done(2, "b.iso")], t0 + Duration::from_secs(10))
                .len(),
            1
        );
        let third = p.plan(
            &[done(3, "c.iso"), done(4, "d.iso"), done(5, "e.iso")],
            t0 + Duration::from_secs(20),
        );
        assert_eq!(third.len(), 1, "{third:?}");
        assert_eq!(third[0].title, "5 downloads finished");
        assert_eq!(third[0].body.as_deref(), Some("c.iso, d.iso and 1 more"));
        // More in the same minute: no new toast.
        assert_eq!(
            p.plan(&[done(6, "f.iso")], t0 + Duration::from_secs(30)),
            []
        );
        // Two minutes later, a lone completion is its own toast again.
        let later = p.plan(&[done(7, "g.iso")], t0 + Duration::from_secs(150));
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].title, "g.iso");
    }

    #[test]
    fn attention_says_why_and_offers_the_fix() {
        let n = Notice::Attention {
            id: DownloadId(4),
            name: "big.zip".into(),
            state: DownloadState::Failed,
            error: Some(DownloadError {
                kind: ErrorKind::Permanent,
                key: "error.http.not_found".into(),
                message: "404".into(),
            }),
        };
        let toasts = Planner::default().plan(&[n], Instant::now());
        assert_eq!(
            toasts[0].body.as_deref(),
            Some("The file is not on the server anymore.")
        );
        assert_eq!(
            toasts[0].buttons,
            [("Fix…".into(), Action::Fix(DownloadId(4)))]
        );
        let expired = Notice::Attention {
            id: DownloadId(5),
            name: "v.mp4".into(),
            state: DownloadState::NeedsNewLink,
            error: None,
        };
        let toasts = Planner::default().plan(&[expired], Instant::now());
        assert_eq!(
            toasts[0].body.as_deref(),
            Some("The link has expired. Give it a new link to continue.")
        );
    }

    #[test]
    fn actions_survive_the_round_trip_through_windows() {
        for a in [
            Action::Show,
            Action::Select(DownloadId(3)),
            Action::Open(DownloadId(3)),
            Action::Reveal(DownloadId(12)),
            Action::Fix(DownloadId(9)),
        ] {
            assert_eq!(Action::decode(&a.encode()), Some(a));
        }
        assert_eq!(Action::decode("launch"), None);
        assert_eq!(Action::decode("open:x"), None);
    }
}
