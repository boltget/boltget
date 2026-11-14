//! The browser extension, as the native host relays it over the pipe
//! (docs/BROWSER-INTEGRATION.md, "Capture flow"; the protocol is
//! `boltget_ipc::browser`), and the browsers' registration in Settings.
//!
//! A captured download starts at once, in the main queue, in its
//! category's folder, unless "Ask before adding" is on: then the add sheet
//! opens with the link, and the request context (cookies, headers,
//! referrer) waits here for the sheet's preview and its Download button.
//! Either way the reply is "accepted", since the browser's copy is dropped
//! as soon as the app answers.
//!
//! Before it answers, the app makes its own first request for the link
//! (with the browser's cookies and headers). If the server refuses it (a
//! one-time link the browser has used) or does not answer within
//! [`PROBE_BUDGET`], the reply is "rejected" and the browser keeps the
//! download: losing a download is worse than not accelerating it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use boltget_engine::{
    AddRequest, DEFAULT_QUEUE, Destination, Engine, ErrorKind, RequestContext, redact_url,
};
use boltget_ipc::BrowserReply;
use boltget_ipc::browser::{
    Accepted, AddLinks, Capture, CaptureRules, ErrorReason, HelloReply, MAX_LINKS, Message,
    RejectReason, Reply, StatusReply,
};
use boltget_native_host::registration::{self, Flavor, Registration, State as RegState};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::bridge::Bridge;
use crate::bridge::dto::DownloadState;
use crate::error::UiError;
use crate::settings::{self, AppSettings};
use crate::shell::{self, ShellRequest};

/// How long the app's own first request for a captured link may take
/// before the browser is told to keep the download. Within the host's
/// reply budget (6 s), which is within the extension's timeout.
pub(crate) const PROBE_BUDGET: Duration = Duration::from_secs(4);

/// How long a captured link's request context waits for the add sheet.
const KEEP: Duration = Duration::from_secs(30 * 60);

/// Request contexts of links the browser handed over for the add sheet, by
/// link, oldest first, so the sheet's preview and its Download button make
/// the browser's request. Never saved; cookies stay in memory only.
#[derive(Debug, Default)]
pub(crate) struct Captured(Mutex<VecDeque<(String, RequestContext, Instant)>>);

impl Captured {
    fn remember(&self, url: String, ctx: RequestContext) {
        let mut list = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        list.retain(|(u, _, at)| *u != url && at.elapsed() < KEEP);
        while list.len() >= MAX_LINKS {
            list.pop_front();
        }
        list.push_back((url, ctx, Instant::now()));
    }

    /// The context for `url`, if the browser handed it over lately.
    pub(crate) fn get(&self, url: &str) -> Option<RequestContext> {
        let list = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        list.iter()
            .find(|(u, _, at)| u == url && at.elapsed() < KEEP)
            .map(|(_, ctx, _)| ctx.clone())
    }

    /// The context for `url`, once: the download now has it.
    pub(crate) fn take(&self, url: &str) -> Option<RequestContext> {
        let mut list = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let i = list.iter().position(|(u, _, _)| u == url)?;
        list.remove(i)
            .filter(|(_, _, at)| at.elapsed() < KEEP)
            .map(|(_, ctx, _)| ctx)
    }
}

/// What handling the browser needs from the app; tests give it without a
/// window.
pub(crate) struct Host<'a> {
    pub(crate) bridge: &'a Bridge,
    pub(crate) captured: &'a Captured,
    pub(crate) settings: AppSettings,
    /// Windows' Downloads folder, the last resort for a download's folder.
    pub(crate) downloads: Option<PathBuf>,
    /// Opens the add sheet with these links, one per line.
    pub(crate) open_sheet: &'a (dyn Fn(String) + Sync),
}

/// The app's answer to a message from the extension.
pub(crate) fn reply(app: AppHandle, engine: Engine, message: Message) -> BrowserReply {
    Box::pin(async move {
        let bridge = app.state::<Bridge>();
        let captured = app.state::<Captured>();
        let open_sheet = |links: String| shell::request(&app, &ShellRequest::Add { links });
        let host = Host {
            bridge: &bridge,
            captured: &captured,
            settings: bridge.settings(),
            downloads: app.path().download_dir().ok(),
            open_sheet: &open_sheet,
        };
        handle(&host, &engine, message).await
    })
}

