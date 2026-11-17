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
//! A capture that is a download waiting for a new link (docs/ENGINE.md,
//! "Link refresh": the same file name and size as exactly one download in
//! `NeedsNewLink`) is not added: the page offers "Use this link to resume
//! …?", and the request context waits here for the answer. The reply is
//! "accepted" with `confirm` as soon as the offer is made, because the user
//! may take longer than the extension waits; whatever the answer, the app
//! then has the download (resumed with the new link, or added as a new one,
//! also when the new link turns out to be another file), so nothing is lost.
//!
//! A capture from a private window (`incognito`) adds a download marked
//! private, also through the add sheet and a declined resume offer: with
//! "Forget private-window downloads" on, the list forgets it once it
//! completes (docs/BROWSER-INTEGRATION.md, "What is never captured").
//!
//! A page the extension sends for its media (`media_page`) opens the media
//! sheet with it; its cookies, headers and hints wait in memory
//! ([`crate::media::MediaPages`]) for the sheet's probe and download. The
//! reply is "accepted" with `confirm` at once: the user picks in the app.
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
    AddRequest, DEFAULT_QUEUE, Destination, DownloadSummary, Engine, ErrorKind, ProbeResult,
    RequestContext, ResumeOptions, State, redact_url,
};
use boltget_ipc::BrowserReply;
use boltget_ipc::browser::{
    Accepted, AddLinks, Capture, ErrorReason, HelloReply, MAX_LINKS, MediaPage, Message,
    RejectReason, Reply, StatusReply,
};
use boltget_native_host::registration::{self, Flavor, Registration, State as RegState};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::bridge::Bridge;
use crate::bridge::dto::{DownloadId, DownloadRow, DownloadState};
use crate::error::UiError;
use crate::media::MediaPages;
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
/// the browser's request, with whether they came from a private window; and
/// captures waiting for the answer to a resume offer. Never saved; cookies
/// stay in memory only.
#[derive(Debug, Default)]
pub(crate) struct Captured {
    links: Mutex<VecDeque<CapturedLink>>,
    offers: Mutex<Offers>,
}

#[derive(Debug)]
struct CapturedLink {
    url: String,
    ctx: RequestContext,
    /// From a private window.
    private: bool,
    at: Instant,
}

/// Captures waiting for the answer to "Use this link to resume …?".
#[derive(Debug, Default)]
struct Offers {
    next: u32,
    list: Vec<PendingOffer>,
}

#[derive(Debug)]
struct PendingOffer {
    offer: ResumeOffer,
    /// The browser's request for the new link.
    ctx: RequestContext,
    /// The captured file's name, for its folder if it is added after all.
    name: String,
    /// From a private window.
    private: bool,
    at: Instant,
}

/// At most this many resume offers wait at once; the oldest goes first.
const MAX_OFFERS: usize = 16;

/// "Use this link to resume `name` (`percent`% done)?": a capture that is
/// the download `id`, waiting for a new link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResumeOffer {
    /// Names the offer in [`answer_resume_offer`]; the link itself stays in
    /// the backend.
    pub(crate) offer: u32,
    pub(crate) id: DownloadId,
    /// The download's file name.
    pub(crate) name: String,
    /// How much of it is downloaded, 0 to 99.
    pub(crate) percent: u8,
}

impl Captured {
    /// Keeps the request context of a link the browser handed over for the
    /// add sheet; `private` when it came from a private window.
    pub(crate) fn remember(&self, url: String, ctx: RequestContext, private: bool) {
        let mut list = self.links.lock().unwrap_or_else(PoisonError::into_inner);
        list.retain(|l| l.url != url && l.at.elapsed() < KEEP);
        while list.len() >= MAX_LINKS {
            list.pop_front();
        }
        list.push_back(CapturedLink {
            url,
            ctx,
            private,
            at: Instant::now(),
        });
    }

    /// The context for `url`, if the browser handed it over lately.
    pub(crate) fn get(&self, url: &str) -> Option<RequestContext> {
        self.get_link(url).map(|(ctx, _)| ctx)
    }

    /// The context for `url` and whether it came from a private window, if
    /// the browser handed it over lately.
    pub(crate) fn get_link(&self, url: &str) -> Option<(RequestContext, bool)> {
        let list = self.links.lock().unwrap_or_else(PoisonError::into_inner);
        list.iter()
            .find(|l| l.url == url && l.at.elapsed() < KEEP)
            .map(|l| (l.ctx.clone(), l.private))
    }

