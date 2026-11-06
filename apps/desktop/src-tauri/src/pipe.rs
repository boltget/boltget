//! The named pipe the app serves to the `boltget` CLI (docs/ARCHITECTURE.md,
//! "CLI to app (named pipe)"): the protocol and its security live in
//! `boltget-ipc`; here the app connects it to its engine and its list.
//!
//! The pipe is served from the start, before the engine is open: a client
//! that arrives early waits for it, and one that arrives while a standalone
//! `boltget` holds the database is told to open it itself.

use std::path::PathBuf;
use std::sync::Arc;

use boltget_engine::{DownloadId, DownloadSummary};
use boltget_ipc::{Endpoint, Owner, Server};
use tauri::{AppHandle, Manager};

use crate::bridge::Bridge;
use crate::bridge::dto::DownloadRow;

/// Keeps the app's list current with what clients change.
struct App(AppHandle);

impl Owner for App {
    fn changed(&self, summary: &DownloadSummary) {
        self.0.state::<Bridge>().upsert(DownloadRow::from(summary));
    }

    fn removed(&self, id: DownloadId) {
        self.0.state::<Bridge>().removed(id.into());
    }

    fn limit_changed(&self, bytes_per_second: Option<u64>) {
        self.0.state::<Bridge>().limit_changed(bytes_per_second);
    }
}

/// Serves the pipe for the database in `dir`, for as long as the app runs.
pub(crate) fn start(app: &AppHandle, dir: PathBuf) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // The endpoint names the directory as the CLI will find it, which
        // needs it to exist.
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(error = %e, "no data directory; the CLI cannot reach the app");
            return;
        }
        let endpoint = match Endpoint::for_data_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "no pipe name; the CLI cannot reach the app");
                return;
            }
        };
        let server = match Server::bind(endpoint).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "could not serve the pipe; the CLI cannot reach the app");
                return;
            }
        };
        tracing::info!(pipe = server.endpoint().display(), "serving the CLI");
        let ownership = app.state::<Bridge>().ownership();
        let name = format!("Boltget {}", env!("CARGO_PKG_VERSION"));
        server
            .serve(ownership, Arc::new(App(app.clone())), name)
            .await;
    });
}
