//! The task that feeds subscribed pages: it owns the [`Model`], applies
//! engine events and command results to it, samples progress once per tick
//! and sends each subscriber one coalesced batch per tick.
//!
//! Everything a page receives goes through this one task, in order: a
//! subscriber's first message is a snapshot taken here, so nothing that
//! happens between the snapshot and the first batch can be lost.

use std::future::pending;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use boltget_engine::{Engine, EngineEvent};
use tauri::ipc::Channel;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use super::activity::{Activity, Notice, Tracker};
use super::dto::{
    AppSettings, DownloadError, DownloadId, DownloadRow, DownloadState, EngineMessage,
    EngineStatus, SegmentSpan, path_string,
};
use super::model::{Model, Sample};

/// How often progress is sampled and batches are sent: 4 Hz, the list's
/// design target (docs/ARCHITECTURE.md, "UI to backend").
pub(crate) const TICK: Duration = Duration::from_millis(250);

/// Where messages for one page go.
pub(crate) trait Sink: Send + 'static {
    /// Delivers a message; `false` if the page is gone.
    fn deliver(&self, msg: EngineMessage) -> bool;
}

impl Sink for Channel<EngineMessage> {
    fn deliver(&self, msg: EngineMessage) -> bool {
        self.send(msg).is_ok()
    }
}

/// Requests to the hub.
pub(crate) enum Request {
    /// Start streaming to a page; it first receives a snapshot.
    Subscribe(Box<dyn Sink>),
    /// The engine is open. `events` was subscribed before anything ran.
    Ready {
        engine: Engine,
        events: broadcast::Receiver<EngineEvent>,
        rows: Vec<DownloadRow>,
        settings: AppSettings,
    },
    Status(EngineStatus),
    Settings(AppSettings),
    /// A row to add or update (a download just added, or read again from the
    /// database).
    Upsert(DownloadRow),
    /// A state set without an engine event (pausing a download that was not
    /// running).
    State(DownloadId, DownloadState),
    Removed(DownloadId),
    SpeedLimit(DownloadId, Option<u64>),
    Focus(Option<DownloadId>),
    /// The window was hidden (no batches) or shown again (a snapshot).
    Visible(bool),
    /// The window gained focus: the taskbar's batch can settle.
    Acknowledge,
    /// The current rows, for `list_downloads`.
    List(oneshot::Sender<Vec<DownloadRow>>),
}

/// Sends requests to the hub task. Cheap to clone.
#[derive(Debug, Clone)]
pub(crate) struct HubHandle {
    tx: mpsc::UnboundedSender<Request>,
    activity: watch::Sender<Activity>,
    notices: broadcast::Sender<Notice>,
}

impl HubHandle {
    /// Creates the hub; run the returned task on the async runtime.
    pub(crate) fn new(tick: Duration) -> (Self, impl Future<Output = ()> + Send + 'static) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (activity, _) = watch::channel(Activity::default());
        let (notices, _) = broadcast::channel(64);
        let outputs = Outputs {
            activity: activity.clone(),
            notices: notices.clone(),
        };
        (
            Self {
                tx,
                activity,
                notices,
            },
            run(rx, tick, outputs),
        )
    }

    /// The aggregate for the tray and the taskbar, updated every tick.
    pub(crate) fn activity(&self) -> watch::Receiver<Activity> {
        self.activity.subscribe()
    }

    /// Completions and failures, for notifications.
    pub(crate) fn notices(&self) -> broadcast::Receiver<Notice> {
        self.notices.subscribe()
    }

    pub(crate) fn send(&self, req: Request) {
        // Fails only once the hub has stopped, at exit.
        let _ = self.tx.send(req);
    }

    pub(crate) async fn list(&self) -> Vec<DownloadRow> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::List(tx));
        rx.await.unwrap_or_default()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A row for a download the engine just announced.
pub(crate) fn new_row(id: DownloadId, url: String) -> DownloadRow {
    DownloadRow {
        id,
        state: DownloadState::Queued,
        url,
        file_name: None,
        path: None,
        total: None,
        downloaded: 0,
        bps: 0,
        connections: 0,
        resumable: false,
        speed_limit: None,
        error: None,
        created_at: now_ms(),
        completed_at: None,
    }
}

/// Where the hub publishes what the window does not show.
struct Outputs {
    activity: watch::Sender<Activity>,
    notices: broadcast::Sender<Notice>,
}

struct Hub {
    model: Model,
    sinks: Vec<Box<dyn Sink>>,
    engine: Option<Engine>,
    status: EngineStatus,
    settings: AppSettings,
    visible: bool,
    tracker: Tracker,
    outputs: Outputs,
}

async fn run(mut rx: mpsc::UnboundedReceiver<Request>, tick: Duration, outputs: Outputs) {
    let mut hub = Hub {
        model: Model::default(),
        sinks: Vec::new(),
        engine: None,
        status: EngineStatus::Starting,
        settings: AppSettings::default(),
        visible: true,
        tracker: Tracker::default(),
        outputs,
    };
    let mut events: Option<broadcast::Receiver<EngineEvent>> = None;
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            req = rx.recv() => match req {
                Some(Request::Ready { engine, events: rx_events, rows, settings }) => {
                    events = Some(rx_events);
                    hub.engine = Some(engine);
                    hub.settings = settings;
                    hub.status = EngineStatus::Ready;
                    hub.model.load(rows);
                    hub.broadcast_snapshot();
                }
                Some(req) => hub.handle(req),
                None => break,
            },
            event = next_event(&mut events) => match event {
                Ok(event) => hub.apply(&event),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "engine events were missed; resyncing the list");
                    hub.resync().await;
                }
                Err(broadcast::error::RecvError::Closed) => events = None,
            },
            _ = ticker.tick() => hub.tick(),
        }
    }
}

