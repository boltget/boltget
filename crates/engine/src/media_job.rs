//! Media jobs: downloads of video or audio from a web page, carried out by
//! yt-dlp (`docs/VIDEO.md`). The engine only stores them (the `media_job`
//! table, `docs/ARCHITECTURE.md`, "Data model"); `boltget-media` probes pages
//! and runs the sidecars.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::events::DownloadId;

/// Identifies a media job. Persistent and never reused, like download ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct MediaJobId(pub u64);

impl fmt::Display for MediaJobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The yt-dlp output template a job uses unless it names another: the title,
/// then the site's id for the video, which keeps two videos with one title
/// apart (`docs/VIDEO.md`, "Format and quality picker UX").
pub const DEFAULT_OUTPUT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

/// The picker's container toggle (`docs/VIDEO.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerPreference {
    /// "Prefer MP4" (the default, for compatibility): MP4 video and M4A audio
    /// where the site offers them, and the result is put in an MP4.
    #[default]
    Mp4,
    /// "Prefer original": whatever the site's best streams are, in the
    /// container they fit (often `WebM` or `MKV`).
    Original,
}

impl ContainerPreference {
    /// `mp4` or `original`, as stored.
    pub fn as_str(self) -> &'static str {
        match self {
            ContainerPreference::Mp4 => "mp4",
            ContainerPreference::Original => "original",
        }
    }

    /// The inverse of [`ContainerPreference::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mp4" => Some(ContainerPreference::Mp4),
            "original" => Some(ContainerPreference::Original),
            _ => None,
        }
    }
}

/// Where a media job is. Steps 3 and 6 of v0.4 move jobs through these; the
/// table only records them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaJobState {
    /// Created, not started.
    Queued,
    /// yt-dlp is downloading the streams.
    Downloading,
    /// FFmpeg is merging or converting them.
    Processing,
    /// Stopped by the user; continues from yt-dlp's partial files.
    Paused,
    /// Done.
    Completed,
    /// Stopped with an error.
    Failed,
    /// Stopped by the caller.
    Cancelled,
}

impl MediaJobState {
    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            MediaJobState::Queued => "queued",
            MediaJobState::Downloading => "downloading",
            MediaJobState::Processing => "processing",
            MediaJobState::Paused => "paused",
            MediaJobState::Completed => "completed",
            MediaJobState::Failed => "failed",
            MediaJobState::Cancelled => "cancelled",
        }
    }

    /// The inverse of [`MediaJobState::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => MediaJobState::Queued,
            "downloading" => MediaJobState::Downloading,
            "processing" => MediaJobState::Processing,
            "paused" => MediaJobState::Paused,
            "completed" => MediaJobState::Completed,
            "failed" => MediaJobState::Failed,
            "cancelled" => MediaJobState::Cancelled,
            _ => return None,
        })
    }
}

/// The subtitles a job asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubtitleChoice {
    /// Language codes as the site names them (`en`, `pt-BR`).
    pub languages: Vec<String>,
    /// Whether automatic captions count when a language has no subtitles of
    /// its own.
    #[serde(default)]
    pub automatic: bool,
    /// Embedded in the video (`true`) or saved next to it as files.
    #[serde(default)]
    pub embed: bool,
}

/// Why a media job failed, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaJobError {
    /// Message-catalog key (`error.media.*`).
    pub key: String,
    /// The diagnostic text (yt-dlp's message, for example).
    pub message: String,
}

/// A media job to create with [`Engine::add_media_job`](crate::Engine::add_media_job).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMediaJob {
    /// The web page the media is on (`http` or `https`).
    pub page_url: String,
    /// The title the probe found.
    pub title: Option<String>,
    /// The yt-dlp format expression (`-f`).
    pub format_expression: String,
    /// The preset it came from (`best`, `1080p`, `720p`, `small`, `audio`),
    /// or `None` for an expression the user typed.
    pub preset: Option<String>,
    /// The container toggle.
    pub container: ContainerPreference,
    /// Subtitles, if any.
    pub subtitles: Option<SubtitleChoice>,
    /// The yt-dlp output template.
    pub output_template: String,
    /// The folder the result goes to.
    pub dest_dir: PathBuf,
    /// The download row it shows as, if it has one.
    pub download: Option<DownloadId>,
}

impl NewMediaJob {
    /// A job for `page_url` with `format_expression` into `dest_dir`, with the
    /// default container preference and output template and no subtitles.
    pub fn new(
        page_url: impl Into<String>,
        format_expression: impl Into<String>,
        dest_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            page_url: page_url.into(),
            title: None,
            format_expression: format_expression.into(),
            preset: None,
            container: ContainerPreference::default(),
            subtitles: None,
            output_template: DEFAULT_OUTPUT_TEMPLATE.into(),
            dest_dir: dest_dir.into(),
            download: None,
        }
    }
}

/// A stored media job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct MediaJob {
    /// Its id.
    pub id: MediaJobId,
    /// The download row it shows as, if it has one.
    pub download: Option<DownloadId>,
    /// Where it is.
    pub state: MediaJobState,
    /// The web page.
    pub page_url: String,
    /// The title the probe found.
    pub title: Option<String>,
    /// The yt-dlp format expression.
    pub format_expression: String,
    /// The preset it came from.
    pub preset: Option<String>,
    /// The container toggle.
    pub container: ContainerPreference,
    /// Subtitles, if any.
    pub subtitles: Option<SubtitleChoice>,
    /// The yt-dlp output template.
    pub output_template: String,
    /// The folder the result goes to.
    pub dest_dir: PathBuf,
    /// Why it failed, if it did.
    pub error: Option<MediaJobError>,
    /// Creation time, Unix milliseconds.
    pub created_at: u64,
    /// Last change, Unix milliseconds.
    pub updated_at: u64,
}

/// A change to a media job ([`Engine::update_media_job`](crate::Engine::update_media_job)):
/// every field that is `Some` replaces the stored value (`Some(None)` clears
/// an optional one).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaJobUpdate {
    /// A new state.
    pub state: Option<MediaJobState>,
    /// A new title.
    pub title: Option<Option<String>>,
    /// A new format expression.
    pub format_expression: Option<String>,
    /// A new preset.
    pub preset: Option<Option<String>>,
    /// A new container preference.
    pub container: Option<ContainerPreference>,
    /// New subtitles.
    pub subtitles: Option<Option<SubtitleChoice>>,
    /// A new output template.
    pub output_template: Option<String>,
    /// A new folder.
    pub dest_dir: Option<PathBuf>,
    /// A new download row.
    pub download: Option<Option<DownloadId>>,
    /// A new error.
    pub error: Option<Option<MediaJobError>>,
}

/// Checks what a job must have: an http(s) page URL, a format expression and
/// an output template.
pub(crate) fn validate(
    page_url: &str,
    format_expression: &str,
    output_template: &str,
) -> crate::Result<()> {
    match url::Url::parse(page_url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.has_host() => {}
        Ok(u) => return Err(crate::Error::InvalidUrl(crate::redact_url(&u))),
        Err(e) => return Err(crate::Error::InvalidUrl(e.to_string())),
    }
    if format_expression.trim().is_empty() || output_template.trim().is_empty() {
        return Err(crate::Error::InvalidMediaJob(
            "it needs a format expression and an output template".into(),
        ));
    }
    Ok(())
}
