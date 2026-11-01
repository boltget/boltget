//! The engine bridge: the app owns one [`Engine`] with its database, exposes
//! it to the page as commands (`commands.rs`) and streams what happens as
//! coalesced batches (`hub.rs`, `model.rs`). docs/ARCHITECTURE.md, "IPC and
//! events", describes the contract; docs/APP-DESIGN.md, "The engine bridge as
//! built", the decisions.

pub(crate) mod commands;
pub(crate) mod dto;
mod hub;
mod model;
mod speed;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use boltget_engine::{Engine, EngineConfig, Error, ResumeOptions};

use self::dto::{AppSettings, DownloadId, DownloadRow, DownloadState, EngineStatus};
use self::hub::{HubHandle, Request};
pub(crate) use self::hub::{Sink, TICK};
use crate::error::UiError;

/// How often the app tries again to take the database while another process
/// (a `boltget` CLI download) owns it.
pub(crate) const LOCK_RETRY: Duration = Duration::from_secs(2);

/// How long quitting waits for running downloads to checkpoint.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// The engine and the hub, managed as Tauri state.
#[derive(Debug)]
pub(crate) struct Bridge {
    engine: OnceLock<Engine>,
    hub: HubHandle,
    status: Mutex<EngineStatus>,
}

impl Bridge {
    /// Creates the bridge and the hub task, which the caller spawns.
    pub(crate) fn new(tick: Duration) -> (Self, impl Future<Output = ()> + Send + 'static) {
        let (hub, task) = HubHandle::new(tick);
        let bridge = Self {
            engine: OnceLock::new(),
            hub,
            status: Mutex::new(EngineStatus::Starting),
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
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status.clone();
        self.hub.send(Request::Status(status));
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

    pub(crate) fn settings_changed(&self, settings: AppSettings) {
        self.hub.send(Request::Settings(settings));
    }

    pub(crate) fn focus(&self, id: Option<DownloadId>) {
        self.hub.send(Request::Focus(id));
    }

    pub(crate) fn visible(&self, visible: bool) {
        self.hub.send(Request::Visible(visible));
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
        let resume = to_resume(engine.recovered(), &rows);
        let _ = self.engine.set(engine.clone());
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = EngineStatus::Ready;
        self.hub.send(Request::Ready {
            engine: engine.clone(),
            events,
            rows,
            settings: AppSettings::from(&settings),
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
    /// closes the database. Downloads paused this way stay paused.
    pub(crate) async fn shutdown(&self) {
        if let Some(engine) = self.engine.get()
            && tokio::time::timeout(SHUTDOWN_TIMEOUT, engine.shutdown())
                .await
                .is_err()
        {
            tracing::warn!("downloads did not checkpoint in time; quitting anyway");
        }
    }
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
