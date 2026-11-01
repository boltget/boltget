//! The bridge against a real engine and the local test server: batched
//! streaming, recovery on start, and waiting out another owner of the
//! database. No webview is involved; a test sink stands in for the page.

use std::path::Path;
use std::time::Duration;

use boltget_engine::{
    AddRequest, Destination, DownloadHandle, Engine, EngineConfig, RequestContext,
};
use boltget_testserver::TestServer;
use tokio::sync::mpsc;

use super::dto::{
    DownloadEvent, DownloadId, DownloadRow, DownloadState, EngineMessage, EngineStatus,
};
use super::{Bridge, Sink, to_resume};

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
fn resumes_recovered_and_queued_downloads_once() {
    let row = |id: u64, state: DownloadState| DownloadRow {
        id: DownloadId(id),
        state,
        url: String::new(),
        file_name: None,
        path: None,
        total: None,
        downloaded: 0,
        bps: 0,
        connections: 0,
        resumable: true,
        speed_limit: None,
        error: None,
        created_at: 0,
        completed_at: None,
    };
    let rows = [
        row(1, DownloadState::Paused),
        row(2, DownloadState::Paused),
        row(3, DownloadState::Queued),
        row(4, DownloadState::Completed),
        row(5, DownloadState::Failed),
    ];
    let recovered = [boltget_engine::DownloadId(2), boltget_engine::DownloadId(3)];
    assert_eq!(
        to_resume(&recovered, &rows),
        vec![DownloadId(2), DownloadId(3)]
    );
}
