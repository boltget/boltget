//! Bandwidth limiting against the local test server: per-download and global
//! token buckets, and limit changes on running downloads.

mod support;

use std::time::{Duration, Instant};

use boltget_engine::{Engine, LimitScope};
use boltget_testserver::TestServer;
use support::{MIB, assert_content, cfg, get_with, request};

#[tokio::test]
async fn per_download_limit_is_accurate() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 3 * MIB;
    let t0 = Instant::now();
    let done = get_with(
        &engine,
        request(&srv.url(&format!("/data?size={size}")), dir.path())
            .connections(4)
            .speed_limit(Some(MIB)),
    )
    .await
    .unwrap();
    let secs = t0.elapsed().as_secs_f64();
    assert_content(&done, 1, size);
    // 3 MiB at 1 MiB/s, minus the 256 KiB burst: 2.75 s.
    // The lower bound proves the limit; the upper one only catches a hang (CI
    // runners have taken 3.8 s).
    assert!(secs > 2.5 && secs < 6.0, "took {secs:.2} s");
    assert!(
        done.max_connections > 1,
        "the limit is shared by all workers"
    );
}

#[tokio::test]
async fn global_limit_is_shared_by_downloads() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.speed_limit = Some(2 * MIB);
    let engine = Engine::new(c).unwrap();
    let size = 3 * MIB;
    let a = request(&srv.url(&format!("/data?size={size}&key=a")), dir.path());
    let b = request(
        &srv.url(&format!("/data?size={size}&key=b&seed=2")),
        dir.path(),
    );
    let t0 = Instant::now();
    let (a, b) = tokio::join!(get_with(&engine, a), get_with(&engine, b));
    let secs = t0.elapsed().as_secs_f64();
    assert_content(&a.unwrap(), 1, size);
    assert_content(&b.unwrap(), 2, size);
    // 6 MiB at 2 MiB/s, minus the 512 KiB burst: 2.75 s.
    // The lower bound proves the limit; the upper one only catches a hang (CI
    // runners have taken 3.8 s).
    assert!(secs > 2.5 && secs < 6.0, "took {secs:.2} s");
    assert_eq!(engine.limit(LimitScope::Global), Some(2 * MIB));
}

#[tokio::test]
async fn limit_changes_apply_to_a_running_download() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 4 * MIB;
    let h = engine.add(
        request(&srv.url(&format!("/data?size={size}")), dir.path()).speed_limit(Some(128 * 1024)),
    );
    let id = h.id();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(engine.limit(LimitScope::Download(id)), Some(128 * 1024));
    let before = h.progress().downloaded;
    // At 128 KiB/s the whole file would take 32 s.
    assert!(
        before < MIB,
        "limit not applied: {before} bytes after 0.6 s"
    );
    let t0 = Instant::now();
    engine
        .set_limit(LimitScope::Download(id), None)
        .await
        .unwrap();
    let done = tokio::time::timeout(Duration::from_secs(20), h.wait())
        .await
        .expect("finished after the limit was lifted")
        .unwrap();
    assert_content(&done, 1, size);
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
    // Unknown downloads are an error.
    let e = engine
        .set_limit(
            LimitScope::Download(boltget_engine::DownloadId(999)),
            Some(1),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind(), boltget_engine::ErrorKind::NotFound);
}

#[tokio::test]
async fn limits_are_persisted() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    {
        let engine = Engine::open(cfg(), state.path()).unwrap();
        engine
            .set_limit(LimitScope::Global, Some(3 * MIB))
            .await
            .unwrap();
        assert_eq!(engine.settings().await.unwrap().speed_limit, Some(3 * MIB));
        // A paused download keeps its own limit.
        let h = engine
            .add(request(&srv.url("/data?size=4194304"), dir.path()).speed_limit(Some(64 * 1024)));
        tokio::time::sleep(Duration::from_millis(400)).await;
        let id = h.id();
        engine.pause(id).await.unwrap();
        engine.shutdown().await;
    }
    assert_eq!(
        Engine::peek_settings(state.path()).unwrap().speed_limit,
        Some(3 * MIB)
    );
    let engine = Engine::open(cfg(), state.path()).unwrap();
    assert_eq!(engine.limit(LimitScope::Global), Some(3 * MIB));
    let id = boltget_engine::DownloadId(1);
    let h = engine
        .resume(id, boltget_engine::ResumeOptions::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(engine.limit(LimitScope::Download(id)), Some(64 * 1024));
    engine
        .set_limit(LimitScope::Download(id), None)
        .await
        .unwrap();
    engine.set_limit(LimitScope::Global, None).await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(20), h.wait())
        .await
        .unwrap()
        .unwrap();
    assert_content(&done, 1, 4 * MIB);
    engine.shutdown().await;
    assert_eq!(
        Engine::peek_settings(state.path()).unwrap().speed_limit,
        None
    );
}
