//! One download: the supervisor ("download actor"), its workers and its writer.
//!
//! Flow (see `docs/ENGINE.md`, "Overview of a download"):
//! probe → resolve destination → create sparse `.boltget` file → initial split →
//! worker 0 continues the probe response, workers 1..k-1 start staggered →
//! idle workers take unassigned segments or split the largest remaining one →
//! final flush → rename → Mark of the Web.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bytes::BytesMut;
use http::StatusCode;
use rustls::ClientConfig;
use tokio::sync::{Notify, broadcast, mpsc};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::config::EngineConfig;
use crate::engine::{AddRequest, ConflictPolicy, Destination, Limits};
use crate::error::{Error, ErrorKind, Result};
use crate::events::{Completed, DownloadId, DownloadInfo, EngineEvent, Progress, State};
use crate::filename::{self, NameSources};
use crate::headers::{self, ContentRange, Validators};
use crate::http::{Conn, Connector, Origin, Response};
use crate::platform::{self, Preallocation};
use crate::probe::{self, ProbeKind, ProbeResult, RangeSupport, sleep_or_cancel};
use crate::request::{RequestContext, redact_url};
use crate::segment::{SegmentTable, SplitConfig, Work, WorkerId};
use crate::writer::{self, FlushPolicy, WriteChunk};

/// Suffix of in-progress files.
pub(crate) const PARTIAL_SUFFIX: &str = ".boltget";

/// State shared between the download task and its [`DownloadHandle`](crate::DownloadHandle).
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) id: DownloadId,
    pub(crate) cancel: CancellationToken,
    state: Mutex<State>,
    downloaded: AtomicU64,
    connections: AtomicU32,
    max_connections: AtomicU32,
    endgame_takeovers: AtomicU32,
    table: Mutex<Option<SegmentTable>>,
    info: OnceLock<DownloadInfo>,
    events: broadcast::Sender<EngineEvent>,
}

impl Shared {
    pub(crate) fn new(
        id: DownloadId,
        cancel: CancellationToken,
        events: broadcast::Sender<EngineEvent>,
    ) -> Self {
        Self {
            id,
            cancel,
            state: Mutex::new(State::Queued),
            downloaded: AtomicU64::new(0),
            connections: AtomicU32::new(0),
            max_connections: AtomicU32::new(0),
            endgame_takeovers: AtomicU32::new(0),
            table: Mutex::new(None),
            info: OnceLock::new(),
            events,
        }
    }

    fn emit(&self, e: EngineEvent) {
        // No subscribers is fine.
        let _ = self.events.send(e);
    }

    pub(crate) fn state(&self) -> State {
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_state(&self, s: State, err: Option<&Error>) {
        {
            let mut g = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *g == s {
                return;
            }
            *g = s;
        }
        self.emit(EngineEvent::StateChanged {
            id: self.id,
            state: s,
            error: err.map(Into::into),
        });
    }

    pub(crate) fn info(&self) -> Option<&DownloadInfo> {
        self.info.get()
    }

    pub(crate) fn progress(&self) -> Progress {
        let (segments, durable, total) = {
            let g = self
                .table
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match g.as_ref() {
                Some(t) => (t.segments(), t.durable_total(), t.total()),
                None => (Vec::new(), 0, None),
            }
        };
        Progress {
            id: self.id,
            state: self.state(),
            downloaded: self.downloaded.load(Ordering::Relaxed),
            durable,
            total,
            connections: self.connections.load(Ordering::Relaxed),
            segments,
        }
    }
}

/// Engine-wide resources a download needs.
#[derive(Debug, Clone)]
pub(crate) struct Env {
    pub(crate) cfg: Arc<EngineConfig>,
    pub(crate) tls: Arc<ClientConfig>,
    pub(crate) limits: Limits,
    pub(crate) reserved: Arc<Mutex<HashSet<PathBuf>>>,
}

/// Per-worker counters read by the supervisor.
#[derive(Debug, Default)]
struct WorkerStat {
    /// Payload bytes read from the network (including bytes later discarded).
    net: AtomicU64,
    /// Current segment, `u32::MAX` if none.
    seg: AtomicU32,
    /// Set by the slow-connection detector: reconnect from the cursor.
    kick: Notify,
}

#[derive(Debug)]
enum SupMsg {
    /// The verification request confirmed range support.
    RangesConfirmed,
}

/// State shared by the workers of one download.
#[derive(Debug)]
struct Core {
    shared: Arc<Shared>,
    connector: Connector,
    limits: Limits,
    cfg: Arc<EngineConfig>,
    ctx: RequestContext,
    original: Origin,
    worker_origin: Origin,
    final_url: Mutex<Url>,
    validators: Validators,
    if_range: Option<String>,
    total: Option<u64>,
    /// Byte ranges confirmed to work (resume from the cursor, splits allowed).
    resumable: AtomicBool,
    /// The server encodes the body: never send ranges.
    encoded: bool,
    writer: Mutex<Option<mpsc::Sender<WriteChunk>>>,
    next_worker: AtomicU32,
    rtt_micros: AtomicU64,
    retrying: AtomicU32,
    stats: Mutex<HashMap<WorkerId, Arc<WorkerStat>>>,
    sup: mpsc::UnboundedSender<SupMsg>,
    /// Cancels workers (user cancel or a fatal error).
    stop: CancellationToken,
}

impl Core {
    fn table<R>(&self, f: impl FnOnce(&mut SegmentTable) -> R) -> Result<R> {
        let mut g = self
            .shared
            .table
            .lock()
            .map_err(|_| Error::Internal("segment table lock poisoned".into()))?;
        let t = g
            .as_mut()
            .ok_or_else(|| Error::Internal("segment table missing".into()))?;
        Ok(f(t))
    }

