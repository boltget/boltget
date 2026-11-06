//! The engine bridge: the app owns one [`Engine`] with its database, exposes
//! it to the page as commands (`commands.rs`) and streams what happens as
//! coalesced batches (`hub.rs`, `model.rs`). docs/ARCHITECTURE.md, "IPC and
//! events", describes the contract; docs/APP-DESIGN.md, "The engine bridge as
//! built", the decisions.

pub(crate) mod activity;
pub(crate) mod commands;
pub(crate) mod dto;
mod hub;
mod model;
mod speed;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use boltget_engine::{Engine, EngineConfig, Error, LimitScope, ResumeOptions, State};
use boltget_ipc::Ownership;
use tokio::sync::watch;

use self::dto::{DownloadId, DownloadRow, DownloadState, EngineStatus};
use self::hub::{HubHandle, Request};
pub(crate) use self::hub::{Sink, TICK};
use crate::error::UiError;
use crate::settings::{self, AppSettings};

/// How often the app tries again to take the database while another process
/// (a `boltget` CLI download) owns it.
pub(crate) const LOCK_RETRY: Duration = Duration::from_secs(2);

/// How long quitting waits for running downloads to checkpoint.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the end of the session waits: Windows ends a process that is
/// still busy about 5 s after `WM_ENDSESSION`.
const SESSION_END_TIMEOUT: Duration = Duration::from_secs(4);

/// The app setting that lists the downloads to continue on the next start:
/// those that were running when the session ended (logoff, shutdown).
const RESUME_ON_START_KEY: &str = "resume_on_start";

/// The engine and the hub, managed as Tauri state.
#[derive(Debug)]
pub(crate) struct Bridge {
    engine: OnceLock<Engine>,
    hub: HubHandle,
    status: Mutex<EngineStatus>,
    /// Whether the engine is ours, for the named pipe.
    ownership: watch::Sender<Ownership>,
    /// Set once the engine has been shut down.
    shut: AtomicBool,
    /// The settings as stored, for everything that reads them (defaults
    /// until the engine is open).
    settings: watch::Sender<AppSettings>,
    /// One settings change at a time.
    updating: tokio::sync::Mutex<()>,
}

impl Bridge {
    /// Creates the bridge and the hub task, which the caller spawns.
    pub(crate) fn new(tick: Duration) -> (Self, impl Future<Output = ()> + Send + 'static) {
        let (hub, task) = HubHandle::new(tick);
        let bridge = Self {
            engine: OnceLock::new(),
            hub,
            status: Mutex::new(EngineStatus::Starting),
            ownership: watch::channel(Ownership::Starting).0,
            shut: AtomicBool::new(false),
            settings: watch::channel(AppSettings::default()).0,
            updating: tokio::sync::Mutex::new(()),
        };
        (bridge, task)
    }

    /// The engine, once it is open.
    pub(crate) fn engine(&self) -> Result<&Engine, UiError> {
        self.engine.get().ok_or_else(UiError::engine_unavailable)
    }

    pub(crate) fn status(&self) -> EngineStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_status(&self, status: EngineStatus) {
        if !matches!(status, EngineStatus::Starting | EngineStatus::Ready) {
            // Locked or failed: a pipe client should open the database itself.
            self.ownership.send_replace(Ownership::NotOwner);
        }
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status.clone();
        self.hub.send(Request::Status(status));
    }

    /// Whether the engine is ours, for the named pipe's server.
    pub(crate) fn ownership(&self) -> watch::Receiver<Ownership> {
        self.ownership.subscribe()
    }

    pub(crate) fn subscribe(&self, sink: Box<dyn Sink>) {
        self.hub.send(Request::Subscribe(sink));
    }

    pub(crate) async fn list(&self) -> Vec<DownloadRow> {
        self.hub.list().await
    }

    pub(crate) fn upsert(&self, row: DownloadRow) {
        self.hub.send(Request::Upsert(row));
    }

    pub(crate) fn set_state(&self, id: DownloadId, state: DownloadState) {
        self.hub.send(Request::State(id, state));
    }

    pub(crate) fn removed(&self, id: DownloadId) {
        self.hub.send(Request::Removed(id));
    }

    pub(crate) fn speed_limit(&self, id: DownloadId, limit: Option<u64>) {
        self.hub.send(Request::SpeedLimit(id, limit));
    }

    /// The settings now.
    pub(crate) fn settings(&self) -> AppSettings {
        self.settings.borrow().clone()
    }

    /// The settings, as they change.
    pub(crate) fn watch_settings(&self) -> watch::Receiver<AppSettings> {
        self.settings.subscribe()
    }

    /// Records new settings and sends them to every page.
    fn settings_changed(&self, settings: AppSettings) {
        self.settings.send_replace(settings.clone());
        self.hub.send(Request::Settings(settings));
    }

    /// The global speed limit changed (here, in the tray or from the CLI).
    pub(crate) fn limit_changed(&self, limit: Option<u64>) {
        let mut next = self.settings();
        next.speed_limit = limit.filter(|&b| b > 0);
        self.settings_changed(next);
    }

