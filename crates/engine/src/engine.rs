//! The public engine API.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use ::http::{HeaderName, HeaderValue};
use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::{EngineConfig, Settings};
use crate::download::{self, Env, Job, Shared};
use crate::error::{Error, Result};
use crate::events::{Completed, DownloadId, DownloadInfo, EngineEvent, Progress, State};
use crate::filename::{self, NameSources};
use crate::http::{self, Connector, Origin};
use crate::integrity::{Checksum, ChecksumSource};
use crate::probe::{self, ProbeKind, ProbeResult, RangeSupport};
use crate::proxy::ProxyChoice;
use crate::ratelimit::RateLimiter;
use crate::request::{RequestContext, redact_url};
use crate::store::{DownloadSummary, Store, records};

/// Where to save a download.
#[derive(Debug, Clone)]
pub enum Destination {
    /// Save into this directory under the resolved file name.
    Directory(PathBuf),
    /// Save at this path (the file name counts as "typed by the user").
    File(PathBuf),
}

/// What to do when the destination file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// Append ` (1)`, ` (2)`, ... before the extension.
    #[default]
    Rename,
    /// Replace the existing file on completion.
    Overwrite,
    /// Fail with a disk error.
    Fail,
}

/// A request to start a download.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AddRequest {
    /// URL, headers and browser hints.
    pub ctx: RequestContext,
    /// Where to save.
    pub destination: Destination,
    /// Connections to use (defaults to [`EngineConfig::initial_connections`],
    /// capped at [`EngineConfig::max_connections`]).
    pub connections: Option<u32>,
    /// Behaviour when the target exists.
    pub conflict: ConflictPolicy,
    /// This download's bandwidth limit in bytes per second (`None`: only the
    /// global limit applies).
    pub speed_limit: Option<u64>,
    /// The checksum the file must have. Checked after the last byte is on disk;
    /// a mismatch leaves the download `Corrupt` instead of renaming the file.
    pub checksum: Option<Checksum>,
    /// Without [`AddRequest::checksum`], verify against a digest the server
    /// advertises (`Repr-Digest`, `Digest`, `Content-MD5`, ...). On by default.
    pub server_digest: bool,
}

impl AddRequest {
    /// A request with default options.
    pub fn new(ctx: RequestContext, destination: Destination) -> Self {
        Self {
            ctx,
            destination,
            connections: None,
            conflict: ConflictPolicy::Rename,
            speed_limit: None,
            checksum: None,
            server_digest: true,
        }
    }

    /// Sets the connection count.
    #[must_use]
    pub fn connections(mut self, n: u32) -> Self {
        self.connections = Some(n);
        self
    }

    /// Sets the conflict policy.
    #[must_use]
    pub fn conflict(mut self, policy: ConflictPolicy) -> Self {
        self.conflict = policy;
        self
    }

    /// Limits this download to `bytes_per_sec` (`None` or 0: no own limit).
    #[must_use]
    pub fn speed_limit(mut self, bytes_per_sec: Option<u64>) -> Self {
        self.speed_limit = bytes_per_sec.filter(|r| *r > 0);
        self
    }

    /// Sets the checksum the file must have.
    #[must_use]
    pub fn checksum(mut self, checksum: Checksum) -> Self {
        self.checksum = Some(checksum);
        self
    }

    /// Whether to verify against a digest the server advertises (default true).
    #[must_use]
    pub fn server_digest(mut self, verify: bool) -> Self {
        self.server_digest = verify;
        self
    }
}

/// What [`Engine::set_limit`] applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitScope {
    /// All downloads together. Stored in the settings when the engine has a
    /// database, so it is the default for later runs.
    Global,
    /// One download (running or not); stored with it.
    Download(DownloadId),
}

/// Holds one per-host and one global connection slot.
#[derive(Debug)]
pub(crate) struct ConnPermit {
    _host: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

/// A connection count per host learned by earlier downloads, with its expiry.
/// Keyed by origin (`scheme://host:port`).
#[derive(Debug, Clone, Default)]
pub(crate) struct HostCounts(Arc<Mutex<HashMap<String, (u32, SystemTime)>>>);

impl HostCounts {
    /// The count for `origin`, if one is in force.
    pub(crate) fn get(&self, origin: &Origin) -> Option<u32> {
        let map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(&origin.to_string())
            .filter(|(_, until)| *until > SystemTime::now())
            .map(|(n, _)| *n)
    }