    fn resumable(&self) -> bool {
        self.resumable.load(Ordering::Acquire)
    }

    fn writer_tx(&self) -> Result<mpsc::Sender<WriteChunk>> {
        self.writer
            .lock()
            .map_err(|_| Error::Internal("writer lock poisoned".into()))?
            .clone()
            .ok_or_else(|| Error::Internal("writer already closed".into()))
    }

    fn current_url(&self) -> Result<Url> {
        Ok(self
            .final_url
            .lock()
            .map_err(|_| Error::Internal("url lock poisoned".into()))?
            .clone())
    }

    fn rtt(&self) -> Duration {
        Duration::from_micros(self.rtt_micros.load(Ordering::Relaxed))
    }
}

/// Entry point of the download task.
pub(crate) async fn run(shared: Arc<Shared>, env: Env, req: AddRequest) -> Result<Completed> {
    let started = Instant::now();
    let result = run_inner(&shared, &env, req, started).await;
    match &result {
        Ok(c) => {
            shared.set_state(State::Completed, None);
            shared.emit(EngineEvent::Completed {
                id: shared.id,
                path: c.path.clone(),
            });
        }
        Err(Error::Cancelled) => shared.set_state(State::Cancelled, None),
        Err(e) => {
            tracing::warn!(id = %shared.id, error = %e, "download failed");
            shared.set_state(State::Failed, Some(e));
        }
    }
    result
}

/// Releases a reserved destination path when dropped.
struct Reservation {
    set: Arc<Mutex<HashSet<PathBuf>>>,
    path: PathBuf,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut s) = self.set.lock() {
            s.remove(&self.path);
        }
    }
}

fn partial_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(PARTIAL_SUFFIX);
    PathBuf::from(s)
}

/// Resolves the final path for the download and reserves it.
fn choose_path(
    env: &Env,
    req: &AddRequest,
    probe: &probe::Probed,
) -> Result<(PathBuf, Reservation)> {
    let (dir, user_name) = match &req.destination {
        Destination::Directory(d) => (d.clone(), None),
        Destination::File(f) => {
            let dir = f
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .ok_or_else(|| Error::disk(f, std::io::Error::other("not a file path")))?;
            (dir, Some(name))
        }
    };
    let name = filename::resolve(NameSources {
        user: user_name.as_deref(),
        content_disposition: probe.content_disposition.as_deref(),
        suggested: req.ctx.suggested_name(),
        final_url: Some(&probe.final_url),
        content_type: probe.content_type.as_deref(),
    });
    let mut reserved = env
        .reserved
        .lock()
        .map_err(|_| Error::Internal("reservation lock poisoned".into()))?;
    let path = match req.conflict {
        ConflictPolicy::Rename => filename::non_colliding(&dir, &name, |p| {
            p.exists() || partial_path(p).exists() || reserved.contains(p)
        }),
        ConflictPolicy::Overwrite => dir.join(&name),
        ConflictPolicy::Fail => {
            let p = dir.join(&name);
            if p.exists() || reserved.contains(&p) {
                return Err(Error::disk(
                    &p,
                    std::io::Error::from(std::io::ErrorKind::AlreadyExists),
                ));
            }
            p
        }
    };
    reserved.insert(path.clone());
    Ok((
        path.clone(),
        Reservation {
            set: env.reserved.clone(),
            path,
        },
    ))
}

fn check_space(dir: &Path, size: Option<u64>, margin: u64) -> Result<()> {
    let Some(size) = size else { return Ok(()) };
    if let Some(available) = platform::available_space(dir) {
        let needed = size.saturating_add(margin);
        if available < needed {
            return Err(Error::InsufficientSpace {
                path: dir.to_path_buf(),
                needed,
                available,
            });
        }
    }
    Ok(())
}

