//! The messages (docs/ARCHITECTURE.md, "CLI to app (named pipe)").
//!
//! Every message is a JSON object with a `type`. Requests are strict: an
//! unknown type or field is refused rather than ignored, so a client never
//! believes an option was applied when the owner did not know it. Responses
//! are read leniently, so an owner may add fields within a version.
//!
//! Ids are the download ids `boltget list` shows. Paths are absolute: the
//! client resolves them against its own working directory before sending.

use std::fmt;
use std::path::PathBuf;

use boltget_engine::{ErrorKind, SavedError};
use serde::{Deserialize, Serialize};

/// The protocol version this build speaks. It changes when a message changes
/// meaning or a request gains a field (requests are strict); a response
/// gaining a field does not change it.
pub const PROTOCOL_VERSION: u32 = 1;

/// The oldest client version the owner still serves.
pub const OLDEST_SUPPORTED: u32 = 1;

/// The key of the error an owner answers a client of another version with.
pub const VERSION_ERROR_KEY: &str = "error.protocol_version";

/// The key of the error for a message the owner cannot accept.
pub const INVALID_REQUEST_KEY: &str = "error.invalid_request";

/// A link for logs: no query, fragment or credentials.
fn redact(url: &str) -> String {
    let cut = url.find(['?', '#']).unwrap_or(url.len());
    let (base, rest) = url.split_at(cut);
    let base = match base.split_once("://") {
        Some((scheme, after)) => {
            let host_end = after.find('/').unwrap_or(after.len());
            let host = after[..host_end].rsplit('@').next().unwrap_or_default();
            format!("{scheme}://{host}{}", &after[host_end..])
        }
        None => base.to_owned(),
    };
    if rest.is_empty() {
        base
    } else {
        format!("{base}?…")
    }
}

/// Client to owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Request {
    /// The first message on every connection.
    Hello {
        /// The client's [`PROTOCOL_VERSION`].
        v: u32,
        /// Who is asking, for logs (`boltget-cli 0.2.0`).
        client: String,
    },
    /// Every download: [`Response::Downloads`].
    List,
    /// Starts a download: [`Response::Added`].
    Add(AddSpec),
    /// Pauses a download after its final checkpoint: [`Response::Paused`].
    Pause {
        /// The download.
        id: u64,
    },
    /// Continues a stopped download: [`Response::Ok`].
    Resume(ResumeSpec),
    /// Removes a download from the list: [`Response::Removed`].
    Remove {
        /// The download.
        id: u64,
        /// Also delete its partial file, or its file if it completed.
        #[serde(default)]
        delete_file: bool,
    },
    /// Gives a stopped download a new link to the same file:
    /// [`Response::Refreshed`]. It stays stopped; resume it to continue.
    Refresh(RefreshSpec),
    /// The global settings: [`Response::Settings`].
    Settings,
    /// Changes the global limit, or one download's: [`Response::Settings`].
    SetLimit {
        /// The download; absent for the global limit.
        #[serde(default)]
        id: Option<u64>,
        /// Bytes per second; absent or 0 removes the limit.
        #[serde(default)]
        bps: Option<u64>,
    },
    /// Streams a download until it stops: [`Response::Event`] and
    /// [`Response::Progress`] messages, then one [`Response::Finished`].
    /// The connection takes requests again afterwards.
    Watch {
        /// The download.
        id: u64,
    },
}

const fn yes() -> bool {
    true
}

/// [`Request::Add`].
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[expect(clippy::struct_excessive_bools, reason = "the command line's flags")]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AddSpec {
    /// The link (http or https).
    pub url: String,
    /// An absolute folder to save into, or with `to_file` the file itself.
    pub destination: PathBuf,
    /// `destination` names the file, not a folder.
    #[serde(default)]
    pub to_file: bool,
    /// Replace an existing file instead of saving as `name (1).ext`.
    #[serde(default)]
    pub overwrite: bool,
    /// A fixed connection count (1 to 32); absent: adaptive.
    #[serde(default)]
    pub connections: Option<u32>,
    /// This download's limit in bytes per second; absent or 0: none.
    #[serde(default)]
    pub limit: Option<u64>,
    /// The checksum the file must have (`sha256:<hex>`, ...).
    #[serde(default)]
    pub checksum: Option<String>,
    /// Verify a digest the server advertises (default true).
    #[serde(default = "yes")]
    pub server_digest: bool,
    /// Record it paused instead of starting it.
    #[serde(default)]
    pub paused: bool,
    /// Headers, cookies and proxy.
    #[serde(default)]
    pub request: RequestSpec,
}

