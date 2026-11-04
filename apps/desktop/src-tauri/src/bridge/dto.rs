//! The UI's view of the engine: the types that cross the IPC boundary.
//!
//! They mirror the engine's types but are owned by the app, so the contract
//! with the page is explicit (camelCase fields, paths as strings, no engine
//! internals) and the engine does not depend on the binding generator. The
//! TypeScript in `apps/desktop/src/lib/bindings.ts` is generated from them
//! (`crate::ipc`). Byte counts are `u64` in Rust and `number` in TypeScript:
//! exact up to 2^53 bytes (8 PiB).

use std::path::Path;

use boltget_engine as engine;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::UiError;

/// Identifies a download; the same number the CLI shows.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub(crate) struct DownloadId(pub(crate) u64);

impl From<engine::DownloadId> for DownloadId {
    fn from(id: engine::DownloadId) -> Self {
        Self(id.0)
    }
}

impl From<DownloadId> for engine::DownloadId {
    fn from(id: DownloadId) -> Self {
        Self(id.0)
    }
}

/// Where a download is in its life (docs/ARCHITECTURE.md, "Download state
/// machine").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DownloadState {
    Queued,
    Probing,
    Downloading,
    Retrying,
    Verifying,
    Paused,
    Completed,
    Failed,
    Cancelled,
    NeedsNewLink,
    Corrupt,
}

impl DownloadState {
    /// Whether a running task works on it, so it has live progress.
    pub(crate) fn is_active(self) -> bool {
        matches!(
            self,
            Self::Probing | Self::Downloading | Self::Retrying | Self::Verifying
        )
    }
}

impl From<engine::State> for DownloadState {
    fn from(s: engine::State) -> Self {
        match s {
            engine::State::Queued => Self::Queued,
            engine::State::Probing => Self::Probing,
            engine::State::Downloading => Self::Downloading,
            engine::State::Retrying => Self::Retrying,
            engine::State::Verifying => Self::Verifying,
            engine::State::Paused => Self::Paused,
            engine::State::Completed => Self::Completed,
            engine::State::Cancelled => Self::Cancelled,
            engine::State::NeedsNewLink => Self::NeedsNewLink,
            engine::State::Corrupt => Self::Corrupt,
            // `State` is non-exhaustive: a state this app does not know yet
            // is shown as stopped with an error rather than as running.
            _ => Self::Failed,
        }
    }
}

/// How an error should be presented (docs/ARCHITECTURE.md, "Error handling").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    Transient,
    Throttled,
    LinkExpired,
    ResourceChanged,
    RangeUnsupported,
    Disk,
    Permanent,
    Integrity,
    Cancelled,
    Internal,
    Busy,
    Database,
    NotFound,
    InvalidState,
}

impl From<engine::ErrorKind> for ErrorKind {
    fn from(k: engine::ErrorKind) -> Self {
        match k {
            engine::ErrorKind::Transient => Self::Transient,
            engine::ErrorKind::Throttled => Self::Throttled,
            engine::ErrorKind::LinkExpired => Self::LinkExpired,
            engine::ErrorKind::ResourceChanged => Self::ResourceChanged,
            engine::ErrorKind::RangeUnsupported => Self::RangeUnsupported,
            engine::ErrorKind::Disk => Self::Disk,
            engine::ErrorKind::Permanent => Self::Permanent,
            engine::ErrorKind::Integrity => Self::Integrity,
            engine::ErrorKind::Cancelled => Self::Cancelled,
            engine::ErrorKind::Busy => Self::Busy,
            engine::ErrorKind::Database => Self::Database,
            engine::ErrorKind::NotFound => Self::NotFound,
            engine::ErrorKind::InvalidState => Self::InvalidState,
            // Non-exhaustive, like `State`.
            _ => Self::Internal,
        }
    }
}

/// Why a download stopped: a catalog key plus an English diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub(crate) struct DownloadError {
    pub(crate) kind: ErrorKind,
    /// Message-catalog key, such as `error.http.not_found`.
    pub(crate) key: String,
    /// English diagnostic for logs and the details pane's log; not UI text.
    pub(crate) message: String,
}

impl From<&engine::ErrorInfo> for DownloadError {
    fn from(e: &engine::ErrorInfo) -> Self {
        Self {
            kind: e.kind.into(),
            key: e.key.to_owned(),
            message: e.message.clone(),
        }
    }
}