async fn run_inner(
    shared: &Arc<Shared>,
    env: &Env,
    req: AddRequest,
    started: Instant,
) -> Result<Completed> {
    let cfg = env.cfg.clone();
    let connector = Connector::new(env.tls.clone(), cfg.clone());
    let original = Origin::of(req.ctx.url())?;

    // ---- Probe (its connection becomes worker 0) ----
    shared.set_state(State::Probing, None);
    let permit0 = tokio::select! {
        () = shared.cancel.cancelled() => return Err(Error::Cancelled),
        p = env.limits.acquire(&original) => p,
    };
    let probed = probe::probe(&connector, &req.ctx, &shared.cancel).await?;
    let worker_origin = Origin::of(&probed.final_url)?;
    let permit0 = if worker_origin == original {
        permit0
    } else {
        drop(permit0);
        tokio::select! {
            () = shared.cancel.cancelled() => return Err(Error::Cancelled),
            p = env.limits.acquire(&worker_origin) => p,
        }
    };
    let kind = probed.kind;
    let total = kind.total();
    tracing::debug!(id = %shared.id, ?kind, url = %redact_url(&probed.final_url), "probe done");

    // ---- Destination ----
    let (path, _reservation) = choose_path(env, &req, &probed)?;
    let dir = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    check_space(&dir, total, cfg.disk_margin)?;
    let partial = partial_path(&path);
    let (file, prealloc) =
        platform::create_preallocated(&partial, total).map_err(|e| Error::disk(&partial, e))?;
    if let (Some(size), Some(fs)) = (total, platform::filesystem_name(&file))
        && size >= 4 * 1024 * 1024 * 1024
        && (fs.eq_ignore_ascii_case("FAT32") || fs.eq_ignore_ascii_case("FAT"))
    {
        drop(file);
        let _ = std::fs::remove_file(&partial);
        return Err(Error::FileTooLarge { fs, size });
    }

    // ---- Segment table ----
    let split_cfg = SplitConfig {
        align: cfg.segment_align,
        min_split_size: cfg.min_split_size,
    };
    let wanted = req
        .connections
        .unwrap_or(cfg.initial_connections)
        .clamp(1, cfg.max_connections);
    let available = env.limits.available(&worker_origin) as u64 + 1;
    let k_for = |n: u64| -> u32 {
        let by_size = (n / cfg.min_split_size).max(1);
        u64::from(wanted).min(available).min(by_size) as u32
    };
    let (table, resumable, k) = match kind {
        ProbeKind::Ranged { total } => {
            let k = k_for(total);
            (SegmentTable::new_split(total, k, split_cfg), true, k)
        }
        ProbeKind::Empty => (SegmentTable::new_split(0, 1, split_cfg), true, 0),
        ProbeKind::Full { total } => (
            SegmentTable::new_single(total, split_cfg),
            false,
            k_for(total),
        ),
        ProbeKind::Encoded { .. } | ProbeKind::Stream => {
            (SegmentTable::new_open(split_cfg), false, 1)
        }
        ProbeKind::RangedUnknownSize => (SegmentTable::new_open(split_cfg), true, 1),
    };
    // For `Encoded` with a length we still treat the size as unknown: the length
    // describes the encoded bytes we write, but we cannot trust it for ranges.
    let total = match kind {
        ProbeKind::Ranged { total } | ProbeKind::Full { total } => Some(total),
        ProbeKind::Empty => Some(0),
        _ => None,
    };
    *shared
        .table
        .lock()
        .map_err(|_| Error::Internal("table lock poisoned".into()))? = Some(table);

    let probe_result = ProbeResult {
        final_url: probed.final_url.clone(),
        redirects: probed.redirects,
        size: total,
        ranges: if resumable {
            RangeSupport::Yes
        } else {
            RangeSupport::No
        },
        validators: probed.validators.clone(),
        content_type: probed.content_type.clone(),
        filename: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    let info = DownloadInfo {
        probe: probe_result,
        path: path.clone(),
        partial_path: partial.clone(),
        preallocation: prealloc,
        initial_connections: k.max(1),
    };
    let _ = shared.info.set(info.clone());
    shared.emit(EngineEvent::Probed {
        id: shared.id,
        info: Box::new(info.clone()),
    });

    // ---- Writer ----
    let path_arc = Arc::new(partial.clone());
    let mut w = writer::spawn(
        file,
        path_arc,
        cfg.writer_queue,
        FlushPolicy {
            interval: cfg.flush_interval,
            bytes: cfg.flush_bytes,
        },
    )?;
    let (sup_tx, mut sup_rx) = mpsc::unbounded_channel();
    let if_range = probed.validators.if_range().map(str::to_owned);
    let core = Arc::new(Core {
        shared: shared.clone(),
        connector,
        limits: env.limits.clone(),
        cfg: cfg.clone(),
        ctx: req.ctx.clone(),
        original,
        worker_origin,
        final_url: Mutex::new(probed.final_url.clone()),
        validators: probed.validators.clone(),
        if_range,
        total,
        resumable: AtomicBool::new(resumable),
        encoded: matches!(kind, ProbeKind::Encoded { .. }),
        writer: Mutex::new(Some(w.tx.clone())),
        next_worker: AtomicU32::new(0),
        rtt_micros: AtomicU64::new(0),
        retrying: AtomicU32::new(0),
        stats: Mutex::new(HashMap::new()),
        sup: sup_tx,
        stop: shared.cancel.child_token(),
    });
    drop(w.tx);

    // ---- Workers ----
    shared.set_state(State::Downloading, None);
    let mut workers: JoinSet<(WorkerId, Exit)> = JoinSet::new();
    let spawn = |workers: &mut JoinSet<(WorkerId, Exit)>,
                 start: Start,
                 permit: Option<crate::engine::ConnPermit>| {
        let id = core.next_worker.fetch_add(1, Ordering::Relaxed);
        let c = core.clone();
        workers.spawn(async move { (id, worker(c, id, start, permit).await) });
    };
    let mut extra_on_confirm = 0u32;
    if matches!(kind, ProbeKind::Empty) {
        drop(permit0);
        drop(probed.response);
    } else {
        // Worker 0 owns segment 0 and continues the probe response.
        core.table(|t| t.assign(0, 0))?;
        spawn(
            &mut workers,
            Start::Live {
                seg: 0,
                live: Box::new(Live {
                    resp: probed.response,
                    end: total,
                }),
            },
            Some(permit0),
        );
        match kind {
            ProbeKind::Ranged { .. } => {
                for i in 1..k {
                    spawn(
                        &mut workers,
                        Start::Fresh {
                            delay: cfg.worker_stagger * i,
                        },
                        None,
                    );
                }
            }
            ProbeKind::Full { total } if k >= 2 && total >= 2 * cfg.min_split_size => {
                // Verify range support on a second connection (it becomes worker 1).
                let m = verification_offset(total, cfg.segment_align);
                spawn(&mut workers, Start::Verify { m }, None);
                extra_on_confirm = k - 2;
            }
            _ => {}
        }
    }

    // ---- Supervise ----
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut monitor = Monitor::default();
    let mut last_error: Option<Error> = None;
    let mut fatal: Option<Error> = None;
    let mut respawns = 0u32;
    let mut writer_result: Option<Result<std::fs::File>> = None;
    let mut seg_count = core.table(|t| t.segments().len())?;

    loop {
        if workers.is_empty() {
            let complete = core.table(|t| t.is_complete())?;
            if complete || fatal.is_some() || shared.cancel.is_cancelled() {
                break;
            }
            if let Some(e) = last_error.take() {
                fatal = Some(e);
                break;
            }
            // Workers left without an error while work remains (a race between a
            // worker going idle and another releasing its segment).
            respawns += 1;
            if respawns > 3 {
                fatal = Some(Error::Internal("no progress and no workers".into()));
                break;
            }
            spawn(
                &mut workers,
                Start::Fresh {
                    delay: Duration::ZERO,
                },
                None,
            );
        }
        tokio::select! {
            () = shared.cancel.cancelled() => {
                core.stop.cancel();
                while workers.join_next().await.is_some() {}
                break;
            }
            Some(report) = w.durable.recv() => {
                core.table(|t| for (seg, upto) in report { t.mark_durable(seg, upto); })?;
            }
            res = &mut w.done, if writer_result.is_none() => {
                // The writer only ends early on an error.
                let r = res.map_err(|_| Error::Internal("writer thread vanished".into()))?;
                if let Err(e) = r {
                    fatal = Some(e);
                    core.stop.cancel();
                    while workers.join_next().await.is_some() {}
                    break;
                }
                writer_result = Some(r);
            }
            Some(msg) = sup_rx.recv() => match msg {
                SupMsg::RangesConfirmed => {
                    for i in 0..extra_on_confirm {
                        spawn(&mut workers, Start::Fresh { delay: cfg.worker_stagger * (i + 1) }, None);
                    }
                    extra_on_confirm = 0;
                }
            },
            Some(joined) = workers.join_next() => {
                let (id, exit) = match joined {
                    Ok(v) => v,
                    Err(e) if e.is_panic() => {
                        fatal = Some(Error::Internal(format!("worker panicked: {e}")));
                        core.stop.cancel();
                        continue;
                    }
                    Err(_) => continue,
                };
                tracing::debug!(id = %shared.id, worker = id, ?exit, "worker exited");
                let _ = core.table(|t| t.release(id));
                match exit {
                    Exit::Idle | Exit::Cancelled => {}
                    Exit::Fatal(e) => {
                        if fatal.is_none() {
                            fatal = Some(e);
                        }
                        core.stop.cancel();
                    }
                    Exit::GaveUp(e) => last_error = Some(e),
                    Exit::Throttled(e) => {
                        // Fewer connections is the point; if nobody is left, carry on with one.
                        let delay = e.retry_after().unwrap_or(cfg.backoff_base);
                        last_error = Some(e);
                        if workers.is_empty() {
                            spawn(&mut workers, Start::Fresh { delay }, None);
                        }
                    }
                }
            }
            _ = tick.tick() => {
                monitor.tick(&core);
                let n = core.table(|t| t.segments().len())?;
                if n != seg_count {
                    seg_count = n;
                    shared.emit(EngineEvent::SegmentsChanged { id: shared.id });
                }
                let active = shared.connections.load(Ordering::Relaxed);
                let retrying = core.retrying.load(Ordering::Relaxed);
                if active > 0 && retrying >= active {
                    shared.set_state(State::Retrying, last_error.as_ref());
                } else if shared.state() == State::Retrying {
                    shared.set_state(State::Downloading, None);
                }
            }
        }
    }

    // ---- Finish: close the writer, collect final durable offsets ----
    drop(
        core.writer
            .lock()
            .map_err(|_| Error::Internal("writer lock".into()))?
            .take(),
    );
    let file_res = match writer_result {
        Some(r) => r,
        None => loop {
            tokio::select! {
                Some(report) = w.durable.recv() => {
                    core.table(|t| for (seg, upto) in report { t.mark_durable(seg, upto); })?;
                }
                res = &mut w.done => break res.map_err(|_| Error::Internal("writer thread vanished".into()))?,
            }
        },
    };
    while let Ok(report) = w.durable.try_recv() {
        core.table(|t| {
            for (seg, upto) in report {
                t.mark_durable(seg, upto);
            }
        })?;
    }
    if shared.cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    // A writer error (disk full, ...) is the root cause of any "writer closed"
    // failure the workers saw, so it wins.
    let file = match (file_res, fatal) {
        (Err(e), _) | (Ok(_), Some(e)) => return Err(e),
        (Ok(f), None) => f,
    };

    // ---- Verify our own bookkeeping, then rename ----
    let (bytes, segments) = core.table(|t| -> Result<(u64, usize)> {
        t.check_invariants().map_err(Error::Internal)?;
        let total = t
            .total()
            .ok_or_else(|| Error::Internal("size unknown at completion".into()))?;
        if !t.is_durable() || t.received_total() != total {
            return Err(Error::Internal(format!(
                "byte count mismatch: received {} durable {} total {}",
                t.received_total(),
                t.durable_total(),
                total
            )));
        }
        Ok((total, t.segments().len()))
    })??;
    if core.total.is_none() || prealloc == Preallocation::None {
        // Unknown size: trim anything a restarted attempt may have left past the end.
        file.set_len(bytes).map_err(|e| Error::disk(&partial, e))?;
        file.sync_data().map_err(|e| Error::disk(&partial, e))?;
    }
    drop(file);

    let final_path = if req.conflict == ConflictPolicy::Rename && path.exists() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        filename::non_colliding(&dir, &name, Path::exists)
    } else {
        path.clone()
    };
    std::fs::rename(&partial, &final_path).map_err(|e| Error::disk(&final_path, e))?;
    if cfg.mark_of_the_web {
        let url = core
            .final_url
            .lock()
            .map_err(|_| Error::Internal("url lock".into()))?
            .clone();
        let host = redact_for_motw(&url);
        let referrer = req
            .ctx
            .headers()
            .get(http::header::REFERER)
            .and_then(|v| v.to_str().ok());
        platform::mark_of_the_web(&final_path, &host, referrer);
    }
    let mut info = info;
    info.path.clone_from(&final_path);
    info.probe.ranges = if core.resumable() {
        RangeSupport::Yes
    } else {
        RangeSupport::No
    };
    Ok(Completed {
        id: shared.id,
        path: final_path,
        bytes,
        elapsed: started.elapsed(),
        max_connections: shared.max_connections.load(Ordering::Relaxed),
        segments,
        endgame_takeovers: shared.endgame_takeovers.load(Ordering::Relaxed),
        info,
    })
}

