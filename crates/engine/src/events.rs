//! Public event, state and progress types.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::error::{Error, ErrorKind};
use crate::platform::Preallocation;
use crate::probe::ProbeResult;
use crate::segment::Segment;

/// Identifies a download. With a database ([`Engine::open`](crate::Engine::open))
/// ids are persistent and never reused; in memory they count from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct DownloadId(pub u64);

impl std::fmt::Display for DownloadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The download state machine (the subset implemented in v0.1; see
/// `docs/ARCHITECTURE.md`, "Download state machine").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum State {
    /// Created, not started yet.
    Queued,
    /// Sending the probe request.
    Probing,
    /// Transferring data.
    Downloading,
    /// Every connection is failing; waiting to retry.
    Retrying,
    /// Stopped with progress checkpointed; can be resumed.
    Paused,
    /// Finished; the file has its final name.
    Completed,
    /// Stopped with an error.
    Failed,
    /// Stopped by the caller.
    Cancelled,
}

impl State {
    /// Whether the download task has stopped: completed, failed, paused or
    /// cancelled. Only `Completed` is final; the others can be resumed.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            State::Completed | State::Failed | State::Cancelled | State::Paused
        )
    }

    /// Whether a download in this state is being worked on by a running task.
    pub fn is_active(self) -> bool {
        matches!(self, State::Probing | State::Downloading | State::Retrying)
    }

    /// The `snake_case` name used in JSON and in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Queued => "queued",
            State::Probing => "probing",
            State::Downloading => "downloading",
            State::Retrying => "retrying",
            State::Paused => "paused",
            State::Completed => "completed",
            State::Failed => "failed",
            State::Cancelled => "cancelled",
        }
    }

    /// Parses [`State::as_str`] output.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => State::Queued,
            "probing" => State::Probing,
            "downloading" => State::Downloading,
            "retrying" => State::Retrying,
            "paused" => State::Paused,
            "completed" => State::Completed,
            "failed" => State::Failed,
            "cancelled" => State::Cancelled,
            _ => return None,
        })
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A serializable summary of an [`Error`] for front ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorInfo {
    /// Classification.
    pub kind: ErrorKind,
    /// Message-catalog key.
    pub key: &'static str,
    /// English diagnostic text (for logs and the CLI, not for UI strings).
    pub message: String,
}

impl From<&Error> for ErrorInfo {
    fn from(e: &Error) -> Self {
        Self {
            kind: e.kind(),
            key: e.message_key(),
            message: e.to_string(),
        }
    }
}

/// Where a download is going, known once the probe succeeded.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct DownloadInfo {
    /// What the probe found.
    pub probe: ProbeResult,
    /// The final path of the completed file.
    pub path: PathBuf,
    /// The in-progress file (`<path>.boltget`).
    pub partial_path: PathBuf,
    /// How the partial file was preallocated.
    pub preallocation: Preallocation,
    /// Connections the download started with.
    pub initial_connections: u32,
}

/// Events published on [`Engine::subscribe`](crate::Engine::subscribe).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineEvent {
    /// A download was added.
    Added {
        /// Download.
        id: DownloadId,
        /// The original URL, redacted.
        url: String,
    },
    /// The probe succeeded and the destination is fixed.
    Probed {
        /// Download.
        id: DownloadId,
        /// Details.
        info: Box<DownloadInfo>,
    },
    /// The state changed.
    StateChanged {
        /// Download.
        id: DownloadId,
        /// New state.
        state: State,
        /// The error, for `Failed` (and `Retrying`).
        error: Option<ErrorInfo>,
    },
    /// A download continues from a checkpoint (sent after `probed`).
    Resumed {
        /// Download.
        id: DownloadId,
        /// Durable bytes at the checkpoint it continues from.
        downloaded: u64,
        /// Total size, if known.
        total: Option<u64>,
    },
    /// Segments were split or reassigned (pull details with `progress`).
    SegmentsChanged {
        /// Download.
        id: DownloadId,
    },
    /// The download completed.
    Completed {
        /// Download.
        id: DownloadId,
        /// Final path.
        path: PathBuf,
    },
}

/// A point-in-time view of a download, cheap to take.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Progress {
    /// Download.
    pub id: DownloadId,
    /// Current state.
    pub state: State,
    /// Bytes received and handed to the writer.
    pub downloaded: u64,
    /// Bytes known durable on disk.
    pub durable: u64,
    /// Total size, if known.
    pub total: Option<u64>,
    /// Open connections.
    pub connections: u32,
    /// The segment table, ordered by offset.
    pub segments: Vec<Segment>,
}

/// The result of a successful download.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Completed {
    /// Download.
    pub id: DownloadId,
    /// Final path.
    pub path: PathBuf,
    /// File size.
    pub bytes: u64,
    /// Wall-clock time from start to rename.
    #[serde(serialize_with = "ser_millis")]
    pub elapsed: Duration,
    /// Highest number of simultaneous connections.
    pub max_connections: u32,
    /// Number of segments at the end (initial split plus dynamic splits).
    pub segments: usize,
    /// Times an endgame duplicate took over the tail of a slow segment.
    pub endgame_takeovers: u32,
    /// Probe details.
    pub info: DownloadInfo,
}

fn ser_millis<S: serde::Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u64(d.as_millis() as u64)
}
