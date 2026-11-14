//! The browser extension's protocol (docs/BROWSER-INTEGRATION.md, "Native
//! messaging"; docs/ARCHITECTURE.md, "Extension to app").
//!
//! The extension talks to `boltget-native-host`, which the browser starts,
//! over stdio; the host relays each message to the app over the pipe
//! ([`crate::Request::Browser`]) and the app's answer back. These are the
//! messages of that conversation, the limits every message must keep, and
//! [`decode`], the host's reader of what the browser sends.
//!
//! - **Envelope.** Every message is a JSON object with `v` (the
//!   [`VERSION`] the extension speaks), `type`, and an optional `id` (an
//!   integer the reply carries back, so a port can match replies).
//! - **Strict.** An unknown `type` or field, a value out of its limits, a
//!   URL that is not `http`/`https`, a header off the allowlist: the whole
//!   message is refused ([`ErrorReason::Invalid`]). A request gaining a
//!   field changes [`VERSION`]; a reply gaining one does not (the extension
//!   ignores what it does not know).
//! - **Never lose a download.** Only [`Reply::Accepted`] means the app took
//!   it. Everything else, and no reply at all, means the browser keeps it.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use boltget_engine::{Cookie, HeaderValue, RequestContext};
use serde::{Deserialize, Serialize};

/// The protocol version this build speaks. It changes when a message
/// changes meaning or an extension message gains a field.
pub const VERSION: u32 = 1;

/// The oldest extension version the host and app still serve.
pub const OLDEST_VERSION: u32 = 1;

/// The native messaging host's name in release builds, as registered with
/// the browsers.
pub const HOST_NAME: &str = "app.boltget.native_host";

/// The host's name for development builds and the development extension,
/// so a development extension never talks to a release install.
pub const DEV_HOST_NAME: &str = "app.boltget.native_host_dev";

/// The largest message the host accepts from the browser (1 MiB).
pub const MAX_MESSAGE: usize = 1 << 20;

/// The largest frame the host reads at all. Chromium sends at most 64 MiB
/// to a host; a frame between [`MAX_MESSAGE`] and this is skipped and
/// answered with [`ErrorReason::TooLarge`], a larger one ends the session.
pub const MAX_FRAME: u32 = 64 << 20;

/// The largest message the host may send to the browser: browsers refuse
/// anything over 1 MB from a host.
pub const MAX_REPLY: usize = 1 << 20;

/// The longest URL (32 KiB; docs/ARCHITECTURE.md, Security).
pub const MAX_URL: usize = 32 * 1024;
/// The most cookie records in one message.
pub const MAX_COOKIES: usize = 300;
/// The most bytes of cookie names and values in one message (64 KiB).
pub const MAX_COOKIE_BYTES: usize = 64 * 1024;
/// The longest header value.
pub const MAX_HEADER_VALUE: usize = 8 * 1024;
/// The most links in [`Message::AddLinks`].
pub const MAX_LINKS: usize = 5000;
/// The longest file name, link text or page title, in bytes.
pub const MAX_TEXT: usize = 1024;
/// The longest short value: a MIME type, a cookie's domain, a version.
pub const MAX_SHORT: usize = 255;

/// The request headers the extension may pass on, lowercase
/// (docs/BROWSER-INTEGRATION.md, "Cookies, headers and referrer
/// forwarding"; the engine's own allowlist is the same).
pub const ALLOWED_HEADERS: [&str; 6] = [
    "user-agent",
    "referer",
    "accept",
    "accept-language",
    "origin",
    "authorization",
];

#[cfg(feature = "typescript")]
use specta::Type;

// Messages from the extension ------------------------------------------------

/// A message from the extension, after its envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Message {
    /// Who is there: answered with [`Reply::Hello`]. Never starts the app.
    Hello(Hello),
    /// A download the extension intercepted: [`Reply::Accepted`] when the
    /// app took it, else the browser keeps it. Starts the app if needed.
    Capture(Capture),
    /// "Download all links" or a selection's links: the app shows them to
    /// pick from. Starts the app if needed.
    AddLinks(AddLinks),
    /// "Send page to Boltget": the page for the app's media extraction
    /// (v0.4; until then [`RejectReason::Unsupported`]). Starts the app if
    /// needed.
    MediaPage(MediaPage),
    /// What the app is doing, for the popup: [`Reply::Status`]. Never
    /// starts the app.
    Status(Status),
}

