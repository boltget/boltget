//! Sorting yt-dlp's failures into the few categories the user sees
//! (docs/VIDEO.md, "Errors and pause/resume: as built").
//!
//! yt-dlp says why it failed in its last `ERROR:` line, in English, with the
//! extractor's name first (`ERROR: [youtube] abc: Video unavailable`). There
//! is no machine-readable code, so the line is matched against phrases that
//! yt-dlp's own messages use; the order of the checks decides between
//! messages that would fit two categories. A message that fits none is
//! [`ErrorCategory::Other`], shown with yt-dlp's words under a disclosure.
//!
//! The decisions where the categories overlap:
//!
//! - **Age-restricted** pages are [`ErrorCategory::SignIn`]: yt-dlp needs a
//!   signed-in account's cookies for them, which the browser extension
//!   passes, so the sign-in advice is the one that helps.
//! - **Private** videos are [`ErrorCategory::Unavailable`], although
//!   `YouTube`'s message adds "Sign in if you've been granted access": most
//!   private videos are not shared with the user.
//! - **"Unsupported URL"** is [`ErrorCategory::NoMedia`], not "the site
//!   changed": it is what yt-dlp's generic extractor says when a page has no
//!   video or audio it can find.
//! - **A message with yt-dlp's bug-report request** ("please report this
//!   issue on …", "Confirm you are on the latest version") is
//!   [`ErrorCategory::SiteChanged`] whatever else it says: yt-dlp adds it
//!   only to failures it did not expect, which are almost always an
//!   extractor that no longer matches the site.
//! - **HTTP 403** while fetching the video's data or its fragments is
//!   [`ErrorCategory::SiteChanged`] (the addresses the extractor produced no
//!   longer work, which a newer yt-dlp usually fixes); on the page itself it
//!   is [`ErrorCategory::Unavailable`].
//! - **A proxy** that fails or wants credentials is
//!   [`ErrorCategory::Network`], checked before sign-in ("407 Proxy
//!   Authentication Required" is not the site's sign-in).
//! - **"Requested format is not available"** is [`ErrorCategory::Other`]: the
//!   format expression (a typed one, or a playlist entry without the
//!   preset's streams) found nothing, and yt-dlp's message says so best.
//! - **Post-processing** (FFmpeg merging, converting, embedding) is
//!   [`ErrorCategory::Processing`], checked first: those messages name files
//!   and codecs that would otherwise look like other categories.

use std::fmt;

use serde::Serialize;

/// The most of yt-dlp's message kept (characters).
const MAX_MESSAGE: usize = 2000;

/// What kind of failure a yt-dlp error is, as the user sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// The site wants a signed-in account (or an age check): the browser
    /// extension passes the browser's cookies.
    SignIn,
    /// Removed, private, blocked in this region, not live yet.
    Unavailable,
    /// yt-dlp's extractor no longer matches the site: a newer yt-dlp is
    /// likely to fix it.
    SiteChanged,
    /// DRM-protected: Boltget does not download it.
    Drm,
    /// The connection failed (time-outs, resets, proxy errors, HTTP 5xx and
    /// 429): trying again later may work.
    Network,
    /// There is no video or audio on the page that yt-dlp can find.
    NoMedia,
    /// FFmpeg failed while merging, converting or embedding.
    Processing,
    /// Anything else; yt-dlp's message says what.
    Other,
}

impl ErrorCategory {
    /// Every category.
    pub const ALL: [ErrorCategory; 8] = [
        ErrorCategory::SignIn,
        ErrorCategory::Unavailable,
        ErrorCategory::SiteChanged,
        ErrorCategory::Drm,
        ErrorCategory::Network,
        ErrorCategory::NoMedia,
        ErrorCategory::Processing,
        ErrorCategory::Other,
    ];