/// The split point for the verification request: the middle, aligned.
fn verification_offset(total: u64, align: u64) -> u64 {
    crate::segment::align_down(total / 2, align).max(1)
}

/// `HostUrl` for the Mark of the Web: scheme, host and path, without credentials
/// or query (signed URLs carry secrets there).
fn redact_for_motw(url: &Url) -> String {
    let mut u = url.clone();
    u.set_query(None);
    u.set_fragment(None);
    let _ = u.set_username("");
    let _ = u.set_password(None);
    u.to_string()
}

/// Throughput tracking and slow-connection detection, run on the supervisor tick.
#[derive(Debug, Default)]
struct Monitor {
    last: Option<Instant>,
    per_worker: HashMap<WorkerId, (u64, f64, Option<Instant>)>,
    /// Median per-worker throughput, last measured while at least two workers
    /// were transferring. Kept after the others finish, so a lone straggler at
    /// the end of a download is still recognised as slow.
    reference: f64,
}

impl Monitor {
    fn tick(&mut self, core: &Core) {
        let now = Instant::now();
        let dt = self
            .last
            .map_or(0.5, |l| now.duration_since(l).as_secs_f64())
            .max(1e-3);
        self.last = Some(now);
        let stats: Vec<(WorkerId, Arc<WorkerStat>)> = match core.stats.lock() {
            Ok(s) => s.iter().map(|(k, v)| (*k, v.clone())).collect(),
            Err(_) => return,
        };
        self.per_worker
            .retain(|id, _| stats.iter().any(|(w, _)| w == id));
        let reaction = core.rtt() + core.cfg.reaction_slack;
        let mut rates = Vec::new();
        for (id, stat) in &stats {
            let net = stat.net.load(Ordering::Relaxed);
            let entry = self.per_worker.entry(*id).or_insert((net, 0.0, None));
            let inst = net.saturating_sub(entry.0) as f64 / dt;
            entry.0 = net;
            entry.1 = if entry.1 == 0.0 {
                inst
            } else {
                0.7 * entry.1 + 0.3 * inst
            };
            let seg = stat.seg.load(Ordering::Relaxed);
            if seg != u32::MAX {
                let ahead =
                    core.cfg.coalesce_size as u64 + (entry.1 * reaction.as_secs_f64()) as u64;
                let _ = core.table(|t| t.update_stats(seg, entry.1, ahead));
                rates.push(entry.1);
            }
        }
        if rates.len() >= 2 {
            rates.sort_by(f64::total_cmp);
            let median = rates[rates.len() / 2];
            if median > 0.0 {
                self.reference = median;
            }
        }
        let median = self.reference;
        if core.cfg.slow_window.is_zero() || median <= 0.0 {
            return;
        }
        for (id, stat) in &stats {
            let Some(entry) = self.per_worker.get_mut(id) else {
                continue;
            };
            if stat.seg.load(Ordering::Relaxed) == u32::MAX {
                continue;
            }
            if entry.1 < core.cfg.slow_fraction * median {
                let since = *entry.2.get_or_insert(now);
                if now.duration_since(since) >= core.cfg.slow_window {
                    tracing::debug!(
                        worker = id,
                        rate = entry.1,
                        median,
                        "slow connection; reconnecting"
                    );
                    stat.kick.notify_one();
                    entry.2 = None;
                }
            } else {
                entry.2 = None;
            }
        }
    }
}

