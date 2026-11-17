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

/// Adds a download as a browser capture does: marked private or not, and
/// its first row given to the bridge.
fn add_captured(bridge: &Bridge, url: &str, dir: &Path, private: bool) -> DownloadHandle {
    let engine = bridge.engine().unwrap();
    let h = engine.add(
        AddRequest::new(
            RequestContext::new(url).unwrap(),
            Destination::Directory(dir.to_path_buf()),
        )
        .private(private),
    );
    bridge.added(h.id().into(), url.to_owned(), private);
    h
}

async fn forget_private(bridge: &Bridge, on: bool) {
    let patch = crate::settings::SettingsPatch {
        forget_private: Some(on),
        ..crate::settings::SettingsPatch::default()
    };
    assert_eq!(
        bridge.update_settings(&patch).await.unwrap().forget_private,
        on
    );
}

/// Waits until the list has no row `id`.
async fn until_gone(bridge: &Bridge, id: DownloadId) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while bridge.list().await.iter().any(|r| r.id == id) {
        assert!(tokio::time::Instant::now() < deadline, "still listed");
        tokio::time::sleep(TEST_TICK).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn private_downloads_leave_the_list_once_complete_and_keep_their_file() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge();
    let mut notices = bridge.notices();
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    forget_private(&bridge, true).await;
    let mut rx = subscribe(&bridge);

    let private = add_captured(
        &bridge,
        &srv.url("/data?size=300000&seed=71&throttle=1048576"),
        dir.path(),
        true,
    );
    let usual = add_captured(
        &bridge,
        &srv.url("/data?size=300000&seed=72&throttle=1048576"),
        dir.path(),
        false,
    );
    let (pid, uid) = (DownloadId::from(private.id()), DownloadId::from(usual.id()));
    let file = private.wait().await.unwrap().path;
    usual.wait().await.unwrap();
    until_gone(&bridge, pid).await;
    until(
        &mut rx,
        |m| matches!(m, EngineMessage::Batch { removed, .. } if removed.contains(&pid)),
    )
    .await;

    assert!(file.is_file(), "the file stays");
    assert_eq!(std::fs::metadata(&file).unwrap().len(), 300_000);
    let engine = bridge.engine().unwrap().clone();
    let ids: Vec<DownloadId> = engine
        .list()
        .await
        .unwrap()
        .iter()
        .map(|s| s.id.into())
        .collect();
    assert_eq!(ids, [uid], "only the usual download is kept");
    let rows = bridge.list().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, DownloadState::Completed);
    // Its notification names nothing, and its buttons find the file.
    let mut private_notice = None;
    while let Ok(n) = notices.try_recv() {
        if let super::activity::Notice::Completed { id, private, .. } = n {
            if id == pid {
                private_notice = Some(private);
            } else {
                assert!(!private, "the usual download is named");
            }
        }
    }
    assert_eq!(private_notice, Some(true));
    assert_eq!(bridge.forgotten_path(pid), Some(file));
    assert_eq!(bridge.forgotten_path(uid), None);
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_setting_off_private_downloads_stay_until_it_is_turned_on() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge();
    let mut notices = bridge.notices();
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let h = add_captured(
        &bridge,
        &srv.url("/data?size=200000&seed=73"),
        dir.path(),
        true,
    );
    let id = DownloadId::from(h.id());
    let file = h.wait().await.unwrap().path;
    tokio::time::sleep(TEST_TICK * 6).await;
    let rows = bridge.list().await;
    assert_eq!(rows.len(), 1, "kept");
    assert!(rows[0].private && rows[0].state == DownloadState::Completed);
    let notice = tokio::time::timeout(Duration::from_secs(10), notices.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            notice,
            super::activity::Notice::Completed { private: false, .. }
        ),
        "named as usual: {notice:?}"
    );

    // Turning the setting on forgets it now, and the file stays.
    forget_private(&bridge, true).await;
    until_gone(&bridge, id).await;
    assert!(bridge.engine().unwrap().list().await.unwrap().is_empty());
    assert!(file.is_file());
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn on_start_completed_private_downloads_go_and_unfinished_ones_wait() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();

    // A run that quit before it could forget a completed private download
    // (or a CLI that ran it), with another private download paused.
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let private = |seed: u32, throttle: &str| {
        AddRequest::new(
            RequestContext::new(&srv.url(&format!("/data?size={}&seed={seed}{throttle}", 2 * MIB)))
                .unwrap(),
            Destination::Directory(dir.path().to_path_buf()),
        )
        .private(true)
    };
    let done = engine.add(private(74, "")).wait().await.unwrap();
    let paused = engine.add(private(75, "&throttle=262144"));
    let pid = paused.id();
    until_durable(&paused, 64 * KIB).await;
    engine.pause(pid).await.unwrap();
    let _ = paused.wait().await;
    engine
        .set_app_setting("forget_private", Some(serde_json::json!(true)))
        .await
        .unwrap();
    engine.shutdown().await;

    let bridge = bridge();
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let rows = bridge.list().await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].id, DownloadId::from(pid));
    assert!(rows[0].private && rows[0].state == DownloadState::Paused);
    assert!(done.path.is_file(), "the file stays");
    assert_eq!(bridge.engine().unwrap().list().await.unwrap().len(), 1);

    // Resumed, it goes once it completes.
    let engine = bridge.engine().unwrap().clone();
    let h = engine
        .resume(pid, boltget_engine::ResumeOptions::default())
        .await
        .unwrap();
    let file = h.wait().await.unwrap().path;
    until_gone(&bridge, pid.into()).await;
    assert!(file.is_file());
    assert!(engine.list().await.unwrap().is_empty());
    bridge.shutdown().await;
}

