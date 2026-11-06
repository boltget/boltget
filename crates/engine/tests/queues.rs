//! Queues and the scheduler against the local test server (docs/ENGINE.md,
//! "Queues"): the order and the limit, slots freeing, stopping and starting a
//! queue, moving downloads, persistence across a reopen and a crash, start
//! times with a clock the test moves, daily times, and the finish action.

// Helpers outside `#[test]` functions panic on failure just like the tests do.
#![allow(clippy::unwrap_used)]

mod support;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use boltget_engine::{
    AddRequest, Clock, DEFAULT_QUEUE, DailyWindow, DownloadId, Engine, EngineEvent, FinishAction,
    QueueChange, QueueId, Shift, State, SystemClock,
};
use boltget_testserver::TestServer;
use support::{KIB, MIB, cfg, request};
use tokio::sync::broadcast;

/// A clock the test moves by hand, in UTC.
#[derive(Debug)]
struct ManualClock(Mutex<SystemTime>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(SystemTime::now())))
    }

    fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap()
    }

    fn utc_offset(&self, _at: SystemTime) -> i32 {
        0
    }
}

const WAIT: Duration = Duration::from_secs(60);

fn open(dir: &Path) -> Engine {
    let mut c = cfg();
    c.initial_connections = 2;
    Engine::open(c, dir).unwrap()
}

/// A download of `size` bytes at about `bps` per connection.
fn slow(srv: &TestServer, seed: u64, size: u64, bps: u64, dir: &Path) -> AddRequest {
    request(
        &srv.url(&format!("/data?size={size}&seed={seed}&throttle={bps}")),
        dir,
    )
}

async fn state_of(engine: &Engine, id: DownloadId) -> State {
    engine
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == id)
        .map(|s| s.state)
        .unwrap()
}