// ------------------------------------------------------------------ workers

/// How a worker starts.
#[derive(Debug)]
enum Start {
    /// Continue an already open response for `seg` (worker 0, the probe).
    Live { seg: u32, live: Box<Live> },
    /// Send the range verification request at `m`, then work normally.
    Verify { m: u64 },
    /// Wait `delay` (stagger), then ask for work.
    Fresh { delay: Duration },
}

/// An open response and the end (exclusive) of the range it was asked for
/// (`None`: to the end of an unknown-size resource).
#[derive(Debug)]
struct Live {
    resp: Response,
    end: Option<u64>,
}

/// Why a worker stopped.
#[derive(Debug)]
enum Exit {
    /// No work left.
    Idle,
    /// Stopped by cancellation.
    Cancelled,
    /// Retries exhausted on its segment (released for others).
    GaveUp(Error),
    /// The server pushed back; segment released to reduce concurrency.
    Throttled(Error),
    /// The download cannot continue.
    Fatal(Error),
}

/// Decrements the connection gauge and unregisters stats when a worker ends.
struct WorkerGuard<'a> {
    core: &'a Core,
    id: WorkerId,
}

impl Drop for WorkerGuard<'_> {
    fn drop(&mut self) {
        self.core.shared.connections.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut s) = self.core.stats.lock() {
            s.remove(&self.id);
        }
        let _ = self.core.table(|t| t.release(self.id));
    }
}