impl Message {
    /// The `type` as it travels, for logs.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::Capture(_) => "capture",
            Self::AddLinks(_) => "add_links",
            Self::MediaPage(_) => "media_page",
            Self::Status(_) => "status",
        }
    }

    /// Whether the host starts the app to answer it.
    pub fn starts_app(&self) -> bool {
        matches!(
            self,
            Self::Capture(_) | Self::AddLinks(_) | Self::MediaPage(_)
        )
    }

    /// Checks every limit and rule of the protocol (the module docs). The
    /// host checks before relaying, the app again on receipt.
    pub fn validate(&self) -> Result<(), Invalid> {
        match self {
            Self::Hello(h) => {
                short("extension", &h.extension)?;
                Ok(())
            }
            Self::Capture(c) => c.validate(),
            Self::AddLinks(a) => a.validate(),
            Self::MediaPage(m) => m.validate(),
            Self::Status(Status {}) => Ok(()),
        }
    }
}

/// [`Message::Hello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Hello {
    /// The extension's version (`0.3.0`), for logs.
    pub extension: String,
    /// The browser it runs in.
    pub browser: Browser,
}

/// A browser, as the extension reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Browser {
    /// Google Chrome (and Chromium browsers that are not listed).
    Chrome,
    /// Microsoft Edge.
    Edge,
    /// Mozilla Firefox.
    Firefox,
    /// Anything else.
    Other,
}

/// [`Message::Status`]: no fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(deny_unknown_fields)]
pub struct Status {}

/// [`Message::Capture`]: everything the app needs to make the browser's
/// request again (docs/ENGINE.md, "Request context").
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Capture {
    /// The link the download started from (`DownloadItem.url`).
    pub url: String,
    /// Where redirects ended (`DownloadItem.finalUrl`), when it differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    /// The browser's file name (`onDeterminingFilename`), a name without
    /// folders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// The size, when the browser knows it (`totalBytes` > 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", specta(type = Option<u32>))]
    pub size: Option<u64>,
    /// The response's MIME type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// `DownloadItem.referrer`, else the tab's URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    /// The tab's URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    /// From a private (incognito) window.
    #[serde(default)]
    pub incognito: bool,
    /// The user asked for Boltget (the link menu, or Alt+Shift), whatever
    /// the capture rules say.
    #[serde(default)]
    pub forced: bool,
    /// Cookies for `url` and `final_url` (`cookies.getAll`).
    #[serde(default)]
    pub cookies: Vec<CookieRecord>,
    /// Request headers the browser sent, from [`ALLOWED_HEADERS`] only,
    /// `[name, value]`.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

impl Capture {
    /// A capture of `url` with nothing else known.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            final_url: None,
            filename: None,
            size: None,
            mime: None,
            referrer: None,
            page_url: None,
            incognito: false,
            forced: false,
            cookies: Vec::new(),
            headers: Vec::new(),
        }
    }

    fn validate(&self) -> Result<(), Invalid> {
        link("url", &self.url)?;
        optional_link("final_url", self.final_url.as_deref())?;
        optional_link("referrer", self.referrer.as_deref())?;
        optional_link("page_url", self.page_url.as_deref())?;
        if let Some(name) = &self.filename {
            file_name(name)?;
        }
        if let Some(mime) = &self.mime {
            mime_type(mime)?;
        }
        cookies(&self.cookies)?;
        headers(&self.headers)?;
        Ok(())
    }

    /// The request context for the engine: the link (the final one when
    /// the browser followed redirects, since the original may have been a
    /// one-time redirector), its headers, cookies and the browser's name.
    pub fn context(&self) -> Result<RequestContext, Invalid> {
        let url = self.final_url.as_deref().unwrap_or(&self.url);
        let mut ctx = context(url, &self.headers, self.referrer.as_deref(), &self.cookies)?;
        if let Some(name) = &self.filename {
            ctx = ctx.with_suggested_name(name.clone());
        }
        Ok(ctx)
    }
}