    /// The context for `url`, once: the download now has it.
    pub(crate) fn take(&self, url: &str) -> Option<RequestContext> {
        let mut list = self.links.lock().unwrap_or_else(PoisonError::into_inner);
        let i = list.iter().position(|l| l.url == url)?;
        list.remove(i)
            .filter(|l| l.at.elapsed() < KEEP)
            .map(|l| l.ctx)
    }

    /// Keeps a capture that is the download `waiting` until the user
    /// answers the offer it returns. A newer capture of the same download
    /// replaces an older one.
    fn offer(
        &self,
        waiting: &Candidate,
        ctx: RequestContext,
        name: &str,
        private: bool,
    ) -> ResumeOffer {
        let mut offers = self.offers.lock().unwrap_or_else(PoisonError::into_inner);
        offers.next = offers.next.wrapping_add(1);
        let offer = ResumeOffer {
            offer: offers.next,
            id: waiting.id.into(),
            name: waiting.name.clone().unwrap_or_else(|| name.to_owned()),
            percent: percent(waiting.downloaded, waiting.total),
        };
        let id = offer.id;
        offers
            .list
            .retain(|p| p.offer.id != id && p.at.elapsed() < KEEP);
        while offers.list.len() >= MAX_OFFERS {
            offers.list.remove(0);
        }
        offers.list.push(PendingOffer {
            offer: offer.clone(),
            ctx,
            name: name.to_owned(),
            private,
            at: Instant::now(),
        });
        offer
    }

    /// The offers waiting for an answer, oldest first.
    pub(crate) fn offers(&self) -> Vec<ResumeOffer> {
        let offers = self.offers.lock().unwrap_or_else(PoisonError::into_inner);
        offers
            .list
            .iter()
            .filter(|p| p.at.elapsed() < KEEP)
            .map(|p| p.offer.clone())
            .collect()
    }

    /// The capture behind `offer`, once.
    fn take_offer(&self, offer: u32) -> Option<PendingOffer> {
        let mut offers = self.offers.lock().unwrap_or_else(PoisonError::into_inner);
        let i = offers.list.iter().position(|p| p.offer.offer == offer)?;
        Some(offers.list.remove(i)).filter(|p| p.at.elapsed() < KEEP)
    }
}

/// Whole percent done, below 100 for a download that has not finished.
fn percent(downloaded: u64, total: Option<u64>) -> u8 {
    match total {
        Some(total) if total > 0 => {
            let p = u128::from(downloaded.min(total)) * 100 / u128::from(total);
            u8::try_from(p).unwrap_or(100).min(99)
        }
        _ => 0,
    }
}

/// A download as [`waiting_match`] sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) id: boltget_engine::DownloadId,
    pub(crate) state: State,
    pub(crate) name: Option<String>,
    pub(crate) total: Option<u64>,
    pub(crate) downloaded: u64,
}

impl From<&DownloadSummary> for Candidate {
    fn from(d: &DownloadSummary) -> Self {
        Self {
            id: d.id,
            state: d.state,
            name: d.file_name.clone(),
            total: d.total,
            downloaded: d.downloaded,
        }
    }
}

/// The one download waiting for a new link that a captured file `name` of
/// `size` bytes is: the same name (ignoring case, and a ` (n)` the browser
/// or Boltget added to avoid a clash) and exactly the same size. None when
/// the size is unknown, or when two or more would match, since only the
/// user could tell them apart.
pub(crate) fn waiting_match<'a>(
    downloads: &'a [Candidate],
    name: &str,
    size: Option<u64>,
) -> Option<&'a Candidate> {
    let size = size?;
    let key = name_key(name);
    let mut found = downloads.iter().filter(|d| {
        d.state == State::NeedsNewLink
            && d.total == Some(size)
            && d.name.as_deref().is_some_and(|n| name_key(n) == key)
    });
    let one = found.next()?;
    found.next().is_none().then_some(one)
}

/// A file name as [`waiting_match`] compares it: lower case, without a
/// ` (n)` before the extension.
fn name_key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let (stem, ext) = match lower.rfind('.') {
        Some(i) if i > 0 => lower.split_at(i),
        _ => (lower.as_str(), ""),
    };
    let stem = match stem.strip_suffix(')').and_then(|s| s.rsplit_once(" (")) {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => stem,
    };
    format!("{stem}{ext}")
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
    /// Shows the page an offer to resume a download with a captured link.
    pub(crate) offer: &'a (dyn Fn(&ResumeOffer) + Sync),
    /// Pages the extension sent for their media, waiting for the media
    /// sheet.
    pub(crate) media_pages: &'a MediaPages,
    /// Brings the window forward and has it open the media sheet for the
    /// pages waiting.
    pub(crate) open_media: &'a (dyn Fn() + Sync),
}