    /// Applies a settings change: validated, stored, then in effect.
    pub(crate) async fn update_settings(
        &self,
        patch: &settings::SettingsPatch,
    ) -> Result<AppSettings, UiError> {
        let engine = self.engine()?;
        let _one_at_a_time = self.updating.lock().await;
        let old = self.settings();
        let new = settings::patched(&old, patch)?;
        settings::store(engine, &old, &new).await?;
        if old.proxy != new.proxy {
            apply_proxy(engine, &new);
        }
        if old.max_connections != new.max_connections {
            engine.set_max_connections(new.max_connections);
        }
        // The speed limit may have changed meanwhile (the tray, the CLI).
        let mut stored = new;
        stored.speed_limit = self.settings().speed_limit;
        self.settings_changed(stored.clone());
        Ok(stored)
    }

    /// Windows' proxy settings changed: downloads follow them if they use
    /// the system proxy.
    pub(crate) fn system_proxy_changed(&self) {
        if let Ok(engine) = self.engine() {
            let current = self.settings();
            if current.proxy.mode == settings::ProxyMode::System {
                apply_proxy(engine, &current);
            }
        }
    }

    pub(crate) fn focus(&self, id: Option<DownloadId>) {
        self.hub.send(Request::Focus(id));
    }

    pub(crate) fn visible(&self, visible: bool) {
        self.hub.send(Request::Visible(visible));
    }

    /// The window gained focus (the taskbar's batch can settle).
    pub(crate) fn acknowledge(&self) {
        self.hub.send(Request::Acknowledge);
    }

    /// The aggregate for the tray and the taskbar.
    pub(crate) fn activity(&self) -> tokio::sync::watch::Receiver<activity::Activity> {
        self.hub.activity()
    }

    /// Completions and failures, for notifications.
    pub(crate) fn notices(&self) -> tokio::sync::broadcast::Receiver<activity::Notice> {
        self.hub.notices()
    }

    /// Opens the engine in `dir`, waiting while another process owns the
    /// database, then resumes what a previous run left unfinished. Run once,
    /// in the background: the window does not wait for it.
    pub(crate) async fn open(&self, dir: PathBuf, config: EngineConfig, retry: Duration) {
        let engine = loop {
            let (d, c) = (dir.clone(), config.clone());
            match tokio::task::spawn_blocking(move || Engine::open(c, d)).await {
                Ok(Ok(engine)) => break engine,
                Ok(Err(Error::DatabaseLocked { dir })) => {
                    self.set_status(EngineStatus::Locked {
                        dir: dto::path_string(&dir),
                    });
                    tokio::time::sleep(retry).await;
                }
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "could not open the download database");
                    self.set_status(EngineStatus::Failed {
                        error: UiError::from(&e),
                    });
                    return;
                }
                Err(e) => {
                    tracing::error!(error = %e, "opening the download database panicked");
                    self.set_status(EngineStatus::Failed {
                        error: UiError::with_message("error.internal", e.to_string()),
                    });
                    return;
                }
            }
        };
        // Subscribe before anything runs, so the hub misses no event.
        let events = engine.subscribe();
        let (list, settings) = match (engine.list().await, engine.settings().await) {
            (Ok(list), Ok(settings)) => (list, settings),
            (Err(e), _) | (_, Err(e)) => {
                tracing::error!(error = %e, "could not read the download database");
                self.set_status(EngineStatus::Failed {
                    error: UiError::from(&e),
                });
                return;
            }
        };
        let rows: Vec<DownloadRow> = list.iter().map(DownloadRow::from).collect();
        // The app's settings apply before anything continues.
        let mut prefs = match settings::load(&engine).await {
            Ok(prefs) => prefs,
            Err(e) => {
                tracing::warn!(error = ?e, "could not read the settings; using the defaults");
                AppSettings::default()
            }
        };
        prefs.speed_limit = settings.speed_limit;
        apply_proxy(&engine, &prefs);
        engine.set_max_connections(prefs.max_connections);
        self.settings.send_replace(prefs.clone());
        let mut resume = to_resume(engine.recovered(), &rows);
        resume.extend(take_session_end_list(&engine, &rows).await);
        resume.sort_unstable();
        resume.dedup();
        let _ = self.engine.set(engine.clone());
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = EngineStatus::Ready;
        self.ownership
            .send_replace(Ownership::Owner(engine.clone()));
        self.hub.send(Request::Ready {
            engine: engine.clone(),
            events,
            rows,
            settings: prefs,
        });
        resume_all(&engine, &resume).await;
    }

    /// The OS reports no home directory, so there is nowhere to keep the
    /// database.
    pub(crate) fn no_data_dir(&self) {
        self.set_status(EngineStatus::Failed {
            error: UiError::key("error.no_data_dir"),
        });
    }

    /// Pauses every running download (each writes its final checkpoint) and
    /// closes the database. Downloads paused this way stay paused. Only the
    /// first call does anything.
    pub(crate) async fn shutdown(&self) {
        self.shutdown_within(SHUTDOWN_TIMEOUT).await;
    }

    async fn shutdown_within(&self, timeout: Duration) {
        if self.shut.swap(true, Ordering::AcqRel) {
            return;
        }
        self.ownership.send_replace(Ownership::NotOwner);
        if let Some(engine) = self.engine.get()
            && tokio::time::timeout(timeout, engine.shutdown())
                .await
                .is_err()
        {
            tracing::warn!("downloads did not checkpoint in time; quitting anyway");
        }
    }

    /// The session ends (logoff, restart, shutdown): remembers what runs, so
    /// the next start continues it (ROADMAP v0.2), then checkpoints
    /// everything quickly. If Windows ends the process first, recovery on
    /// start continues the downloads anyway.
    pub(crate) async fn shutdown_for_session_end(&self) {
        if let Some(engine) = self.engine.get()
            && !self.shut.load(Ordering::Acquire)
        {
            let running: Vec<u64> = match engine.list().await {
                Ok(list) => list
                    .iter()
                    .filter(|s| s.state.is_active() || s.state == State::Queued)
                    .map(|s| s.id.0)
                    .collect(),
                Err(e) => {
                    tracing::warn!(error = %e, "could not list the running downloads");
                    Vec::new()
                }
            };
            if !running.is_empty() {
                tracing::info!(
                    count = running.len(),
                    "the session ends; downloads continue on the next start"
                );
                if let Err(e) = engine
                    .set_app_setting(RESUME_ON_START_KEY, Some(serde_json::json!(running)))
                    .await
                {
                    tracing::warn!(error = %e, "could not remember the running downloads");
                }
            }
        }
        self.shutdown_within(SESSION_END_TIMEOUT).await;
    }

    /// Pauses every running download (the tray's "Pause all").
    pub(crate) async fn pause_all(&self) {
        let Ok(engine) = self.engine() else { return };
        let Ok(list) = engine.list().await else {
            return;
        };
        for s in list
            .iter()
            .filter(|s| s.state.is_active() || s.state == State::Queued)
        {
            match engine.pause(s.id).await {
                Ok(state) => self.set_state(s.id.into(), state.into()),
                Err(e) => tracing::warn!(id = s.id.0, error = %e, "could not pause"),
            }
        }
    }

    /// Continues every paused download (the tray's "Resume all"; failed
    /// ones wait for the user, as in the window).
    pub(crate) async fn resume_all(&self) {
        let Ok(engine) = self.engine() else { return };
        let Ok(list) = engine.list().await else {
            return;
        };
        for s in list.iter().filter(|s| s.state == State::Paused) {
            if let Err(e) = engine.resume(s.id, ResumeOptions::default()).await {
                tracing::warn!(id = s.id.0, error = %e, "could not resume");
            }
        }
    }

    /// Changes the global speed limit (the tray's "Speed limit").
    pub(crate) async fn set_global_limit(&self, bytes_per_second: Option<u64>) {
        let Ok(engine) = self.engine() else { return };
        let limit = bytes_per_second.filter(|&b| b > 0);
        match engine.set_limit(LimitScope::Global, limit).await {
            Ok(()) => self.limit_changed(limit),
            Err(e) => tracing::warn!(error = %e, "could not change the speed limit"),
        }
    }
}

