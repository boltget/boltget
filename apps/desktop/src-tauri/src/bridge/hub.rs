//! The task that feeds subscribed pages: it owns the [`Model`], applies
//! engine events and command results to it, samples progress once per tick
//! and sends each subscriber one coalesced batch per tick.
//!
//! Everything a page receives goes through this one task, in order: a
//! subscriber's first message is a snapshot taken here, so nothing that
//! happens between the snapshot and the first batch can be lost.

use std::collections::VecDeque;
use std::future::pending;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use boltget_engine::{Engine, EngineEvent};
use tauri::ipc::Channel;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use super::activity::{Activity, Notice, Tracker};
use super::dto::{
    AppSettings, DownloadError, DownloadId, DownloadRow, DownloadState, EngineMessage,
    EngineStatus, FinishAction, FinishCountdown, QueueId, QueueInfo, SegmentSpan, path_string,
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
        queues: Vec<QueueInfo>,
    },
    Status(EngineStatus),
    /// A finish action's countdown started or ended.
    Countdown(Option<FinishCountdown>),
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

/// How many forgotten private downloads keep their path for their
/// notification's buttons.
const FORGOTTEN_PATHS: usize = 32;

/// Paths of private downloads the list forgot as they completed, newest
/// last; in memory only.
type Forgotten = Arc<Mutex<VecDeque<(DownloadId, PathBuf)>>>;

/// Sends requests to the hub task. Cheap to clone.
#[derive(Debug, Clone)]
pub(crate) struct HubHandle {
    tx: mpsc::UnboundedSender<Request>,
    activity: watch::Sender<Activity>,
    notices: broadcast::Sender<Notice>,
    finishes: broadcast::Sender<(QueueId, FinishAction)>,
    forgotten: Forgotten,
}