/// Header values and cookies are credentials, links may be signed: none of
/// them in logs.
impl fmt::Debug for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Capture")
            .field("url", &crate::proto::redact(&self.url))
            .field("filename", &self.filename)
            .field("size", &self.size)
            .field("mime", &self.mime)
            .field("incognito", &self.incognito)
            .field("forced", &self.forced)
            .field("cookies", &self.cookies.len())
            .field("headers", &header_names(&self.headers))
            .finish_non_exhaustive()
    }
}

/// [`Message::AddLinks`].
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AddLinks {
    /// The links, in page order, `http`/`https` only.
    pub links: Vec<Link>,
    /// The page they are on, also the referrer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    /// From a private (incognito) window.
    #[serde(default)]
    pub incognito: bool,
    /// Cookies for the links' sites.
    #[serde(default)]
    pub cookies: Vec<CookieRecord>,
    /// Request headers, from [`ALLOWED_HEADERS`] only.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

impl AddLinks {
    fn validate(&self) -> Result<(), Invalid> {
        if self.links.is_empty() {
            return Err(Invalid::new("links", "no links"));
        }
        if self.links.len() > MAX_LINKS {
            return Err(Invalid::new("links", format!("more than {MAX_LINKS}")));
        }
        for l in &self.links {
            link("links.url", &l.url)?;
            if let Some(text) = &l.text {
                text_value("links.text", text)?;
            }
        }
        optional_link("page_url", self.page_url.as_deref())?;
        cookies(&self.cookies)?;
        headers(&self.headers)?;
        Ok(())
    }

    /// The request context for one of the links.
    pub fn context(&self, url: &str) -> Result<RequestContext, Invalid> {
        context(url, &self.headers, self.page_url.as_deref(), &self.cookies)
    }
}

impl fmt::Debug for AddLinks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddLinks")
            .field("links", &self.links.len())
            .field("incognito", &self.incognito)
            .field("cookies", &self.cookies.len())
            .field("headers", &header_names(&self.headers))
            .finish_non_exhaustive()
    }
}

/// One link of [`AddLinks`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Link {
    /// The link.
    pub url: String,
    /// Its text on the page, for the picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Link {
    /// A link without text.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            text: None,
        }
    }
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("url", &crate::proto::redact(&self.url))
            .finish_non_exhaustive()
    }
}

/// [`Message::MediaPage`].
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MediaPage {
    /// The page, which the app gives to its media extractor.
    pub page_url: String,
    /// A media URL the extension saw on the page: used only when the
    /// extractor knows nothing of the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_url: Option<String>,
    /// The page's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// From a private (incognito) window.
    #[serde(default)]
    pub incognito: bool,
    /// Cookies for the page's site.
    #[serde(default)]
    pub cookies: Vec<CookieRecord>,
    /// Request headers, from [`ALLOWED_HEADERS`] only.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

impl MediaPage {
    fn validate(&self) -> Result<(), Invalid> {
        link("page_url", &self.page_url)?;
        optional_link("media_url", self.media_url.as_deref())?;
        if let Some(t) = &self.title {
            text_value("title", t)?;
        }
        cookies(&self.cookies)?;
        headers(&self.headers)?;
        Ok(())
    }
}

impl fmt::Debug for MediaPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaPage")
            .field("page_url", &crate::proto::redact(&self.page_url))
            .field("incognito", &self.incognito)
            .field("cookies", &self.cookies.len())
            .finish_non_exhaustive()
    }
}