impl AddSpec {
    /// A download of `url` into the folder (or file) `destination`, with
    /// every option at its default.
    pub fn new(url: impl Into<String>, destination: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            destination: destination.into(),
            to_file: false,
            overwrite: false,
            connections: None,
            limit: None,
            checksum: None,
            server_digest: true,
            paused: false,
            request: RequestSpec::default(),
        }
    }
}

impl fmt::Debug for AddSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddSpec")
            .field("url", &redact(&self.url))
            .field("destination", &self.destination)
            .field("to_file", &self.to_file)
            .field("connections", &self.connections)
            .field("limit", &self.limit)
            .field("paused", &self.paused)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

/// How requests for a download are made: what `-H`, `--cookie`,
/// `--cookie-file`, `--user` and `--proxy` give on the command line. The
/// owner applies the same rules as the engine (some headers are refused).
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RequestSpec {
    /// Extra headers, `[name, value]`.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Cookies for the link's host, `[name, value]`.
    #[serde(default)]
    pub cookies: Vec<(String, String)>,
    /// A Netscape `cookies.txt`, read by the owner.
    #[serde(default)]
    pub cookie_file: Option<PathBuf>,
    /// `direct`, or a proxy URL (it may hold a password).
    #[serde(default)]
    pub proxy: Option<String>,
}

impl RequestSpec {
    /// Whether nothing is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Header values, cookies and proxies can be credentials: their names only.
impl fmt::Debug for RequestSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = |v: &[(String, String)]| v.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        f.debug_struct("RequestSpec")
            .field("headers", &names(&self.headers))
            .field("cookies", &names(&self.cookies))
            .field("cookie_file", &self.cookie_file)
            .field("proxy", &self.proxy.as_ref().map(|_| "<set>"))
            .finish()
    }
}

/// [`Request::Resume`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ResumeSpec {
    /// The download.
    pub id: u64,
    /// Throw the saved progress away and start again.
    #[serde(default)]
    pub restart: bool,
    /// A fixed connection count for this run.
    #[serde(default)]
    pub connections: Option<u32>,
    /// A new limit for this download (0 removes it); absent keeps the saved one.
    #[serde(default)]
    pub limit: Option<u64>,
    /// A new checksum; absent keeps the saved one.
    #[serde(default)]
    pub checksum: Option<String>,
    /// Forget the saved checksum.
    #[serde(default)]
    pub no_checksum: bool,
    /// Headers, cookies and proxy for this run (nothing secret is saved).
    #[serde(default)]
    pub request: RequestSpec,
}

impl ResumeSpec {
    /// Continue download `id` as it was.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            restart: false,
            connections: None,
            limit: None,
            checksum: None,
            no_checksum: false,
            request: RequestSpec::default(),
        }
    }
}

/// [`Request::Refresh`].
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RefreshSpec {
    /// The download.
    pub id: u64,
    /// The new link.
    pub url: String,
    /// Headers, cookies and proxy for the new link.
    #[serde(default)]
    pub request: RequestSpec,
}

impl RefreshSpec {
    /// A new link `url` for download `id`.
    pub fn new(id: u64, url: impl Into<String>) -> Self {
        Self {
            id,
            url: url.into(),
            request: RequestSpec::default(),
        }
    }
}

impl fmt::Debug for RefreshSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshSpec")
            .field("id", &self.id)
            .field("url", &redact(&self.url))
            .field("request", &self.request)
            .finish()
    }
}

