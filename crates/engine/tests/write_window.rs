//! The write window for partial files that are not sparse (exFAT, FAT32):
//! forced on an ordinary temporary directory, since CI has no exFAT drive.
//! Downloads must stay byte-exact with several connections, and no request
//! may begin far beyond the lowest offset not yet received.

#![allow(clippy::unwrap_used)]

mod support;

use std::time::Duration;

use boltget_engine::{DownloadHandle, Engine, Error, ResumeOptions};
use boltget_testserver::{TestServer, content};
use support::{KIB, MIB, cfg, get_with, request, sha};

/// Start offset of a `Range: bytes=S-...` header.
fn range_start(range: &str) -> u64 {
    range
        .strip_prefix("bytes=")
        .and_then(|r| r.split('-').next())
        .and_then(|s| s.parse().ok())
        .unwrap()
}

fn assert_file(path: &std::path::Path, seed: u64, size: u64) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len() as u64, size);
    assert_eq!(sha(&data), sha(&content(seed, size)), "content mismatch");
}

#[tokio::test(flavor = "multi_thread")]
async fn windowed_download_is_byte_identical_and_stays_in_the_window() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let window = 512 * KIB;
    let mut c = cfg();
    c.write_window = window;
    c.force_write_window = true;
    let engine = Engine::new(c).unwrap();
    let size = 8 * MIB + 4_321;
    let bps = MIB; // per connection
    let conns = 4;
    let done = get_with(
        &engine,
        request(
            &srv.url(&format!("/data?size={size}&seed=41&throttle={bps}")),
            dir.path(),
        )
        .connections(conns),
    )
    .await
    .unwrap();
    assert_file(&done.path, 41, size);
    assert_eq!(done.info.write_window, Some(window));
    assert!(done.max_connections > 1, "expected parallel connections");
    // The initial split has 4 segments, the window forces many more, smaller ones.
    assert!(done.segments > 8, "{} segments", done.segments);

    // Every byte below the lowest incomplete offset has been received, and the
    // server sends at most `conns * bps` per second, so a request may start at
    // most `window` beyond what could have arrived by then (plus slack for one
    // chunk in flight per connection). Without the window, requests at 2, 4 and
    // 6 MiB would go out within milliseconds.
    let reqs = srv.requests();
    let t0 = reqs[0].at;
    for r in &reqs[1..] {
        let start = range_start(r.range.as_deref().unwrap());
        let elapsed = r.at.saturating_sub(t0).as_secs_f64();
        let bound = window + (f64::from(conns) * bps as f64 * elapsed) as u64 + 256 * KIB;
        assert!(
            start <= bound,
            "request at {start} after {elapsed:.3} s, beyond the window (bound {bound})"
        );
    }
}

/// Waits until at least `bytes` are durable (checkpointable).
async fn until_durable(h: &DownloadHandle, bytes: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while h.progress().durable < bytes {
        assert!(!h.progress().state.is_terminal(), "download stopped early");
        assert!(
            tokio::time::Instant::now() < deadline,
            "no durable progress"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A download paused without a window has progress in all four initial
/// segments, far apart. Resumed with the window, it must start from the lowest
/// segment, keep new requests near it and still finish byte-exact.
#[tokio::test(flavor = "multi_thread")]
async fn scattered_checkpoint_resumes_in_order_with_the_window() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let size = 4 * MIB + 77;
    let url = srv.url(&format!("/data?size={size}&seed=42&throttle=262144"));

    let mut free = cfg();
    free.adaptive = false;
    let engine = Engine::open(free, state.path()).unwrap();
    let h = engine.add(request(&url, dir.path()).connections(4));
    let id = h.id();
    until_durable(&h, 512 * KIB).await;
    engine.shutdown().await;
    assert!(matches!(h.wait().await, Err(Error::Cancelled)));

    let window = 256 * KIB;
    let mut windowed = cfg();
    windowed.adaptive = false;
    windowed.write_window = window;
    windowed.force_write_window = true;
    let engine = Engine::open(windowed, state.path()).unwrap();
    let before = srv.requests().len();
    let h = engine.resume(id, ResumeOptions::default()).await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(90), h.wait())
        .await
        .expect("download timed out")
        .unwrap();
    assert_file(&done.path, 42, size);
    assert_eq!(done.info.write_window, Some(window));
    assert!(done.resumed_from > 0);

    // The second initial segment starts at 1 MiB. At 4 x 256 KiB/s the lowest
    // incomplete offset (about 128 KiB at the pause) cannot reach 1 MiB minus
    // the window within 200 ms, so nothing resumed in that time may start there.
    let resumed = &srv.requests()[before..];
    assert!(!resumed.is_empty());
    let t0 = resumed[0].at;
    for r in resumed {
        let start = range_start(r.range.as_deref().unwrap());
        if r.at.saturating_sub(t0) < Duration::from_millis(200) {
            assert!(start < MIB, "resumed request at {start} beyond the window");
        }
    }
    engine.shutdown().await;
}