async fn next_event(
    rx: &mut Option<broadcast::Receiver<EngineEvent>>,
) -> Result<EngineEvent, broadcast::error::RecvError> {
    match rx {
        Some(rx) => rx.recv().await,
        None => pending().await,
    }
}

impl Hub {
    fn snapshot(&self) -> EngineMessage {
        EngineMessage::Snapshot {
            status: self.status.clone(),
            downloads: self.model.snapshot(),
            settings: self.settings.clone(),
        }
    }

    fn broadcast(&mut self, msg: &EngineMessage) {
        self.sinks.retain(|s| s.deliver(msg.clone()));
    }

    fn broadcast_snapshot(&mut self) {
        let msg = self.snapshot();
        self.broadcast(&msg);
        self.model.discard_pending();
    }

    fn handle(&mut self, req: Request) {
        match req {
            Request::Subscribe(sink) => {
                if sink.deliver(self.snapshot()) {
                    self.sinks.push(sink);
                }
            }
            // Handled in `run`, which owns the event receiver.
            Request::Ready { .. } => {}
            Request::Status(status) => {
                if self.status != status {
                    self.status = status.clone();
                    self.broadcast(&EngineMessage::Status { status });
                }
            }
            Request::Settings(settings) => {
                if self.settings != settings {
                    self.settings = settings.clone();
                    self.broadcast(&EngineMessage::Settings { settings });
                }
            }
            Request::Upsert(row) => {
                if self.model.row(row.id).is_some() {
                    self.model.replace(row);
                } else {
                    self.model.added(row);
                }
            }
            Request::State(id, state) => {
                if self.model.row(id).is_some_and(|r| r.state != state) {
                    let error = self.model.row(id).and_then(|r| r.error.clone());
                    self.model.state_changed(id, state, error);
                    // The user's own doing (a pause): tracked, not announced.
                    if let Some(row) = self.model.row(id) {
                        self.tracker.state_changed(row, None, Instant::now());
                    }
                }
            }
            Request::Removed(id) => {
                self.model.removed(id);
                self.tracker.removed(id);
            }
            Request::Acknowledge => self.tracker.acknowledge(&self.model.rows()),
            Request::SpeedLimit(id, limit) => self.model.set_speed_limit(id, limit),
            Request::Focus(id) => self.model.set_focus(id),
            Request::Visible(visible) => {
                let shown = visible && !self.visible;
                self.visible = visible;
                if shown {
                    self.broadcast_snapshot();
                }
            }
            Request::List(reply) => {
                let _ = reply.send(self.model.snapshot());
            }
        }
    }

    fn apply(&mut self, event: &EngineEvent) {
        match event {
            EngineEvent::Added { id, url } => {
                self.model.added(new_row((*id).into(), url.clone()));
            }
            EngineEvent::Probed { id, info } => {
                self.model.probed(
                    (*id).into(),
                    info.path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned()),
                    path_string(&info.path),
                    info.probe.size,
                    info.probe.resumable(),
                );
            }
            EngineEvent::StateChanged { id, state, error } => {
                let id = (*id).into();
                let previous = self.model.row(id).map(|r| r.state);
                self.model.state_changed(
                    id,
                    (*state).into(),
                    error.as_ref().map(DownloadError::from),
                );
                self.track(id, previous);
            }
            EngineEvent::Resumed {
                id,
                downloaded,
                total,
            } => self.model.resumed((*id).into(), *downloaded, *total),
            EngineEvent::SegmentsChanged { id } => self.model.segments_changed((*id).into()),
            EngineEvent::Completed { id, path } => {
                let id = (*id).into();
                let previous = self.model.row(id).map(|r| r.state);
                self.model.completed(id, path_string(path), now_ms());
                self.track(id, previous);
            }
            // Events added to the engine later: the next resync covers them.
            _ => {}
        }
    }

    /// Follows a state change for the tray, the taskbar and notifications.
    fn track(&mut self, id: DownloadId, previous: Option<DownloadState>) {
        let Some(row) = self.model.row(id) else {
            return;
        };
        // A completion arrives as a state change and as its own event; the
        // second finds nothing new.
        if previous == Some(row.state) && !row.state.is_active() {
            return;
        }
        if let Some(notice) = self.tracker.state_changed(row, previous, Instant::now()) {
            let _ = self.outputs.notices.send(notice);
        }
    }

    /// Reads the list again after missing events, and sends a snapshot.
    async fn resync(&mut self) {
        let Some(engine) = &self.engine else { return };
        match engine.list().await {
            Ok(list) => {
                self.model.load(list.iter().map(DownloadRow::from));
                self.broadcast_snapshot();
            }
            Err(e) => tracing::warn!(error = %e, "could not read the download list"),
        }
    }

    fn tick(&mut self) {
        let samples = match &self.engine {
            Some(engine) => {
                let ids: Vec<_> = self
                    .model
                    .active_ids()
                    .into_iter()
                    .map(Into::into)
                    .collect();
                engine
                    .progress(&ids)
                    .iter()
                    .map(|p| Sample {
                        id: p.id.into(),
                        downloaded: p.downloaded,
                        total: p.total,
                        connections: p.connections,
                        segments: p.segments.iter().map(SegmentSpan::from).collect(),
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        // The model is kept current while hidden, so speeds stay right; the
        // batch is dropped and a snapshot follows when the window shows.
        if let Some(batch) = self.model.tick(samples, Instant::now())
            && self.visible
        {
            self.broadcast(&batch);
        }
        let activity = self.tracker.activity(&self.model.rows());
        self.outputs.activity.send_if_modified(|a| {
            let changed = *a != activity;
            *a = activity;
            changed
        });
    }
}