/// A cookie as `cookies.getAll` reports it. `same_site` and `http_only`
/// mean nothing to a downloader and are only checked.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CookieRecord {
    /// Its name (may be empty, as browsers allow).
    pub name: String,
    /// Its value.
    pub value: String,
    /// The domain, with or without a leading dot.
    pub domain: String,
    /// Its path (default `/`).
    #[serde(default = "root")]
    pub path: String,
    /// Sent to `domain` only, not its subdomains.
    #[serde(default)]
    pub host_only: bool,
    /// Sent over `https` only.
    #[serde(default)]
    pub secure: bool,
    /// Hidden from page scripts.
    #[serde(default)]
    pub http_only: bool,
    /// The browser's `sameSite`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<SameSite>,
    /// Expiry in seconds since 1970 (`expirationDate`); absent for a
    /// session cookie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
}

fn root() -> String {
    "/".into()
}

impl CookieRecord {
    /// A session cookie for `domain` and its subdomains, path `/`.
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: root(),
            host_only: false,
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        }
    }

    /// The engine's cookie.
    pub fn to_cookie(&self) -> Cookie {
        let expires = self
            .expires
            .filter(|s| s.is_finite() && *s > 0.0)
            .and_then(|s| UNIX_EPOCH.checked_add(Duration::from_secs_f64(s.min(MAX_EXPIRY))));
        Cookie::from_browser(
            &self.name,
            &self.value,
            &self.domain,
            self.host_only,
            &self.path,
            self.secure,
            expires,
        )
    }
}

/// The value is a secret.
impl fmt::Debug for CookieRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CookieRecord")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// A cookie's `sameSite`, as browsers name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SameSite {
    /// `SameSite=None`.
    NoRestriction,
    /// `SameSite=Lax`.
    Lax,
    /// `SameSite=Strict`.
    Strict,
    /// Not set.
    Unspecified,
}

/// The latest expiry accepted, in seconds since 1970: the end of year 9999.
const MAX_EXPIRY: f64 = 253_402_300_799.0;

// Replies to the extension ------------------------------------------------

/// What the host sends back for one message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Reply {
    /// The answer to [`Message::Hello`].
    Hello(HelloReply),
    /// The app took the download (or the links): the browser drops its
    /// copy. The only reply that means that.
    Accepted(Accepted),
    /// The app saw the message and declined it: the browser keeps the
    /// download.
    Rejected(Rejected),
    /// The answer to [`Message::Status`].
    Status(StatusReply),
    /// No app could be asked: the browser keeps the download.
    Unavailable(Unavailable),
    /// The message itself was refused: the browser keeps the download.
    Error(ErrorReply),
}

impl Reply {
    /// [`Reply::Rejected`] with a diagnostic.
    pub fn rejected(reason: RejectReason, message: impl Into<String>) -> Self {
        Self::Rejected(Rejected {
            reason,
            message: message.into(),
        })
    }

    /// [`Reply::Unavailable`] with a diagnostic.
    pub fn unavailable(reason: UnavailableReason, message: impl Into<String>) -> Self {
        Self::Unavailable(Unavailable::new(reason, message))
    }

    /// [`Reply::Error`] with a diagnostic.
    pub fn error(reason: ErrorReason, message: impl Into<String>) -> Self {
        Self::Error(ErrorReply {
            reason,
            message: message.into(),
        })
    }

    /// The `type` as it travels, for logs.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::Accepted(_) => "accepted",
            Self::Rejected(_) => "rejected",
            Self::Status(_) => "status",
            Self::Unavailable(_) => "unavailable",
            Self::Error(_) => "error",
        }
    }
}

/// [`Reply::Hello`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct HelloReply {
    /// The app (`Boltget 0.3.0`); `null` when it is not running (it starts
    /// for the first download).
    pub app: Option<String>,
    /// The host's version (`0.3.0`).
    pub host: String,
    /// The oldest protocol version served.
    pub oldest: u32,
    /// The newest protocol version served.
    pub current: u32,
    /// The app's capture rules; `null` when the app is not running (keep
    /// the last ones it sent).
    pub rules: Option<CaptureRules>,
}

impl HelloReply {
    /// A hello from `app` with its `rules`; the host fills in its own
    /// version.
    pub fn new(app: Option<String>, rules: Option<CaptureRules>) -> Self {
        Self {
            app,
            host: String::new(),
            oldest: OLDEST_VERSION,
            current: VERSION,
            rules,
        }
    }
}