    /// Records `n` for `origin` until `until`. Returns the whole (unexpired)
    /// table, to be saved.
    pub(crate) fn set(
        &self,
        origin: &Origin,
        n: u32,
        until: SystemTime,
    ) -> HashMap<String, (u32, SystemTime)> {
        let mut map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = SystemTime::now();
        map.retain(|_, (_, u)| *u > now);
        map.insert(origin.to_string(), (n.max(1), until));
        map.clone()
    }

    /// Loads saved counts (expired ones are dropped).
    pub(crate) fn load(&self, saved: HashMap<String, (u32, SystemTime)>) {
        let now = SystemTime::now();
        let mut map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.extend(saved.into_iter().filter(|(_, (_, u))| *u > now));
    }
}

/// Per-host and global connection caps, shared by all downloads.
#[derive(Debug, Clone)]
pub(crate) struct Limits {
    global: Arc<Semaphore>,
    per_host: Arc<Mutex<HashMap<Origin, Arc<Semaphore>>>>,
    max_per_host: usize,
    /// Connection ceilings learned from hosts that pushed back (`429`/`503`).
    pub(crate) ceilings: HostCounts,
    /// Start counts learned from hosts that limit each connection: the count
    /// that paid off last time.
    pub(crate) starts: HostCounts,
}

impl Limits {
    fn new(max_per_host: u32, max_total: u32) -> Self {
        Self {
            global: Arc::new(Semaphore::new(max_total as usize)),
            per_host: Arc::default(),
            max_per_host: max_per_host as usize,
            ceilings: HostCounts::default(),
            starts: HostCounts::default(),
        }
    }

    fn host(&self, origin: &Origin) -> Arc<Semaphore> {
        let mut map = self
            .per_host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.entry(origin.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(self.max_per_host)))
            .clone()
    }

    /// Waits for a connection slot to `origin`.
    pub(crate) async fn acquire(&self, origin: &Origin) -> ConnPermit {
        let host = self
            .host(origin)
            .acquire_owned()
            .await
            .expect("semaphores are never closed");
        let global = self
            .global
            .clone()
            .acquire_owned()
            .await
            .expect("semaphores are never closed");
        ConnPermit {
            _host: host,
            _global: global,
        }
    }

    /// Slots currently free for `origin`.
    pub(crate) fn available(&self, origin: &Origin) -> usize {
        self.host(origin)
            .available_permits()
            .min(self.global.available_permits())
    }
}

/// Options for [`Engine::resume`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ResumeOptions {
    /// Connections to use (defaults to the count the download was added with).
    pub connections: Option<u32>,
    /// Headers to add or replace. Secrets such as `Authorization` are not stored
    /// in the database, so a download that needed them needs them again here.
    pub headers: Vec<(HeaderName, HeaderValue)>,
    /// Discard the saved progress and download again from the beginning (same
    /// destination). Needed when the file changed on the server.
    pub restart: bool,
    /// Replace the download's own speed limit (`Some(None)`: remove it). `None`
    /// keeps the stored one.
    pub speed_limit: Option<Option<u64>>,
    /// Replace the expected checksum (`Some(None)`: verify nothing, which is how
    /// a `Corrupt` download is kept as it is). `None` keeps the stored one.
    pub checksum: Option<Option<Checksum>>,
    /// Use this proxy setting for this run instead of the saved one (which never
    /// includes proxy credentials).
    pub proxy: Option<ProxyChoice>,
    /// Load cookies from this `cookies.txt` instead of the saved one (and save
    /// its path for later resumes).
    pub cookie_file: Option<PathBuf>,
    /// Extra cookies `(name, value)` for the download URL's host, as
    /// [`RequestContext::cookie`]. Not saved.
    pub cookies: Vec<(String, String)>,
}

impl ResumeOptions {
    /// Overrides the proxy for this run.
    #[must_use]
    pub fn proxy(mut self, proxy: ProxyChoice) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Replaces the download's own speed limit (`None` or 0: remove it).
    #[must_use]
    pub fn speed_limit(mut self, bytes_per_sec: Option<u64>) -> Self {
        self.speed_limit = Some(bytes_per_sec.filter(|r| *r > 0));
        self
    }