pub(crate) async fn handle(host: &Host<'_>, engine: &Engine, message: Message) -> Reply {
    match message {
        Message::Hello(_) => Reply::Hello(HelloReply::new(
            Some(format!("Boltget {}", env!("CARGO_PKG_VERSION"))),
            Some(CaptureRules::default()),
        )),
        Message::Capture(c) => capture(host, engine, &c).await,
        Message::AddLinks(a) => add_links(host, &a),
        Message::Status(_) => status(host.bridge).await,
        // Media arrives with v0.4 (docs/VIDEO.md).
        Message::MediaPage(_) => Reply::rejected(
            RejectReason::Unsupported,
            "this Boltget does not download media from pages yet",
        ),
        other => Reply::rejected(
            RejectReason::Unsupported,
            format!("{} is not handled", other.kind()),
        ),
    }
}

async fn capture(host: &Host<'_>, engine: &Engine, c: &Capture) -> Reply {
    let ctx = match c.context() {
        Ok(ctx) => ctx,
        Err(e) => return Reply::error(ErrorReason::Invalid, e.to_string()),
    };
    let link = ctx.url().as_str().to_owned();
    if host.settings.browser_ask {
        host.captured.remember(link.clone(), ctx);
        (host.open_sheet)(link);
        return Reply::Accepted(Accepted::confirming());
    }
    let probe = match tokio::time::timeout(PROBE_BUDGET, engine.probe(&ctx)).await {
        Ok(Ok(probe)) => probe,
        Ok(Err(e)) => {
            let reason = match e.kind() {
                ErrorKind::LinkExpired | ErrorKind::Permanent | ErrorKind::Throttled => {
                    RejectReason::LinkRefused
                }
                _ => RejectReason::Failed,
            };
            return Reply::rejected(reason, e.to_string());
        }
        Err(_) => {
            return Reply::rejected(RejectReason::Failed, "the server did not answer in time");
        }
    };
    let name = c.filename.as_deref().unwrap_or(&probe.filename);
    let Some(dir) = settings::folder_for(&host.settings, Some(name), host.downloads.as_deref())
    else {
        return Reply::rejected(RejectReason::Failed, "no download folder");
    };
    let url = redact_url(ctx.url());
    let req = AddRequest::new(ctx, Destination::Directory(dir)).queue(DEFAULT_QUEUE);
    match engine.enqueue(req).await {
        Ok(id) => {
            host.bridge.added(id.into(), url);
            Reply::Accepted(Accepted::added(id.0))
        }
        Err(e) => Reply::rejected(RejectReason::Failed, e.to_string()),
    }
}

/// "Download all links": the add sheet opens with them to choose from.
fn add_links(host: &Host<'_>, a: &AddLinks) -> Reply {
    let mut links = Vec::with_capacity(a.links.len());
    for l in &a.links {
        match a.context(&l.url) {
            Ok(ctx) => {
                let link = ctx.url().as_str().to_owned();
                host.captured.remember(link.clone(), ctx);
                links.push(link);
            }
            Err(e) => return Reply::error(ErrorReason::Invalid, e.to_string()),
        }
    }
    (host.open_sheet)(links.join(
        "
",
    ));
    Reply::Accepted(Accepted::confirming())
}

async fn status(bridge: &Bridge) -> Reply {
    let (running, bps) = {
        let a = bridge.activity();
        let a = a.borrow();
        (a.running, a.bps)
    };
    let queued = bridge
        .list()
        .await
        .iter()
        .filter(|r| r.state == DownloadState::Queued)
        .count();
    Reply::Status(StatusReply::new(
        u32::try_from(running).unwrap_or(u32::MAX),
        u32::try_from(queued).unwrap_or(u32::MAX),
        bps,
    ))
}

// The browsers' registration, for Settings ---------------------------------

/// A browser in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BrowserName {
    Chrome,
    Edge,
    Firefox,
}

/// How a browser's registration stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RegistrationState {
    /// The browser finds Boltget.
    Registered,
    /// Not registered.
    Missing,
    /// Registered for a Boltget elsewhere, or files are gone.
    WrongPath,
    /// Registered by an older release.
    Outdated,
}

/// One browser's registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserRegistration {
    pub(crate) browser: BrowserName,
    pub(crate) state: RegistrationState,
}

/// What Settings shows about the browsers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserIntegration {
    /// Whether this system has browser integration (Windows, for now).
    pub(crate) supported: bool,
    /// Every browser, Chrome, Edge, Firefox; empty when not supported.
    pub(crate) browsers: Vec<BrowserRegistration>,
}

fn registration() -> Option<Registration> {
    let exe = std::env::current_exe().ok()?;
    Some(Registration::in_dir(exe.parent()?, Flavor::current()))
}