/// Builds the [`Host`] for `app` and runs `f` with it.
async fn with_host<T>(app: &AppHandle, f: impl AsyncFnOnce(&Host<'_>) -> T) -> T {
    let bridge = app.state::<Bridge>();
    let captured = app.state::<Captured>();
    let media_pages = app.state::<MediaPages>();
    let open_sheet = |links: String| shell::request(app, &ShellRequest::Add { links });
    let open_media = || shell::request(app, &ShellRequest::MediaPage);
    let offer = |offer: &ResumeOffer| {
        shell::request(
            app,
            &ShellRequest::ResumeOffer {
                offer: offer.clone(),
            },
        );
    };
    let host = Host {
        bridge: &bridge,
        captured: &captured,
        settings: bridge.settings(),
        downloads: app.path().download_dir().ok(),
        open_sheet: &open_sheet,
        offer: &offer,
        media_pages: &media_pages,
        open_media: &open_media,
    };
    f(&host).await
}

/// The app's answer to a message from the extension.
pub(crate) fn reply(app: AppHandle, engine: Engine, message: Message) -> BrowserReply {
    Box::pin(
        async move { with_host(&app, async |host| handle(host, &engine, message).await).await },
    )
}

pub(crate) async fn handle(host: &Host<'_>, engine: &Engine, message: Message) -> Reply {
    match message {
        // The rules as saved in Settings, Browser: the extension takes them
        // (docs/BROWSER-INTEGRATION.md, "Capture rules").
        Message::Hello(_) => Reply::Hello(HelloReply::new(
            Some(format!("Boltget {}", env!("CARGO_PKG_VERSION"))),
            Some(host.settings.capture.rules()),
        )),
        Message::Capture(c) => capture(host, engine, &c).await,
        Message::AddLinks(a) => add_links(host, &a),
        Message::Status(_) => status(host.bridge).await,
        Message::MediaPage(m) => media_page(host, m),
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
    let ask = host.settings.browser_ask;
    let waiting: Vec<Candidate> = match engine.list().await {
        Ok(list) => list
            .iter()
            .filter(|d| d.state == State::NeedsNewLink)
            .map(Candidate::from)
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list the downloads");
            Vec::new()
        }
    };
    // The app's own first request: always when the download is to start at
    // once, and when a download waits for a new link (the server's name
    // and size find it).
    let probe = if ask && waiting.is_empty() {
        None
    } else {
        match first_request(engine, &ctx).await {
            Ok(probe) => Some(probe),
            // The add sheet tries again and says what went wrong.
            Err(_) if ask => None,
            Err(reply) => return *reply,
        }
    };
    if let Some(p) = &probe {
        let name = c.filename.as_deref().unwrap_or(&p.filename);
        if let Some(d) = waiting_match(&waiting, name, p.size.or(c.size)) {
            let offer = host.captured.offer(d, ctx, name, c.incognito);
            (host.offer)(&offer);
            return Reply::Accepted(Accepted::confirming());
        }
    }
    if ask {
        host.captured.remember(link.clone(), ctx, c.incognito);
        (host.open_sheet)(link);
        return Reply::Accepted(Accepted::confirming());
    }
    let Some(probe) = probe else {
        // Not reached: without "Ask before adding" the request was made.
        return Reply::rejected(RejectReason::Failed, "the link was not checked");
    };
    let name = c.filename.as_deref().unwrap_or(&probe.filename).to_owned();
    match add_new(host, engine, ctx, &name, c.incognito).await {
        Ok(id) => Reply::Accepted(Accepted::added(id.0)),
        Err(e) => Reply::rejected(RejectReason::Failed, e),
    }
}

/// The app's own first request for a captured link, within [`PROBE_BUDGET`];
/// on failure, the reply that leaves the download to the browser.
async fn first_request(engine: &Engine, ctx: &RequestContext) -> Result<ProbeResult, Box<Reply>> {
    match tokio::time::timeout(PROBE_BUDGET, engine.probe(ctx)).await {
        Ok(Ok(probe)) => Ok(probe),
        Ok(Err(e)) => {
            let reason = match e.kind() {
                ErrorKind::LinkExpired | ErrorKind::Permanent | ErrorKind::Throttled => {
                    RejectReason::LinkRefused
                }
                _ => RejectReason::Failed,
            };
            Err(Box::new(Reply::rejected(reason, e.to_string())))
        }
        Err(_) => Err(Box::new(Reply::rejected(
            RejectReason::Failed,
            "the server did not answer in time",
        ))),
    }
}

/// Adds a captured download to the main queue, in its category's folder;
/// `private` when it came from a private window.
async fn add_new(
    host: &Host<'_>,
    engine: &Engine,
    ctx: RequestContext,
    name: &str,
    private: bool,
) -> Result<boltget_engine::DownloadId, String> {
    let dir = settings::folder_for(&host.settings, Some(name), host.downloads.as_deref())
        .ok_or("no download folder")?;
    let url = redact_url(ctx.url());
    let req = AddRequest::new(ctx, Destination::Directory(dir))
        .queue(DEFAULT_QUEUE)
        .private(private);
    let id = engine.enqueue(req).await.map_err(|e| e.to_string())?;
    host.bridge.added(id.into(), url, private);
    Ok(id)
}

/// What became of a captured download after its resume offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OfferOutcome {
    /// The download continues with the new link.
    Resumed,
    /// Added as a new download.
    Added,
    /// The add sheet opened with it ("Ask before adding" is on).
    Sheet,
}