    /// Replaces the expected checksum (`None`: do not verify).
    #[must_use]
    pub fn checksum(mut self, checksum: Option<Checksum>) -> Self {
        self.checksum = Some(checksum);
        self
    }

    /// Sets the connection count.
    #[must_use]
    pub fn connections(mut self, n: u32) -> Self {
        self.connections = Some(n);
        self
    }

    /// Adds a request header (allowlisted ones only, as for [`RequestContext::header`]).
    #[must_use]
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.push((name, value));
        self
    }

    /// Starts over from byte 0.
    #[must_use]
    pub fn restart(mut self, restart: bool) -> Self {
        self.restart = restart;
        self
    }
}

/// What [`Engine::remove`] did.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Removed {
    /// The removed download.
    pub id: DownloadId,
    /// Files deleted from disk (with `delete_file`).
    pub deleted: Vec<PathBuf>,
}

#[derive(Debug)]
struct Inner {
    cfg: Arc<EngineConfig>,
    env: Env,
    events: broadcast::Sender<EngineEvent>,
    next_id: AtomicU64,
    downloads: Mutex<HashMap<DownloadId, Arc<Shared>>>,
    root: CancellationToken,
    store: Option<Store>,
}

/// The download engine. Cheap to clone; all clones share state.
///
/// [`Engine::open`] keeps downloads in a database: they get persistent ids,
/// progress is checkpointed, and paused or interrupted downloads continue with
/// [`Engine::resume`]. [`Engine::new`] keeps everything in memory.
#[derive(Debug, Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

fn conflict_name(c: ConflictPolicy) -> &'static str {
    match c {
        ConflictPolicy::Rename => "rename",
        ConflictPolicy::Overwrite => "overwrite",
        ConflictPolicy::Fail => "fail",
    }
}

fn conflict_from(s: &str) -> ConflictPolicy {
    match s {
        "overwrite" => ConflictPolicy::Overwrite,
        "fail" => ConflictPolicy::Fail,
        _ => ConflictPolicy::Rename,
    }
}

/// Makes a destination absolute, so a saved download does not depend on the
/// working directory of the process that resumes it.
fn absolute(dest: Destination) -> Destination {
    let abs = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
    match dest {
        Destination::Directory(d) => Destination::Directory(abs(d)),
        Destination::File(f) => Destination::File(abs(f)),
    }
}

impl Engine {
    /// Creates an in-memory engine: no database, downloads do not survive the
    /// process. Construction does not need a Tokio runtime; adding downloads does.
    pub fn new(config: EngineConfig) -> Result<Self> {
        Self::build(config, None, 1)
    }

    /// Opens the engine with its database in `data_dir` (created if needed):
    /// takes the single-owner lock (`boltget.db.lock`), applies migrations, and
    /// marks downloads that a previous process left running as paused.
    ///
    /// Fails with [`Error::DatabaseLocked`] if another process owns the database.
    pub fn open(config: EngineConfig, data_dir: impl AsRef<Path>) -> Result<Self> {
        let opened = Store::open(data_dir.as_ref())?;
        let mut config = config;
        if opened.settings.speed_limit.is_some() {
            config.speed_limit = opened.settings.speed_limit;
        }
        let engine = Self::build(config, Some(opened.store), opened.next_id)?;
        engine.inner.env.limits.ceilings.load(opened.ceilings);
        engine.inner.env.limits.starts.load(opened.starts);
        Ok(engine)
    }

    /// Lists the downloads in `data_dir` without taking ownership (read-only),
    /// so it works while another process owns the database. The list reflects
    /// that process's last checkpoint.
    pub fn peek(data_dir: impl AsRef<Path>) -> Result<Vec<DownloadSummary>> {
        Store::peek(data_dir.as_ref())
    }

    /// The settings stored in `data_dir`, read without taking ownership
    /// (defaults if there is no database yet).
    pub fn peek_settings(data_dir: impl AsRef<Path>) -> Result<Settings> {
        Store::peek_settings(data_dir.as_ref())
    }