fn integration() -> BrowserIntegration {
    let (Some(r), Ok(reg)) = (registration(), registration::system()) else {
        return BrowserIntegration {
            supported: false,
            browsers: Vec::new(),
        };
    };
    let browsers = r
        .status(&reg)
        .into_iter()
        .map(|s| BrowserRegistration {
            browser: match s.browser {
                registration::Browser::Chrome => BrowserName::Chrome,
                registration::Browser::Edge => BrowserName::Edge,
                registration::Browser::Firefox => BrowserName::Firefox,
            },
            state: match s.state {
                RegState::Registered => RegistrationState::Registered,
                RegState::Missing => RegistrationState::Missing,
                RegState::WrongPath => RegistrationState::WrongPath,
                RegState::Outdated => RegistrationState::Outdated,
            },
        })
        .collect();
    BrowserIntegration {
        supported: true,
        browsers,
    }
}

/// Whether each browser finds Boltget's native messaging host.
#[tauri::command]
#[specta::specta]
pub(crate) fn browser_integration() -> BrowserIntegration {
    integration()
}

/// "Repair browser integration": writes the manifests and registry keys
/// again for this install, then says how each browser stands.
#[tauri::command]
#[specta::specta]
pub(crate) fn repair_browser_integration() -> Result<BrowserIntegration, UiError> {
    let r = registration().ok_or_else(|| UiError::key("error.browser_integration"))?;
    let reg = registration::system()
        .map_err(|e| UiError::with_message("error.browser_integration", e.to_string()))?;
    r.register(&reg).map_err(|e| match e {
        registration::RegistrationError::NoHost(_) => {
            UiError::with_message("error.browser_host_missing", e.to_string())
        }
        e => UiError::with_message("error.browser_integration", e.to_string()),
    })?;
    Ok(integration())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex as StdMutex;

    use boltget_engine::EngineConfig;
    use boltget_ipc::browser::{CookieRecord, Link};
    use boltget_testserver::TestServer;

    struct Fixture {
        bridge: Bridge,
        captured: Captured,
        engine: Engine,
        _data: tempfile::TempDir,
        downloads: tempfile::TempDir,
        sheet: StdMutex<Vec<String>>,
    }

    impl Fixture {
        fn new() -> Self {
            let (bridge, hub) = Bridge::new(crate::bridge::TICK);
            tokio::spawn(hub);
            let data = tempfile::tempdir().unwrap();
            Self {
                bridge,
                captured: Captured::default(),
                engine: Engine::open(EngineConfig::default(), data.path()).unwrap(),
                _data: data,
                downloads: tempfile::tempdir().unwrap(),
                sheet: StdMutex::new(Vec::new()),
            }
        }

        async fn send(&self, ask: bool, message: Message) -> Reply {
            let open_sheet = |links: String| self.sheet.lock().unwrap().push(links);
            let host = Host {
                bridge: &self.bridge,
                captured: &self.captured,
                settings: AppSettings {
                    browser_ask: ask,
                    ..AppSettings::default()
                },
                downloads: Some(self.downloads.path().to_path_buf()),
                open_sheet: &open_sheet,
            };
            handle(&host, &self.engine, message).await
        }
    }

    fn capture_of(url: String) -> Capture {
        let mut c = Capture::new(url);
        c.filename = Some("disc.iso".into());
        c.cookies
            .push(CookieRecord::new("sid", "s3cret", "127.0.0.1"));
        c.headers
            .push(("Referer".into(), "https://example.com/".into()));
        c
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_capture_downloads_with_the_browsers_cookies() {
        let srv = TestServer::start().await;
        let f = Fixture::new();
        f.engine
            .start_queues(std::sync::Arc::new(boltget_engine::SystemClock));
        // The server refuses every request without the cookie; several
        // connections.
        let url = srv.url("/cookie?need=sid%3Ds3cret&size=4194304&throttle=4194304");
        let reply = f.send(false, Message::Capture(capture_of(url))).await;
        let Reply::Accepted(a) = reply else {
            panic!("{reply:?}")
        };
        assert!(!a.confirm);
        let id = a.download.expect("added at once");
        let deadline = Instant::now() + Duration::from_secs(60);
        let done = loop {
            let list = f.engine.list().await.unwrap();
            let d = list.into_iter().find(|d| d.id.0 == id).expect("listed");
            if d.state.is_terminal() {
                break d;
            }
            assert!(Instant::now() < deadline, "still {}", d.state);
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(
            done.state,
            boltget_engine::State::Completed,
            "{:?}",
            done.error
        );
        // The app's own first request, the download's probe and every
        // connection after it.
        let reqs = srv.requests();
        assert!(reqs.len() >= 3, "{reqs:?}");
        for r in &reqs {
            assert_eq!(r.cookie.as_deref(), Some("sid=s3cret"), "{r:?}");
        }
        assert!(f.sheet.lock().unwrap().is_empty());
        f.engine.shutdown().await;
    }

    #[tokio::test]
    async fn a_refused_link_stays_in_the_browser() {
        let srv = TestServer::start().await;
        let f = Fixture::new();
        for (status, reason) in [
            (403, RejectReason::LinkRefused),
            (404, RejectReason::LinkRefused),
            (410, RejectReason::LinkRefused),
        ] {
            let url = srv.url(&format!("/data?fail_status={status}"));
            let reply = f.send(false, Message::Capture(capture_of(url))).await;
            assert!(
                matches!(&reply, Reply::Rejected(r) if r.reason == reason),
                "{status}: {reply:?}"
            );
        }
        assert!(
            f.engine.list().await.unwrap().is_empty(),
            "nothing was added"
        );
        f.engine.shutdown().await;
    }

    #[tokio::test]
    async fn asking_first_opens_the_sheet_with_the_context_waiting() {
        let f = Fixture::new();
        let reply = f
            .send(
                true,
                Message::Capture(capture_of("http://127.0.0.1:9/a.iso".into())),
            )
            .await;
        assert!(matches!(reply, Reply::Accepted(a) if a.confirm && a.download.is_none()));
        assert_eq!(*f.sheet.lock().unwrap(), ["http://127.0.0.1:9/a.iso"]);
        let ctx = f.captured.get("http://127.0.0.1:9/a.iso").expect("waiting");
        assert_eq!(ctx.suggested_name(), Some("disc.iso"));
        assert!(
            f.engine.list().await.unwrap().is_empty(),
            "the user decides"
        );

        let mut links = AddLinks::new(vec![
            Link::new("https://x.test/a.zip"),
            Link::new("https://x.test/b.zip"),
        ]);
        links.page_url = Some("https://x.test/".into());
        let reply = f.send(false, Message::AddLinks(links)).await;
        assert!(matches!(reply, Reply::Accepted(a) if a.confirm));
        assert_eq!(
            f.sheet.lock().unwrap()[1],
            "https://x.test/a.zip
https://x.test/b.zip"
        );
        assert!(f.captured.get("https://x.test/b.zip").is_some());
        f.engine.shutdown().await;
    }

    #[tokio::test]
    async fn hello_status_and_what_is_not_handled_yet() {
        let f = Fixture::new();
        let hello =
            boltget_ipc::browser::Hello::new("0.3.0", boltget_ipc::browser::Browser::Chrome);
        let Reply::Hello(h) = f.send(false, Message::Hello(hello)).await else {
            panic!("a hello")
        };
        assert!(h.app.unwrap().starts_with("Boltget "));
        assert_eq!(h.rules, Some(CaptureRules::default()));
        assert!(matches!(
            f.send(false, Message::Status(boltget_ipc::browser::Status {})).await,
            Reply::Status(s) if s.active == 0
        ));
        let page: boltget_ipc::browser::MediaPage =
            serde_json::from_value(serde_json::json!({"page_url": "https://v.test/watch"}))
                .unwrap();
        assert!(matches!(
            f.send(false, Message::MediaPage(page)).await,
            Reply::Rejected(r) if r.reason == RejectReason::Unsupported
        ));
        f.engine.shutdown().await;
    }

    #[test]
    fn captured_contexts_wait_for_the_sheet_once() {
        let captured = Captured::default();
        let ctx = RequestContext::new("https://x.test/a.iso").unwrap();
        captured.remember("https://x.test/a.iso".into(), ctx);
        assert!(
            captured.get("https://x.test/a.iso").is_some(),
            "the preview"
        );
        assert!(captured.get("https://x.test/a.iso").is_some(), "and again");
        assert!(
            captured.take("https://x.test/a.iso").is_some(),
            "the download"
        );
        assert!(captured.take("https://x.test/a.iso").is_none(), "once");
        assert!(captured.get("https://x.test/b.iso").is_none());
    }

    #[test]
    fn captured_contexts_are_bounded() {
        let captured = Captured::default();
        for i in 0..=MAX_LINKS {
            let url = format!("https://x.test/{i}");
            captured.remember(url.clone(), RequestContext::new(&url).unwrap());
        }
        assert_eq!(captured.0.lock().unwrap().len(), MAX_LINKS);
        assert!(
            captured.get("https://x.test/0").is_none(),
            "the oldest went"
        );
        assert!(
            captured
                .get(&format!("https://x.test/{MAX_LINKS}"))
                .is_some()
        );
    }
}
