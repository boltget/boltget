//! Line-delimited JSON output (`--json`).

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use boltget_engine::{Completed, DownloadHandle, EngineEvent, Error, ErrorInfo};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::Speed;

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Line<'a> {
    Progress {
        downloaded: u64,
        total: Option<u64>,
        speed: u64,
        eta_secs: Option<u64>,
        connections: u32,
        segments: usize,
    },
    Completed {
        path: &'a std::path::Path,
        bytes: u64,
        elapsed_ms: u64,
        speed: u64,
        max_connections: u32,
        segments: usize,
    },
    Failed {
        error: ErrorInfo,
        exit_code: u8,
    },
    #[serde(rename = "failed")]
    UsageFailed {
        error: UsageError,
        exit_code: u8,
    },
}

#[derive(Serialize)]
struct UsageError {
    kind: &'static str,
    key: &'static str,
    message: String,
}

fn emit<T: Serialize>(v: &T) {
    let mut out = std::io::stdout().lock();
    if serde_json::to_writer(&mut out, v).is_ok() {
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}

/// Runs the download, printing engine events and periodic progress.
pub(crate) async fn run(
    handle: DownloadHandle,
    mut events: broadcast::Receiver<EngineEvent>,
) -> Result<Completed, (Error, Option<PathBuf>)> {
    let mut speed = Speed::default();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.tick().await;
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                // The CLI prints its own richer `completed` line below, and a
                // lagging receiver only misses events, not state.
                Ok(EngineEvent::Completed { .. }) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Ok(e) => emit(&e),
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = tick.tick() => progress(&handle, &mut speed),
        }
        if handle.state().is_terminal() {
            break;
        }
    }
    // A last progress line, so consumers always see the final byte count.
    progress(&handle, &mut speed);
    // Flush events that arrived together with the terminal state.
    while let Ok(e) = events.try_recv() {
        if !matches!(e, EngineEvent::Completed { .. }) {
            emit(&e);
        }
    }
    let partial = handle.info().map(|i| i.partial_path.clone());
    let res = handle.wait().await.map_err(|e| (e, partial));
    if let Ok(done) = &res {
        let secs = done.elapsed.as_secs_f64().max(1e-3);
        emit(&Line::Completed {
            path: &done.path,
            bytes: done.bytes,
            elapsed_ms: done.elapsed.as_millis() as u64,
            speed: (done.bytes as f64 / secs) as u64,
            max_connections: done.max_connections,
            segments: done.segments,
        });
    }
    res
}

fn progress(handle: &DownloadHandle, speed: &mut Speed) {
    let p = handle.progress();
    let bps = speed.sample(p.downloaded);
    emit(&Line::Progress {
        downloaded: p.downloaded,
        total: p.total,
        speed: bps as u64,
        eta_secs: p
            .total
            .filter(|_| bps > 0.0)
            .map(|t| (t.saturating_sub(p.downloaded) as f64 / bps) as u64),
        connections: p.connections,
        segments: p.segments.len(),
    });
}

/// Prints the `failed` line for an engine error.
pub(crate) fn failed(e: &Error, exit_code: u8) {
    emit(&Line::Failed {
        error: e.into(),
        exit_code,
    });
}

/// Prints the `failed` line for a bad command line or URL.
pub(crate) fn failed_usage(e: &anyhow::Error, exit_code: u8) {
    emit(&Line::UsageFailed {
        error: UsageError {
            kind: "permanent",
            key: "error.invalid_request",
            message: format!("{e:#}"),
        },
        exit_code,
    });
}