    fn build(config: EngineConfig, store: Option<Store>, next_id: u64) -> Result<Self> {
        let cfg = Arc::new(config.normalized());
        let tls = http::tls_config()?;
        let limits = Limits::new(cfg.max_per_host, cfg.max_total);
        let (events, _) = broadcast::channel(1024);
        Ok(Self {
            inner: Arc::new(Inner {
                env: Env {
                    cfg: cfg.clone(),
                    tls,
                    limits,
                    reserved: Arc::new(Mutex::new(HashSet::new())),
                    global_limit: Arc::new(RateLimiter::new(cfg.speed_limit)),
                },
                cfg,
                events,
                next_id: AtomicU64::new(next_id),
                downloads: Mutex::new(HashMap::new()),
                root: CancellationToken::new(),
                store,
            }),
        })
    }

    /// The effective configuration.
    pub fn config(&self) -> &EngineConfig {
        &self.inner.cfg
    }

    /// The data directory, for an engine with a database.
    pub fn data_dir(&self) -> Option<&Path> {
        self.inner.store.as_ref().map(Store::dir)
    }

    fn store(&self) -> Result<&Store> {
        self.inner
            .store
            .as_ref()
            .ok_or_else(|| Error::Database("this engine has no download database".into()))
    }

    fn downloads(&self) -> std::sync::MutexGuard<'_, HashMap<DownloadId, Arc<Shared>>> {
        self.inner
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn running(&self, id: DownloadId) -> Option<Arc<Shared>> {
        self.downloads().get(&id).cloned()
    }

    /// Probes a URL without downloading it (for an "add" dialog): size, range
    /// support, validators and the resolved file name. Costs one request, or two
    /// when the server answers `200` and range support must be verified.
    pub async fn probe(&self, ctx: &RequestContext) -> Result<ProbeResult> {
        let connector = Connector::new(
            self.inner.env.tls.clone(),
            self.inner.cfg.clone(),
            ctx.proxy().clone(),
        );
        let cancel = self.inner.root.child_token();
        let probed = probe::probe(&connector, ctx, &cancel).await?;
        let size = probed.kind.total();
        let ranges = match probed.kind {
            ProbeKind::Ranged { .. } | ProbeKind::RangedUnknownSize | ProbeKind::Empty => {
                RangeSupport::Yes
            }
            ProbeKind::Full { total } if total >= 2 => {
                let original = Origin::of(ctx.url())?;
                let m = total / 2;
                let ok = probe::verify_ranges(
                    &connector,
                    &probed.final_url,
                    &original,
                    ctx,
                    m,
                    total,
                    &probed.validators,
                )
                .await
                .is_some();
                if ok {
                    RangeSupport::Yes
                } else {
                    RangeSupport::No
                }
            }
            _ => RangeSupport::No,
        };
        let filename = filename::resolve(NameSources {
            user: None,
            content_disposition: probed.content_disposition.as_deref(),
            suggested: ctx.suggested_name(),
            final_url: Some(&probed.final_url),
            content_type: probed.content_type.as_deref(),
        });
        Ok(ProbeResult {
            final_url: probed.final_url,
            redirects: probed.redirects,
            size: match probed.kind {
                ProbeKind::Encoded { .. } => None,
                _ => size,
            },
            ranges,
            validators: probed.validators,
            content_type: probed.content_type,
            filename,
        })
    }

    /// Starts a download. Requires a Tokio runtime. With a database, the download
    /// is recorded before this returns (the write is queued in order ahead of
    /// everything the download does later).
    pub fn add(&self, mut req: AddRequest) -> DownloadHandle {
        req.destination = absolute(req.destination);
        let id = DownloadId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        if let Some(store) = &self.inner.store {
            let (kind, dest) = match &req.destination {
                Destination::Directory(d) => ("dir", d),
                Destination::File(f) => ("file", f),
            };
            let rec = records::NewRecord {
                id,
                url: req.ctx.url().to_string(),
                request_ctx: req.ctx.to_saved(),
                requested_kind: kind,
                requested_dest: dest.to_string_lossy().into_owned(),
                conflict: conflict_name(req.conflict),
                max_connections: req.connections,
                speed_limit: req.speed_limit,
            };
            store.send("new download", move |c| records::insert(c, &rec));
            if let Some(c) = &req.checksum {
                let pair = (c.clone(), ChecksumSource::User);
                store.send("checksum", move |conn| {
                    records::set_checksum(conn, id, Some(&pair))
                });
            }
        }
        let shared = self.new_shared(id, req.speed_limit);
        self.downloads().insert(id, shared.clone());
        let _ = self.inner.events.send(EngineEvent::Added {
            id,
            url: redact_url(req.ctx.url()),
        });
        self.spawn(shared, Job::new(req))
    }