/// What the extension captures (docs/BROWSER-INTEGRATION.md, "Capture
/// rules"). The app owns them; the extension applies them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct CaptureRules {
    /// Capture at all.
    pub enabled: bool,
    /// The smallest download captured, in bytes, when its size is known.
    #[cfg_attr(feature = "typescript", specta(type = u32))]
    pub min_size: u64,
    /// File extensions captured, lowercase, without dots.
    pub extensions: Vec<String>,
    /// Hosts never captured (matched as suffixes).
    pub never_sites: Vec<String>,
    /// Hosts always captured (matched as suffixes).
    pub always_sites: Vec<String>,
    /// Capture in private windows too.
    pub private_windows: bool,
}

/// The extensions captured by default (docs/BROWSER-INTEGRATION.md).
pub const DEFAULT_EXTENSIONS: &str = "zip rar 7z tar gz bz2 xz zst iso img \
    exe msi msix appx dmg pkg deb rpm apk \
    mp4 mkv webm mov avi m4v mp3 flac wav m4a ogg opus bin dat";

impl Default for CaptureRules {
    fn default() -> Self {
        Self {
            enabled: true,
            min_size: 1 << 20,
            extensions: DEFAULT_EXTENSIONS
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            never_sites: Vec::new(),
            always_sites: Vec::new(),
            private_windows: false,
        }
    }
}

/// [`Reply::Accepted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct Accepted {
    /// The new download's id, when the app added it at once.
    #[cfg_attr(feature = "typescript", specta(type = Option<u32>))]
    pub download: Option<u64>,
    /// The app asks the user first (its add sheet is open with the link).
    pub confirm: bool,
}

impl Accepted {
    /// Added at once as download `id`.
    pub fn added(id: u64) -> Self {
        Self {
            download: Some(id),
            confirm: false,
        }
    }

    /// Shown to the user to confirm.
    pub fn confirming() -> Self {
        Self {
            download: None,
            confirm: true,
        }
    }
}

/// [`Reply::Rejected`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct Rejected {
    /// Why.
    pub reason: RejectReason,
    /// English diagnostic, for logs.
    pub message: String,
}

/// Why the app declined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RejectReason {
    /// The server refused the app's own request for the link (a one-time
    /// link the browser already used, say): the browser's paused download
    /// still holds the original response.
    LinkRefused,
    /// This app does not handle the message yet (media before v0.4).
    Unsupported,
    /// The app could not add it (no download folder, say).
    Failed,
}

/// [`Reply::Status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct StatusReply {
    /// Downloads running now.
    pub active: u32,
    /// Downloads waiting in a queue.
    pub queued: u32,
    /// Bytes per second, all downloads together.
    #[cfg_attr(feature = "typescript", specta(type = u32))]
    pub speed: u64,
}

impl StatusReply {
    /// A status.
    pub fn new(active: u32, queued: u32, speed: u64) -> Self {
        Self {
            active,
            queued,
            speed,
        }
    }
}

/// [`Reply::Unavailable`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct Unavailable {
    /// Why.
    pub reason: UnavailableReason,
    /// English diagnostic, for logs.
    pub message: String,
}

impl Unavailable {
    /// No app, for `reason`, with a diagnostic.
    pub fn new(reason: UnavailableReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

/// Why no app could be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnavailableReason {
    /// The app is not running, and the message does not start it
    /// ([`Message::Status`]).
    NotRunning,
    /// The host found no app to start next to it.
    NotInstalled,
    /// The app was started but did not answer in time, or stopped
    /// answering.
    Timeout,
    /// Another Boltget (a standalone `boltget` command) owns the downloads.
    Busy,
    /// The app speaks another pipe protocol than the host: the two are from
    /// different releases (reinstall Boltget).
    Mismatch,
}

/// [`Reply::Error`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct ErrorReply {
    /// Why.
    pub reason: ErrorReason,
    /// English diagnostic, for logs.
    pub message: String,
}