/// The answer to [`answer_resume_offer`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfferAnswer {
    pub(crate) outcome: OfferOutcome,
    /// Why the download could not take the new link (another file, say),
    /// when the user asked to resume and the capture went the usual way
    /// instead.
    pub(crate) refused: Option<UiError>,
}

/// Resumes the download behind `offer` with the captured link (`resume`),
/// or handles the capture like any other: added, or the add sheet with
/// "Ask before adding". A link the download refuses is handled like that
/// too, so the captured download is never lost.
pub(crate) async fn answer(
    host: &Host<'_>,
    engine: &Engine,
    offer: u32,
    resume: bool,
) -> Result<OfferAnswer, UiError> {
    let pending = host
        .captured
        .take_offer(offer)
        .ok_or_else(|| UiError::key("error.offer_gone"))?;
    let mut refused = None;
    if resume {
        let id = pending.offer.id;
        match engine.refresh_link(id.into(), pending.ctx.clone()).await {
            Ok(_) => {
                // The link, state and error changed in the database only.
                if let Ok(list) = engine.list().await
                    && let Some(s) = list.iter().find(|s| DownloadId::from(s.id) == id)
                {
                    host.bridge.upsert(DownloadRow::from(s));
                }
                engine.resume(id.into(), ResumeOptions::default()).await?;
                return Ok(OfferAnswer {
                    outcome: OfferOutcome::Resumed,
                    refused: None,
                });
            }
            Err(e) => {
                tracing::info!(error = %e, "the download refused the captured link");
                refused = Some(UiError::from(&e));
            }
        }
    }
    let outcome = if host.settings.browser_ask {
        let link = pending.ctx.url().as_str().to_owned();
        host.captured
            .remember(link.clone(), pending.ctx, pending.private);
        (host.open_sheet)(link);
        OfferOutcome::Sheet
    } else {
        add_new(host, engine, pending.ctx, &pending.name, pending.private)
            .await
            .map_err(|e| UiError::with_message("error.internal", e))?;
        OfferOutcome::Added
    };
    Ok(OfferAnswer { outcome, refused })
}

/// Resume offers waiting for an answer, oldest first: the page asks when it
/// starts, since an offer can arrive before it listens.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn resume_offers(captured: tauri::State<'_, Captured>) -> Vec<ResumeOffer> {
    captured.offers()
}

/// Answers "Use this link to resume …?": `resume` with the captured link,
/// or not (the capture is added as usual).
#[tauri::command]
#[specta::specta]
pub(crate) async fn answer_resume_offer(
    app: AppHandle,
    offer: u32,
    resume: bool,
) -> Result<OfferAnswer, UiError> {
    let engine = app.state::<Bridge>().engine()?.clone();
    with_host(&app, async |host| {
        answer(host, &engine, offer, resume).await
    })
    .await
}