    fn new_shared(&self, id: DownloadId, speed_limit: Option<u64>) -> Arc<Shared> {
        Arc::new(Shared::new(
            id,
            self.inner.root.child_token(),
            self.inner.events.clone(),
            self.inner.store.clone(),
            speed_limit,
        ))
    }

    fn spawn(&self, shared: Arc<Shared>, job: Job) -> DownloadHandle {
        let env = self.inner.env.clone();
        let task_shared = shared.clone();
        let inner = self.inner.clone();
        let join = tokio::spawn(async move {
            // Unregisters the download and signals `done` even if the task panics.
            let _finish = Finish {
                inner,
                id: task_shared.id,
                done: task_shared.done.clone(),
            };
            download::run(task_shared, env, job).await
        });
        DownloadHandle { shared, join }
    }

    /// Continues a paused, interrupted, failed or cancelled download from its
    /// last checkpoint (the resume procedure in `docs/ENGINE.md`): the partial
    /// file is checked against the checkpoint, every request carries the saved
    /// validators, and a changed file fails with [`Error::ResourceChanged`]
    /// instead of mixing two versions. Downloads the server cannot resume start
    /// over. Requires a Tokio runtime.
    pub async fn resume(&self, id: DownloadId, opts: ResumeOptions) -> Result<DownloadHandle> {
        let store = self.store()?.clone();
        if self.running(id).is_some() {
            return Err(Error::InvalidState {
                id,
                reason: "is already running".into(),
            });
        }
        let saved = store
            .call(move |c| records::load(c, id))
            .await?
            .ok_or(Error::UnknownDownload(id))?;
        if saved.state == State::Completed {
            return Err(Error::InvalidState {
                id,
                reason: "is already completed".into(),
            });
        }
        let mut ctx = RequestContext::from_saved(&saved.url, &saved.request_ctx)?;
        for (name, value) in opts.headers {
            ctx.header(name, value);
        }
        if let Some(p) = opts.proxy {
            ctx.set_proxy(p);
        }
        if let Some(file) = opts.cookie_file {
            ctx = ctx.with_cookie_file(file)?;
            let saved_ctx = ctx.to_saved();
            store
                .call(move |c| records::set_request_ctx(c, id, &saved_ctx))
                .await?;
        }
        for (name, value) in &opts.cookies {
            ctx.cookie(name, value);
        }
        let dest = PathBuf::from(&saved.requested_dest);
        let destination = if saved.requested_kind == "file" {
            Destination::File(dest)
        } else {
            Destination::Directory(dest)
        };
        let mut req = AddRequest::new(ctx, destination).conflict(conflict_from(&saved.conflict));
        req.connections = opts.connections.or(saved.max_connections);
        let speed_limit = match opts.speed_limit {
            Some(new) => {
                store
                    .call(move |c| records::set_speed_limit(c, id, new))
                    .await?;
                new
            }
            None => saved.speed_limit,
        };
        req.speed_limit = speed_limit;
        let mut saved = saved;
        if let Some(new) = opts.checksum {
            let pair = new.map(|c| (c, ChecksumSource::User));
            let stored = pair.clone();
            store
                .call(move |c| records::set_checksum(c, id, stored.as_ref()))
                .await?;
            if pair.is_none() {
                // "Verify nothing" also covers a digest the server advertises.
                req.server_digest = false;
            }
            saved.checksum = pair;
        }
        // If the download has to start over, the probe finds a server digest
        // again; a checksum the user gave travels with the request.
        req.checksum = saved
            .checksum
            .as_ref()
            .filter(|(_, s)| *s == ChecksumSource::User)
            .map(|(c, _)| c.clone());
        let job = if opts.restart {
            store.call(move |c| records::reset_progress(c, id)).await?;
            Job {
                req,
                fixed_path: saved.path(),
                resume: None,
            }
        } else {
            Job {
                req,
                fixed_path: None,
                resume: Some(Box::new(saved)),
            }
        };
        let shared = {
            let mut map = self.downloads();
            if map.contains_key(&id) {
                return Err(Error::InvalidState {
                    id,
                    reason: "is already running".into(),
                });
            }
            let shared = self.new_shared(id, speed_limit);
            map.insert(id, shared.clone());
            shared
        };
        Ok(self.spawn(shared, job))
    }