/// Why a message was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorReason {
    /// Not a message of this protocol, or a value out of its limits.
    Invalid,
    /// Over [`MAX_MESSAGE`].
    TooLarge,
    /// The extension speaks a version older than [`OLDEST_VERSION`]:
    /// update the extension.
    ExtensionTooOld,
    /// The extension speaks a version newer than [`VERSION`]: update
    /// Boltget.
    AppTooOld,
}

// Envelopes ------------------------------------------------------------------

/// A message with its envelope, as the extension sends it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct Envelope {
    /// The protocol version the extension speaks.
    pub v: u32,
    /// Carried back in the reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// The message.
    #[serde(flatten)]
    pub message: Message,
}

impl Envelope {
    /// `message` at the current version, without an id.
    pub fn new(message: Message) -> Self {
        Self {
            v: VERSION,
            id: None,
            message,
        }
    }
}

/// A reply with its envelope, as the host sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(Type))]
#[non_exhaustive]
pub struct ReplyEnvelope {
    /// The protocol version of the reply: the extension's, when it is
    /// served.
    pub v: u32,
    /// The message's `id`, when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// The reply.
    #[serde(flatten)]
    pub reply: Reply,
}

impl ReplyEnvelope {
    /// `reply` to a message at version `v` with `id`.
    pub fn new(v: u32, id: Option<u32>, reply: Reply) -> Self {
        Self { v, id, reply }
    }

    /// The reply as it goes to the browser: JSON, at most [`MAX_REPLY`]
    /// bytes (a reply that would be larger becomes an error reply, which
    /// never is).
    pub fn to_json(&self) -> Vec<u8> {
        match serde_json::to_vec(self) {
            Ok(bytes) if bytes.len() <= MAX_REPLY => bytes,
            _ => {
                let fallback = Self::new(
                    self.v,
                    self.id,
                    Reply::error(ErrorReason::Invalid, "the reply was too large"),
                );
                serde_json::to_vec(&fallback).unwrap_or_default()
            }
        }
    }
}

/// Why [`decode`] refused a message: the error to answer with instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rejection {
    /// The version to answer in.
    pub v: u32,
    /// The message's `id`, if it could be read.
    pub id: Option<u32>,
    /// The error.
    pub error: ErrorReply,
}

impl Rejection {
    fn new(v: u32, id: Option<u32>, reason: ErrorReason, message: impl Into<String>) -> Self {
        Self {
            v,
            id,
            error: ErrorReply {
                reason,
                message: message.into(),
            },
        }
    }

    /// The error reply with its envelope.
    pub fn into_envelope(self) -> ReplyEnvelope {
        ReplyEnvelope::new(self.v, self.id, Reply::Error(self.error))
    }
}

/// Reads one message from the browser: size, UTF-8, JSON, envelope,
/// version, strict shape, then every limit ([`Message::validate`]). Never
/// panics, whatever the bytes.
pub fn decode(bytes: &[u8]) -> Result<Envelope, Rejection> {
    use serde_json::Value;

    let invalid = |id, message: String| Rejection::new(VERSION, id, ErrorReason::Invalid, message);
    if bytes.len() > MAX_MESSAGE {
        return Err(Rejection::new(
            VERSION,
            None,
            ErrorReason::TooLarge,
            format!("a message of {} bytes exceeds the 1 MiB limit", bytes.len()),
        ));
    }
    let mut object = match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(o)) => o,
        Ok(_) => return Err(invalid(None, "a message is a JSON object".into())),
        Err(e) => return Err(invalid(None, format!("not JSON: {e}"))),
    };
    let id = match object.remove("id") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
            Some(n) => Some(n),
            None => {
                return Err(invalid(
                    None,
                    "id must be an integer from 0 to 2^32-1".into(),
                ));
            }
        },
    };
    let v = match object.remove("v").as_ref().map(Value::as_u64) {
        Some(Some(v)) => u32::try_from(v).unwrap_or(u32::MAX),
        Some(None) => return Err(invalid(id, "v must be an integer".into())),
        None => return Err(invalid(id, "v (the protocol version) is missing".into())),
    };
    if v > VERSION {
        return Err(Rejection::new(
            VERSION,
            id,
            ErrorReason::AppTooOld,
            format!(
                "the extension speaks protocol {v}, this Boltget {OLDEST_VERSION} to {VERSION}: update Boltget"
            ),
        ));
    }
    if v < OLDEST_VERSION {
        return Err(Rejection::new(
            VERSION,
            id,
            ErrorReason::ExtensionTooOld,
            format!(
                "the extension speaks protocol {v}, this Boltget {OLDEST_VERSION} to {VERSION}: update the extension"
            ),
        ));
    }
    let message: Message = serde_json::from_value(Value::Object(object))
        .map_err(|e| Rejection::new(v, id, ErrorReason::Invalid, e.to_string()))?;
    message
        .validate()
        .map_err(|e| Rejection::new(v, id, ErrorReason::Invalid, e.to_string()))?;
    Ok(Envelope { v, id, message })
}

