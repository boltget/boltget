//! The bridge against a real engine and the local test server: batched
//! streaming, recovery on start, and waiting out another owner of the
//! database. No webview is involved; a test sink stands in for the page.

use std::path::Path;
use std::time::Duration;

use std::sync::{Arc, Mutex};

use boltget_engine::{
    AddRequest, DEFAULT_QUEUE, Destination, DownloadHandle, Engine, EngineConfig, QueueChange,
    RequestContext,
};
use boltget_testserver::TestServer;
use tokio::sync::mpsc;

use super::dto::{
    DownloadEvent, DownloadId, DownloadRow, DownloadState, EngineMessage, EngineStatus,
    FinishAction,
};
use super::{Bridge, Sink, to_resume};
use crate::power::Power;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;
const TEST_TICK: Duration = Duration::from_millis(50);

struct TestSink(mpsc::UnboundedSender<EngineMessage>);

impl Sink for TestSink {
    fn deliver(&self, msg: EngineMessage) -> bool {
        self.0.send(msg).is_ok()
    }
}

fn cfg() -> EngineConfig {
    let mut c = EngineConfig::default();
    c.min_split_size = 64 * KIB;
    c.segment_align = 4 * KIB;
    c.worker_stagger = Duration::from_millis(5);
    c.flush_interval = Duration::from_millis(50);
    c.backoff_base = Duration::from_millis(10);
    c.backoff_max = Duration::from_millis(100);
    c.disk_margin = 0;
    c.initial_connections = 4;
    c
}

fn bridge() -> Bridge {
    let (bridge, hub) = Bridge::new(TEST_TICK);
    tokio::spawn(hub);
    bridge
}

fn subscribe(bridge: &Bridge) -> mpsc::UnboundedReceiver<EngineMessage> {
    let (tx, rx) = mpsc::unbounded_channel();
    bridge.subscribe(Box::new(TestSink(tx)));
    rx
}

async fn next(rx: &mut mpsc::UnboundedReceiver<EngineMessage>) -> EngineMessage {
    tokio::time::timeout(Duration::from_secs(60), rx.recv())
        .await
        .expect("a message in time")
        .expect("the hub is running")
}

/// Receives until `done` accepts a message; returns everything received.
async fn until(
    rx: &mut mpsc::UnboundedReceiver<EngineMessage>,
    mut done: impl FnMut(&EngineMessage) -> bool,
) -> Vec<EngineMessage> {
    let mut seen = Vec::new();
    loop {
        let msg = next(rx).await;
        let stop = done(&msg);
        seen.push(msg);
        if stop {
            return seen;
        }
    }
}

fn completed(msg: &EngineMessage, id: DownloadId) -> bool {
    matches!(msg, EngineMessage::Batch { events, .. }
        if events.iter().any(|e| matches!(e, DownloadEvent::Completed { id: c, .. } if *c == id)))
}

fn add(engine: &Engine, url: &str, dir: &Path) -> DownloadHandle {
    engine.add(AddRequest::new(
        RequestContext::new(url).unwrap(),
        Destination::Directory(dir.to_path_buf()),
    ))
}