impl From<&engine::SavedError> for DownloadError {
    fn from(e: &engine::SavedError) -> Self {
        Self {
            kind: e.kind.into(),
            key: e.key.clone(),
            message: e.message.clone(),
        }
    }
}

/// One row of the list: everything the row and the details pane's header
/// show. Sent whole in snapshots and whenever something other than progress
/// changes; progress alone travels as [`ProgressItem`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadRow {
    pub(crate) id: DownloadId,
    pub(crate) state: DownloadState,
    /// The link, redacted (no query string or credentials).
    pub(crate) url: String,
    /// Resolved once the link has been probed.
    pub(crate) file_name: Option<String>,
    /// The final path, once probed.
    pub(crate) path: Option<String>,
    /// Total size in bytes, if known.
    pub(crate) total: Option<u64>,
    /// Bytes received so far.
    pub(crate) downloaded: u64,
    /// Current speed in bytes per second (0 when not running).
    pub(crate) bps: u64,
    /// Open connections (0 when not running).
    pub(crate) connections: u32,
    /// Whether the server lets an interrupted download continue.
    pub(crate) resumable: bool,
    /// The download's own speed limit in bytes per second.
    pub(crate) speed_limit: Option<u64>,
    /// Why it stopped, for failed (and retrying) downloads.
    pub(crate) error: Option<DownloadError>,
    /// Unix milliseconds.
    pub(crate) created_at: u64,
    /// Unix milliseconds.
    pub(crate) completed_at: Option<u64>,
}

pub(crate) fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl From<&engine::DownloadSummary> for DownloadRow {
    fn from(s: &engine::DownloadSummary) -> Self {
        Self {
            id: s.id.into(),
            state: s.state.into(),
            url: s.url.clone(),
            file_name: s.file_name.clone(),
            path: s.path.as_deref().map(path_string),
            total: s.total,
            downloaded: s.downloaded,
            bps: 0,
            connections: 0,
            resumable: s.resumable,
            speed_limit: s.speed_limit,
            error: s.error.as_ref().map(DownloadError::from),
            created_at: s.created_at,
            completed_at: s.completed_at,
        }
    }
}

/// A byte range of a download and how far it has been received: one run of
/// squares in the details pane's file map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub(crate) struct SegmentSpan {
    /// First byte (inclusive).
    pub(crate) start: u64,
    /// One past the last byte; `null` while the total size is unknown.
    pub(crate) end: Option<u64>,
    /// Bytes received from `start`.
    pub(crate) received: u64,
    /// The connection working on it, if any.
    pub(crate) connection: Option<u32>,
}

impl From<&engine::segment::Segment> for SegmentSpan {
    fn from(s: &engine::segment::Segment) -> Self {
        Self {
            start: s.start,
            end: (s.end != u64::MAX).then_some(s.end),
            received: s.received,
            connection: s.worker,
        }
    }
}

/// Live progress of one running download, sent only when it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProgressItem {
    pub(crate) id: DownloadId,
    pub(crate) downloaded: u64,
    pub(crate) total: Option<u64>,
    /// Bytes per second, averaged over the last three seconds.
    pub(crate) bps: u64,
    pub(crate) connections: u32,
    /// The file map, only for the focused download (the one in the details
    /// pane) and only when it changed; otherwise `null`.
    pub(crate) segments: Option<Vec<SegmentSpan>>,
}

/// Something that happened to a download, for notifications and motion. The
/// row in the same batch already carries the resulting state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum DownloadEvent {
    Added {
        id: DownloadId,
    },
    StateChanged {
        id: DownloadId,
        state: DownloadState,
        error: Option<DownloadError>,
    },
    Completed {
        id: DownloadId,
        path: String,
    },
    Removed {
        id: DownloadId,
    },
}

/// Settings the engine keeps. Grows with the settings window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSettings {
    /// Global speed limit in bytes per second; `null` is unlimited.
    pub(crate) speed_limit: Option<u64>,
}

impl From<&engine::Settings> for AppSettings {
    fn from(s: &engine::Settings) -> Self {
        Self {
            speed_limit: s.speed_limit,
        }
    }
}