/// "Download all links": the add sheet opens with them to choose from.
fn add_links(host: &Host<'_>, a: &AddLinks) -> Reply {
    let mut links = Vec::with_capacity(a.links.len());
    for l in &a.links {
        match a.context(&l.url) {
            Ok(ctx) => {
                let link = ctx.url().as_str().to_owned();
                host.captured.remember(link.clone(), ctx, a.incognito);
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

/// "Download video from this page": the media sheet opens with the page, its
/// cookies, headers and hints waiting for it (docs/VIDEO.md, "The
/// extension's media panel: as built"). The user decides in the app, so the
/// answer is at once.
fn media_page(host: &Host<'_>, page: MediaPage) -> Reply {
    host.media_pages.add(page);
    (host.open_media)();
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
        offers: StdMutex<Vec<ResumeOffer>>,
        media_pages: MediaPages,
        media_opened: StdMutex<u32>,
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
                offers: StdMutex::new(Vec::new()),
                media_pages: MediaPages::default(),
                media_opened: StdMutex::new(0),
            }
        }

        async fn send(&self, ask: bool, message: Message) -> Reply {
            let settings = AppSettings {
                browser_ask: ask,
                ..AppSettings::default()
            };
            self.send_with(settings, message).await
        }

        async fn send_with(&self, settings: AppSettings, message: Message) -> Reply {
            let open_sheet = |links: String| self.sheet.lock().unwrap().push(links);
            let offer = |o: &ResumeOffer| self.offers.lock().unwrap().push(o.clone());
            let open_media = || *self.media_opened.lock().unwrap() += 1;
            let host = Host {
                bridge: &self.bridge,
                captured: &self.captured,
                settings,
                downloads: Some(self.downloads.path().to_path_buf()),
                open_sheet: &open_sheet,
                offer: &offer,
                media_pages: &self.media_pages,
                open_media: &open_media,
            };
            handle(&host, &self.engine, message).await
        }

        async fn answer(
            &self,
            ask: bool,
            offer: u32,
            resume: bool,
        ) -> Result<OfferAnswer, UiError> {
            let open_sheet = |links: String| self.sheet.lock().unwrap().push(links);
            let offer_fn = |o: &ResumeOffer| self.offers.lock().unwrap().push(o.clone());
            let open_media = || *self.media_opened.lock().unwrap() += 1;
            let host = Host {
                bridge: &self.bridge,
                captured: &self.captured,
                settings: AppSettings {
                    browser_ask: ask,
                    ..AppSettings::default()
                },
                downloads: Some(self.downloads.path().to_path_buf()),
                open_sheet: &open_sheet,
                offer: &offer_fn,
                media_pages: &self.media_pages,
                open_media: &open_media,
            };
            answer(&host, &self.engine, offer, resume).await
        }

        /// A download of `disc.iso` (4 MiB) whose link expires part-way.
        async fn waiting_download(
            &self,
            srv: &TestServer,
            key: &str,
        ) -> boltget_engine::DownloadId {
            let url = srv.url(&format!(
                "/data?size={SIZE}&throttle=2097152&slow_index=1&slow_bps=131072\
                 &fail_status=403&fail_from=4&key={key}"
            ));
            let req = AddRequest::new(
                RequestContext::new(&url).unwrap(),
                Destination::File(self.downloads.path().join("disc.iso")),
            )
            .connections(4);
            let h = self.engine.add(req);
            let id = h.id();
            let err = tokio::time::timeout(Duration::from_secs(30), h.wait())
                .await
                .expect("the download stops")
                .expect_err("the link expires");
            assert_eq!(err.kind(), ErrorKind::LinkExpired, "{err}");
            id
        }

        async fn wait_done(&self, id: boltget_engine::DownloadId) -> DownloadSummary {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                let list = self.engine.list().await.unwrap();
                let d = list.into_iter().find(|d| d.id == id).expect("listed");
                if d.state == State::Completed {
                    break d;
                }
                assert!(Instant::now() < deadline, "still {}", d.state);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }

    const SIZE: u64 = 4 * 1024 * 1024;

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

    /// A capture from a private window adds a download marked private, also
    /// through the add sheet; one from a usual window does not.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_private_capture_is_marked_private_all_the_way() {
        let srv = TestServer::start().await;
        let f = Fixture::new();
        let mut ids = Vec::new();
        for (seed, incognito) in [(81, true), (82, false)] {
            let mut c = capture_of(srv.url(&format!("/data?size=1000&seed={seed}")));
            c.incognito = incognito;
            let reply = f.send(false, Message::Capture(c)).await;
            let Reply::Accepted(a) = reply else {
                panic!("{reply:?}")
            };
            ids.push((a.download.expect("added"), incognito));
        }
        let list = f.engine.list().await.unwrap();
        for (id, incognito) in ids {
            let s = list.iter().find(|s| s.id.0 == id).expect("listed");
            assert_eq!(s.private, incognito, "{id}");
        }

        // With "Ask before adding", the mark waits with the link for the
        // sheet, as do the links of "Download all links".
        let mut c = capture_of("http://127.0.0.1:9/p.iso".into());
        c.incognito = true;
        f.send(true, Message::Capture(c)).await;
        let (_, private) = f.captured.get_link("http://127.0.0.1:9/p.iso").unwrap();
        assert!(private);
        let mut links = AddLinks::new(vec![Link::new("https://x.test/p.zip")]);
        links.incognito = true;
        f.send(false, Message::AddLinks(links)).await;
        assert!(f.captured.get_link("https://x.test/p.zip").unwrap().1);
        f.send(
            true,
            Message::Capture(capture_of("http://127.0.0.1:9/u.iso".into())),
        )
        .await;
        assert!(!f.captured.get_link("http://127.0.0.1:9/u.iso").unwrap().1);
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

    /// A page from the extension opens the media sheet, its cookies,
    /// headers and hints waiting in memory for the sheet, once.
    #[tokio::test]
    async fn a_media_page_opens_the_media_sheet() {
        let f = Fixture::new();
        let page: MediaPage = serde_json::from_value(serde_json::json!({
            "page_url": "https://v.test/watch?v=1",
            "title": "A clip",
            "media_url": "https://cdn.v.test/el.m3u8",
            "cookies": [{"name": "sid", "value": "s3cret", "domain": "v.test"}],
            "headers": [["User-Agent", "Browser UA"], ["Referer", "https://v.test/watch?v=1"]],
            "hints": [
                {"url": "https://cdn.v.test/el.m3u8", "kind": "hls"},
                {"url": "https://cdn.v.test/a.mp4", "kind": "file", "mime": "video/mp4", "size": 7}
            ]
        }))
        .unwrap();
        let reply = f.send(false, Message::MediaPage(page)).await;
        assert!(
            matches!(&reply, Reply::Accepted(a) if a.confirm && a.download.is_none()),
            "{reply:?}"
        );
        assert_eq!(*f.media_opened.lock().unwrap(), 1);
        assert!(
            f.engine.list().await.unwrap().is_empty(),
            "the user decides"
        );
        let pages = f.media_pages.take_new();
        assert_eq!(pages.len(), 1);
        let p = &pages[0];
        assert_eq!(p.url, "https://v.test/watch?v=1");
        assert_eq!(p.title.as_deref(), Some("A clip"));
        // The element's source first, then the other hints, once each.
        let urls: Vec<&str> = p.hints.iter().map(|h| h.url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://cdn.v.test/el.m3u8", "https://cdn.v.test/a.mp4"]
        );
        assert_eq!(p.hints[1].size, Some(7));
        assert_eq!(f.media_pages.take_new().len(), 0, "opened once");
        // The browser context stays for the probe and the download.
        let kept = f.media_pages.get(p.page).expect("kept");
        assert_eq!(kept.cookie_jar().len(), 1);
        assert_eq!(kept.header("user-agent"), Some("Browser UA"));
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
        assert_eq!(
            h.rules,
            Some(boltget_ipc::browser::CaptureRules::default()),
            "nothing changed in Settings: the defaults"
        );
        // The rules saved in Settings are the ones the extension gets.
        let mut settings = AppSettings::default();
        settings.capture.min_size = 0;
        settings.capture.extensions = vec!["pdf".into()];
        settings.capture.never_sites = vec!["bank.example".into()];
        settings.capture.private_windows = true;
        let hello =
            boltget_ipc::browser::Hello::new("0.3.0", boltget_ipc::browser::Browser::Chrome);
        let Reply::Hello(h) = f.send_with(settings, Message::Hello(hello)).await else {
            panic!("a hello")
        };
        let rules = h.rules.expect("rules");
        assert_eq!(rules.min_size, 0);
        assert_eq!(rules.extensions, ["pdf"]);
        assert_eq!(rules.never_sites, ["bank.example"]);
        assert!(rules.enabled && rules.private_windows);
        assert!(matches!(
            f.send(false, Message::Status(boltget_ipc::browser::Status {})).await,
            Reply::Status(s) if s.active == 0
        ));
        f.engine.shutdown().await;
    }

    #[test]
    fn captured_contexts_wait_for_the_sheet_once() {
        let captured = Captured::default();
        let ctx = RequestContext::new("https://x.test/a.iso").unwrap();
        captured.remember("https://x.test/a.iso".into(), ctx, false);
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
            captured.remember(url.clone(), RequestContext::new(&url).unwrap(), false);
        }
        assert_eq!(captured.links.lock().unwrap().len(), MAX_LINKS);
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

    fn candidate(id: u64, state: State, name: &str, total: Option<u64>) -> Candidate {
        Candidate {
            id: boltget_engine::DownloadId(id),
            state,
            name: Some(name.into()),
            total,
            downloaded: 0,
        }
    }

    #[test]
    fn a_capture_matches_one_download_waiting_for_a_new_link() {
        let waiting = State::NeedsNewLink;
        let list = [
            candidate(1, waiting, "disc.iso", Some(100)),
            candidate(2, waiting, "other.iso", Some(100)),
            candidate(3, State::Paused, "paused.iso", Some(100)),
            candidate(4, State::Failed, "failed.iso", Some(100)),
        ];
        let id = |m: Option<&Candidate>| m.map(|c| c.id.0);
        // The same name and size, also with another case or a ` (n)` the
        // browser or Boltget added.
        assert_eq!(id(waiting_match(&list, "disc.iso", Some(100))), Some(1));
        assert_eq!(id(waiting_match(&list, "DISC.ISO", Some(100))), Some(1));
        assert_eq!(id(waiting_match(&list, "disc (2).iso", Some(100))), Some(1));
        // No match: another name, another size, an unknown size, or a
        // download not waiting for a new link.
        assert_eq!(id(waiting_match(&list, "disc2.iso", Some(100))), None);
        assert_eq!(id(waiting_match(&list, "disc.iso", Some(101))), None);
        assert_eq!(id(waiting_match(&list, "disc.iso", None)), None);
        assert_eq!(id(waiting_match(&list, "paused.iso", Some(100))), None);
        assert_eq!(id(waiting_match(&list, "failed.iso", Some(100))), None);
        assert_eq!(id(waiting_match(&[], "disc.iso", Some(100))), None);
        // Two that match: only the user could tell, so neither.
        let two = [
            candidate(1, waiting, "disc.iso", Some(100)),
            candidate(2, waiting, "disc (1).iso", Some(100)),
        ];
        assert_eq!(id(waiting_match(&two, "disc.iso", Some(100))), None);
        // A download whose size is unknown never matches.
        let unknown = [candidate(1, waiting, "disc.iso", None)];
        assert_eq!(id(waiting_match(&unknown, "disc.iso", Some(100))), None);
    }

    #[test]
    fn names_compare_without_case_and_a_counter() {
        assert_eq!(name_key("Disc (12).ISO"), "disc.iso");
        assert_eq!(name_key("disc (x).iso"), "disc (x).iso");
        assert_eq!(name_key("disc ().iso"), "disc ().iso");
        assert_eq!(name_key("README (1)"), "readme");
        assert_eq!(name_key(".hidden"), ".hidden");
        assert_eq!(percent(0, Some(100)), 0);
        assert_eq!(percent(73, Some(100)), 73);
        assert_eq!(percent(100, Some(100)), 99);
        assert_eq!(percent(5, None), 0);
    }

    #[test]
    fn offers_wait_once_and_a_newer_capture_replaces_one() {
        let captured = Captured::default();
        let d = candidate(7, State::NeedsNewLink, "disc.iso", Some(100));
        let ctx = |u: &str| RequestContext::new(u).unwrap();
        let first = captured.offer(&d, ctx("https://x.test/1"), "disc.iso", false);
        let second = captured.offer(&d, ctx("https://x.test/2"), "disc.iso", false);
        assert_ne!(first.offer, second.offer);
        assert_eq!(captured.offers(), std::slice::from_ref(&second));
        assert!(captured.take_offer(first.offer).is_none(), "replaced");
        let p = captured.take_offer(second.offer).expect("waiting");
        assert_eq!(p.ctx.url().as_str(), "https://x.test/2");
        assert!(captured.take_offer(second.offer).is_none(), "once");
        for i in 0..=MAX_OFFERS as u64 {
            let d = candidate(i, State::NeedsNewLink, "disc.iso", Some(100));
            captured.offer(&d, ctx("https://x.test/a"), "disc.iso", false);
        }
        assert_eq!(captured.offers().len(), MAX_OFFERS);
    }

    /// The browser captures a download waiting for a new link again: the app
    /// offers to resume it, and resuming continues the download with the new
    /// link and the browser's cookies.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_recaptured_download_resumes_with_the_new_link_and_cookies() {
        let srv = TestServer::start().await;
        let f = Fixture::new();
        let id = f.waiting_download(&srv, "w2").await;
        let before = srv.requests().len();
        let fresh = srv.url(&format!("/cookie?need=sid%3Ds3cret&size={SIZE}&key=fresh"));
        let reply = f.send(false, Message::Capture(capture_of(fresh))).await;
        assert!(
            matches!(&reply, Reply::Accepted(a) if a.confirm && a.download.is_none()),
            "{reply:?}"
        );
        let offer = f.offers.lock().unwrap().pop().expect("an offer");
        assert_eq!(offer.id, DownloadId::from(id));
        assert_eq!(offer.name, "disc.iso");
        assert!(offer.percent > 0 && offer.percent < 100, "{offer:?}");
        assert_eq!(f.captured.offers(), std::slice::from_ref(&offer));
        assert_eq!(f.engine.list().await.unwrap().len(), 1, "nothing added");

        let answer = f.answer(false, offer.offer, true).await.unwrap();
        assert_eq!(answer.outcome, OfferOutcome::Resumed);
        assert_eq!(answer.refused, None);
        let done = f.wait_done(id).await;
        assert_eq!(done.downloaded, SIZE);
        assert_eq!(f.engine.list().await.unwrap().len(), 1, "nothing added");
        let reqs = &srv.requests()[before..];
        assert!(reqs.len() >= 2, "{reqs:?}");
        for r in reqs {
            assert_eq!(r.cookie.as_deref(), Some("sid=s3cret"), "{r:?}");
            assert!(r.status < 400, "{r:?}");
        }
        assert_eq!(f.captured.offers().len(), 0);
        assert!(matches!(
            f.answer(false, offer.offer, true).await,
            Err(e) if e.key == "error.offer_gone"
        ));
        f.engine.shutdown().await;
    }

    /// Declining the offer, or a new link that is another file, handles the
    /// capture as usual: added, or the add sheet with "Ask before adding".
    #[tokio::test(flavor = "multi_thread")]
    async fn a_declined_or_refused_offer_adds_the_capture() {
        let srv = TestServer::start().await;
        let f = Fixture::new();
        let id = f.waiting_download(&srv, "w2").await;
        let same = srv.url(&format!("/data?size={SIZE}&key=fresh"));
        // Another file of the same size and name.
        let other = srv.url(&format!("/data?size={SIZE}&seed=2&etag_salt=x"));

        // Declined.
        f.send(false, Message::Capture(capture_of(same.clone())))
            .await;
        let offer = f.offers.lock().unwrap().pop().expect("an offer");
        let answer = f.answer(false, offer.offer, false).await.unwrap();
        assert_eq!(answer.outcome, OfferOutcome::Added);
        assert_eq!(answer.refused, None);
        let list = f.engine.list().await.unwrap();
        assert_eq!(list.len(), 2, "added");
        assert!(
            list.iter()
                .any(|d| d.id == id && d.state == State::NeedsNewLink),
            "still waiting"
        );
        for d in list.iter().filter(|d| d.id != id) {
            f.engine.remove(d.id, true).await.unwrap();
        }

        // Refused: the new link is another file.
        f.send(false, Message::Capture(capture_of(other))).await;
        let offer = f.offers.lock().unwrap().pop().expect("an offer");
        let answer = f.answer(false, offer.offer, true).await.unwrap();
        assert_eq!(answer.outcome, OfferOutcome::Added);
        assert_eq!(
            answer.refused.map(|e| e.key).as_deref(),
            Some("error.refresh_rejected")
        );
        let list = f.engine.list().await.unwrap();
        assert_eq!(list.len(), 2, "added instead");
        for d in list.iter().filter(|d| d.id != id) {
            f.engine.remove(d.id, true).await.unwrap();
        }

        // With "Ask before adding", the offer comes first, and declining
        // opens the add sheet with the link, its context waiting.
        let reply = f
            .send(true, Message::Capture(capture_of(same.clone())))
            .await;
        assert!(
            matches!(&reply, Reply::Accepted(a) if a.confirm),
            "{reply:?}"
        );
        assert!(f.sheet.lock().unwrap().is_empty(), "the offer first");
        let offer = f.offers.lock().unwrap().pop().expect("an offer");
        let answer = f.answer(true, offer.offer, false).await.unwrap();
        assert_eq!(answer.outcome, OfferOutcome::Sheet);
        assert_eq!(*f.sheet.lock().unwrap(), std::slice::from_ref(&same));
        assert!(f.captured.get(&same).is_some());
        assert_eq!(f.engine.list().await.unwrap().len(), 1, "the sheet adds it");
        f.engine.shutdown().await;
    }

    /// Two downloads that would match: no offer, the capture is added.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_waiting_downloads_with_the_name_and_size_get_no_offer() {
        let srv = TestServer::start().await;
        let f = Fixture::new();
        f.waiting_download(&srv, "w2").await;
        f.waiting_download(&srv, "w3").await;
        let names: Vec<_> = f
            .engine
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.file_name)
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        let same = srv.url(&format!("/data?size={SIZE}&key=fresh"));
        let reply = f.send(false, Message::Capture(capture_of(same))).await;
        assert!(
            matches!(&reply, Reply::Accepted(a) if !a.confirm && a.download.is_some()),
            "{reply:?}"
        );
        assert!(f.offers.lock().unwrap().is_empty());
        assert_eq!(f.engine.list().await.unwrap().len(), 3);
        f.engine.shutdown().await;
    }
}