async fn until_durable(h: &DownloadHandle, bytes: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while h.progress().durable < bytes {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no durable progress"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_a_snapshot_then_coalesced_batches() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge();
    let mut rx = subscribe(&bridge);
    match next(&mut rx).await {
        EngineMessage::Snapshot {
            status, downloads, ..
        } => {
            assert_eq!(status, EngineStatus::Starting);
            assert_eq!(downloads, Vec::<DownloadRow>::new());
        }
        other => panic!("first message must be a snapshot: {other:?}"),
    }
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    assert!(matches!(
        next(&mut rx).await,
        EngineMessage::Snapshot {
            status: EngineStatus::Ready,
            ..
        }
    ));

    let size = 2 * MIB + 5;
    let engine = bridge.engine().unwrap().clone();
    let h = add(
        &engine,
        &srv.url(&format!("/data?size={size}&seed=3&throttle=1048576")),
        dir.path(),
    );
    let id = DownloadId::from(h.id());
    bridge.focus(Some(id));
    let started = std::time::Instant::now();
    let seen = until(&mut rx, |m| completed(m, id)).await;
    let elapsed = started.elapsed();

    let mut progress_items = 0;
    let mut segment_maps = 0;
    let mut last_row: Option<DownloadRow> = None;
    for msg in &seen {
        let EngineMessage::Batch { rows, progress, .. } = msg else {
            panic!("only batches after the snapshot: {msg:?}");
        };
        for p in progress {
            assert_eq!(p.id, id);
            progress_items += 1;
            segment_maps += usize::from(p.segments.is_some());
        }
        if let Some(r) = rows.iter().rfind(|r| r.id == id) {
            last_row = Some(r.clone());
        }
    }
    // One message per tick at most: the stream is paced by the tick, not by
    // network reads.
    let ticks = elapsed.as_millis() / TEST_TICK.as_millis() + 2;
    assert!(
        seen.len() as u128 <= ticks,
        "{} messages in {elapsed:?}",
        seen.len()
    );
    assert!(progress_items >= 3, "progress streamed: {progress_items}");
    assert!(segment_maps >= 1, "the focused download's map streamed");
    let row = last_row.expect("the row was sent");
    assert_eq!(row.state, DownloadState::Completed);
    assert_eq!(row.downloaded, size);
    assert_eq!(row.bps, 0);
    assert!(row.file_name.is_some());

    // A second page gets the same state as a snapshot.
    let mut other = subscribe(&bridge);
    match next(&mut other).await {
        EngineMessage::Snapshot { downloads, .. } => {
            assert_eq!(downloads.len(), 1);
            assert_eq!(downloads[0].state, DownloadState::Completed);
        }
        m => panic!("{m:?}"),
    }
    assert_eq!(bridge.list().await.len(), 1);

    // Removing it streams a removal.
    engine.remove(id.into(), true).await.unwrap();
    bridge.removed(id);
    let seen = until(
        &mut rx,
        |m| matches!(m, EngineMessage::Batch { removed, .. } if removed.contains(&id)),
    )
    .await;
    assert_ne!(seen.len(), 0);
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_page_gets_no_batches_then_a_snapshot() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge();
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let mut rx = subscribe(&bridge);
    assert!(matches!(
        next(&mut rx).await,
        EngineMessage::Snapshot { .. }
    ));
    bridge.visible(false);
    let engine = bridge.engine().unwrap().clone();
    let h = add(&engine, &srv.url("/data?size=300000&seed=4"), dir.path());
    h.wait().await.unwrap();
    tokio::time::sleep(TEST_TICK * 4).await;
    assert!(rx.try_recv().is_err(), "nothing while hidden");
    bridge.visible(true);
    match next(&mut rx).await {
        EngineMessage::Snapshot { downloads, .. } => {
            assert_eq!(downloads[0].state, DownloadState::Completed);
        }
        m => panic!("{m:?}"),
    }
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_downloads_resume_on_start_and_paused_ones_do_not() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let size = 3 * MIB + 17;

    // A first run with two downloads under way.
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let a = add(
        &engine,
        &srv.url(&format!("/data?size={size}&seed=11&throttle=262144")),
        dir.path(),
    );
    let b = add(
        &engine,
        &srv.url(&format!("/data?size={size}&seed=12&throttle=262144")),
        dir.path(),
    );
    let (ida, idb) = (a.id(), b.id());
    until_durable(&a, 128 * KIB).await;
    until_durable(&b, 128 * KIB).await;
    // The user pauses `b`; then the app is killed while `a` runs. A kill
    // leaves `a` marked as downloading in the database.
    engine.pause(idb).await.unwrap();
    engine.shutdown().await;
    let conn = rusqlite::Connection::open(state.path().join(boltget_engine::DB_FILE)).unwrap();
    conn.execute(
        "UPDATE download SET state = 'downloading' WHERE id = ?1",
        [i64::try_from(ida.0).unwrap()],
    )
    .unwrap();
    drop(conn);

    // The next start resumes `a` only.
    let bridge = bridge();
    let mut rx = subscribe(&bridge);
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    until(&mut rx, |m| completed(m, ida.into())).await;
    let rows = bridge.list().await;
    let row = |id: boltget_engine::DownloadId| {
        rows.iter()
            .find(|r| r.id == DownloadId::from(id))
            .unwrap()
            .clone()
    };
    assert_eq!(row(ida).state, DownloadState::Completed);
    assert_eq!(row(ida).downloaded, size);
    assert_eq!(row(idb).state, DownloadState::Paused);
    let file = row(ida).path.expect("a completed download has a path");
    assert_eq!(std::fs::metadata(&file).unwrap().len(), size);
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn waits_while_another_process_owns_the_database() {
    let state = tempfile::tempdir().unwrap();
    let cli = Engine::open(cfg(), state.path()).unwrap();
    let bridge = std::sync::Arc::new(bridge());
    let mut rx = subscribe(&bridge);
    assert!(matches!(
        next(&mut rx).await,
        EngineMessage::Snapshot { .. }
    ));
    let opener = {
        let bridge = bridge.clone();
        let dir = state.path().to_path_buf();
        tokio::spawn(async move { bridge.open(dir, cfg(), Duration::from_millis(50)).await })
    };
    match next(&mut rx).await {
        EngineMessage::Status {
            status: EngineStatus::Locked { dir },
        } => assert_ne!(dir, ""),
        m => panic!("{m:?}"),
    }
    assert!(bridge.engine().is_err());
    assert_eq!(bridge.engine().unwrap_err().key, "error.engine_unavailable");
    // The CLI finishes; the app takes over.
    cli.shutdown().await;
    opener.await.unwrap();
    until(&mut rx, |m| {
        matches!(
            m,
            EngineMessage::Snapshot {
                status: EngineStatus::Ready,
                ..
            }
        )
    })
    .await;
    assert!(bridge.engine().is_ok());
    bridge.shutdown().await;
}

#[test]
fn resumes_recovered_downloads_once() {
    let recovered = [
        boltget_engine::DownloadId(3),
        boltget_engine::DownloadId(2),
        boltget_engine::DownloadId(3),
    ];
    assert_eq!(to_resume(&recovered), vec![DownloadId(2), DownloadId(3)]);
}

/// Records finish actions instead of carrying them out: tests never sleep
/// or shut the machine down.
#[derive(Debug, Default)]
struct Recorded(Mutex<Vec<FinishAction>>);

impl Power for Recorded {
    fn act(&self, action: FinishAction) {
        self.0.lock().unwrap().push(action);
    }
}

fn bridge_with(power: Arc<Recorded>, countdown: Duration) -> Arc<Bridge> {
    let (bridge, hub) = Bridge::with_power(TEST_TICK, power, countdown);
    tokio::spawn(hub);
    let bridge = Arc::new(bridge);
    let runner = bridge.clone();
    tokio::spawn(async move { runner.run_finish_actions(|| {}).await });
    bridge
}

async fn finishing_queue(bridge: &Bridge, srv: &TestServer, dir: &Path) {
    let engine = bridge.engine().unwrap().clone();
    let mut change = QueueChange::default();
    change.on_finish = Some(boltget_engine::FinishAction::ShutDown);
    change.max_concurrent = Some(1);
    engine.update_queue(DEFAULT_QUEUE, change).await.unwrap();
    for seed in [31, 32] {
        let req = AddRequest::new(
            RequestContext::new(&srv.url(&format!("/data?size=100000&seed={seed}"))).unwrap(),
            Destination::Directory(dir.to_path_buf()),
        );
        engine.enqueue(req).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn queues_stream_to_the_page_and_follow_moves() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge();
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let mut rx = subscribe(&bridge);
    match next(&mut rx).await {
        EngineMessage::Snapshot { queues, .. } => {
            assert_eq!(queues.len(), 1);
            assert!(queues[0].is_default && queues[0].started);
            assert_eq!(queues[0].name, None);
        }
        m => panic!("{m:?}"),
    }
    let engine = bridge.engine().unwrap().clone();
    let night = engine.create_queue("Night").await.unwrap();
    engine.stop_queue(night.id).await.unwrap();
    until(&mut rx, |m| {
        matches!(m, EngineMessage::Queues { queues } if queues.len() == 2 && !queues[1].started)
    })
    .await;
    let mut ids = Vec::new();
    for seed in [41, 42] {
        let req = AddRequest::new(
            RequestContext::new(&srv.url(&format!("/data?size=1000&seed={seed}"))).unwrap(),
            Destination::Directory(dir.path().to_path_buf()),
        )
        .queue(night.id);
        ids.push(DownloadId::from(engine.enqueue(req).await.unwrap()));
    }
    // The rows arrive with their queue and place.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let placed = bridge
            .list()
            .await
            .iter()
            .filter(|r| r.queue.0 == night.id.0 && r.queue_position < i64::MAX)
            .count();
        if placed == 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "rows with their queue"
        );
        tokio::time::sleep(TEST_TICK).await;
    }
    engine
        .shift(ids[1].into(), boltget_engine::Shift::Top)
        .await
        .unwrap();
    // The hub reads the order again and the page gets the moved rows.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let rows = bridge.list().await;
        let pos = |id| rows.iter().find(|r| r.id == id).unwrap().queue_position;
        if pos(ids[1]) < pos(ids[0]) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "first now");
        tokio::time::sleep(TEST_TICK).await;
    }
    let mut sent = false;
    while let Ok(m) = rx.try_recv() {
        sent |=
            matches!(m, EngineMessage::Batch { rows, .. } if rows.iter().any(|r| r.id == ids[1]));
    }
    assert!(
        sent || until_row(&mut rx, ids[1]).await,
        "the page heard of it"
    );
    bridge.shutdown().await;
}