    /// Gives a stopped download a new link (`docs/ENGINE.md`, "Link refresh"),
    /// typically one waiting in [`State::NeedsNewLink`]. Before anything from the
    /// new link is written, its size must match and either its strong `ETag`
    /// must equal the old one or sample ranges must equal the bytes on disk;
    /// otherwise this fails with [`Error::RefreshRejected`] and nothing changes.
    ///
    /// On success the new link (and `ctx`'s headers, without secrets) replaces
    /// the old one for good: re-resolution starts there, and the old link stays
    /// in the download's history. Continue the download with [`Engine::resume`],
    /// passing secret headers such as `Authorization` again.
    pub async fn refresh_link(
        &self,
        id: DownloadId,
        ctx: RequestContext,
    ) -> Result<crate::refresh::RefreshOutcome> {
        let store = self.store()?.clone();
        if self.running(id).is_some() {
            return Err(Error::InvalidState {
                id,
                reason: "is running; pause it before giving it a new link".into(),
            });
        }
        let saved = store
            .call(move |c| records::load(c, id))
            .await?
            .ok_or(Error::UnknownDownload(id))?;
        if saved.state == State::Completed {
            return Err(Error::InvalidState {
                id,
                reason: "is already completed".into(),
            });
        }
        let mut ctx = ctx;
        if *ctx.proxy() == ProxyChoice::Default {
            // No proxy choice with the new link: keep the download's.
            let old = RequestContext::from_saved(&saved.url, &saved.request_ctx)?;
            ctx.set_proxy(old.proxy().clone());
        }
        if ctx.cookie_file().is_none()
            && let Ok(old) = RequestContext::from_saved(&saved.url, &saved.request_ctx)
            && let Some(file) = old.cookie_file()
        {
            // No cookies with the new link: keep the download's cookie file.
            match ctx.clone().with_cookie_file(file) {
                Ok(c) => ctx = c,
                Err(e) => tracing::warn!(error = %e, "could not read the saved cookie file"),
            }
        }
        let connector = Connector::new(
            self.inner.env.tls.clone(),
            self.inner.cfg.clone(),
            ctx.proxy().clone(),
        );
        let cancel = self.inner.root.child_token();
        let partial = saved.temp_path.as_deref().map(Path::new);
        let target = crate::refresh::Target {
            total: saved.total_size,
            validators: &saved.validators,
            segments: &saved.segments,
            partial,
        };
        let accepted = crate::refresh::validate(&connector, &ctx, &target, &cancel).await?;
        tracing::info!(
            id = %id,
            evidence = ?accepted.evidence,
            url = %redact_url(ctx.url()),
            "new link accepted"
        );
        let refreshed = records::Refreshed {
            url: ctx.url().to_string(),
            request_ctx: ctx.to_saved(),
            final_url: accepted.final_url.to_string(),
            validators: accepted.validators,
            mime: accepted.content_type,
        };
        store
            .call(move |c| records::apply_refresh(c, id, &refreshed))
            .await?;
        Ok(crate::refresh::RefreshOutcome {
            id,
            evidence: accepted.evidence,
            final_url: redact_url(&accepted.final_url),
        })
    }

    /// The links a download has used, oldest first, with their role
    /// (`primary`: as added, `refreshed`: given later). Redacted.
    pub async fn sources(&self, id: DownloadId) -> Result<Vec<(String, String)>> {
        let store = self.store()?;
        let rows = store.call(move |c| records::sources(c, id)).await?;
        Ok(rows
            .into_iter()
            .map(|(u, role)| {
                let shown = url::Url::parse(&u).map_or_else(|_| u.clone(), |u| redact_url(&u));
                (shown, role)
            })
            .collect())
    }

    /// Asks a running download to pause without waiting. Returns false if it is
    /// not running in this engine.
    pub fn request_pause(&self, id: DownloadId) -> bool {
        self.running(id).is_some_and(|s| {
            s.request_pause();
            true
        })
    }