/// Whether the engine is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum EngineStatus {
    /// Opening the download database.
    Starting,
    Ready,
    /// Another process (a `boltget` CLI) owns the database in `dir`. The app
    /// keeps trying and becomes ready once it is released.
    Locked {
        dir: String,
    },
    /// The database could not be opened.
    Failed {
        error: UiError,
    },
}

/// Everything the backend streams to a subscribed page, in order. The first
/// message is always a `snapshot`; later ones are applied on top of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum EngineMessage {
    /// The whole state: on subscribe, when the window becomes visible again,
    /// and after the backend lost track of engine events.
    Snapshot {
        status: EngineStatus,
        downloads: Vec<DownloadRow>,
        settings: AppSettings,
    },
    /// The engine's status changed.
    Status { status: EngineStatus },
    /// Settings changed.
    Settings { settings: AppSettings },
    /// Everything that changed during one tick (about 250 ms), coalesced:
    /// rows that were added or changed, rows that were removed, progress of
    /// running downloads, and what happened.
    Batch {
        rows: Vec<DownloadRow>,
        removed: Vec<DownloadId>,
        progress: Vec<ProgressItem>,
        events: Vec<DownloadEvent>,
    },
}

/// What a link points at, for the add sheet's preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbePreview {
    /// Where the link led after redirects, redacted.
    pub(crate) final_url: String,
    pub(crate) file_name: String,
    pub(crate) size: Option<u64>,
    /// Whether an interrupted download can continue.
    pub(crate) resumable: bool,
    pub(crate) content_type: Option<String>,
    /// The link returns a web page rather than a file (it may need a sign-in).
    pub(crate) web_page: bool,
    pub(crate) redirects: u32,
    /// A checksum of the file the server sent; downloads verify it unless the
    /// user gives one.
    pub(crate) checksum: Option<ServerChecksum>,
}

/// A checksum the server advertised for the whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerChecksum {
    /// `md5`, `sha1`, `sha256` or `sha512`.
    pub(crate) algorithm: String,
    /// Lowercase hex.
    pub(crate) digest: String,
    /// The response header it came from.
    pub(crate) header: String,
}

impl From<&engine::ProbeResult> for ProbePreview {
    fn from(p: &engine::ProbeResult) -> Self {
        let web_page = p.content_type.as_deref().is_some_and(|t| {
            let t = t.trim().to_ascii_lowercase();
            t.starts_with("text/html") || t.starts_with("application/xhtml+xml")
        });
        Self {
            final_url: engine::redact_url(&p.final_url),
            file_name: p.filename.clone(),
            size: p.size,
            resumable: p.resumable(),
            content_type: p.content_type.clone(),
            web_page,
            redirects: u32::try_from(p.redirects).unwrap_or(u32::MAX),
            checksum: p.server_digest.as_ref().map(|d| ServerChecksum {
                algorithm: d.checksum.algo.as_str().to_owned(),
                digest: d.checksum.hex.clone(),
                header: d.header.clone(),
            }),
        }
    }
}

/// What the details pane shows beyond the row: the checksum, the links the
/// download has used and where its partial file is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadDetails {
    pub(crate) id: DownloadId,
    /// The in-progress file, while the download is not completed.
    pub(crate) partial_path: Option<String>,
    /// The expected checksum and, once checked, the actual digest.
    pub(crate) checksum: Option<ChecksumInfo>,
    /// The links the download has used, oldest first, redacted.
    pub(crate) sources: Vec<LinkSource>,
}

/// A checksum the download is verified against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChecksumInfo {
    /// `md5`, `sha1`, `sha256` or `sha512`.
    pub(crate) algorithm: String,
    /// The expected digest, lowercase hex.
    pub(crate) expected: String,
    /// What the file hashed to once verified (lowercase hex).
    pub(crate) actual: Option<String>,
    /// Where the expected digest came from: `null` when the user gave it,
    /// otherwise the response header the server sent it in.
    pub(crate) header: Option<String>,
}

impl From<&engine::ChecksumStatus> for ChecksumInfo {
    fn from(c: &engine::ChecksumStatus) -> Self {
        Self {
            algorithm: c.expected.algo.as_str().to_owned(),
            expected: c.expected.hex.clone(),
            actual: c.actual.clone(),
            header: match &c.source {
                engine::ChecksumSource::User => None,
                engine::ChecksumSource::Server(h) => Some(h.clone()),
            },
        }
    }
}

