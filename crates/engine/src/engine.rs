//! The public engine API.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::EngineConfig;
use crate::download::{self, Env, Shared};
use crate::error::{Error, Result};
use crate::events::{Completed, DownloadId, DownloadInfo, EngineEvent, Progress, State};
use crate::filename::{self, NameSources};
use crate::http::{self, Connector, Origin};
use crate::probe::{self, ProbeKind, ProbeResult, RangeSupport};
use crate::request::{RequestContext, redact_url};

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
}

impl AddRequest {
    /// A request with default options.
    pub fn new(ctx: RequestContext, destination: Destination) -> Self {
        Self {
            ctx,
            destination,
            connections: None,
            conflict: ConflictPolicy::Rename,
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
}

/// Holds one per-host and one global connection slot.
#[derive(Debug)]
pub(crate) struct ConnPermit {
    _host: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

/// Per-host and global connection caps, shared by all downloads.
#[derive(Debug, Clone)]
pub(crate) struct Limits {
    global: Arc<Semaphore>,
    per_host: Arc<Mutex<HashMap<Origin, Arc<Semaphore>>>>,
    max_per_host: usize,
}

impl Limits {
    fn new(max_per_host: u32, max_total: u32) -> Self {
        Self {
            global: Arc::new(Semaphore::new(max_total as usize)),
            per_host: Arc::default(),
            max_per_host: max_per_host as usize,
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

#[derive(Debug)]
struct Inner {
    cfg: Arc<EngineConfig>,
    env: Env,
    events: broadcast::Sender<EngineEvent>,
    next_id: AtomicU64,
    downloads: Mutex<HashMap<DownloadId, Arc<Shared>>>,
    root: CancellationToken,
}

/// The download engine. Cheap to clone; all clones share state.
///
/// Phase 1 keeps everything in memory: there is no database, so downloads do not
/// survive the process (checkpointing and resume arrive in phase 2).
#[derive(Debug, Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    /// Creates an engine. Must be called inside a Tokio runtime only when
    /// downloads are added; construction itself does not need one.
    pub fn new(config: EngineConfig) -> Result<Self> {
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
                },
                cfg,
                events,
                next_id: AtomicU64::new(1),
                downloads: Mutex::new(HashMap::new()),
                root: CancellationToken::new(),
            }),
        })
    }

    /// The effective configuration.
    pub fn config(&self) -> &EngineConfig {
        &self.inner.cfg
    }

    /// Probes a URL without downloading it (for an "add" dialog): size, range
    /// support, validators and the resolved file name. Costs one request, or two
    /// when the server answers `200` and range support must be verified.
    pub async fn probe(&self, ctx: &RequestContext) -> Result<ProbeResult> {
        let connector = Connector::new(self.inner.env.tls.clone(), self.inner.cfg.clone());
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

    /// Starts a download. Requires a Tokio runtime.
    pub fn add(&self, req: AddRequest) -> DownloadHandle {
        let id = DownloadId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let shared = Arc::new(Shared::new(
            id,
            self.inner.root.child_token(),
            self.inner.events.clone(),
        ));
        self.inner
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, shared.clone());
        let _ = self.inner.events.send(EngineEvent::Added {
            id,
            url: redact_url(req.ctx.url()),
        });
        let env = self.inner.env.clone();
        let task_shared = shared.clone();
        let inner = self.inner.clone();
        let join = tokio::spawn(async move {
            let r = download::run(task_shared, env, req).await;
            inner
                .downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            r
        });
        DownloadHandle { shared, join }
    }

    /// Subscribes to engine events. Slow subscribers miss events (`Lagged`)
    /// rather than slowing downloads down; use [`Engine::progress`] for state.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// Progress snapshots for the given running downloads (unknown ids are skipped).
    pub fn progress(&self, ids: &[DownloadId]) -> Vec<Progress> {
        let map = self
            .inner
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ids.iter()
            .filter_map(|id| map.get(id).map(|s| s.progress()))
            .collect()
    }

    /// Cancels a running download. Returns false if it is not running.
    pub fn cancel(&self, id: DownloadId) -> bool {
        let map = self
            .inner
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(&id).is_some_and(|s| {
            s.cancel.cancel();
            true
        })
    }

    /// Cancels every download. Partial files are flushed and kept.
    pub fn shutdown(&self) {
        self.inner.root.cancel();
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

    /// Destination details, once the probe succeeded.
    pub fn info(&self) -> Option<&DownloadInfo> {
        self.shared.info()
    }

    /// Requests cancellation. The partial file is flushed and kept.
    pub fn cancel(&self) {
        self.shared.cancel.cancel();
    }

    /// A token that cancels this download when cancelled.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.shared.cancel.clone()
    }

    /// Waits for the download to finish.
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