async fn until_row(rx: &mut mpsc::UnboundedReceiver<EngineMessage>, id: DownloadId) -> bool {
    until(
        rx,
        |m| matches!(m, EngineMessage::Batch { rows, .. } if rows.iter().any(|r| r.id == id)),
    )
    .await;
    true
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_queue_counts_down_then_acts_once() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let power = Arc::new(Recorded::default());
    let bridge = bridge_with(power.clone(), Duration::from_millis(400));
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let mut rx = subscribe(&bridge);
    finishing_queue(&bridge, &srv, dir.path()).await;
    let seen = until(&mut rx, |m| {
        matches!(m, EngineMessage::Countdown { countdown: Some(_) })
    })
    .await;
    let Some(EngineMessage::Countdown { countdown: Some(c) }) = seen.last() else {
        unreachable!()
    };
    assert_eq!(c.action, FinishAction::ShutDown);
    assert_eq!(
        *power.0.lock().unwrap(),
        [],
        "not before the countdown ends"
    );
    until(&mut rx, |m| {
        matches!(m, EngineMessage::Countdown { countdown: None })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(*power.0.lock().unwrap(), [FinishAction::ShutDown]);
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_countdown_does_nothing() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let power = Arc::new(Recorded::default());
    let bridge = bridge_with(power.clone(), Duration::from_secs(30));
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let mut rx = subscribe(&bridge);
    finishing_queue(&bridge, &srv, dir.path()).await;
    until(&mut rx, |m| {
        matches!(m, EngineMessage::Countdown { countdown: Some(_) })
    })
    .await;
    bridge.cancel_finish_action();
    until(&mut rx, |m| {
        matches!(m, EngineMessage::Countdown { countdown: None })
    })
    .await;
    // A page that subscribes now sees no countdown.
    let mut late = subscribe(&bridge);
    match next(&mut late).await {
        EngineMessage::Snapshot { countdown, .. } => assert_eq!(countdown, None),
        m => panic!("{m:?}"),
    }
    assert_eq!(*power.0.lock().unwrap(), []);
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_running_when_the_session_ends_continue_on_the_next_start() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let size = 3 * MIB + 17;

    let first = bridge();
    first
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let engine = first.engine().unwrap().clone();
    let a = add(
        &engine,
        &srv.url(&format!("/data?size={size}&seed=21&throttle=262144")),
        dir.path(),
    );
    let b = add(
        &engine,
        &srv.url(&format!("/data?size={size}&seed=22&throttle=262144")),
        dir.path(),
    );
    let (ida, idb) = (a.id(), b.id());
    until_durable(&a, 128 * KIB).await;
    until_durable(&b, 128 * KIB).await;
    // The user pauses `b`; then Windows ends the session while `a` runs.
    engine.pause(idb).await.unwrap();
    first.shutdown_for_session_end().await;
    drop(engine);
    // Quitting afterwards (Tauri's exit) changes nothing.
    first.shutdown().await;

    let second = bridge();
    let mut rx = subscribe(&second);
    second
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    until(&mut rx, |m| completed(m, ida.into())).await;
    let rows = second.list().await;
    let state_of = |id: boltget_engine::DownloadId| {
        rows.iter()
            .find(|r| r.id == DownloadId::from(id))
            .unwrap()
            .state
    };
    assert_eq!(state_of(ida), DownloadState::Completed);
    assert_eq!(state_of(idb), DownloadState::Paused, "paused by the user");
    second.shutdown().await;

    // The list is used once: a third start continues nothing.
    let engine = Engine::open(cfg(), state.path()).unwrap();
    assert_eq!(engine.app_setting("resume_on_start").await.unwrap(), None);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn completions_reach_notifications_and_the_taskbar_settles() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge();
    let mut notices = bridge.notices();
    let mut activity = bridge.activity();
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let engine = bridge.engine().unwrap().clone();
    let h = add(
        &engine,
        &srv.url(&format!("/data?size={MIB}&seed=6&throttle=1048576")),
        dir.path(),
    );
    let id = DownloadId::from(h.id());

    let mut saw_running = false;
    let notice = loop {
        tokio::select! {
            n = notices.recv() => break n.unwrap(),
            _ = activity.changed() => {
                let a = activity.borrow_and_update().clone();
                saw_running |= a.running == 1
                    && matches!(a.taskbar, super::activity::Taskbar::Normal(_));
            }
        }
    };
    assert!(saw_running, "the taskbar showed the download");
    match notice {
        super::activity::Notice::Completed {
            id: n,
            size,
            elapsed,
            ..
        } => {
            assert_eq!(n, id);
            assert_eq!(size, Some(MIB));
            assert!(elapsed.is_some(), "the whole run was seen");
        }
        other @ super::activity::Notice::Attention { .. } => panic!("{other:?}"),
    }
    // Nothing runs and everything completed: the button clears.
    tokio::time::timeout(
        Duration::from_secs(10),
        activity.wait_for(|a| a.taskbar == super::activity::Taskbar::None && a.running == 0),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(notices.try_recv().is_err(), "one notice per completion");
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pipe_learns_who_owns_the_engine() {
    let state = tempfile::tempdir().unwrap();
    let cli = Engine::open(cfg(), state.path()).unwrap();
    let bridge = std::sync::Arc::new(bridge());
    let ownership = bridge.ownership();
    assert!(matches!(
        *ownership.borrow(),
        boltget_ipc::Ownership::Starting
    ));
    let opener = {
        let bridge = bridge.clone();
        let dir = state.path().to_path_buf();
        tokio::spawn(async move { bridge.open(dir, cfg(), Duration::from_millis(50)).await })
    };
    let mut o = ownership.clone();
    o.wait_for(|o| matches!(o, boltget_ipc::Ownership::NotOwner))
        .await
        .unwrap();
    cli.shutdown().await;
    opener.await.unwrap();
    assert!(matches!(
        *ownership.borrow(),
        boltget_ipc::Ownership::Owner(_)
    ));
    bridge.shutdown().await;
    assert!(matches!(
        *ownership.borrow(),
        boltget_ipc::Ownership::NotOwner
    ));
}