    /// Pauses a download: a running one stops, flushes and writes a final
    /// checkpoint before this returns; a queued one is marked paused. Returns the
    /// resulting state (`Paused`, or the state a download that was not running is
    /// in, such as `Failed`). A completed download is an [`Error::InvalidState`].
    pub async fn pause(&self, id: DownloadId) -> Result<State> {
        if let Some(shared) = self.running(id) {
            shared.request_pause();
            shared.done.cancelled().await;
            return Ok(shared.state());
        }
        let Some(store) = &self.inner.store else {
            return Err(Error::UnknownDownload(id));
        };
        store
            .call(move |c| {
                let s = records::get_summary(c, id)?.ok_or(Error::UnknownDownload(id))?;
                match s.state {
                    State::Completed => Err(Error::InvalidState {
                        id,
                        reason: "is already completed".into(),
                    }),
                    // Queued, or left "running" by a process that is gone.
                    State::Queued
                    | State::Probing
                    | State::Downloading
                    | State::Retrying
                    | State::Verifying => {
                        records::set_state(c, id, State::Paused, None)?;
                        Ok(State::Paused)
                    }
                    other => Ok(other),
                }
            })
            .await
    }

    /// Removes a download from the list (pausing it first if it runs). With
    /// `delete_file`, also deletes its partial file and, if it completed, the
    /// downloaded file.
    pub async fn remove(&self, id: DownloadId, delete_file: bool) -> Result<Removed> {
        if let Some(shared) = self.running(id) {
            shared.request_pause();
            shared.done.cancelled().await;
        }
        let store = self.store().map_err(|_| Error::UnknownDownload(id))?;
        let summary = store
            .call(move |c| {
                let s = records::get_summary(c, id)?.ok_or(Error::UnknownDownload(id))?;
                records::delete(c, id)?;
                Ok(s)
            })
            .await?;
        let mut deleted = Vec::new();
        if delete_file {
            let mut paths: Vec<PathBuf> = summary.partial_path.into_iter().collect();
            if summary.state == State::Completed {
                paths.extend(summary.path);
            }
            for p in paths {
                match std::fs::remove_file(&p) {
                    Ok(()) => deleted.push(p),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(Error::disk(&p, e)),
                }
            }
        }
        Ok(Removed { id, deleted })
    }

    /// All downloads in the database, oldest first (empty without a database).
    pub async fn list(&self) -> Result<Vec<DownloadSummary>> {
        match &self.inner.store {
            Some(store) => store.call(|c| records::list(c)).await,
            None => Ok(Vec::new()),
        }
    }

    /// Changes a bandwidth limit (`None` or 0: unlimited). It takes effect within
    /// one refill step for running downloads, without reconnecting. With a
    /// database, the global limit is stored in the settings (the default for
    /// later runs) and a download's limit is stored with the download.
    pub async fn set_limit(&self, scope: LimitScope, bytes_per_sec: Option<u64>) -> Result<()> {
        let bps = bytes_per_sec.filter(|r| *r > 0);
        match scope {
            LimitScope::Global => {
                self.inner.env.global_limit.set_rate(bps);
                if let Some(store) = &self.inner.store {
                    let value = bps.map(serde_json::Value::from);
                    store
                        .call(move |c| {
                            records::put_setting(c, records::SETTING_SPEED_LIMIT, value.as_ref())
                        })
                        .await?;
                }
            }
            LimitScope::Download(id) => {
                let running = self.running(id);
                if let Some(store) = &self.inner.store {
                    store
                        .call(move |c| {
                            if records::get_summary(c, id)?.is_none() {
                                return Err(Error::UnknownDownload(id));
                            }
                            records::set_speed_limit(c, id, bps)
                        })
                        .await?;
                } else if running.is_none() {
                    return Err(Error::UnknownDownload(id));
                }
                if let Some(s) = running {
                    s.limiter.set_rate(bps);
                }
            }
        }
        Ok(())
    }

    /// The current limit for `scope` in bytes per second (`None`: unlimited, or
    /// a download that is not running).
    pub fn limit(&self, scope: LimitScope) -> Option<u64> {
        match scope {
            LimitScope::Global => self.inner.env.global_limit.rate(),
            LimitScope::Download(id) => self.running(id).and_then(|s| s.limiter.rate()),
        }
    }