async fn wait_state(engine: &Engine, id: DownloadId, want: impl Fn(State) -> bool) -> State {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let s = state_of(engine, id).await;
        if want(s) {
            return s;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "download {id} is still {s}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Follows events: which downloads started (in order), how many ran at
/// most at once, and which queues finished.
#[derive(Debug, Default)]
struct Watch {
    started: Vec<DownloadId>,
    active: HashSet<DownloadId>,
    most: usize,
    finished: Vec<(QueueId, FinishAction)>,
}

impl Watch {
    fn see(&mut self, e: &EngineEvent) {
        match e {
            EngineEvent::StateChanged { id, state, .. } => {
                if state.is_active() {
                    if self.active.insert(*id) && !self.started.contains(id) {
                        self.started.push(*id);
                    }
                    self.most = self.most.max(self.active.len());
                } else {
                    self.active.remove(id);
                }
            }
            EngineEvent::Completed { id, .. } => {
                self.active.remove(id);
            }
            EngineEvent::QueueFinished { queue, action } => self.finished.push((*queue, *action)),
            _ => {}
        }
    }
}

fn watch(engine: &Engine) -> Arc<Mutex<Watch>> {
    let watch = Arc::new(Mutex::new(Watch::default()));
    let mut rx = engine.subscribe();
    let w = watch.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(e) => w.lock().unwrap().see(&e),
                Err(broadcast::error::RecvError::Lagged(_)) => panic!("events lagged"),
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
    watch
}

async fn limit(engine: &Engine, queue: QueueId, n: u32) {
    let mut change = QueueChange::default();
    change.max_concurrent = Some(n);
    engine.update_queue(queue, change).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queue_runs_its_downloads_in_order_within_its_limit() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = open(state.path());
    let seen = watch(&engine);
    limit(&engine, DEFAULT_QUEUE, 2).await;
    let mut ids = Vec::new();
    for seed in 0..5 {
        let req = slow(&srv, seed, 256 * KIB, 256 * KIB, dir.path());
        ids.push(engine.enqueue(req).await.unwrap());
    }
    // Without the scheduler nothing starts.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(state_of(&engine, ids[0]).await, State::Queued);

    engine.start_queues(Arc::new(SystemClock));
    for &id in &ids {
        wait_state(&engine, id, |s| s == State::Completed).await;
    }
    let (started, most) = {
        let w = seen.lock().unwrap();
        (w.started.clone(), w.most)
    };
    assert_eq!(started, ids, "started in queue order");
    assert_eq!(most, 2, "two at once, never more");
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_queue_puts_its_downloads_back_and_starting_continues_them() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = open(state.path());
    engine.start_queues(Arc::new(SystemClock));
    limit(&engine, DEFAULT_QUEUE, 1).await;
    let a = engine
        .enqueue(slow(&srv, 1, 3 * MIB, 512 * KIB, dir.path()))
        .await
        .unwrap();
    let b = engine
        .enqueue(slow(&srv, 2, 64 * KIB, 512 * KIB, dir.path()))
        .await
        .unwrap();
    wait_state(&engine, a, State::is_active).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state_of(&engine, b).await, State::Queued, "the limit holds");

    engine.stop_queue(DEFAULT_QUEUE).await.unwrap();
    assert_eq!(
        state_of(&engine, a).await,
        State::Queued,
        "back in its place"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state_of(&engine, a).await, State::Queued);
    assert_eq!(state_of(&engine, b).await, State::Queued);
    assert!(!engine.queues().await.unwrap()[0].started);

    // A download started by hand runs in a stopped queue.
    engine
        .resume(b, boltget_engine::ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();

    engine.start_queue(DEFAULT_QUEUE).await.unwrap();
    wait_state(&engine, a, |s| s == State::Completed).await;
    let s = engine
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == a)
        .unwrap();
    let path = s.path.unwrap();
    support::assert_content_at(&path, 1, 3 * MIB);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_move_within_and_between_queues() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = open(state.path());
    let mut ids = Vec::new();
    for seed in 0..4 {
        let req = slow(&srv, seed, 1000, 0, dir.path());
        ids.push(engine.enqueue(req).await.unwrap());
    }
    let order = |queue: QueueId| {
        let engine = engine.clone();
        async move {
            let mut list: Vec<_> = engine
                .list()
                .await
                .unwrap()
                .into_iter()
                .filter(|s| s.queue == queue)
                .collect();
            list.sort_by_key(|s| s.queue_position);
            list.into_iter().map(|s| s.id).collect::<Vec<_>>()
        }
    };
    assert_eq!(order(DEFAULT_QUEUE).await, ids);
    engine.shift(ids[2], Shift::Up).await.unwrap();
    assert_eq!(order(DEFAULT_QUEUE).await, [ids[0], ids[2], ids[1], ids[3]]);
    engine.shift(ids[3], Shift::Top).await.unwrap();
    assert_eq!(order(DEFAULT_QUEUE).await, [ids[3], ids[0], ids[2], ids[1]]);
    engine.shift(ids[3], Shift::Bottom).await.unwrap();
    engine.shift(ids[0], Shift::Down).await.unwrap();
    assert_eq!(order(DEFAULT_QUEUE).await, [ids[2], ids[0], ids[1], ids[3]]);

    let night = engine.create_queue("Night").await.unwrap();
    assert!(
        engine.create_queue(" night ").await.is_err(),
        "names are unique"
    );
    assert!(engine.create_queue("main").await.is_err(), "Main is taken");
    assert_eq!(engine.queue_named("NIGHT").await.unwrap().id, night.id);
    assert_eq!(engine.queue_named("main").await.unwrap().id, DEFAULT_QUEUE);
    engine
        .move_downloads(&[ids[1], ids[3]], night.id, None)
        .await
        .unwrap();
    assert_eq!(order(night.id).await, [ids[1], ids[3]]);
    engine
        .move_downloads(&[ids[2]], night.id, Some(ids[3]))
        .await
        .unwrap();
    assert_eq!(order(night.id).await, [ids[1], ids[2], ids[3]]);
    assert_eq!(order(DEFAULT_QUEUE).await, [ids[0]]);
    let listed = engine.list().await.unwrap();
    assert_eq!(listed[1].queue_name, "Night");
    assert_eq!(listed[0].queue_name, "Main");

    // Deleting a queue moves its downloads to the end of Main, in order.
    assert!(engine.delete_queue(DEFAULT_QUEUE).await.is_err());
    engine.delete_queue(night.id).await.unwrap();
    assert_eq!(order(DEFAULT_QUEUE).await, [ids[0], ids[1], ids[2], ids[3]]);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queues_and_their_order_survive_a_reopen() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = open(state.path());
    let night = engine.create_queue("Night").await.unwrap();
    let mut change = QueueChange::default();
    change.max_concurrent = Some(1);
    change.on_finish = Some(FinishAction::Sleep);
    change.window = Some(DailyWindow {
        start: Some(23 * 60),
        stop: Some(6 * 60),
    });
    engine.update_queue(night.id, change).await.unwrap();
    engine.stop_queue(night.id).await.unwrap();
    let a = engine
        .enqueue(slow(&srv, 1, 1000, 0, dir.path()).queue(night.id))
        .await
        .unwrap();
    let b = engine
        .enqueue(slow(&srv, 2, 1000, 0, dir.path()).queue(night.id))
        .await
        .unwrap();
    engine.shift(b, Shift::Top).await.unwrap();
    engine.shutdown().await;

    let engine = open(state.path());
    let queues = engine.queues().await.unwrap();
    assert_eq!(queues.len(), 2);
    let q = &queues[1];
    assert_eq!(q.name.as_deref(), Some("Night"));
    assert_eq!(q.max_concurrent, 1);
    assert!(!q.started);
    assert_eq!(q.on_finish, FinishAction::Sleep);
    assert_eq!(q.window.start, Some(23 * 60));
    let list = engine.list().await.unwrap();
    let pos = |id| list.iter().find(|s| s.id == id).unwrap().queue_position;
    assert!(pos(b) < pos(a));
    assert!(
        list.iter()
            .all(|s| s.state == State::Queued && s.queue == night.id)
    );
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_crash_the_interrupted_download_goes_first_and_the_limit_holds() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = open(state.path());
    engine.start_queues(Arc::new(SystemClock));
    limit(&engine, DEFAULT_QUEUE, 1).await;
    let a = engine
        .enqueue(slow(&srv, 1, 3 * MIB, 256 * KIB, dir.path()))
        .await
        .unwrap();
    let b = engine
        .enqueue(slow(&srv, 2, 64 * KIB, 0, dir.path()))
        .await
        .unwrap();
    wait_state(&engine, a, |s| s == State::Downloading).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    // A quit puts a running download of a started queue back in its place.
    engine.shutdown().await;
    let engine = open(state.path());
    assert_eq!(state_of(&engine, a).await, State::Queued);
    // A kill leaves it running in the database instead.
    let conn = rusqlite::Connection::open(state.path().join(boltget_engine::DB_FILE)).unwrap();
    conn.execute(
        "UPDATE download SET state = 'downloading' WHERE id = ?1",
        [i64::try_from(a.0).unwrap()],
    )
    .unwrap();
    drop(conn);
    engine.shutdown().await;

    let engine = open(state.path());
    assert_eq!(engine.recovered(), [a]);
    let seen = watch(&engine);
    // The app continues what was interrupted, then turns the queues on.
    let h = engine
        .resume(a, boltget_engine::ResumeOptions::default())
        .await
        .unwrap();
    engine.start_queues(Arc::new(SystemClock));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state_of(&engine, b).await, State::Queued, "the limit holds");
    h.wait().await.unwrap();
    wait_state(&engine, b, |s| s == State::Completed).await;
    assert_eq!(seen.lock().unwrap().started, [a, b]);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn start_times_wait_for_the_clock_and_survive_a_restart() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new();
    let engine = open(state.path());
    engine.start_queues(clock.clone());
    engine.stop_queue(DEFAULT_QUEUE).await.unwrap();
    let in_an_hour = clock.now() + Duration::from_secs(3600);
    let a = engine
        .enqueue(slow(&srv, 1, 1000, 0, dir.path()).start_at(in_an_hour))
        .await
        .unwrap();
    let b = engine
        .enqueue(slow(&srv, 2, 1000, 0, dir.path()).start_at(in_an_hour))
        .await
        .unwrap();
    let listed = engine.list().await.unwrap();
    assert!(listed.iter().all(|s| s.start_at.is_some()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state_of(&engine, a).await, State::Queued);
    clock.advance(Duration::from_secs(3601));
    engine.wake_queues();
    // In a stopped queue, at its time.
    wait_state(&engine, a, |s| s == State::Completed).await;
    wait_state(&engine, b, |s| s == State::Completed).await;
    let done = engine.list().await.unwrap();
    assert!(done.iter().all(|s| s.start_at.is_none()), "spent");

    // A time that passed while the engine was closed starts on the next open.
    let c = engine
        .enqueue(slow(&srv, 3, 1000, 0, dir.path()).start_at(clock.now() + Duration::from_secs(60)))
        .await
        .unwrap();
    engine.shutdown().await;
    let engine = open(state.path());
    assert_eq!(state_of(&engine, c).await, State::Queued);
    let later = ManualClock::new();
    later.advance(Duration::from_secs(7200));
    engine.start_queues(later);
    wait_state(&engine, c, |s| s == State::Completed).await;

    // Giving a paused download a time makes it wait for it.
    let d = engine
        .add_paused(slow(&srv, 4, 1000, 0, dir.path()))
        .unwrap();
    let st = engine
        .set_start_at(d, Some(SystemTime::now() + Duration::from_secs(36_000)))
        .await
        .unwrap();
    assert_eq!(st, State::Queued);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daily_start_time_starts_a_stopped_queue() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new();
    let engine = open(state.path());
    let night = engine.create_queue("Night").await.unwrap();
    engine.stop_queue(night.id).await.unwrap();
    let a = engine
        .enqueue(slow(&srv, 1, 1000, 0, dir.path()).queue(night.id))
        .await
        .unwrap();
    engine.start_queues(clock.clone());
    // Start in two minutes (UTC, as the clock says), stop in an hour.
    let minute_now = (clock
        .now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 60)
        % 1440;
    let at = |m: u64| u16::try_from((minute_now + m) % 1440).unwrap();
    let mut change = QueueChange::default();
    change.window = Some(DailyWindow {
        start: Some(at(2)),
        stop: Some(at(60)),
    });
    engine.update_queue(night.id, change).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state_of(&engine, a).await, State::Queued);
    clock.advance(Duration::from_secs(180));
    engine.wake_queues();
    wait_state(&engine, a, |s| s == State::Completed).await;
    assert!(engine.queue_named("Night").await.unwrap().started);
    // At the stop time it stops again.
    clock.advance(Duration::from_secs(3600));
    engine.wake_queues();
    let deadline = tokio::time::Instant::now() + WAIT;
    while engine.queue_named("Night").await.unwrap().started {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the queue did not stop"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_queue_asks_for_its_action_once() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = open(state.path());
    let seen = watch(&engine);
    let mut change = QueueChange::default();
    change.max_concurrent = Some(2);
    change.on_finish = Some(FinishAction::ShutDown);
    engine.update_queue(DEFAULT_QUEUE, change).await.unwrap();
    engine.start_queues(Arc::new(SystemClock));
    let mut ids = Vec::new();
    for seed in 0..3 {
        let req = slow(&srv, seed, 128 * KIB, 512 * KIB, dir.path());
        ids.push(engine.enqueue(req).await.unwrap());
    }
    for &id in &ids {
        wait_state(&engine, id, |s| s == State::Completed).await;
    }
    let deadline = tokio::time::Instant::now() + WAIT;
    while seen.lock().unwrap().finished.is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "no finish");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        seen.lock().unwrap().finished,
        [(DEFAULT_QUEUE, FinishAction::ShutDown)],
        "once"
    );

    // Pausing the last running download does not finish the queue.
    let p = engine
        .enqueue(slow(&srv, 9, 4 * MIB, 128 * KIB, dir.path()))
        .await
        .unwrap();
    wait_state(&engine, p, State::is_active).await;
    engine.pause(p).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(seen.lock().unwrap().finished.len(), 1);
    engine.shutdown().await;
}
