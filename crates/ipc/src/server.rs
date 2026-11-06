//! The owner's side: accepts clients and runs their requests on the engine.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use boltget_engine::{
    AddRequest, Checksum, ConflictPolicy, Destination, DownloadId, DownloadSummary, Engine,
    EngineEvent, HeaderName, HeaderValue, LimitScope, Proxy, ProxyChoice, RequestContext,
    ResumeOptions, State,
};
use tokio::sync::{broadcast, watch};

use crate::frame::{read_message, write_message};
use crate::proto::{INVALID_REQUEST_KEY, NOT_OWNER_KEY, VERSION_ERROR_KEY};
use crate::transport::{BoxStream, Listener};
use crate::{
    AddSpec, Endpoint, Error, Finished, OLDEST_SUPPORTED, PROTOCOL_VERSION, ProgressReport,
    RefreshSpec, Request, RequestSpec, Response, ResumeSpec, WireError,
};

/// How long a client may take to say hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a hello waits for an engine that is still opening.
const STARTING_TIMEOUT: Duration = Duration::from_secs(8);
/// How often [`Request::Watch`] reports progress.
const WATCH_TICK: Duration = Duration::from_millis(500);

/// What the owner learns about changes clients make that the engine does
/// not announce as events, so its own view (the app's list) stays current.
/// Everything has a default that does nothing.
pub trait Owner: Send + Sync + 'static {
    /// A download changed in the database without an engine event: paused
    /// while not running, given a new link, or given a new limit.
    fn changed(&self, summary: &DownloadSummary) {
        let _ = summary;
    }

    /// A download was removed.
    fn removed(&self, id: DownloadId) {
        let _ = id;
    }

    /// The global speed limit changed.
    fn limit_changed(&self, bytes_per_second: Option<u64>) {
        let _ = bytes_per_second;
    }
}

/// An owner that needs to hear nothing.
impl Owner for () {}

/// Whether the serving process owns the engine right now. A process may
/// serve its endpoint before its engine is open (so a client never finds
/// the database locked and nobody to ask), and while another process holds
/// the database (a standalone `boltget`), when it owns nothing.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Ownership {
    /// The engine is opening: a hello waits for it.
    Starting,
    /// The engine is open.
    Owner(Engine),
    /// Another process owns the database: clients are told so and should
    /// open it themselves (and find it locked, or read it).
    NotOwner,
}

/// Serves one endpoint.
#[derive(Debug)]
pub struct Server {
    listener: Listener,
    endpoint: Endpoint,
}

impl Server {
    /// Starts listening. Fails with [`Error::InUse`] while another process
    /// serves the endpoint.
    pub async fn bind(endpoint: Endpoint) -> Result<Self, Error> {
        let listener = Listener::bind(&endpoint).await?;
        Ok(Self { listener, endpoint })
    }