    /// The message-catalog key front ends show (`error.media.sign_in`, ...).
    pub fn message_key(self) -> &'static str {
        match self {
            ErrorCategory::SignIn => "error.media.sign_in",
            ErrorCategory::Unavailable => "error.media.unavailable",
            ErrorCategory::SiteChanged => "error.media.site_changed",
            ErrorCategory::Drm => "error.media.drm",
            ErrorCategory::Network => "error.media.network",
            ErrorCategory::NoMedia => "error.media.no_media",
            ErrorCategory::Processing => "error.media.processing",
            ErrorCategory::Other => "error.media.other",
        }
    }

    /// The category a catalog key names (the inverse of
    /// [`ErrorCategory::message_key`], also for the keys that add the update's
    /// state to "the site changed", such as `error.media.site_changed.current`).
    pub fn from_key(key: &str) -> Option<Self> {
        ErrorCategory::ALL.into_iter().find(|c| {
            let k = c.message_key();
            key == k
                || key
                    .strip_prefix(k)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
    }

    /// Whether the same request may work when tried again later without
    /// changing anything (only network failures).
    pub fn retryable(self) -> bool {
        self == ErrorCategory::Network
    }
}

/// An English description, for the command line.
impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ErrorCategory::SignIn => {
                "this site needs you to be signed in (send the page from the browser extension, which passes its cookies)"
            }
            ErrorCategory::Unavailable => {
                "this video is not available: it may be private, removed or blocked in your region"
            }
            ErrorCategory::SiteChanged => {
                "the site changed and the downloader needs an update"
            }
            ErrorCategory::Drm => "DRM-protected: Boltget cannot download this",
            ErrorCategory::Network => "the connection failed; try again",
            ErrorCategory::NoMedia => "no video or audio found on this page",
            ErrorCategory::Processing => "processing failed (FFmpeg)",
            ErrorCategory::Other => "yt-dlp failed",
        })
    }
}

/// A failed yt-dlp run: its category and its own message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The category.
    pub category: ErrorCategory,
    /// yt-dlp's message: its last `ERROR:` line without the prefix, else
    /// its last line of output, cut at 2,000 characters.
    pub message: String,
}

impl Failure {
    /// The failure a yt-dlp run's standard error describes; `status` names
    /// the exit when it printed nothing.
    pub fn from_stderr(stderr: &str, status: impl fmt::Display) -> Self {
        let message = error_message(stderr, status);
        Failure {
            category: classify(&message),
            message,
        }
    }
}

/// yt-dlp's message: its last `ERROR:` line without the prefix, else its
/// last line of output, else the exit status; cut at [`MAX_MESSAGE`].
pub(crate) fn error_message(stderr: &str, status: impl fmt::Display) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let message = lines
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("ERROR:"))
        .or_else(|| lines.last().copied())
        .map_or_else(
            || format!("yt-dlp exited with {status}"),
            |m| m.trim().to_owned(),
        );
    match message.char_indices().nth(MAX_MESSAGE) {
        Some((cut, _)) => format!("{}…", &message[..cut]),
        None => message,
    }
}