impl HubHandle {
    /// Creates the hub; run the returned task on the async runtime.
    pub(crate) fn new(tick: Duration) -> (Self, impl Future<Output = ()> + Send + 'static) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (activity, _) = watch::channel(Activity::default());
        let (notices, _) = broadcast::channel(64);
        let (finishes, _) = broadcast::channel(16);
        let forgotten = Forgotten::default();
        let outputs = Outputs {
            activity: activity.clone(),
            notices: notices.clone(),
            finishes: finishes.clone(),
            forgotten: forgotten.clone(),
            hub: tx.downgrade(),
        };
        (
            Self {
                tx,
                activity,
                notices,
                finishes,
                forgotten,
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

    /// Queues that finished, with what they ask for.
    pub(crate) fn finishes(&self) -> broadcast::Receiver<(QueueId, FinishAction)> {
        self.finishes.subscribe()
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

    /// Where a private download the list forgot was saved.
    pub(crate) fn forgotten_path(&self, id: DownloadId) -> Option<PathBuf> {
        self.forgotten
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .rev()
            .find(|(i, _)| *i == id)
            .map(|(_, p)| p.clone())
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
        queue: QueueId(1),
        // Last until the queues are read again (within a tick).
        queue_position: i64::MAX,
        start_at: None,
        private: false,
    }
}

/// Where the hub publishes what the window does not show.
struct Outputs {
    activity: watch::Sender<Activity>,
    notices: broadcast::Sender<Notice>,
    finishes: broadcast::Sender<(QueueId, FinishAction)>,
    forgotten: Forgotten,
    /// The hub itself, for what a task it starts finds out later; weak, so
    /// the hub still stops when the bridge goes.
    hub: mpsc::WeakUnboundedSender<Request>,
}

struct Hub {
    model: Model,
    sinks: Vec<Box<dyn Sink>>,
    engine: Option<Engine>,
    status: EngineStatus,
    settings: AppSettings,
    queues: Vec<QueueInfo>,
    /// The queues, or which downloads they hold, changed: read them again
    /// at the next tick.
    queues_dirty: bool,
    countdown: Option<FinishCountdown>,
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
        queues: Vec::new(),
        queues_dirty: false,
        countdown: None,
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
                Some(Request::Ready { engine, events: rx_events, rows, settings, queues }) => {
                    events = Some(rx_events);
                    hub.engine = Some(engine);
                    hub.settings = settings;
                    hub.queues = queues;
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
            _ = ticker.tick() => {
                if hub.queues_dirty {
                    hub.refresh_queues().await;
                }
                hub.tick();
            }
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
            queues: self.queues.clone(),
            countdown: self.countdown.clone(),
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
            Request::Countdown(countdown) => {
                if self.countdown != countdown {
                    self.countdown.clone_from(&countdown);
                    self.broadcast(&EngineMessage::Countdown { countdown });
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
                if self.settings.forget_private && self.model.row(id).is_some_and(|r| r.private) {
                    self.forget(id, path.clone());
                }
            }
            EngineEvent::QueuesChanged => self.queues_dirty = true,
            EngineEvent::QueueFinished { queue, action } => {
                let _ = self
                    .outputs
                    .finishes
                    .send(((*queue).into(), (*action).into()));
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
        if let Some(mut notice) = self.tracker.state_changed(row, previous, Instant::now()) {
            // Only a download the list forgets is announced without its name.
            if let Notice::Completed { private, .. } = &mut notice {
                *private &= self.settings.forget_private;
            }
            let _ = self.outputs.notices.send(notice);
        }
    }

    /// A private download completed with "Forget private-window downloads"
    /// on: its record goes (the file stays), then its row. The notice for
    /// it went out first. Its path stays in memory for the notification.
    fn forget(&self, id: DownloadId, path: PathBuf) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        {
            let mut paths = self
                .outputs
                .forgotten
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            paths.retain(|(i, _)| *i != id);
            while paths.len() >= FORGOTTEN_PATHS {
                paths.pop_front();
            }
            paths.push_back((id, path));
        }
        let hub = self.outputs.hub.clone();
        tokio::spawn(async move {
            // The database decides: only a completed private download goes.
            let forget = engine.list().await.is_ok_and(|list| {
                list.iter().any(|s| {
                    DownloadId::from(s.id) == id
                        && s.private
                        && s.state == boltget_engine::State::Completed
                })
            });
            if !forget {
                return;
            }
            match engine.remove(id.into(), false).await {
                Ok(_) => {
                    tracing::info!(id = id.0, "forgot a completed private download");
                    if let Some(hub) = hub.upgrade() {
                        let _ = hub.send(Request::Removed(id));
                    }
                }
                Err(e) => {
                    tracing::warn!(id = id.0, error = %e, "could not forget a private download");
                }
            }
        });
    }

    /// Reads the queues and every download's place in them again, after
    /// the engine said they changed: the queues go out at once if they
    /// changed, the rows that moved with the next batch.
    async fn refresh_queues(&mut self) {
        self.queues_dirty = false;
        let Some(engine) = &self.engine else { return };
        match (engine.queues().await, engine.list().await) {
            (Ok(queues), Ok(list)) => {
                let queues: Vec<QueueInfo> = queues.iter().map(QueueInfo::from).collect();
                if queues != self.queues {
                    self.queues.clone_from(&queues);
                    if self.visible {
                        self.broadcast(&EngineMessage::Queues { queues });
                    }
                }
                for s in &list {
                    let row = DownloadRow::from(s);
                    if self.model.row(row.id).is_some() {
                        self.model.set_queue_fields(&row);
                    } else {
                        self.model.added(row);
                    }
                }
            }
            (Err(e), _) | (_, Err(e)) => tracing::warn!(error = %e, "could not read the queues"),
        }
    }

    /// Reads the list again after missing events, and sends a snapshot.
    async fn resync(&mut self) {
        // Missed events may have moved downloads between queues too.
        self.queues_dirty = true;
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
