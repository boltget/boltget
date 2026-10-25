//! Persistence and resume: downloads recorded in the database, paused,
//! "crashed" (state left running), reopened and continued from their checkpoint,
//! verified byte for byte (SHA-256) against the generated content.

// Helpers outside `#[test]` functions panic on failure just like the tests do.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use boltget_engine::{
    AddRequest, Completed, Destination, DownloadHandle, DownloadId, Engine, EngineConfig,
    EngineEvent, Error, ErrorKind, RangeSupport, RequestContext, ResumeOptions, State,
};
use boltget_testserver::{TestServer, byte_at, content};
use sha2::{Digest, Sha256};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

/// Small, fast tunables (as in `download.rs`), with frequent flushes so
/// checkpoints happen within a test's time frame.
fn cfg() -> EngineConfig {
    let mut c = EngineConfig::default();
    c.min_split_size = 64 * KIB;
    c.segment_align = 4 * KIB;
    c.coalesce_size = 32 * 1024;
    c.worker_stagger = Duration::from_millis(5);
    c.flush_interval = Duration::from_millis(50);
    c.backoff_base = Duration::from_millis(10);
    c.backoff_max = Duration::from_millis(100);
    c.stall_timeout = Duration::from_secs(5);
    c.first_byte_timeout = Duration::from_secs(10);
    c.connect_timeout = Duration::from_secs(5);
    c.disk_margin = 0;
    c.initial_connections = 4;
    c
}

fn sha_file(p: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(p).expect("read downloaded file")).to_vec()
}

fn assert_content(done: &Completed, seed: u64, size: u64) {
    assert_eq!(done.bytes, size);
    assert_eq!(
        sha_file(&done.path),
        Sha256::digest(content(seed, size)).to_vec(),
        "content mismatch"
    );
    let partial = format!("{}{}", done.path.display(), boltget_engine::PARTIAL_SUFFIX);
    assert!(!Path::new(&partial).exists(), "partial file left behind");
}

async fn wait(h: DownloadHandle) -> Result<Completed, Error> {
    tokio::time::timeout(Duration::from_secs(90), h.wait())
        .await
        .expect("download timed out")
}

