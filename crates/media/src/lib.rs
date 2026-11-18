//! # boltget-media
//!
//! The media sidecars of Boltget: [yt-dlp](https://github.com/yt-dlp/yt-dlp)
//! and [FFmpeg](https://ffmpeg.org), installed on demand, verified, kept side
//! by side and updated, and a process runner that starts them safely.
//!
//! - [`Sidecars`] manages `%LOCALAPPDATA%\Boltget\tools`: it downloads a tool
//!   with the Boltget engine (from pinned hosts only), verifies it (yt-dlp: a
//!   signature by yt-dlp's pinned release key over its `SHA2-256SUMS`, then
//!   the binary's SHA-256; FFmpeg: a pinned SHA-256), unpacks it into its own
//!   version directory, smoke-tests it, and only then makes it the active
//!   version by rewriting the manifest atomically. The previous version is
//!   kept for rollback; older ones are pruned.
//! - [`runner`] starts a sidecar with an explicit argument vector (never a
//!   shell), a fixed working directory and a reduced environment, inside a
//!   Windows Job Object that kills it with Boltget, with a timeout, and only
//!   with arguments from an allowlist (no option that runs commands, loads
//!   code or reads configuration).
//! - [`Prober`] describes a web page with `yt-dlp -J` ([`MediaInfo`]: formats,
//!   subtitles, thumbnails, playlist entries), passing cookies through a
//!   temporary file ([`CookieFile`]), and [`presets()`] turns the formats into
//!   the picker's presets; [`playlist`] lists a playlist's entries and names
//!   their folder and files.
//! - [`Downloader`] downloads a page's media with yt-dlp, reading its
//!   progress from machine-readable `--progress-template` lines
//!   ([`progress`]), and can be stopped and restarted (yt-dlp continues from
//!   its partial files); [`partial`] finds a job's partial and temporary
//!   files to delete when it is removed.
//! - [`failure`] sorts yt-dlp's failures into the categories the user sees
//!   ([`ErrorCategory`]).
//!
//! The design is in `docs/VIDEO.md` ("Install, verification and updates") and
//! `docs/ARCHITECTURE.md` ("Sidecars").
#![warn(missing_docs)]

pub mod args;
mod cookies;
mod download;
mod error;
mod extract;
pub mod failure;
pub mod info;
mod manager;
mod manifest;
pub mod partial;
pub mod pgp;
mod platform;
pub mod playlist;
pub mod presets;
mod probe;
pub mod progress;
pub mod runner;
pub mod sources;
mod thumbnail;

use std::fmt;

pub use boltget_engine::{ContainerPreference, SubtitleChoice};
pub use cookies::CookieFile;
pub use download::{
    CONCURRENT_FRAGMENTS, DownloadOptions, DownloadOutcome, Downloader, PROGRESS_INTERVAL,
    STALL_TIMEOUT, proxy_for,
};
pub use error::{Error, Result};
pub use failure::{ErrorCategory, Failure, classify};
pub use info::{Codec, Entry, Format, MediaInfo, SubtitleLanguage, SubtitleTrack, Thumbnail};
pub use manager::{
    AfterFailure, DAILY, FAILURE_RECHECK, InstallEvent, Outcome, Release, Sidecars, Status,
    ToolStatus,
};
pub use playlist::{ItemRange, PlaylistEntry};
pub use presets::{Preset, ResolvedPreset, SizeEstimate, codec_label, presets};
pub use probe::{MAX_INFO_JSON, PROBE_TIMEOUT, PlaylistMode, ProbeOptions, Prober};
pub use progress::{MediaProgress, Phase, ProgressTracker};
pub use sources::{Channel, Sources};
pub use thumbnail::{MAX_THUMBNAIL, ThumbnailImage};

/// A sidecar tool, installed and updated as one unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tool {
    /// yt-dlp: extraction and media downloading.
    YtDlp,
    /// FFmpeg (with ffprobe): merging and converting.
    Ffmpeg,
}

impl Tool {
    /// Both tools, in install order.
    pub const ALL: [Tool; 2] = [Tool::YtDlp, Tool::Ffmpeg];

    /// `yt-dlp` or `ffmpeg`: the name of its directory and manifest entry.
    pub fn id(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
        }
    }

    /// The programs this tool provides.
    pub fn programs(self) -> &'static [Program] {
        match self {
            Tool::YtDlp => &[Program::YtDlp],
            Tool::Ffmpeg => &[Program::Ffmpeg, Program::Ffprobe],
        }
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// An executable that a tool provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Program {
    /// `yt-dlp`.
    YtDlp,
    /// `ffmpeg`.
    Ffmpeg,
    /// `ffprobe`.
    Ffprobe,
}

impl Program {
    /// The program's name.
    pub fn name(self) -> &'static str {
        match self {
            Program::YtDlp => "yt-dlp",
            Program::Ffmpeg => "ffmpeg",
            Program::Ffprobe => "ffprobe",
        }
    }

    /// The tool it belongs to.
    pub fn tool(self) -> Tool {
        match self {
            Program::YtDlp => Tool::YtDlp,
            Program::Ffmpeg | Program::Ffprobe => Tool::Ffmpeg,
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "yt-dlp" => Program::YtDlp,
            "ffmpeg" => Program::Ffmpeg,
            "ffprobe" => Program::Ffprobe,
            _ => return None,
        })
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