// Validation -------------------------------------------------------------

/// A value that breaks the protocol's rules.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {problem}")]
#[non_exhaustive]
pub struct Invalid {
    /// The field (`url`, `cookies.domain`, ...).
    pub field: &'static str,
    /// What is wrong, without echoing the value (it may be a secret).
    pub problem: String,
}

impl Invalid {
    fn new(field: &'static str, problem: impl Into<String>) -> Self {
        Self {
            field,
            problem: problem.into(),
        }
    }
}

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// An `http` or `https` link the engine accepts.
fn link(field: &'static str, url: &str) -> Result<(), Invalid> {
    if url.len() > MAX_URL {
        return Err(Invalid::new(field, "longer than 32 KiB"));
    }
    if url.is_empty() || has_control(url) || url.contains(' ') {
        return Err(Invalid::new(field, "not a link"));
    }
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(Invalid::new(field, "only http and https links"));
    }
    RequestContext::new(url)
        .map(drop)
        .map_err(|_| Invalid::new(field, "not a valid link"))
}

fn optional_link(field: &'static str, url: Option<&str>) -> Result<(), Invalid> {
    url.map_or(Ok(()), |u| link(field, u))
}

fn short(field: &'static str, s: &str) -> Result<(), Invalid> {
    if s.len() > MAX_SHORT {
        return Err(Invalid::new(field, "too long"));
    }
    if has_control(s) {
        return Err(Invalid::new(field, "control characters"));
    }
    Ok(())
}

fn text_value(field: &'static str, s: &str) -> Result<(), Invalid> {
    if s.len() > MAX_TEXT {
        return Err(Invalid::new(field, "too long"));
    }
    if s.contains('\0') {
        return Err(Invalid::new(field, "a NUL character"));
    }
    Ok(())
}

/// A file name: no folders, no control characters.
fn file_name(name: &str) -> Result<(), Invalid> {
    text_value("filename", name)?;
    if name.trim().is_empty() || has_control(name) || name.contains(['/', '\\']) {
        return Err(Invalid::new("filename", "not a file name"));
    }
    Ok(())
}

fn mime_type(mime: &str) -> Result<(), Invalid> {
    short("mime", mime)?;
    if !mime.is_ascii() {
        return Err(Invalid::new("mime", "not ASCII"));
    }
    Ok(())
}

fn cookies(list: &[CookieRecord]) -> Result<(), Invalid> {
    if list.len() > MAX_COOKIES {
        return Err(Invalid::new("cookies", format!("more than {MAX_COOKIES}")));
    }
    let mut bytes = 0usize;
    for c in list {
        bytes = bytes.saturating_add(c.name.len() + c.value.len());
        if c.name.is_empty() && c.value.is_empty() {
            return Err(Invalid::new("cookies.name", "empty"));
        }
        if c.name.contains(['=', ';']) || has_control(&c.name) {
            return Err(Invalid::new("cookies.name", "not a cookie name"));
        }
        if c.value.contains(';') || has_control(&c.value) {
            return Err(Invalid::new("cookies.value", "not a cookie value"));
        }
        let domain = c.domain.trim_start_matches('.');
        if domain.is_empty()
            || c.domain.len() > MAX_SHORT
            || !domain.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']')
            })
        {
            return Err(Invalid::new("cookies.domain", "not a domain"));
        }
        if !c.path.starts_with('/') || c.path.len() > MAX_TEXT * 4 || has_control(&c.path) {
            return Err(Invalid::new("cookies.path", "not a path"));
        }
        if let Some(e) = c.expires
            && !(e.is_finite() && (0.0..=MAX_EXPIRY).contains(&e))
        {
            return Err(Invalid::new("cookies.expires", "not a time"));
        }
    }
    if bytes > MAX_COOKIE_BYTES {
        return Err(Invalid::new("cookies", "more than 64 KiB"));
    }
    Ok(())
}