/// Waits until at least `bytes` are durable (checkpointable).
async fn until_durable(h: &DownloadHandle, bytes: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let p = h.progress();
        assert!(
            !p.state.is_terminal(),
            "download stopped early in state {:?}",
            p.state
        );
        if p.durable >= bytes {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no durable progress: {p:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn add(engine: &Engine, url: &str, dir: &Path) -> DownloadHandle {
    engine.add(AddRequest::new(
        RequestContext::new(url).unwrap(),
        Destination::Directory(dir.to_path_buf()),
    ))
}

/// `(start, end, done)` of every checkpointed segment, read straight from the
/// database file (a second read-only connection; the owner lock is separate).
fn db_segments(state: &Path, id: DownloadId) -> Vec<(u64, Option<u64>, u64)> {
    let conn = rusqlite::Connection::open_with_flags(
        state.join(boltget_engine::DB_FILE),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut stmt = conn
        .prepare("SELECT start, \"end\", done FROM segment WHERE download_id = ?1 ORDER BY start")
        .unwrap();
    stmt.query_map([i64::try_from(id.0).unwrap()], |r| {
        Ok((
            r.get::<_, i64>(0)? as u64,
            r.get::<_, Option<i64>>(1)?.map(|e| e as u64),
            r.get::<_, i64>(2)? as u64,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn sql(state: &Path, statement: &str) {
    let conn = rusqlite::Connection::open(state.join(boltget_engine::DB_FILE)).unwrap();
    conn.execute_batch(statement).unwrap();
}

/// The checkpoint invariant, checked from the outside: every byte the
/// database claims is done is in the partial file with the right value.
fn assert_checkpoint_matches_file(state: &Path, id: DownloadId, partial: &Path, seed: u64) -> u64 {
    let data = std::fs::read(partial).unwrap();
    let mut done_total = 0;
    for (start, _, done) in db_segments(state, id) {
        for o in start..start + done {
            assert_eq!(
                data[o as usize],
                byte_at(seed, o),
                "checkpointed byte {o} is not on disk"
            );
        }
        done_total += done;
    }
    done_total
}

fn summary(
    list: &[boltget_engine::DownloadSummary],
    id: DownloadId,
) -> &boltget_engine::DownloadSummary {
    list.iter().find(|s| s.id == id).expect("download listed")
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_and_resume_continue_from_the_checkpoint() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let mut events = engine.subscribe();
    let size = 4 * MIB + 333;
    let url = srv.url(&format!("/data?size={size}&seed=21&throttle=262144"));
    let h = add(&engine, &url, dir.path());
    let id = h.id();
    assert_eq!(id, DownloadId(1));
    until_durable(&h, 256 * KIB).await;
    let partial = h.info().unwrap().partial_path.clone();
    assert_eq!(engine.pause(id).await.unwrap(), State::Paused);
    assert!(matches!(wait(h).await, Err(Error::Cancelled)));

    let list = engine.list().await.unwrap();
    let s = summary(&list, id);
    assert_eq!(s.state, State::Paused);
    assert_eq!(s.total, Some(size));
    assert!(s.downloaded > 0 && s.downloaded < size, "{s:?}");
    assert_eq!(s.partial_path.as_deref(), Some(partial.as_path()));
    let checkpointed = assert_checkpoint_matches_file(state.path(), id, &partial, 21);
    assert_eq!(checkpointed, s.downloaded);

    let sent_before = srv.body_bytes_sent();
    let requests_before = srv.requests().len();
    let h = engine.resume(id, ResumeOptions::default()).await.unwrap();
    assert_eq!(h.id(), id);
    let done = wait(h).await.unwrap();
    assert_content(&done, 21, size);
    assert_eq!(done.info.probe.ranges, RangeSupport::Yes);
    assert_eq!(done.resumed_from, checkpointed);

    // Only the missing part was fetched again, every request validated with
    // If-Range, and nothing restarted from byte 0.
    let resumed = &srv.requests()[requests_before..];
    assert!(!resumed.is_empty());
    for r in resumed {
        assert!(
            r.if_range.is_some(),
            "resume request without If-Range: {r:?}"
        );
        assert!(
            r.range.as_deref() != Some("bytes=0-"),
            "restarted from 0: {r:?}"
        );
    }
    let refetched = srv.body_bytes_sent() - sent_before;
    assert!(
        refetched <= size - checkpointed + 2 * MIB,
        "refetched {refetched} bytes, {checkpointed} were checkpointed"
    );

    let mut saw_resumed = false;
    while let Ok(e) = events.try_recv() {
        if let EngineEvent::Resumed {
            id: rid,
            downloaded,
            total,
        } = e
        {
            assert_eq!(rid, id);
            assert_eq!(downloaded, checkpointed);
            assert_eq!(total, Some(size));
            saw_resumed = true;
        }
    }
    assert!(saw_resumed, "no resumed event");

    let list = engine.list().await.unwrap();
    let s = summary(&list, id);
    assert_eq!(s.state, State::Completed);
    assert_eq!(s.downloaded, size);
    assert_eq!(s.path.as_deref(), Some(done.path.as_path()));
    assert!(s.partial_path.is_none());
    assert!(s.completed_at.is_some());
    assert!(
        db_segments(state.path(), id).is_empty(),
        "segments kept after completion"
    );

    // A completed download cannot be resumed or paused.
    let e = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidState, "{e}");
    let e = engine.pause(id).await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidState, "{e}");
    let e = engine
        .resume(DownloadId(99), ResumeOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound, "{e}");
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_download_is_recovered_after_reopen() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let size = 3 * MIB + 17;
    let url = srv.url(&format!("/data?size={size}&seed=22&throttle=262144"));

    let engine = Engine::open(cfg(), state.path()).unwrap();
    let h = add(&engine, &url, dir.path());
    let id = h.id();
    until_durable(&h, 128 * KIB).await;
    // Shutdown pauses running downloads and releases the database.
    engine.shutdown().await;
    assert!(matches!(wait(h).await, Err(Error::Cancelled)));
    // Pretend the process died mid-download instead.
    sql(
        state.path(),
        &format!(
            "UPDATE download SET state = 'downloading' WHERE id = {}",
            id.0
        ),
    );

    let engine = Engine::open(cfg(), state.path()).unwrap();
    let list = engine.list().await.unwrap();
    assert_eq!(
        summary(&list, id).state,
        State::Paused,
        "recovered as paused"
    );
    // Ids are persistent: the next download gets a new one.
    let other = add(&engine, &srv.url("/data?size=1000&seed=5"), dir.path());
    assert_eq!(other.id(), DownloadId(id.0 + 1));
    wait(other).await.unwrap();

    let done = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap();
    assert_content(&done, 22, size);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_one_process_owns_the_database() {
    let state = tempfile::tempdir().unwrap();
    let a = Engine::open(cfg(), state.path()).unwrap();
    let e = Engine::open(cfg(), state.path()).unwrap_err();
    assert!(matches!(e, Error::DatabaseLocked { .. }), "{e}");
    assert_eq!(e.kind(), ErrorKind::Busy);
    // Read-only listing works without the lock.
    assert!(Engine::peek(state.path()).unwrap().is_empty());
    a.shutdown().await;
    let b = Engine::open(cfg(), state.path()).unwrap();
    b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_file_fails_the_resume_and_restart_starts_over() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut c = cfg();
    // One connection and no dynamic splits: exactly one request before the pause.
    c.initial_connections = 1;
    c.min_split_size = 64 * MIB;
    let engine = Engine::open(c, state.path()).unwrap();
    let size = 2 * MIB;
    // From the second request on, the server has a new version of the file.
    let url = srv.url(&format!(
        "/data?size={size}&seed=30&throttle=262144&change_after=1"
    ));
    let h = add(&engine, &url, dir.path());
    let id = h.id();
    until_durable(&h, 64 * KIB).await;
    engine.pause(id).await.unwrap();
    let _ = wait(h).await;

    let err = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ResourceChanged, "{err}");
    let list = engine.list().await.unwrap();
    let s = summary(&list, id);
    assert_eq!(s.state, State::Failed);
    let saved = s.error.as_ref().expect("error recorded");
    assert_eq!(saved.kind, ErrorKind::ResourceChanged);
    assert_eq!(saved.key, "error.resource_changed");

    // Restart keeps the destination and downloads the new version from zero.
    let first_path = s.path.clone().unwrap();
    let done = wait(
        engine
            .resume(id, ResumeOptions::default().restart(true))
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_content(&done, 31, size);
    assert_eq!(done.path, first_path);
    let list = engine.list().await.unwrap();
    assert!(summary(&list, id).error.is_none());
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_partial_file_is_reported_and_restart_recovers() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let size = 3 * MIB;
    let url = srv.url(&format!("/data?size={size}&seed=40&throttle=262144"));
    let h = add(&engine, &url, dir.path());
    let id = h.id();
    until_durable(&h, 128 * KIB).await;
    let partial = h.info().unwrap().partial_path.clone();
    engine.pause(id).await.unwrap();
    let _ = wait(h).await;
    std::fs::remove_file(&partial).unwrap();

    let err = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::PartialMissing { .. }), "{err}");
    assert_eq!(err.kind(), ErrorKind::Disk);

    // A truncated partial file is refused the same way.
    let h = engine
        .resume(id, ResumeOptions::default().restart(true))
        .await
        .unwrap();
    until_durable(&h, 128 * KIB).await;
    engine.pause(id).await.unwrap();
    let _ = wait(h).await;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&partial)
        .unwrap()
        .set_len(1000)
        .unwrap();
    let err = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::PartialMissing { .. }), "{err}");

    let done = wait(
        engine
            .resume(id, ResumeOptions::default().restart(true))
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_content(&done, 40, size);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn download_without_range_support_starts_over_on_resume() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let size = 2 * MIB + 5;
    let url = srv.url(&format!(
        "/data?size={size}&seed=41&ranges=none&throttle=262144"
    ));
    let h = add(&engine, &url, dir.path());
    let id = h.id();
    until_durable(&h, 64 * KIB).await;
    let path = h.info().unwrap().path.clone();
    engine.pause(id).await.unwrap();
    let _ = wait(h).await;
    let list = engine.list().await.unwrap();
    assert!(!summary(&list, id).resumable);

    let done = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap();
    assert_content(&done, 41, size);
    // Same destination, not "name (1)".
    assert_eq!(done.path, path);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_total_size_resumes_from_its_cursor() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let size = 2 * MIB + 9;
    let url = srv.url(&format!(
        "/data?size={size}&seed=42&star_total=1&throttle=262144"
    ));
    let h = add(&engine, &url, dir.path());
    let id = h.id();
    until_durable(&h, 128 * KIB).await;
    engine.pause(id).await.unwrap();
    let _ = wait(h).await;
    let segs = db_segments(state.path(), id);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].1, None, "open segment has no end");
    let cursor = segs[0].2;
    assert!(cursor > 0);

    let before = srv.requests().len();
    let done = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap();
    assert_content(&done, 42, size);
    let first = &srv.requests()[before];
    assert_eq!(
        first.range.as_deref(),
        Some(format!("bytes={cursor}-").as_str())
    );
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_signed_link_is_re_resolved_on_resume() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.initial_connections = 1;
    c.min_split_size = 64 * MIB;
    let engine = Engine::open(c, state.path()).unwrap();
    let size = 2 * MIB;
    // Each signed link serves one request; the saved one is dead by resume time.
    let url = srv.url(&format!(
        "/signed?valid=1&size={size}&seed=43&throttle=262144&key=sig"
    ));
    let h = add(&engine, &url, dir.path());
    let id = h.id();
    until_durable(&h, 64 * KIB).await;
    engine.pause(id).await.unwrap();
    let _ = wait(h).await;
    let done = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap();
    assert_content(&done, 43, size);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_forgets_the_download_and_optionally_deletes_files() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    // A paused download: removing it with delete_file removes the partial file.
    let h = add(
        &engine,
        &srv.url("/data?size=3145728&seed=50&throttle=262144"),
        dir.path(),
    );
    let paused = h.id();
    until_durable(&h, 64 * KIB).await;
    let partial = h.info().unwrap().partial_path.clone();
    // remove() pauses a running download first.
    let r = engine.remove(paused, true).await.unwrap();
    let _ = wait(h).await;
    assert_eq!(r.deleted, vec![partial.clone()]);
    assert!(!partial.exists());

    // A completed download: without delete_file the file stays.
    let done = wait(add(
        &engine,
        &srv.url("/data?size=5000&seed=51"),
        dir.path(),
    ))
    .await
    .unwrap();
    let r = engine.remove(done.id, false).await.unwrap();
    assert!(r.deleted.is_empty(), "{:?}", r.deleted);
    assert!(done.path.exists());
    // ...and with it, the completed file goes.
    let done2 = wait(add(
        &engine,
        &srv.url("/data?size=6000&seed=52"),
        dir.path(),
    ))
    .await
    .unwrap();
    let r = engine.remove(done2.id, true).await.unwrap();
    assert_eq!(r.deleted, vec![done2.path.clone()]);
    assert!(!done2.path.exists());

    assert!(engine.list().await.unwrap().is_empty());
    let e = engine.remove(paused, false).await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_between_rename_and_completion_record_is_recognised() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let size = 300_000u64;
    let done = wait(add(
        &engine,
        &srv.url(&format!("/data?size={size}&seed=60")),
        dir.path(),
    ))
    .await
    .unwrap();
    let id = done.id;
    engine.shutdown().await;
    // What the database holds if the process died right after the rename: the
    // complete "finishing" checkpoint, the partial path, state still running.
    let partial: PathBuf =
        format!("{}{}", done.path.display(), boltget_engine::PARTIAL_SUFFIX).into();
    sql(
        state.path(),
        &format!(
            "UPDATE download SET state = 'downloading', completed_at = NULL, temp_path = '{}' WHERE id = {id};
             INSERT INTO segment (download_id, idx, start, \"end\", done) VALUES ({id}, 0, 0, {size}, {size});",
            partial.display().to_string().replace('\'', "''"),
            id = id.0,
        ),
    );
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let before = srv.requests().len();
    let again = wait(engine.resume(id, ResumeOptions::default()).await.unwrap())
        .await
        .unwrap();
    assert_eq!(again.path, done.path);
    assert_content(&again, 60, size);
    assert_eq!(srv.requests().len(), before, "no network needed");
    let list = engine.list().await.unwrap();
    assert_eq!(summary(&list, id).state, State::Completed);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_of_a_download_that_is_not_running() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    // A download that failed stays failed.
    let err = wait(add(&engine, &srv.url("/nope"), dir.path()))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Permanent);
    let id = engine.list().await.unwrap()[0].id;
    assert_eq!(engine.pause(id).await.unwrap(), State::Failed);
    let e = engine.pause(DownloadId(42)).await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    engine.shutdown().await;
}