async fn worker(
    core: Arc<Core>,
    id: WorkerId,
    start: Start,
    permit: Option<crate::engine::ConnPermit>,
) -> Exit {
    if let Start::Fresh { delay } = &start
        && !delay.is_zero()
        && sleep_or_cancel(*delay, &core.stop).await.is_err()
    {
        return Exit::Cancelled;
    }
    let _permit = if let Some(p) = permit {
        p
    } else {
        tokio::select! {
            () = core.stop.cancelled() => return Exit::Cancelled,
            p = core.limits.acquire(&core.worker_origin) => p,
        }
    };
    let n = core.shared.connections.fetch_add(1, Ordering::Relaxed) + 1;
    core.shared.max_connections.fetch_max(n, Ordering::Relaxed);
    let _guard = WorkerGuard { core: &core, id };
    let stat = Arc::new(WorkerStat {
        seg: AtomicU32::new(u32::MAX),
        ..WorkerStat::default()
    });
    if let Ok(mut s) = core.stats.lock() {
        s.insert(id, stat.clone());
    }

    let mut conn: Option<Conn> = None;
    let mut pending: Option<(u32, Option<Live>)> = None;
    match start {
        Start::Live { seg, live } => pending = Some((seg, Some(*live))),
        Start::Verify { m } => {
            let url = match core.final_url.lock() {
                Ok(u) => u.clone(),
                Err(_) => return Exit::Fatal(Error::Internal("url lock".into())),
            };
            let total = core.total.unwrap_or(0);
            let verified = tokio::select! {
                () = core.stop.cancelled() => return Exit::Cancelled,
                r = probe::verify_ranges(&core.connector, &url, &core.original, &core.ctx, m, total, &core.validators) => r,
            };
            let Some(resp) = verified else {
                tracing::debug!("range verification failed; single connection");
                return Exit::Idle;
            };
            let split = core.table(|t| {
                t.set_splittable(true);
                t.split_at(0, m, id)
            });
            core.resumable.store(true, Ordering::Release);
            let _ = core.sup.send(SupMsg::RangesConfirmed);
            match split {
                Ok(Some(seg)) => {
                    pending = Some((
                        seg,
                        Some(Live {
                            resp,
                            end: core.total,
                        }),
                    ));
                }
                Ok(None) => {}
                Err(e) => return Exit::Fatal(e),
            }
        }
        Start::Fresh { .. } => {}
    }

    loop {
        let (seg, live) = if let Some(p) = pending.take() {
            p
        } else {
            let work = core.table(|t| {
                t.next_work(id).or_else(|| {
                    core.cfg
                        .endgame
                        .then(|| t.endgame_work(core.cfg.endgame_threshold.as_secs_f64()))
                        .flatten()
                })
            });
            match work {
                Err(e) => return Exit::Fatal(e),
                Ok(None) => return Exit::Idle,
                Ok(Some(Work::Segment { idx })) => (idx, None),
                Ok(Some(Work::Duplicate { of, from, to })) => {
                    match duplicate(&core, id, of, from, to, &mut conn).await {
                        Some((seg, resp)) => (
                            seg,
                            Some(Live {
                                resp,
                                end: Some(to),
                            }),
                        ),
                        None => continue,
                    }
                }
            }
        };
        stat.seg.store(seg, Ordering::Relaxed);
        let r = run_segment(&core, id, &stat, seg, live, &mut conn).await;
        stat.seg.store(u32::MAX, Ordering::Relaxed);
        if let Err(exit) = r {
            return exit;
        }
    }
}

/// Endgame: request `[from, to)` of a slow segment; take it over once the
/// response starts, if the slow owner has not reached `from` yet.
async fn duplicate(
    core: &Core,
    id: WorkerId,
    of: u32,
    from: u64,
    to: u64,
    conn: &mut Option<Conn>,
) -> Option<(u32, Response)> {
    let url = core.current_url().ok()?;
    let resp = tokio::select! {
        () = core.stop.cancelled() => return None,
        r = open_range(core, &url, conn.take(), from, Some(to)) => r.ok()?,
    };
    validate(core, &resp, from, true).ok()?;
    let seg = core.table(|t| t.split_at(of, from, id)).ok()??;
    tracing::debug!(
        worker = id,
        of,
        from,
        to,
        "endgame duplicate took over tail"
    );
    core.shared
        .endgame_takeovers
        .fetch_add(1, Ordering::Relaxed);
    Some((seg, resp))
}

async fn open_range(
    core: &Core,
    url: &Url,
    conn: Option<Conn>,
    start: u64,
    end: Option<u64>,
) -> Result<Response> {
    let ranged = core.resumable() && !core.encoded;
    let (range, if_range) = if ranged {
        (Some((start, end)), core.if_range.as_deref())
    } else {
        (None, None)
    };
    let t0 = Instant::now();
    let resp = core
        .connector
        .get(
            conn,
            url,
            &core.original,
            core.ctx.headers(),
            range,
            if_range,
        )
        .await?;
    let rtt = t0.elapsed().as_micros() as u64;
    let prev = core.rtt_micros.load(Ordering::Relaxed);
    core.rtt_micros.store(
        if prev == 0 { rtt } else { (prev * 7 + rtt) / 8 },
        Ordering::Relaxed,
    );
    if resp.status.as_u16() >= 400 {
        return Err(probe::status_error(
            resp.status,
            &resp.headers,
            &resp.url,
            true,
        ));
    }
    Ok(resp)
}