/// The media crate's fake yt-dlp and FFmpeg (`boltget-fake-sidecar`), when
/// the workspace's tests built it next to this test binary.
fn fake_sidecar() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?;
    let name = if cfg!(windows) {
        "boltget-fake-sidecar.exe"
    } else {
        "boltget-fake-sidecar"
    };
    Some(dir.join(name)).filter(|p| p.is_file())
}

fn media_completed(msg: &EngineMessage, id: DownloadId) -> Option<String> {
    match msg {
        EngineMessage::Batch { events, .. } => events.iter().find_map(|e| match e {
            DownloadEvent::Completed { id: c, path } if *c == id => Some(path.clone()),
            _ => None,
        }),
        _ => None,
    }
}

fn state_of(msg: &EngineMessage, id: DownloadId) -> Option<DownloadState> {
    match msg {
        EngineMessage::Batch { rows, .. } => rows.iter().find(|r| r.id == id).map(|r| r.state),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn media_jobs_run_as_rows_pause_continue_and_go() {
    let Some(fake) = fake_sidecar() else {
        eprintln!("skipped: boltget-fake-sidecar is not built (cargo test --workspace builds it)");
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let videos = state.path().join("Videos");
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let yt_dlp = tools.path().join(format!("yt-dlp{ext}"));
    let ffmpeg = tools.path().join(format!("ffmpeg{ext}"));
    std::fs::copy(&fake, &yt_dlp).unwrap();
    std::fs::copy(&fake, &ffmpeg).unwrap();
    std::fs::write(
        tools.path().join("fake-progress.txt"),
        "[boltget] download {\"status\": \"downloading\", \"downloaded_bytes\": 50, \"total_bytes\": 100, \"filename\": \"f.mp4\", \"speed\": 10}\n\
         [boltget] postprocess {\"status\": \"started\", \"postprocessor\": \"Merger\"}\n",
    )
    .unwrap();
    std::fs::write(tools.path().join("fake-slow"), "").unwrap();

    let bridge = bridge();
    bridge.set_tools_dir(state.path().join("tools"));
    bridge
        .open(state.path().into(), cfg(), Duration::from_millis(50))
        .await;
    let patch = crate::settings::SettingsPatch {
        video: Some(crate::settings::VideoPatch {
            yt_dlp_path: Some(yt_dlp.to_string_lossy().into_owned()),
            ffmpeg_path: Some(ffmpeg.to_string_lossy().into_owned()),
            ..crate::settings::VideoPatch::default()
        }),
        ..crate::settings::SettingsPatch::default()
    };
    bridge.update_settings(&patch).await.unwrap();
    let mut rx = subscribe(&bridge);
    let _snapshot = next(&mut rx).await;

    // A job runs, is processed and completes as a row of the list.
    let mut job = boltget_engine::NewMediaJob::new("https://v.test/watch?v=v1", "b", &videos);
    job.title = Some("fake video".into());
    job.preset = Some("best".into());
    let row = bridge.add_media(job, None).await.unwrap();
    assert!(row.media.is_some());
    assert_eq!(row.file_name.as_deref(), Some("fake video.mp4"));
    let id = row.id;
    assert!(super::media::job_id(id).is_some());
    let seen = until(&mut rx, |m| media_completed(m, id).is_some()).await;
    assert!(
        seen.iter()
            .any(|m| state_of(m, id) == Some(DownloadState::Processing)),
        "never processing"
    );
    let path = seen.iter().find_map(|m| media_completed(m, id)).unwrap();
    assert!(path.ends_with("fake video [v1].mp4"), "{path}");
    let done = bridge
        .list()
        .await
        .into_iter()
        .find(|r| r.id == id)
        .unwrap();
    assert_eq!(done.state, DownloadState::Completed);
    assert_eq!(done.total, Some(10));

    let args = std::fs::read_to_string(tools.path().join("fake-args.txt")).unwrap();
    assert!(
        !args.contains("--cookies") && !args.contains("--referer"),
        "{args}"
    );

    // Another one, from the browser extension with its cookies and headers,
    // is paused while it runs, keeps its partial file, and continues when
    // resumed, with the browser's cookies and headers on both runs.
    std::fs::remove_file(videos.join("fake video [v1].mp4")).unwrap();
    std::fs::write(tools.path().join("fake-hang-download"), "").unwrap();
    let mut job = boltget_engine::NewMediaJob::new("https://v.test/watch?v=v1", "b", &videos);
    job.title = Some("second".into());
    let mut cookies = boltget_engine::CookieJar::new();
    cookies.insert(boltget_engine::Cookie::for_host(
        "v.test",
        "sid",
        "browser-cookie",
    ));
    let auth = super::media::MediaAuth {
        cookies,
        user_agent: Some("Browser UA".into()),
        referer: Some("https://v.test/watch?v=v1".into()),
    };
    let second = bridge.add_media(job, Some(auth)).await.unwrap().id;
    let job2 = super::media::job_id(second).unwrap();
    until(&mut rx, |m| {
        state_of(m, second) == Some(DownloadState::Processing)
    })
    .await;
    // The fake writes its partial file after its last progress line.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        bridge.pause_media(job2).await.unwrap(),
        DownloadState::Paused
    );
    until(&mut rx, |m| {
        state_of(m, second) == Some(DownloadState::Paused)
    })
    .await;
    assert!(videos.join("fake video [v1].mp4.part").exists());
    std::fs::remove_file(tools.path().join("fake-hang-download")).unwrap();
    std::fs::remove_file(tools.path().join("fake-cookies.txt")).unwrap();
    bridge.resume_media(job2).await.unwrap();
    until(&mut rx, |m| media_completed(m, second).is_some()).await;
    assert_eq!(
        std::fs::read_to_string(tools.path().join("fake-resumed.txt")).unwrap(),
        "yes"
    );
    let cookies = std::fs::read_to_string(tools.path().join("fake-cookies.txt")).unwrap();
    assert!(cookies.contains("browser-cookie"), "{cookies}");
    let args = std::fs::read_to_string(tools.path().join("fake-args.txt")).unwrap();
    assert!(
        args.contains(
            "--referer
https://v.test/watch?v=v1"
        ),
        "{args}"
    );
    assert!(args.contains("User-Agent:Browser UA"), "{args}");

    // Removing it with its file deletes the file and the row.
    bridge.remove_media(job2, true).await.unwrap();
    assert!(!videos.join("fake video [v1].mp4").exists());
    until(
        &mut rx,
        |m| matches!(m, EngineMessage::Batch { removed, .. } if removed.contains(&second)),
    )
    .await;
    assert!(bridge.engine().unwrap().media_job(job2).await.is_err());

    // A job whose yt-dlp fails shows yt-dlp's message.
    std::fs::write(
        tools.path().join("fake-download-error.txt"),
        "ERROR: [generic] Unsupported URL: https://v.test/x\n",
    )
    .unwrap();
    let third = bridge
        .add_media(
            boltget_engine::NewMediaJob::new("https://v.test/x", "b", &videos),
            None,
        )
        .await
        .unwrap()
        .id;
    until(&mut rx, |m| {
        state_of(m, third) == Some(DownloadState::Failed)
    })
    .await;
    let failed = bridge
        .list()
        .await
        .into_iter()
        .find(|r| r.id == third)
        .unwrap();
    let error = failed.error.unwrap();
    assert_eq!(error.key, "error.media.failed");
    assert!(
        error.message.contains("Unsupported URL"),
        "{}",
        error.message
    );
    bridge.shutdown().await;
}

/// Two fake tools (yt-dlp and FFmpeg) in a folder, a bridge whose Settings
/// name them, and its stream; each run reports `lines` progress lines 50 ms
/// apart, so runs that could overlap do.
async fn media_bridge(
    fake: &Path,
    state: &Path,
    tools: &Path,
    lines: usize,
) -> (Bridge, mpsc::UnboundedReceiver<EngineMessage>) {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let yt_dlp = tools.join(format!("yt-dlp{ext}"));
    let ffmpeg = tools.join(format!("ffmpeg{ext}"));
    std::fs::copy(fake, &yt_dlp).unwrap();
    std::fs::copy(fake, &ffmpeg).unwrap();
    let line = "[boltget] download {\"status\": \"downloading\", \"downloaded_bytes\": 50, \"total_bytes\": 100, \"filename\": \"f.mp4\"}\n";
    std::fs::write(tools.join("fake-progress.txt"), line.repeat(lines)).unwrap();
    std::fs::write(tools.join("fake-slow"), "").unwrap();
    let bridge = bridge();
    bridge.set_tools_dir(state.join("tools"));
    bridge
        .open(state.into(), cfg(), Duration::from_millis(50))
        .await;
    let patch = crate::settings::SettingsPatch {
        video: Some(crate::settings::VideoPatch {
            yt_dlp_path: Some(yt_dlp.to_string_lossy().into_owned()),
            ffmpeg_path: Some(ffmpeg.to_string_lossy().into_owned()),
            ..crate::settings::VideoPatch::default()
        }),
        ..crate::settings::SettingsPatch::default()
    };
    bridge.update_settings(&patch).await.unwrap();
    let mut rx = subscribe(&bridge);
    let _snapshot = next(&mut rx).await;
    (bridge, rx)
}

fn entry_request(index: u32) -> crate::media::MediaEntryRequest {
    crate::media::MediaEntryRequest {
        index,
        url: format!("https://v.test/watch?v=e{index}"),
        title: Some(format!("Entry {index}")),
    }
}

/// How many fake downloads ran when each one started.
fn overlaps(tools: &Path) -> Vec<usize> {
    std::fs::read_to_string(tools.join("fake-overlap.txt"))
        .unwrap()
        .lines()
        .map(|l| l.parse().unwrap())
        .collect()
}

/// A playlist (docs/VIDEO.md, "Playlists: as built"): each chosen entry is a
/// job and a row of its own, numbered into the playlist's folder; the
/// entries run one at a time while a single page's job may use the other
/// slot; Pause all and Resume all act on the playlist; the playlist goes
/// with its last entry.
#[tokio::test(flavor = "multi_thread")]
async fn a_playlists_entries_run_one_at_a_time_as_rows() {
    let Some(fake) = fake_sidecar() else {
        eprintln!("skipped: boltget-fake-sidecar is not built (cargo test --workspace builds it)");
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let videos = state.path().join("Videos");
    let (bridge, mut rx) = media_bridge(&fake, state.path(), tools.path(), 6).await;

    // Three chosen entries of a twelve-entry playlist.
    let request = crate::media::MediaPlaylistRequest {
        url: "https://v.test/playlist?list=PL1".into(),
        title: Some("Weekend: projects".into()),
        uploader: None,
        id: Some("PL1".into()),
        preset: "720p".into(),
        container: crate::settings::MediaContainer::Mp4,
        subtitles: None,
        entry_count: 12,
        entries: vec![entry_request(1), entry_request(2), entry_request(4)],
        page: None,
    };
    let (list, jobs) = crate::media::playlist_jobs(&request, &videos).unwrap();
    let folder = list.dest_dir.clone();
    assert_eq!(folder, videos.join("Weekend_ projects"));
    let rows = bridge.add_media_playlist(list, jobs, None).await.unwrap();
    assert_eq!(rows.len(), 3);
    let first = rows[0]
        .media
        .as_ref()
        .and_then(|m| m.playlist.clone())
        .unwrap();
    assert_eq!(first.title.as_deref(), Some("Weekend: projects"));
    assert_eq!(first.index, Some(1));
    assert_eq!(rows[2].file_name.as_deref(), Some("04 - Entry 4.mp4"));
    // A single page at the same time takes the other slot.
    let single = bridge
        .add_media(
            boltget_engine::NewMediaJob::new("https://v.test/watch?v=s", "b", &videos),
            None,
        )
        .await
        .unwrap()
        .id;

    let mut ids: Vec<DownloadId> = rows.iter().map(|r| r.id).collect();
    ids.push(single);
    let mut done = std::collections::HashSet::new();
    until(&mut rx, |m| {
        for id in &ids {
            if media_completed(m, *id).is_some() {
                done.insert(*id);
            }
        }
        done.len() == ids.len()
    })
    .await;
    for name in [
        "01 - fake video [v1].mp4",
        "02 - fake video [v1].mp4",
        "04 - fake video [v1].mp4",
    ] {
        assert!(folder.join(name).is_file(), "{name} missing");
    }
    // Never more than two at once; only the single page ran beside an entry.
    let counts = overlaps(tools.path());
    assert_eq!(counts.len(), 4, "{counts:?}");
    assert!(counts.iter().all(|&n| n <= 2), "{counts:?}");
    assert!(
        counts.iter().filter(|&&n| n == 2).count() <= 1,
        "{counts:?}"
    );

    // Pause all and Resume all act on the playlist: all are done, so
    // nothing changes and nothing fails.
    let playlist = boltget_engine::MediaPlaylistId(first.id);
    bridge.pause_media_playlist(playlist).await.unwrap();
    bridge.resume_media_playlist(playlist).await.unwrap();
    // Removing every entry removes the playlist too.
    for row in &rows {
        let job = super::media::job_id(row.id).unwrap();
        bridge.remove_media(job, false).await.unwrap();
    }
    assert_eq!(
        bridge
            .engine()
            .unwrap()
            .media_playlists()
            .await
            .unwrap()
            .len(),
        0
    );
    bridge.shutdown().await;
}

/// The entries of one playlist wait for each other: with three entries and
/// nothing else, no two ever run together; paused, they wait; resumed, they
/// go on in order.
#[tokio::test(flavor = "multi_thread")]
async fn a_playlist_alone_runs_one_entry_at_a_time() {
    let Some(fake) = fake_sidecar() else {
        eprintln!("skipped: boltget-fake-sidecar is not built (cargo test --workspace builds it)");
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let videos = state.path().join("Videos");
    let (bridge, mut rx) = media_bridge(&fake, state.path(), tools.path(), 4).await;
    let request = crate::media::MediaPlaylistRequest {
        url: "https://v.test/playlist?list=PL2".into(),
        title: None,
        uploader: None,
        id: Some("PL2".into()),
        preset: "audio".into(),
        container: crate::settings::MediaContainer::Original,
        subtitles: None,
        entry_count: 3,
        entries: (1..=3).map(entry_request).collect(),
        page: None,
    };
    let (list, jobs) = crate::media::playlist_jobs(&request, &videos).unwrap();
    assert_eq!(list.dest_dir, videos.join("PL2"), "named by its id");
    let rows = bridge.add_media_playlist(list, jobs, None).await.unwrap();
    assert!(rows.iter().all(|r| r.state == DownloadState::Queued));
    let playlist = rows[0]
        .media
        .as_ref()
        .and_then(|m| m.playlist.as_ref())
        .map(|p| boltget_engine::MediaPlaylistId(p.id))
        .unwrap();
    // Paused at once: the first stops (or never starts), the rest wait.
    bridge.pause_media_playlist(playlist).await.unwrap();
    let mut paused = std::collections::HashSet::new();
    until(&mut rx, |m| {
        for r in &rows {
            if state_of(m, r.id) == Some(DownloadState::Paused) {
                paused.insert(r.id);
            }
        }
        paused.len() == rows.len()
    })
    .await;
    let _ = std::fs::remove_file(tools.path().join("fake-overlap.txt"));
    bridge.resume_media_playlist(playlist).await.unwrap();
    let mut done = std::collections::HashSet::new();
    until(&mut rx, |m| {
        for r in &rows {
            if media_completed(m, r.id).is_some() {
                done.insert(r.id);
            }
        }
        done.len() == rows.len()
    })
    .await;
    assert_eq!(overlaps(tools.path()), [1, 1, 1]);
    bridge.shutdown().await;
}