/// Owner to client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Response {
    /// The answer to [`Request::Hello`]: the version both now speak.
    Hello {
        /// The protocol version.
        v: u32,
        /// The owner, for messages (`Boltget 0.2.0`).
        server: String,
    },
    /// The request failed; the connection stays usable unless the request
    /// itself could not be read.
    Error {
        /// What went wrong.
        error: WireError,
    },
    /// [`Request::List`]: each download as `boltget list --json` prints it,
    /// with `downloaded` live for running downloads.
    Downloads {
        /// Oldest first.
        downloads: Vec<serde_json::Value>,
    },
    /// [`Request::Add`].
    Added {
        /// The new download.
        id: u64,
    },
    /// [`Request::Pause`]: the state the download is in now (`paused`, or
    /// the state of one that was not running).
    Paused {
        /// The download.
        id: u64,
        /// Its state.
        state: String,
    },
    /// [`Request::Remove`].
    Removed {
        /// The download.
        id: u64,
        /// Files deleted with it.
        deleted: Vec<PathBuf>,
    },
    /// [`Request::Refresh`]: why the link was accepted, as the engine's
    /// `RefreshOutcome` serializes (`{id, evidence: {kind, ...}, final_url}`).
    Refreshed {
        /// The outcome.
        outcome: serde_json::Value,
    },
    /// [`Request::Settings`] and [`Request::SetLimit`].
    Settings {
        /// The global limit in bytes per second; absent: none.
        limit: Option<u64>,
    },
    /// A request that returns nothing else succeeded.
    Ok,
    /// During [`Request::Watch`]: an engine event for the download, as
    /// `boltget get --json` prints it (`{"event": "probed", ...}`).
    Event {
        /// The event.
        event: serde_json::Value,
    },
    /// During [`Request::Watch`]: the download's progress, every 500 ms.
    Progress(ProgressReport),
    /// The end of [`Request::Watch`]: the download stopped.
    Finished(Finished),
}

/// [`Response::Progress`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProgressReport {
    /// The download.
    pub id: u64,
    /// Its state (`probing`, `downloading`, ...).
    pub state: String,
    /// Bytes received.
    pub downloaded: u64,
    /// The size, once known.
    pub total: Option<u64>,
    /// Open connections.
    pub connections: u32,
    /// Segments in its table.
    pub segments: usize,
}

impl ProgressReport {
    /// A report of `progress`.
    pub fn new(p: &boltget_engine::Progress) -> Self {
        Self {
            id: p.id.0,
            state: p.state.as_str().to_owned(),
            downloaded: p.downloaded,
            total: p.total,
            connections: p.connections,
            segments: p.segments.len(),
        }
    }
}

/// [`Response::Finished`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Finished {
    /// The download.
    pub id: u64,
    /// How it stopped: `completed`, `paused`, `failed`, `corrupt`,
    /// `needs_new_link` or `cancelled`.
    pub state: String,
    /// The file, once its name is known.
    pub path: Option<PathBuf>,
    /// The partial file, while it is not completed.
    pub partial_path: Option<PathBuf>,
    /// Bytes saved.
    pub downloaded: u64,
    /// The size, if known.
    pub total: Option<u64>,
    /// The checksum it was verified against (`sha256:<hex>`).
    pub verified: Option<String>,
    /// Why it stopped, unless it completed or was paused.
    pub error: Option<WireError>,
}

impl Finished {
    /// How a download stands, from its summary.
    pub fn from_summary(s: &boltget_engine::DownloadSummary) -> Self {
        let verified = s.checksum.as_ref().and_then(|c| {
            (c.actual.as_deref() == Some(c.expected.hex.as_str())).then(|| c.expected.to_string())
        });
        Self {
            id: s.id.0,
            state: s.state.as_str().to_owned(),
            path: s.path.clone(),
            partial_path: s.partial_path.clone(),
            downloaded: s.downloaded,
            total: s.total,
            verified,
            error: s.error.as_ref().map(WireError::from),
        }
    }
}

/// An error as it travels: the engine's classification and catalog key,
/// plus an English diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WireError {
    /// `ErrorKind` in `snake_case` (`transient`, `disk`, ...), or `protocol`
    /// for an error of the pipe itself.
    pub kind: String,
    /// The message-catalog key.
    pub key: String,
    /// English diagnostic text.
    pub message: String,
}

impl WireError {
    /// An error of the protocol itself (`kind` = `protocol`).
    pub fn protocol(key: &str, message: impl Into<String>) -> Self {
        Self {
            kind: "protocol".into(),
            key: key.into(),
            message: message.into(),
        }
    }