/// Gives the engine the proxy rules of `prefs` (reading Windows' settings
/// only for the system proxy).
fn apply_proxy(engine: &Engine, prefs: &AppSettings) {
    engine.set_proxy(settings::rules(&prefs.proxy, crate::platform::system_proxy));
}

/// What to continue on start (docs/ARCHITECTURE.md, "Recovery on start"):
/// downloads the engine found interrupted (the app was killed, or the
/// machine went down) and downloads still queued, which only an interrupted
/// add leaves behind while there is no scheduler. Downloads the user paused
/// stay paused.
pub(crate) fn to_resume(
    recovered: &[boltget_engine::DownloadId],
    rows: &[DownloadRow],
) -> Vec<DownloadId> {
    let mut ids: Vec<DownloadId> = recovered.iter().copied().map(Into::into).collect();
    ids.extend(
        rows.iter()
            .filter(|r| r.state == DownloadState::Queued)
            .map(|r| r.id),
    );
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The downloads the end of the last session left running, once: the list
/// is cleared as it is read. Only those still stopped are continued.
async fn take_session_end_list(engine: &Engine, rows: &[DownloadRow]) -> Vec<DownloadId> {
    let ids: Vec<u64> = match engine.app_setting(RESUME_ON_START_KEY).await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_default(),
        _ => return Vec::new(),
    };
    if let Err(e) = engine.set_app_setting(RESUME_ON_START_KEY, None).await {
        tracing::warn!(error = %e, "could not clear the downloads to continue");
    }
    rows.iter()
        .filter(|r| ids.contains(&r.id.0) && r.state == DownloadState::Paused)
        .map(|r| r.id)
        .collect()
}

async fn resume_all(engine: &Engine, ids: &[DownloadId]) {
    for &id in ids {
        match engine.resume(id.into(), ResumeOptions::default()).await {
            Ok(_) => tracing::info!(id = id.0, "resumed an interrupted download"),
            Err(e) => {
                tracing::warn!(id = id.0, error = %e, "could not resume an interrupted download");
            }
        }
    }
}

#[cfg(test)]
mod tests;