    /// The stored settings (defaults without a database).
    pub async fn settings(&self) -> Result<Settings> {
        match &self.inner.store {
            Some(store) => store.call(|c| records::load_settings(c)).await,
            None => Ok(Settings::default()),
        }
    }

    /// Subscribes to engine events. Slow subscribers miss events (`Lagged`)
    /// rather than slowing downloads down; use [`Engine::progress`] for state.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// Progress snapshots for the given running downloads (unknown ids are skipped).
    pub fn progress(&self, ids: &[DownloadId]) -> Vec<Progress> {
        let map = self.downloads();
        ids.iter()
            .filter_map(|id| map.get(id).map(|s| s.progress()))
            .collect()
    }

    /// Cancels a running download (state `Cancelled`; the partial file and, with
    /// a database, the checkpoint are kept). Returns false if it is not running.
    pub fn cancel(&self, id: DownloadId) -> bool {
        self.running(id).is_some_and(|s| {
            s.cancel.cancel();
            true
        })
    }

    /// Pauses every running download (each flushes and checkpoints), then closes
    /// the database and releases the owner lock. Downloads added afterwards stop
    /// immediately.
    pub async fn shutdown(&self) {
        let running: Vec<Arc<Shared>> = self.downloads().values().cloned().collect();
        for s in &running {
            s.request_pause();
        }
        self.inner.root.cancel();
        for s in &running {
            s.done.cancelled().await;
        }
        if let Some(store) = &self.inner.store {
            store.close().await;
        }
    }
}

/// Runs when a download task ends, however it ends.
struct Finish {
    inner: Arc<Inner>,
    id: DownloadId,
    done: CancellationToken,
}

impl Drop for Finish {
    fn drop(&mut self) {
        self.inner
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id);
        self.done.cancel();
    }
}

/// A running download.
#[derive(Debug)]
pub struct DownloadHandle {
    shared: Arc<Shared>,
    join: JoinHandle<Result<Completed>>,
}

impl DownloadHandle {
    /// The download id.
    pub fn id(&self) -> DownloadId {
        self.shared.id
    }

    /// The current state.
    pub fn state(&self) -> State {
        self.shared.state()
    }

    /// A progress snapshot.
    pub fn progress(&self) -> Progress {
        self.shared.progress()
    }

    /// Destination details, once the probe succeeded (or the checkpoint loaded).
    pub fn info(&self) -> Option<&DownloadInfo> {
        self.shared.info()
    }

    /// Requests cancellation. The partial file is flushed and kept.
    pub fn cancel(&self) {
        self.shared.cancel.cancel();
    }

    /// Requests a pause: like [`DownloadHandle::cancel`], but the download ends
    /// in `Paused` and can be resumed. [`DownloadHandle::wait`] then returns
    /// [`Error::Cancelled`].
    pub fn pause(&self) {
        self.shared.request_pause();
    }

    /// A token that cancels this download when cancelled.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.shared.cancel.clone()
    }

    /// Waits for the download to finish (completed, failed, paused or cancelled).
    pub async fn wait(self) -> Result<Completed> {
        match self.join.await {
            Ok(r) => r,
            Err(e) if e.is_panic() => Err(Error::Internal(format!("download task panicked: {e}"))),
            Err(_) => Err(Error::Cancelled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn per_host_and_global_caps() {
        let limits = Limits::new(2, 3);
        let a = Origin::of(&url::Url::parse("http://a.test/").unwrap()).unwrap();
        let b = Origin::of(&url::Url::parse("http://b.test/").unwrap()).unwrap();
        let p1 = limits.acquire(&a).await;
        let _p2 = limits.acquire(&a).await;
        assert_eq!(limits.available(&a), 0);
        // Host A is full.
        assert!(
            tokio::time::timeout(Duration::from_millis(50), limits.acquire(&a))
                .await
                .is_err()
        );
        // Host B has room, but only one global slot is left.
        let _p3 = limits.acquire(&b).await;
        assert_eq!(limits.available(&b), 0);
        drop(p1);
        assert_eq!(limits.available(&a), 1);
        let _p4 = tokio::time::timeout(Duration::from_millis(50), limits.acquire(&a))
            .await
            .expect("slot freed");
    }
}