    /// The engine's classification, when this is an engine error.
    pub fn error_kind(&self) -> Option<ErrorKind> {
        ErrorKind::parse(&self.kind)
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<&boltget_engine::Error> for WireError {
    fn from(e: &boltget_engine::Error) -> Self {
        Self {
            kind: e.kind().as_str().into(),
            key: e.message_key().into(),
            message: e.to_string(),
        }
    }
}

impl From<boltget_engine::Error> for WireError {
    fn from(e: boltget_engine::Error) -> Self {
        Self::from(&e)
    }
}

impl From<&boltget_engine::ErrorInfo> for WireError {
    fn from(e: &boltget_engine::ErrorInfo) -> Self {
        Self {
            kind: e.kind.as_str().into(),
            key: e.key.into(),
            message: e.message.clone(),
        }
    }
}

impl From<&SavedError> for WireError {
    fn from(e: &SavedError) -> Self {
        Self {
            kind: e.kind.as_str().into(),
            key: e.key.clone(),
            message: e.message.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(v: serde_json::Value) -> Result<Request, serde_json::Error> {
        serde_json::from_value(v)
    }

    #[test]
    fn requests_have_a_type_tag() {
        assert_eq!(
            serde_json::to_value(Request::Pause { id: 7 }).unwrap(),
            json!({"type": "pause", "id": 7})
        );
        assert_eq!(
            serde_json::to_value(Request::List).unwrap(),
            json!({"type": "list"})
        );
        let add =
            parse(json!({"type": "add", "url": "https://x.test/a", "destination": "/d"})).unwrap();
        let Request::Add(spec) = add else {
            panic!("expected add")
        };
        assert!(spec.server_digest, "verifies server digests by default");
        assert!(spec.request.is_empty());
    }

    #[test]
    fn requests_are_strict() {
        assert!(parse(json!({"type": "pause", "id": 7, "force": true})).is_err());
        assert!(
            parse(json!({"type": "add", "url": "u", "destination": "/d", "queue": "x"})).is_err()
        );
        assert!(
            parse(json!({"type": "resume", "id": 1, "request": {"referer": "x"}})).is_err(),
            "nested specs too"
        );
        assert!(parse(json!({"type": "explode"})).is_err());
        assert!(parse(json!({"id": 7})).is_err());
    }

    #[test]
    fn requests_round_trip() {
        let mut add = AddSpec::new("https://x.test/a.iso", "/downloads");
        add.connections = Some(4);
        add.request
            .headers
            .push(("Referer".into(), "https://x.test/".into()));
        add.request.proxy = Some("direct".into());
        let mut resume = ResumeSpec::new(3);
        resume.no_checksum = true;
        for req in [
            Request::Hello {
                v: PROTOCOL_VERSION,
                client: "test".into(),
            },
            Request::Add(add),
            Request::Resume(resume),
            Request::Refresh(RefreshSpec::new(3, "https://x.test/b")),
            Request::SetLimit {
                id: None,
                bps: Some(5),
            },
            Request::Watch { id: 3 },
        ] {
            let text = serde_json::to_string(&req).unwrap();
            assert_eq!(
                serde_json::from_str::<Request>(&text).unwrap(),
                req,
                "{text}"
            );
        }
    }

    #[test]
    fn responses_ignore_fields_they_do_not_know() {
        let r: Response =
            serde_json::from_value(json!({"type": "added", "id": 4, "queue": "main"})).unwrap();
        assert_eq!(r, Response::Added { id: 4 });
        assert_eq!(
            serde_json::to_value(Response::Ok).unwrap(),
            json!({"type": "ok"})
        );
    }

    #[test]
    fn debug_output_hides_credentials() {
        let mut spec = AddSpec::new("https://user:pw@x.test/a?token=secret", "/d");
        spec.request
            .headers
            .push(("Authorization".into(), "Basic c2VjcmV0".into()));
        spec.request.cookies.push(("sid".into(), "secret".into()));
        spec.request.proxy = Some("http://u:secret@proxy.test".into());
        let text = format!("{spec:?}");
        assert!(!text.contains("secret") && !text.contains("c2Vj"), "{text}");
        assert!(
            text.contains("Authorization") && text.contains("sid"),
            "{text}"
        );
    }

    #[test]
    fn wire_errors_keep_the_engine_kind() {
        let e = WireError::from(boltget_engine::Error::UnknownDownload(
            boltget_engine::DownloadId(9),
        ));
        assert_eq!(e.key, "error.unknown_download");
        assert_eq!(e.error_kind(), Some(ErrorKind::NotFound));
        assert_eq!(
            WireError::protocol(VERSION_ERROR_KEY, "x").error_kind(),
            None
        );
    }
}