    /// The endpoint served.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Accepts clients until the task running it is dropped, each served on
    /// a task of its own, with the engine `ownership` holds when it says
    /// hello. `name` identifies the owner in its hello (`Boltget 0.2.0`).
    pub async fn serve(
        mut self,
        ownership: watch::Receiver<Ownership>,
        owner: Arc<dyn Owner>,
        name: String,
    ) {
        let shared = Arc::new(Shared { owner, name });
        loop {
            match self.listener.accept().await {
                Ok(io) => {
                    tokio::spawn(connection(io, shared.clone(), ownership.clone()));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "the pipe stopped accepting a client");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

struct Shared {
    owner: Arc<dyn Owner>,
    name: String,
}

struct Ctx {
    engine: Engine,
    owner: Arc<dyn Owner>,
}

/// The engine once it is open; `None` if this process does not own it.
async fn engine(ownership: &mut watch::Receiver<Ownership>) -> Option<Engine> {
    let ready = ownership.wait_for(|o| !matches!(o, Ownership::Starting));
    match tokio::time::timeout(STARTING_TIMEOUT, ready).await {
        Ok(Ok(o)) => match &*o {
            Ownership::Owner(engine) => Some(engine.clone()),
            _ => None,
        },
        _ => None,
    }
}

async fn send(io: &mut BoxStream, msg: &Response) -> Result<(), Error> {
    write_message(io, msg).await
}

fn error(e: impl Into<WireError>) -> WireError {
    e.into()
}

fn invalid(message: impl Into<String>) -> WireError {
    WireError::protocol(INVALID_REQUEST_KEY, message)
}

fn refuse(error: WireError) -> Response {
    Response::Error { error }
}

async fn connection(
    mut io: BoxStream,
    shared: Arc<Shared>,
    mut ownership: watch::Receiver<Ownership>,
) {
    let engine = match tokio::time::timeout(HELLO_TIMEOUT, read_message::<Request, _>(&mut io))
        .await
    {
        Ok(Ok(Some(Request::Hello { v, client }))) => {
            if !(OLDEST_SUPPORTED..=PROTOCOL_VERSION).contains(&v) {
                let msg = format!(
                    "{} speaks pipe protocol {}, {client} speaks {v}: use the same release of both",
                    shared.name,
                    if OLDEST_SUPPORTED == PROTOCOL_VERSION {
                        PROTOCOL_VERSION.to_string()
                    } else {
                        format!("{OLDEST_SUPPORTED} to {PROTOCOL_VERSION}")
                    },
                );
                let _ = send(
                    &mut io,
                    &refuse(WireError::protocol(VERSION_ERROR_KEY, msg)),
                )
                .await;
                return;
            }
            let Some(engine) = engine(&mut ownership).await else {
                let msg = format!("{} does not own the downloads right now", shared.name);
                let _ = send(&mut io, &refuse(WireError::protocol(NOT_OWNER_KEY, msg))).await;
                return;
            };
            tracing::debug!(client, v, "pipe client connected");
            let hello = Response::Hello {
                v,
                server: shared.name.clone(),
            };
            if send(&mut io, &hello).await.is_err() {
                return;
            }
            engine
        }
        Ok(Ok(Some(_))) => {
            let _ = send(&mut io, &refuse(invalid("the first message must be hello"))).await;
            return;
        }
        Ok(Err(e)) => {
            let _ = send(&mut io, &refuse(invalid(e.to_string()))).await;
            return;
        }
        Ok(Ok(None)) | Err(_) => return,
    };
    let ctx = Ctx {
        engine,
        owner: shared.owner.clone(),
    };
    loop {
        let req = match read_message::<Request, _>(&mut io).await {
            Ok(Some(req)) => req,
            Ok(None) => return,
            Err(e) => {
                // A message we cannot read leaves the stream in an unknown
                // place: answer once, then hang up.
                let _ = send(&mut io, &refuse(invalid(e.to_string()))).await;
                return;
            }
        };
        let sent = match req {
            Request::Watch { id } => watch(&mut io, &ctx, DownloadId(id)).await,
            req => {
                let resp = ctx.handle(req).await;
                send(&mut io, &resp).await
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

impl Ctx {
    async fn handle(&self, req: Request) -> Response {
        let result = match req {
            Request::List => self.list().await,
            Request::Add(spec) => self.add(spec).await,
            Request::Pause { id } => self.pause(DownloadId(id)).await,
            Request::Resume(spec) => self.resume(spec).await,
            Request::Remove { id, delete_file } => self.remove(DownloadId(id), delete_file).await,
            Request::Refresh(spec) => self.refresh(spec).await,
            Request::Settings => self.settings().await,
            Request::SetLimit { id, bps } => self.set_limit(id.map(DownloadId), bps).await,
            Request::Hello { .. } => Err(invalid("hello was already said")),
            // Handled by the connection, which streams it.
            Request::Watch { .. } => Err(invalid("watch cannot be nested")),
        };
        result.unwrap_or_else(refuse)
    }

    async fn summary(&self, id: DownloadId) -> Result<DownloadSummary, WireError> {
        self.engine
            .list()
            .await
            .map_err(error)?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| error(boltget_engine::Error::UnknownDownload(id)))
    }

    /// Tells the owner a download changed, from the database's view of it.
    async fn changed(&self, id: DownloadId) {
        if let Ok(s) = self.summary(id).await {
            self.owner.changed(&s);
        }
    }

    async fn list(&self) -> Result<Response, WireError> {
        let list = self.engine.list().await.map_err(error)?;
        let ids: Vec<DownloadId> = list
            .iter()
            .filter(|s| s.state.is_active())
            .map(|s| s.id)
            .collect();
        let live = self.engine.progress(&ids);
        let downloads = list
            .iter()
            .map(|s| {
                let mut v = serde_json::to_value(s).unwrap_or_default();
                if let Some(p) = live.iter().find(|p| p.id == s.id) {
                    v["downloaded"] = p.downloaded.into();
                    if p.total.is_some() {
                        v["total"] = p.total.into();
                    }
                }
                v
            })
            .collect();
        Ok(Response::Downloads { downloads })
    }

    async fn add(&self, spec: AddSpec) -> Result<Response, WireError> {
        let ctx = context(&spec.url, &spec.request)?;
        if !spec.destination.is_absolute() {
            return Err(invalid("the destination must be an absolute path"));
        }
        let destination = if spec.to_file {
            Destination::File(spec.destination)
        } else {
            Destination::Directory(spec.destination)
        };
        let mut req = AddRequest::new(ctx, destination)
            .speed_limit(spec.limit)
            .server_digest(spec.server_digest);
        if let Some(n) = spec.connections {
            if !(1..=32).contains(&n) {
                return Err(invalid("connections must be 1 to 32"));
            }
            req = req.connections(n);
        }
        if let Some(c) = &spec.checksum {
            req = req.checksum(checksum(c)?);
        }
        if spec.overwrite {
            req = req.conflict(ConflictPolicy::Overwrite);
        }
        let queued = match &spec.queue {
            Some(name) => {
                req = req.queue(self.engine.queue_named(name).await.map_err(error)?.id);
                true
            }
            None => false,
        };
        // Without a queue it starts now, as `boltget get` always did; with
        // one it waits for its turn there.
        let id = if spec.paused {
            self.engine.add_paused(req).map_err(error)?
        } else if queued {
            self.engine.enqueue(req).await.map_err(error)?
        } else {
            self.engine.add(req).id()
        };
        Ok(Response::Added { id: id.0 })
    }

    async fn pause(&self, id: DownloadId) -> Result<Response, WireError> {
        let state = self.engine.pause(id).await.map_err(error)?;
        // A download that was not running changes without an event.
        self.changed(id).await;
        Ok(Response::Paused {
            id: id.0,
            state: state.as_str().into(),
        })
    }

    async fn resume(&self, spec: ResumeSpec) -> Result<Response, WireError> {
        let id = DownloadId(spec.id);
        let mut opts = ResumeOptions::default().restart(spec.restart);
        opts.connections = spec.connections;
        let r = &spec.request;
        if let Some(p) = &r.proxy {
            opts.proxy = Some(proxy(p)?);
        }
        opts.cookie_file.clone_from(&r.cookie_file);
        opts.cookies.clone_from(&r.cookies);
        for (name, value) in headers(&r.headers)? {
            opts = opts.header(name, value);
        }
        if let Some(limit) = spec.limit {
            opts = opts.speed_limit(Some(limit));
        }
        if spec.no_checksum {
            opts = opts.checksum(None);
        } else if let Some(c) = &spec.checksum {
            opts = opts.checksum(Some(checksum(c)?));
        }
        self.engine.resume(id, opts).await.map_err(error)?;
        Ok(Response::Ok)
    }

    async fn remove(&self, id: DownloadId, delete_file: bool) -> Result<Response, WireError> {
        let removed = self.engine.remove(id, delete_file).await.map_err(error)?;
        self.owner.removed(id);
        Ok(Response::Removed {
            id: id.0,
            deleted: removed.deleted,
        })
    }

    async fn refresh(&self, spec: RefreshSpec) -> Result<Response, WireError> {
        let id = DownloadId(spec.id);
        let ctx = context(&spec.url, &spec.request)?;
        let outcome = self.engine.refresh_link(id, ctx).await.map_err(error)?;
        self.changed(id).await;
        Ok(Response::Refreshed {
            outcome: serde_json::to_value(&outcome).unwrap_or_default(),
        })
    }

    async fn settings(&self) -> Result<Response, WireError> {
        let s = self.engine.settings().await.map_err(error)?;
        Ok(Response::Settings {
            limit: s.speed_limit,
        })
    }

    async fn set_limit(
        &self,
        id: Option<DownloadId>,
        bps: Option<u64>,
    ) -> Result<Response, WireError> {
        let bps = bps.filter(|&b| b > 0);
        if let Some(id) = id {
            self.engine
                .set_limit(LimitScope::Download(id), bps)
                .await
                .map_err(error)?;
            self.changed(id).await;
        } else {
            self.engine
                .set_limit(LimitScope::Global, bps)
                .await
                .map_err(error)?;
            self.owner.limit_changed(bps);
        }
        self.settings().await
    }
}

fn checksum(s: &str) -> Result<Checksum, WireError> {
    Checksum::parse(s).map_err(|e| invalid(format!("checksum: {e}")))
}

fn proxy(s: &str) -> Result<ProxyChoice, WireError> {
    if s == "direct" {
        return Ok(ProxyChoice::Direct);
    }
    Proxy::parse(s)
        .map(ProxyChoice::Via)
        .map_err(|e| invalid(format!("proxy: {e}")))
}

fn headers(list: &[(String, String)]) -> Result<Vec<(HeaderName, HeaderValue)>, WireError> {
    list.iter()
        .map(|(name, value)| {
            let n: HeaderName = name
                .trim()
                .parse()
                .map_err(|_| invalid(format!("bad header name {name:?}")))?;
            let v: HeaderValue = value
                .trim()
                .parse()
                .map_err(|_| invalid(format!("bad value for header {name}")))?;
            Ok((n, v))
        })
        .collect()
}

/// The request context for a new link, as the CLI builds it.
fn context(url: &str, spec: &RequestSpec) -> Result<RequestContext, WireError> {
    let mut ctx = RequestContext::new(url).map_err(error)?;
    if let Some(p) = &spec.proxy {
        ctx.set_proxy(proxy(p)?);
    }
    for (name, value) in headers(&spec.headers)? {
        if !ctx.header(name.clone(), value) {
            return Err(invalid(format!(
                "header {name} is not allowed (the engine controls it)"
            )));
        }
    }
    if let Some(f) = &spec.cookie_file {
        ctx = ctx.with_cookie_file(Path::new(f)).map_err(error)?;
    }
    for (name, value) in &spec.cookies {
        ctx.cookie(name, value);
    }
    Ok(ctx)
}

fn event_id(event: &EngineEvent) -> Option<DownloadId> {
    match event {
        EngineEvent::Added { id, .. }
        | EngineEvent::Probed { id, .. }
        | EngineEvent::StateChanged { id, .. }
        | EngineEvent::Resumed { id, .. }
        | EngineEvent::SegmentsChanged { id }
        | EngineEvent::Completed { id, .. } => Some(*id),
        _ => None,
    }
}

/// Whether a download in `state` has stopped for good or until resumed.
fn stopped(state: State) -> bool {
    !state.is_active() && state != State::Queued
}

/// Streams one download until it stops (see [`Request::Watch`]).
async fn watch(io: &mut BoxStream, ctx: &Ctx, id: DownloadId) -> Result<(), Error> {
    // Subscribe before looking, so nothing between the two is missed.
    let mut events = ctx.engine.subscribe();
    let summary = match ctx.summary(id).await {
        Ok(s) => s,
        Err(e) => return send(io, &refuse(e)).await,
    };
    if stopped(summary.state) && ctx.engine.progress(&[id]).is_empty() {
        return send(io, &Response::Finished(Finished::from_summary(&summary))).await;
    }
    let mut tick = tokio::time::interval(WATCH_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) if event_id(&event) == Some(id) => {
                    if let Some(done) = forward(io, ctx, id, &event).await? {
                        return send(io, &Response::Finished(done)).await;
                    }
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    return send(io, &refuse(invalid("the engine stopped"))).await;
                }
            },
            _ = tick.tick() => {
                if let Some(p) = ctx.engine.progress(&[id]).first() {
                    send(io, &Response::Progress(ProgressReport::new(p))).await?;
                    continue;
                }
                // Not running: it stopped, and its events may still be queued.
                while let Ok(event) = events.try_recv() {
                    if event_id(&event) == Some(id)
                        && let Some(done) = forward(io, ctx, id, &event).await?
                    {
                        return send(io, &Response::Finished(done)).await;
                    }
                }
                match ctx.summary(id).await {
                    Ok(s) if stopped(s.state) => {
                        return send(io, &Response::Finished(Finished::from_summary(&s))).await;
                    }
                    Ok(_) => {}
                    Err(e) => return send(io, &refuse(e)).await,
                }
            }
        }
    }
}

/// Sends an event of the watched download; returns how it finished if the
/// event ends the watch.
async fn forward(
    io: &mut BoxStream,
    ctx: &Ctx,
    id: DownloadId,
    event: &EngineEvent,
) -> Result<Option<Finished>, Error> {
    match event {
        // The completion is reported by `Finished`, from the database, which
        // has the verified checksum too.
        EngineEvent::Completed { path, .. } => {
            let mut done = match ctx.summary(id).await {
                Ok(s) => Finished::from_summary(&s),
                Err(_) => Finished {
                    id: id.0,
                    state: State::Completed.as_str().into(),
                    path: None,
                    partial_path: None,
                    downloaded: 0,
                    total: None,
                    verified: None,
                    error: None,
                },
            };
            done.state = State::Completed.as_str().into();
            done.path = Some(path.clone());
            done.total = done.total.or(Some(done.downloaded));
            done.downloaded = done.total.unwrap_or(done.downloaded);
            Ok(Some(done))
        }
        EngineEvent::StateChanged { state, error, .. } => {
            send(
                io,
                &Response::Event {
                    event: serde_json::to_value(event).unwrap_or_default(),
                },
            )
            .await?;
            if !state.is_terminal() || *state == State::Completed {
                return Ok(None);
            }
            let mut done = match ctx.summary(id).await {
                Ok(s) => Finished::from_summary(&s),
                Err(_) => Finished {
                    id: id.0,
                    state: String::new(),
                    path: None,
                    partial_path: None,
                    downloaded: 0,
                    total: None,
                    verified: None,
                    error: None,
                },
            };
            // The event is newer than the database row may be.
            done.state = state.as_str().into();
            done.error = error.as_ref().map(WireError::from);
            Ok(Some(done))
        }
        _ => {
            send(
                io,
                &Response::Event {
                    event: serde_json::to_value(event).unwrap_or_default(),
                },
            )
            .await?;
            Ok(None)
        }
    }
}