/// The category of one of yt-dlp's messages (see the module documentation
/// for the order of the checks).
pub fn classify(message: &str) -> ErrorCategory {
    let m = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| m.contains(n));

    // FFmpeg, run by yt-dlp's post-processors (or as its downloader).
    if m.starts_with("postprocessing:")
        || has(&[
            "conversion failed",
            "ffmpeg not found",
            "ffprobe not found",
            "ffprobe and ffmpeg not found",
            "ffmpeg is not installed",
            "ffmpeg exited with code",
            "unable to embed",
            "error opening output file",
            "could not write header",
        ])
    {
        return ErrorCategory::Processing;
    }
    if word(&m, "drm") || has(&["drm-protected", "drm protected"]) {
        return ErrorCategory::Drm;
    }
    // yt-dlp asks for a bug report only when it did not expect the failure.
    if has(&[
        "please report this issue",
        "confirm you are on the latest version",
        "update to the latest version",
    ]) {
        return ErrorCategory::SiteChanged;
    }
    if has(&[
        "requested format is not available",
        "requested format not available",
    ]) {
        return ErrorCategory::Other;
    }
    if has(&[
        "private video",
        "video is private",
        "been made private",
        "this playlist is private",
        "is a private",
    ]) {
        return ErrorCategory::Unavailable;
    }
    // A proxy that wants credentials is the connection's problem, not the
    // site's sign-in.
    if has(&[
        "unable to connect to proxy",
        "cannot connect to proxy",
        "proxyerror",
        "proxy error",
        "tunnel connection failed",
    ]) {
        return ErrorCategory::Network;
    }
    if has(&[
        "sign in",
        "signin",
        "sign-in",
        "log in",
        "login",
        "logged in",
        "logged-in",
        "--cookies",
        "cookies-from-browser",
        "authentication",
        "authenticate",
        "members-only",
        "members only",
        "join this channel",
        "registered users",
        "subscribers only",
        "only available to subscribers",
        "requires a subscription",
        "premium members",
        "age-restricted",
        "age restricted",
        "confirm your age",
        "inappropriate for some users",
        "http error 401",
        "unauthorized",
    ]) {
        return ErrorCategory::SignIn;
    }
    if m.contains("http error 403") {
        return if has(&["video data", "fragment"]) {
            ErrorCategory::SiteChanged
        } else {
            ErrorCategory::Unavailable
        };
    }
    if has(&[
        "timed out",
        "timeout",
        "connection reset",
        "connection refused",
        "connection aborted",
        "forcibly closed",
        "remote end closed connection",
        "network is unreachable",
        "no route to host",
        "getaddrinfo failed",
        "name or service not known",
        "temporary failure in name resolution",
        "nodename nor servname",
        "failed to resolve",
        "urlopen error",
        "eof occurred in violation of protocol",
        "incompleteread",
        "incomplete read",
        "giving up after",
        "unable to continue",
        "too many requests",
        "http error 429",
        "[errno 11001]",
        "[winerror 10054]",
        "[winerror 10060]",
        "[winerror 10061]",
        "[ssl:",
        "ssl: ",
        "transporterror",
        "connectionerror",
    ]) || http_5xx(&m)
    {
        return ErrorCategory::Network;
    }
    if has(&[
        "video unavailable",
        "unavailable",
        "not available",
        "no longer available",
        "been removed",
        "removed by the uploader",
        "been terminated",
        "copyright",
        "geo restrict",
        "geo-restrict",
        "georestrict",
        "in your country",
        "your region",
        "from your location",
        "blocked it",
        "http error 404",
        "http error 410",
        "does not exist",
        "live event will begin",
        "premieres in",
        "has not started",
        "live event has ended",
    ]) {
        return ErrorCategory::Unavailable;
    }
    if has(&[
        "unable to extract",
        "failed to extract",
        "failed to parse",
        "unable to parse",
        "signature extraction failed",
        "nsig extraction failed",
        "could not find js function",
        "unable to decode",
        "jsinterp",
        "player response",
        "traceback",
        "keyerror",
        "typeerror",
        "attributeerror",
        "indexerror",
    ]) {
        return ErrorCategory::SiteChanged;
    }
    if has(&[
        "unsupported url",
        "no media found",
        "no video found",
        "no videos found",
        "no video could be found",
        "no video formats found",
        "no formats found",
        "does not contain any video",
        "does not contain a video",
        "contains no video",
        "there's no video",
        "there is no video",
        "is not a video",
    ]) {
        return ErrorCategory::NoMedia;
    }
    ErrorCategory::Other
}

/// Whether `w` appears in `text` as a word of its own (not inside another).
fn word(text: &str, w: &str) -> bool {
    text.match_indices(w).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + w.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// `HTTP Error 5xx` (a server error).
fn http_5xx(m: &str) -> bool {
    m.match_indices("http error 5").any(|(i, _)| {
        let digits = &m[i + "http error 5".len()..];
        digits.len() >= 2 && digits.as_bytes()[..2].iter().all(u8::is_ascii_digit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_from_standard_error() {
        assert_eq!(
            error_message(
                "WARNING: [youtube] something\nERROR: [youtube] abc: Video unavailable\n",
                "exit code: 1"
            ),
            "[youtube] abc: Video unavailable"
        );
        assert_eq!(error_message("Traceback\n  oops\n\n", 1), "oops");
        assert!(error_message("", "exit code: 1").starts_with("yt-dlp exited with"));
        let long = format!("ERROR: {}", "x".repeat(5000));
        assert_eq!(error_message(&long, 1).chars().count(), 2001);
    }

    #[test]
    fn keys_and_back() {
        for c in ErrorCategory::ALL {
            assert_eq!(ErrorCategory::from_key(c.message_key()), Some(c));
        }
        assert_eq!(
            ErrorCategory::from_key("error.media.site_changed.updated"),
            Some(ErrorCategory::SiteChanged)
        );
        assert_eq!(ErrorCategory::from_key("error.media.site_changedx"), None);
        assert_eq!(ErrorCategory::from_key("error.media.failed"), None);
        assert!(ErrorCategory::Network.retryable());
        assert!(!ErrorCategory::SiteChanged.retryable());
    }

    #[test]
    fn words_and_status_codes() {
        assert!(word("this is drm protected", "drm"));
        assert!(word("drm", "drm"));
        assert!(!word("eardrums", "drm"));
        assert!(http_5xx("http error 503: service unavailable"));
        assert!(!http_5xx("http error 5"));
        assert!(!http_5xx("http error 404"));
    }
}
