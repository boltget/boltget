//! Errors of the sidecar manager and the process runner.

use std::path::PathBuf;
use std::time::Duration;

use crate::Tool;

/// Convenience alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors returned by `boltget-media`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A download (release metadata, checksums, signature or the tool itself)
    /// failed in the engine.
    #[error("download failed: {0}")]
    Download(#[from] boltget_engine::Error),

    /// The release metadata could not be used.
    #[error("{tool}: unusable release information: {detail}")]
    Release {
        /// The tool.
        tool: Tool,
        /// What is wrong.
        detail: String,
    },

    /// The signature of a checksum file did not verify against the pinned key.
    #[error("{tool}: the signature of the checksum file does not verify: {detail}")]
    Signature {
        /// The tool.
        tool: Tool,
        /// Why.
        detail: String,
    },

    /// A file does not have the SHA-256 it must have.
    #[error("{tool}: {file} does not match its checksum (expected {expected}, got {actual})")]
    Checksum {
        /// The tool.
        tool: Tool,
        /// The file name.
        file: String,
        /// The expected SHA-256 (hex).
        expected: String,
        /// What it hashed to (hex).
        actual: String,
    },

    /// The archive could not be unpacked safely.
    #[error("{tool}: cannot unpack {file}: {detail}")]
    Archive {
        /// The tool.
        tool: Tool,
        /// The archive.
        file: String,
        /// Why.
        detail: String,
    },

    /// The freshly installed version did not pass its smoke test; it was
    /// removed and the previous version stays active.
    #[error("{tool} {version} did not pass its smoke test: {detail}")]
    SmokeTest {
        /// The tool.
        tool: Tool,
        /// The version that failed.
        version: String,
        /// What went wrong.
        detail: String,
    },

    /// The tool is not installed.
    #[error("{0} is not installed")]
    NotInstalled(Tool),

    /// No build of the tool is known for this platform.
    #[error("{0} has no build for this platform; point Boltget at an existing installation")]
    Unsupported(Tool),

    /// There is no earlier version to go back to.
    #[error("{0} has no earlier version to go back to")]
    NoPrevious(Tool),

    /// Another process is installing or updating sidecars right now.
    #[error("another Boltget process is installing tools in {}", .0.display())]
    Busy(PathBuf),

    /// An argument that the allowlist does not permit.
    #[error("argument refused for {program}: {detail}")]
    Argument {
        /// The program the arguments were for.
        program: &'static str,
        /// Which argument, and why.
        detail: String,
    },

    /// A sidecar did not finish in time and was stopped.
    #[error("{program} did not finish within {} s and was stopped", .timeout.as_secs())]
    Timeout {
        /// The program.
        program: String,
        /// The time it had.
        timeout: Duration,
    },

    /// A local file system or process error.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// yt-dlp could not read the page (unsupported site, no such video,
    /// sign-in needed, network failure, ...). `message` is yt-dlp's own,
    /// without its `ERROR:` prefix; step 6 of v0.4 sorts these into the
    /// user-facing categories of `docs/VIDEO.md`.
    #[error("yt-dlp could not read the page: {message}")]
    Extraction {
        /// yt-dlp's message.
        message: String,
    },

    /// A media download failed: yt-dlp exited with an error, or did not say
    /// what it wrote. `message` is yt-dlp's last `ERROR:` line (without the
    /// prefix); step 6 of v0.4 sorts these into the user-facing categories.
    #[error("the media download failed: {message}")]
    Failed {
        /// yt-dlp's message.
        message: String,
    },

    /// A program the user chose (Settings, Video, custom paths) did not
    /// start or did not report a version.
    #[error("{program} at {} cannot be used: {detail}", path.display())]
    CustomProgram {
        /// The program.
        program: &'static str,
        /// Where the user pointed.
        path: PathBuf,
        /// What went wrong.
        detail: String,
    },

    /// A thumbnail could not be fetched or is not an image Boltget shows.
    #[error("the thumbnail cannot be shown: {0}")]
    Thumbnail(String),

    /// yt-dlp's description of the page could not be used (not JSON, or too
    /// large).
    #[error("yt-dlp's description of the page cannot be read: {detail}")]
    InfoJson {
        /// What is wrong.
        detail: String,
    },

    /// The installed-tools manifest cannot be read or written.
    #[error("tools manifest {}: {detail}", path.display())]
    Manifest {
        /// The manifest file.
        path: PathBuf,
        /// What is wrong.
        detail: String,
    },
}

impl Error {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn release(tool: Tool, detail: impl Into<String>) -> Self {
        Error::Release {
            tool,
            detail: detail.into(),
        }
    }

    /// A stable message-catalog key for front ends.
    pub fn message_key(&self) -> &'static str {
        match self {
            Error::Download(_) => "error.media.download",
            Error::Release { .. } => "error.media.release",
            Error::Signature { .. } | Error::Checksum { .. } => "error.media.verification",
            Error::Archive { .. } => "error.media.archive",
            Error::SmokeTest { .. } => "error.media.smoke_test",
            Error::NotInstalled(_) => "error.media.not_installed",
            Error::Unsupported(_) => "error.media.unsupported",
            Error::NoPrevious(_) => "error.media.no_previous",
            Error::Busy(_) => "error.media.busy",
            Error::Argument { .. } => "error.media.argument",
            Error::Timeout { .. } => "error.media.timeout",
            Error::Extraction { .. } => "error.media.extraction",
            Error::InfoJson { .. } => "error.media.info_json",
            Error::Failed { .. } => "error.media.failed",
            Error::CustomProgram { .. } => "error.media.custom_program",
            Error::Thumbnail(_) => "error.media.thumbnail",
            Error::Io { .. } | Error::Manifest { .. } => "error.media.disk",
        }
    }
}
