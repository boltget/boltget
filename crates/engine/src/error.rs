//! Typed engine errors and their classification.
//!
//! The classification ([`ErrorKind`]) drives retry decisions inside the engine
//! and tells front ends which kind of message to show. See
//! `docs/ARCHITECTURE.md`, "Error handling".

use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// Convenience alias used throughout the engine.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How an error should be handled. Mirrors the table in `docs/ARCHITECTURE.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorKind {
    /// Timeout, reset, temporary DNS failure, 5xx: retry the connection with backoff.
    Transient,
    /// 429/503 (with or without `Retry-After`): retry after the indicated delay.
    Throttled,
    /// 401/403/410 on a URL that previously worked.
    LinkExpired,
    /// The resource changed under us (validator mismatch, size change).
    ResourceChanged,
    /// The server does not honour byte ranges where they were required.
    RangeUnsupported,
    /// Local disk problem: full, access denied, file system limit.
    Disk,
    /// Will not succeed by retrying: 404, bad URL, TLS certificate invalid, ...
    Permanent,
    /// Checksum mismatch (reserved; checksum verification lands later).
    Integrity,
    /// The operation was cancelled by the caller.
    Cancelled,
    /// A bug or invariant violation inside the engine.
    Internal,
}

/// Errors returned by the engine.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The URL could not be parsed or uses an unsupported scheme.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// The server answered with an HTTP status that ends this request.
    #[error("server answered {status} for {url}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// The (redacted) URL that produced the status.
        url: String,
        /// Parsed `Retry-After`, if the server sent one.
        retry_after: Option<Duration>,
        /// Whether the URL had worked earlier in this download (turns 401/403/410 into
        /// [`ErrorKind::LinkExpired`]).
        previously_worked: bool,
    },

    /// A network-level failure: connect, reset, malformed HTTP, incomplete body.
    #[error("network error ({context}): {source}")]
    Network {
        /// What the engine was doing.
        context: &'static str,
        /// Underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A timeout elapsed (connect, TLS, first byte, stall).
    #[error("timed out: {0}")]
    Timeout(&'static str),

    /// TLS setup or handshake failed.
    #[error("TLS error: {0}")]
    Tls(String),

    /// The server sent a response the engine cannot use (bad `Content-Range`, ...).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Too many redirects.
    #[error("too many redirects (limit {0})")]
    TooManyRedirects(usize),

    /// A redirect from `https` to `http` was refused.
    #[error("refused insecure redirect from https to http: {0}")]
    InsecureRedirect(String),

    /// The resource changed during the download.
    #[error("the file changed on the server: {0}")]
    ResourceChanged(String),

    /// Byte ranges were required but the server did not honour them.
    #[error("server does not support byte ranges: {0}")]
    RangeUnsupported(String),

    /// A local file system error.
    #[error("disk error on {path}: {source}")]
    Disk {
        /// The path involved.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// Not enough free space at the destination.
    #[error("not enough disk space at {path}: need {needed} bytes, {available} available")]
    InsufficientSpace {
        /// Destination directory.
        path: PathBuf,
        /// Bytes needed (file size plus safety margin).
        needed: u64,
        /// Bytes available to the current user.
        available: u64,
    },

    /// The destination file system cannot hold a file this large (FAT32).
    #[error("{fs} cannot store files of {size} bytes (limit is 4 GiB - 1)")]
    FileTooLarge {
        /// File system name as reported by the OS.
        fs: String,
        /// File size.
        size: u64,
    },

    /// The download was cancelled.
    #[error("cancelled")]
    Cancelled,

    /// A bug or invariant violation.
    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    /// The classification of this error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::InvalidUrl(_)
            | Error::TooManyRedirects(_)
            | Error::InsecureRedirect(_)
            | Error::Tls(_) => ErrorKind::Permanent,
            Error::Status {
                status,
                previously_worked,
                ..
            } => classify_status(*status, *previously_worked),
            Error::Network { .. } | Error::Timeout(_) | Error::Protocol(_) => ErrorKind::Transient,
            Error::ResourceChanged(_) => ErrorKind::ResourceChanged,
            Error::RangeUnsupported(_) => ErrorKind::RangeUnsupported,
            Error::Disk { .. } | Error::InsufficientSpace { .. } | Error::FileTooLarge { .. } => {
                ErrorKind::Disk
            }
            Error::Cancelled => ErrorKind::Cancelled,
            Error::Internal(_) => ErrorKind::Internal,
        }
    }

    /// Whether retrying the same request later can succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(self.kind(), ErrorKind::Transient | ErrorKind::Throttled)
    }

    /// A stable message-catalog key for front ends (see `docs/ARCHITECTURE.md`).
    pub fn message_key(&self) -> &'static str {
        match self {
            Error::InvalidUrl(_) => "error.invalid_url",
            Error::Status { status, .. } => match status {
                401 => "error.http.unauthorized",
                403 => "error.http.forbidden",
                404 => "error.http.not_found",
                407 => "error.http.proxy_auth",
                410 => "error.http.gone",
                429 => "error.http.too_many_requests",
                500..=599 => "error.http.server_error",
                _ => "error.http.status",
            },
            Error::Network { .. } => "error.network",
            Error::Timeout(_) => "error.timeout",
            Error::Tls(_) => "error.tls",
            Error::Protocol(_) => "error.protocol",
            Error::TooManyRedirects(_) => "error.too_many_redirects",
            Error::InsecureRedirect(_) => "error.insecure_redirect",
            Error::ResourceChanged(_) => "error.resource_changed",
            Error::RangeUnsupported(_) => "error.range_unsupported",
            Error::Disk { .. } => "error.disk",
            Error::InsufficientSpace { .. } => "error.disk_full",
            Error::FileTooLarge { .. } => "error.file_too_large_for_fs",
            Error::Cancelled => "error.cancelled",
            Error::Internal(_) => "error.internal",
        }
    }

    /// The server-requested retry delay, if any.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Error::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    pub(crate) fn network(
        context: &'static str,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Error::Network {
            context,
            source: source.into(),
        }
    }

    pub(crate) fn disk(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Error::Disk {
            path: path.into(),
            source,
        }
    }
}