/// A link a download has used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkSource {
    /// Redacted (no query string or credentials).
    pub(crate) url: String,
    /// `primary` (as added) or `refreshed` (given later).
    pub(crate) role: String,
}

/// A download to start.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddDownload {
    pub(crate) url: String,
    /// The folder; defaults to the user's Downloads folder.
    #[specta(optional)]
    pub(crate) directory: Option<String>,
    /// A file name typed by the user; defaults to the one the probe resolves.
    #[specta(optional)]
    pub(crate) file_name: Option<String>,
    /// Connections; adaptive when absent.
    #[specta(optional)]
    pub(crate) connections: Option<u32>,
    /// Bytes per second.
    #[specta(optional)]
    pub(crate) speed_limit: Option<u64>,
    /// `sha256:<hex>` (also `md5`, `sha1`, `sha512`) or bare hex.
    #[specta(optional)]
    pub(crate) checksum: Option<String>,
    /// Add it paused instead of starting it.
    #[specta(optional)]
    pub(crate) paused: Option<bool>,
}

/// What the add sheet offers before anything is typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddDefaults {
    /// The user's Downloads folder, if Windows has one.
    pub(crate) downloads_folder: Option<String>,
    /// Folders downloads were last added to, newest first (at most three).
    pub(crate) recent_folders: Vec<String>,
}

/// Where a download would be saved, for the add sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DestinationCheck {
    /// The name it would be saved under: sanitized, with ` (1)` appended if
    /// the name is taken.
    pub(crate) file_name: String,
    /// The name was taken, so a number was appended.
    pub(crate) renamed: bool,
    /// Whether the folder exists (the engine creates it otherwise).
    pub(crate) folder_exists: bool,
    /// Free space on the folder's drive, if known.
    pub(crate) free_bytes: Option<u64>,
}

/// Which speed limit to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum LimitScope {
    /// All downloads together.
    Global,
    /// One download.
    Download { id: DownloadId },
}

impl From<LimitScope> for engine::LimitScope {
    fn from(s: LimitScope) -> Self {
        match s {
            LimitScope::Global => Self::Global,
            LimitScope::Download { id } => Self::Download(id.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_serialize_like_the_engine() {
        let all = [
            engine::State::Queued,
            engine::State::Probing,
            engine::State::Downloading,
            engine::State::Retrying,
            engine::State::Verifying,
            engine::State::Paused,
            engine::State::Completed,
            engine::State::Failed,
            engine::State::Cancelled,
            engine::State::NeedsNewLink,
            engine::State::Corrupt,
        ];
        for s in all {
            let ours = serde_json::to_value(DownloadState::from(s)).unwrap();
            assert_eq!(ours, serde_json::Value::from(s.as_str()), "{s:?}");
            assert_eq!(DownloadState::from(s).is_active(), s.is_active(), "{s:?}");
        }
    }

    #[test]
    fn error_kinds_serialize_like_the_engine() {
        for name in [
            "transient",
            "throttled",
            "link_expired",
            "resource_changed",
            "range_unsupported",
            "disk",
            "permanent",
            "integrity",
            "cancelled",
            "internal",
            "busy",
            "database",
            "not_found",
            "invalid_state",
        ] {
            let k = engine::ErrorKind::parse(name).unwrap();
            let ours = serde_json::to_value(ErrorKind::from(k)).unwrap();
            assert_eq!(ours, serde_json::Value::from(name));
        }
    }

    #[test]
    fn message_shapes() {
        let msg = EngineMessage::Status {
            status: EngineStatus::Locked {
                dir: "C:\\x".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&msg).unwrap(),
            serde_json::json!({"type": "status", "status": {"state": "locked", "dir": "C:\\x"}})
        );
        let ev = DownloadEvent::StateChanged {
            id: DownloadId(3),
            state: DownloadState::NeedsNewLink,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({"kind": "stateChanged", "id": 3, "state": "needs_new_link", "error": null})
        );
        let scope: LimitScope = serde_json::from_str(r#"{"kind":"download","id":7}"#).unwrap();
        assert_eq!(scope, LimitScope::Download { id: DownloadId(7) });
    }
}