/// Checks that a response really carries the bytes starting at `cursor` of the
/// same version of the resource.
fn validate(core: &Core, resp: &Response, cursor: u64, ranged: bool) -> Result<()> {
    if headers::is_encoded(&resp.headers) != core.encoded {
        return Err(Error::ResourceChanged("Content-Encoding changed".into()));
    }
    let later = Validators::from_headers(&resp.headers);
    match resp.status {
        StatusCode::PARTIAL_CONTENT => {
            match ContentRange::from_headers(&resp.headers) {
                Some(ContentRange::Bytes { first, total, .. }) if first == cursor => {
                    if let (Some(t), Some(known)) = (total, core.total)
                        && t != known
                    {
                        return Err(Error::ResourceChanged(format!(
                            "size changed from {known} to {t}"
                        )));
                    }
                }
                other => {
                    return Err(Error::Protocol(format!(
                        "expected range from {cursor}, got Content-Range {other:?}"
                    )));
                }
            }
            if let Some(why) = core.validators.mismatch(&later) {
                return Err(Error::ResourceChanged(why));
            }
            Ok(())
        }
        StatusCode::OK => {
            if cursor != 0 {
                if ranged && core.if_range.is_some() {
                    return Err(Error::ResourceChanged(
                        "server returned the whole file to an If-Range request".into(),
                    ));
                }
                return Err(Error::RangeUnsupported(format!(
                    "200 instead of 206 for a range starting at {cursor}"
                )));
            }
            // A full response is fine at offset 0 as long as it is the same file.
            if let Some(why) = core.validators.mismatch(&later) {
                return Err(Error::ResourceChanged(why));
            }
            if let (Some(len), Some(known)) = (headers::content_length(&resp.headers), core.total)
                && !core.encoded
                && len != known
            {
                return Err(Error::ResourceChanged(format!(
                    "size changed from {known} to {len}"
                )));
            }
            Ok(())
        }
        s => Err(Error::Protocol(format!("unexpected status {s}"))),
    }
}

/// How streaming a response ended (without error).
enum StreamEnd {
    /// The segment is fully received (or was cut short by a split).
    Done,
    /// The server ended the body cleanly before the segment end.
    Short,
}

/// Downloads segment `seg` until it is complete, retrying per `docs/ENGINE.md`.
async fn run_segment(
    core: &Core,
    id: WorkerId,
    stat: &WorkerStat,
    mut seg: u32,
    mut live: Option<Live>,
    conn: &mut Option<Conn>,
) -> Result<(), Exit> {
    let fatal = |e: Error| Exit::Fatal(e);
    let mut attempts = 0u32;
    loop {
        let (cursor, end) = core
            .table(|t| t.get(seg).map(|s| (s.cursor(), s.end)))
            .map_err(fatal)?
            .ok_or_else(|| Exit::Fatal(Error::Internal(format!("segment {seg} vanished"))))?;
        if cursor >= end {
            return Ok(());
        }
        let req_end = core.total.map(|_| end);
        let used_url = core.current_url().map_err(fatal)?;
        let attempt: Result<StreamEnd> = async {
            let (resp, resp_end) = if let Some(l) = live.take() {
                (l.resp, l.end)
            } else {
                let r = tokio::select! {
                    () = core.stop.cancelled() => return Err(Error::Cancelled),
                    r = open_range(core, &used_url, conn.take(), cursor, req_end) => r?,
                };
                validate(core, &r, cursor, core.resumable())?;
                // Re-resolved from the original URL: remember where it led now.
                if used_url == *core.ctx.url()
                    && r.url != used_url
                    && let Ok(mut u) = core.final_url.lock()
                {
                    u.clone_from(&r.url);
                }
                (r, req_end)
            };
            stream(core, id, stat, seg, resp, cursor, resp_end, conn).await
        }
        .await;
        let progressed = core
            .table(|t| t.get(seg).is_some_and(|s| s.cursor() > cursor))
            .unwrap_or(false);
        if progressed {
            attempts = 0;
        }
        let err = match attempt {
            Ok(StreamEnd::Done) => return Ok(()),
            Ok(StreamEnd::Short) if progressed && core.resumable() => continue,
            Ok(StreamEnd::Short) => {
                Error::Protocol("response ended before the expected end".into())
            }
            Err(e) => e,
        };
        if core.stop.is_cancelled() || matches!(err, Error::Cancelled) {
            return Err(Exit::Cancelled);
        }
        tracing::debug!(worker = id, seg, error = %err, attempts, "segment attempt failed");
        match err.kind() {
            ErrorKind::LinkExpired => {
                // Expiring redirect targets (signed URLs): re-resolve from the
                // original URL. If that request itself fails, the link is dead.
                let original = core.ctx.url().clone();
                if used_url != original {
                    tracing::debug!("link expired; re-resolving from the original URL");
                    if let Ok(mut u) = core.final_url.lock() {
                        *u = original;
                    }
                    continue;
                }
                return Err(Exit::Fatal(err));
            }
            ErrorKind::Throttled => {
                let others = core.table(|t| t.active_workers()).unwrap_or(1) > 1;
                if others {
                    return Err(Exit::Throttled(err));
                }
                let Some(delay) = probe::retry_delay(&err, &core.connector, attempts) else {
                    return Err(Exit::Fatal(err));
                };
                attempts += 1;
                if attempts >= core.cfg.max_attempts {
                    return Err(Exit::GaveUp(err));
                }
                backoff_sleep(core, delay).await?;
            }
            ErrorKind::Transient => {
                attempts += 1;
                if attempts >= core.cfg.max_attempts {
                    return Err(Exit::GaveUp(err));
                }
                let Some(delay) = probe::retry_delay(&err, &core.connector, attempts - 1) else {
                    return Err(Exit::Fatal(err));
                };
                backoff_sleep(core, delay).await?;
            }
            _ => return Err(Exit::Fatal(err)),
        }
        if !core.resumable() {
            // Not resumable: start over from byte 0 on a fresh segment.
            let lost = core
                .table(|t| t.get(seg).map_or(0, |s| s.received))
                .map_err(fatal)?;
            if lost > 0 {
                seg = core
                    .table(|t| t.restart(seg, id))
                    .map_err(fatal)?
                    .ok_or_else(|| Exit::Fatal(Error::Internal("restart failed".into())))?;
                stat.seg.store(seg, Ordering::Relaxed);
                core.shared.downloaded.fetch_sub(lost, Ordering::Relaxed);
                tracing::debug!(lost, "server cannot resume; restarting from zero");
            }
        }
    }
}