/// Classifies an HTTP status code. `previously_worked` distinguishes an expired link
/// from a link that never worked.
pub(crate) fn classify_status(status: u16, previously_worked: bool) -> ErrorKind {
    match status {
        401 | 403 | 410 if previously_worked => ErrorKind::LinkExpired,
        429 | 503 => ErrorKind::Throttled,
        408 | 425 | 500..=599 => ErrorKind::Transient,
        416 if previously_worked => ErrorKind::ResourceChanged,
        _ => ErrorKind::Permanent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classification() {
        assert_eq!(classify_status(404, false), ErrorKind::Permanent);
        assert_eq!(classify_status(403, false), ErrorKind::Permanent);
        assert_eq!(classify_status(403, true), ErrorKind::LinkExpired);
        assert_eq!(classify_status(410, true), ErrorKind::LinkExpired);
        assert_eq!(classify_status(429, false), ErrorKind::Throttled);
        assert_eq!(classify_status(503, true), ErrorKind::Throttled);
        assert_eq!(classify_status(500, false), ErrorKind::Transient);
        assert_eq!(classify_status(502, false), ErrorKind::Transient);
        assert_eq!(classify_status(408, false), ErrorKind::Transient);
        assert_eq!(classify_status(416, true), ErrorKind::ResourceChanged);
        assert_eq!(classify_status(400, true), ErrorKind::Permanent);
    }

    #[test]
    fn retryable() {
        assert!(Error::Timeout("stall").is_retryable());
        assert!(!Error::Cancelled.is_retryable());
        assert!(
            Error::Status {
                status: 429,
                url: String::new(),
                retry_after: None,
                previously_worked: false
            }
            .is_retryable()
        );
    }
}