fn headers(list: &[(String, String)]) -> Result<(), Invalid> {
    let mut seen: Vec<String> = Vec::new();
    for (name, value) in list {
        let lower = name.to_ascii_lowercase();
        if !ALLOWED_HEADERS.contains(&lower.as_str()) {
            return Err(Invalid::new(
                "headers",
                format!("{} is not on the allowlist", name.escape_debug()),
            ));
        }
        if seen.contains(&lower) {
            return Err(Invalid::new("headers", format!("{lower} twice")));
        }
        if value.len() > MAX_HEADER_VALUE || HeaderValue::from_str(value).is_err() {
            return Err(Invalid::new("headers", format!("bad value for {lower}")));
        }
        seen.push(lower);
    }
    Ok(())
}

fn header_names(list: &[(String, String)]) -> Vec<&str> {
    list.iter().map(|(n, _)| n.as_str()).collect()
}

/// A request context for `url` with the browser's headers, its referrer
/// (unless a `Referer` header came along) and cookies. The message was
/// validated, so nothing here should fail; if it does, it is invalid.
fn context(
    url: &str,
    headers: &[(String, String)],
    referrer: Option<&str>,
    cookies: &[CookieRecord],
) -> Result<RequestContext, Invalid> {
    use boltget_engine::{HeaderName, header};

    let mut ctx = RequestContext::new(url).map_err(|_| Invalid::new("url", "not a valid link"))?;
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes())
            .map_err(|_| Invalid::new("headers", "bad name"))?;
        let value =
            HeaderValue::from_str(value).map_err(|_| Invalid::new("headers", "bad value"))?;
        ctx.header(name, value);
    }
    if let Some(r) = referrer
        && !ctx.headers().contains_key(header::REFERER)
        && let Ok(value) = HeaderValue::from_str(r)
    {
        ctx.header(header::REFERER, value);
    }
    let now = SystemTime::now();
    for record in cookies {
        let cookie = record.to_cookie();
        if !cookie.is_expired(now) {
            ctx.add_cookie(cookie);
        }
    }
    Ok(ctx)
}

/// The fuzz target `native_message` (`fuzz/fuzz_targets/native_message.rs`):
/// [`decode`] on arbitrary bytes, and the properties the host relies on. A
/// panic is a finding. Not an API; the seed corpus
/// (`fuzz/corpus/native_message/`) also runs in this crate's tests.
#[cfg(any(test, feature = "fuzzing"))]
#[doc(hidden)]
pub fn fuzz(data: &[u8]) {
    match decode(data) {
        Ok(env) => {
            // What was accepted keeps every rule, says the same thing again,
            // and always becomes the engine's request.
            assert!(env.message.validate().is_ok());
            assert!((OLDEST_VERSION..=VERSION).contains(&env.v));
            let again = serde_json::to_vec(&env).expect("an envelope serializes");
            assert_eq!(decode(&again).as_ref(), Ok(&env), "round trip");
            match &env.message {
                Message::Capture(c) => {
                    c.context().expect("a valid capture has a context");
                }
                Message::AddLinks(a) => {
                    for l in &a.links {
                        a.context(&l.url).expect("a valid link has a context");
                    }
                }
                _ => {}
            }
        }
        Err(rejection) => {
            let out = rejection.into_envelope().to_json();
            assert!(out.len() <= MAX_REPLY);
            serde_json::from_slice::<ReplyEnvelope>(&out).expect("the reply reads back");
        }
    }
}

#[cfg(test)]
mod tests;