async fn backoff_sleep(core: &Core, delay: Duration) -> Result<(), Exit> {
    core.retrying.fetch_add(1, Ordering::Relaxed);
    let r = sleep_or_cancel(delay, &core.stop).await;
    core.retrying.fetch_sub(1, Ordering::Relaxed);
    r.map_err(|_| Exit::Cancelled)
}

/// Claims the coalesced bytes in `buf` (at `*buf_start`) and hands them to the
/// writer. Returns false if the segment was cut short by a split, in which case
/// the bytes beyond the new end were discarded.
async fn flush(
    core: &Core,
    id: WorkerId,
    seg: u32,
    tx: &mpsc::Sender<WriteChunk>,
    buf: &mut BytesMut,
    buf_start: &mut u64,
) -> Result<bool> {
    if buf.is_empty() {
        return Ok(true);
    }
    // Reserve the channel slot *before* claiming: there is no await between the
    // claim and the send, so claimed bytes always reach the writer.
    let permit = tokio::select! {
        () = core.stop.cancelled() => return Err(Error::Cancelled),
        p = tx.reserve() => p.map_err(|_| Error::Internal("writer closed".into()))?,
    };
    let len = buf.len() as u64;
    let accepted = core
        .table(|t| t.claim(seg, id, *buf_start, len))?
        .map_err(|e| Error::Internal(format!("claim rejected: {e:?}")))?;
    if accepted > 0 {
        let bytes = buf.split_to(accepted as usize).freeze();
        permit.send(WriteChunk {
            segment: seg,
            offset: *buf_start,
            bytes,
        });
        core.shared
            .downloaded
            .fetch_add(accepted, Ordering::Relaxed);
    }
    buf.clear();
    *buf_start += accepted;
    Ok(accepted == len)
}

/// Reads `resp` into segment `seg` from `cursor`, coalescing into chunks of
/// `coalesce_size`, claiming each chunk in the table just before handing it to
/// the writer (so a split can never cause a double write).
#[allow(clippy::too_many_arguments)]
async fn stream(
    core: &Core,
    id: WorkerId,
    stat: &WorkerStat,
    seg: u32,
    mut resp: Response,
    cursor: u64,
    resp_end: Option<u64>,
    conn: &mut Option<Conn>,
) -> Result<StreamEnd> {
    let tx = core.writer_tx()?;
    let coalesce = core.cfg.coalesce_size;
    let stall = core.cfg.stall_timeout;
    let mut buf = BytesMut::with_capacity(coalesce);
    let mut buf_start = cursor;
    let mut pos = cursor;

    macro_rules! flush {
        () => {
            flush(core, id, seg, &tx, &mut buf, &mut buf_start).await?
        };
    }

    loop {
        let end = core.table(|t| t.end_of(seg))?.unwrap_or(pos);
        if pos >= end {
            // Reached the end of what was requested (not cut short by a split).
            let natural = Some(end) == resp_end;
            if !flush!() {
                return Ok(StreamEnd::Done);
            }
            if natural {
                // Observe the end of the body so the connection can be reused.
                if let Ok(Ok(None)) =
                    tokio::time::timeout(Duration::from_millis(200), resp.chunk(stall)).await
                {
                    *conn = resp.into_conn();
                }
            } else if let Some(c) = resp.into_conn() {
                // HTTP/2: abandoning the stream keeps the connection usable.
                *conn = Some(c);
            }
            return Ok(StreamEnd::Done);
        }
        let next = tokio::select! {
            () = core.stop.cancelled() => return Err(Error::Cancelled),
            () = stat.kick.notified() => return Err(Error::Timeout("slow connection")),
            c = resp.chunk(stall) => c,
        };
        match next {
            Err(e) => {
                // Keep what arrived intact before the failure.
                flush!();
                return Err(e);
            }
            Ok(None) => {
                if !flush!() {
                    return Ok(StreamEnd::Done);
                }
                if core.total.is_none() {
                    // Unknown size: the end of the body is the end of the file.
                    core.table(|t| t.finish_open(seg))?;
                    *conn = resp.into_conn();
                    return Ok(StreamEnd::Done);
                }
                *conn = resp.into_conn();
                return Ok(StreamEnd::Short);
            }
            Ok(Some(data)) => {
                stat.net.fetch_add(data.len() as u64, Ordering::Relaxed);
                let take = (end - pos).min(data.len() as u64) as usize;
                buf.extend_from_slice(&data[..take]);
                pos += take as u64;
                if (buf.len() >= coalesce || pos >= end) && !flush!() {
                    return Ok(StreamEnd::Done);
                }
            }
        }
    }
}
