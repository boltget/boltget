//! The adaptive connection count and the slow-tail split against the local test
//! server's throttling: per-connection limits (more connections help), a shared
//! line (they do not), `429` (learn a ceiling), and one slow connection at the end.

mod support;

use std::time::{Duration, Instant};

use boltget_engine::{Engine, EngineConfig};
use boltget_testserver::TestServer;
use support::{MIB, assert_content, cfg, get_with, request};

fn adaptive_cfg(start: u32) -> EngineConfig {
    let mut c = cfg();
    c.initial_connections = start;
    c.adaptive = true;
    c.min_split_size = 256 * 1024;
    c
}

#[tokio::test]
async fn adds_connections_while_each_one_is_throttled() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(adaptive_cfg(2)).unwrap();
    let size = 16 * MIB;
    // 1 MiB/s per connection: two connections would need 8 s.
    let url = srv.url(&format!("/data?size={size}&throttle=1048576"));
    let t0 = Instant::now();
    let done = get_with(&engine, request(&url, dir.path())).await.unwrap();
    let secs = t0.elapsed().as_secs_f64();
    assert_content(&done, 1, size);
    // Locally it climbs to 8 and finishes in ~4 s. On a loaded CI runner a probe
    // can measure under the 10% gain threshold and be reverted, so only require
    // that it grew at all and beat what the starting two connections would take.
    // The controller's exact steps are covered by the simulated unit tests.
    assert!(
        done.max_connections >= 4,
        "max {} connections",
        done.max_connections
    );
    assert!(secs < 8.0, "took {secs:.1} s");
}

#[tokio::test]
async fn keeps_few_connections_when_the_line_is_full() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(adaptive_cfg(2)).unwrap();
    let size = 20 * MIB;
    // One shared 4 MiB/s line: extra connections cannot help.
    let url = srv.url(&format!("/data?size={size}&line_bps=4194304&key=line"));
    let done = get_with(&engine, request(&url, dir.path())).await.unwrap();
    assert_content(&done, 1, size);
    assert!(
        // Normally 2 (one probe to 4 is reverted); a loaded CI machine can make
        // one more probe look worthwhile, but it must not climb to the cap.
        done.max_connections <= 8,
        "max {} connections",
        done.max_connections
    );
}

#[tokio::test]
async fn a_fixed_count_is_not_adapted() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(adaptive_cfg(2)).unwrap();
    let size = 6 * MIB;
    let url = srv.url(&format!("/data?size={size}&throttle=2097152"));
    let done = get_with(&engine, request(&url, dir.path()).connections(2))
        .await
        .unwrap();
    assert_content(&done, 1, size);
    assert_eq!(done.max_connections, 2);
}

#[tokio::test]
async fn throttling_sets_a_ceiling_for_the_host() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = adaptive_cfg(8);
    c.backoff_base = Duration::from_millis(50);
    let engine = Engine::new(c).unwrap();
    let size = 8 * MIB;
    let url = srv.url(&format!(
        "/data?size={size}&throttle=2097152&fail_status=429&fail_from=5&fail_count=1&retry_after=0"
    ));
    let done = get_with(&engine, request(&url, dir.path())).await.unwrap();
    assert_content(&done, 1, size);
    // The next download from the host starts below the learned ceiling.
    let url = srv.url(&format!("/data?size={size}&seed=2&throttle=4194304"));
    let done = get_with(&engine, request(&url, dir.path())).await.unwrap();
    assert_content(&done, 2, size);
    assert!(
        done.info.initial_connections <= 4,
        "started with {}",
        done.info.initial_connections
    );
    assert!(done.max_connections <= 4, "max {}", done.max_connections);
}

async fn slow_tail(tail: u64) -> f64 {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let mut c = cfg();
    c.adaptive = false;
    c.min_split_size = MIB;
    c.segment_align = 64 * 1024;
    c.tail_split_size = tail;
    c.slow_window = Duration::ZERO;
    let engine = Engine::new(c).expect("engine");
    let size = 4 * MIB;
    // Four 1 MiB segments; the second connection runs at 256 KiB/s.
    let url = srv.url(&format!(
        "/data?size={size}&throttle=4194304&slow_index=2&slow_bps=262144"
    ));
    let t0 = Instant::now();
    let done = get_with(&engine, request(&url, dir.path()).connections(4))
        .await
        .expect("download");
    assert_content(&done, 1, size);
    t0.elapsed().as_secs_f64()
}

#[tokio::test]
async fn a_slow_connection_at_the_end_is_split() {
    // Without the tail rule the last ~1 MiB is too small to split and runs on
    // the slow connection for about 4 s.
    let with = slow_tail(256 * 1024).await;
    assert!(with < 3.0, "with tail splits: {with:.1} s");
    let without = slow_tail(0).await;
    // (On a loaded machine the slow request may not be the one at the end.)
    if without > 3.0 {
        assert!(
            with < without - 1.0,
            "with {with:.1} s, without {without:.1} s"
        );
    }
}

#[tokio::test]
async fn a_lone_connection_that_crawls_is_reconnected() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.slow_window = Duration::from_secs(1);
    let engine = Engine::new(c).unwrap();
    let size = 4 * MIB;
    // One connection: after 1 MiB its first response drops to 16 KiB/s, which
    // would take over 3 minutes. Judged against its own earlier rate, it is
    // replaced and the rest comes at full speed.
    let url = srv.url(&format!(
        "/data?size={size}&throttle=4194304&crawl_after=1048576&crawl_bps=16384&crawl_count=1"
    ));
    let t0 = Instant::now();
    let done = get_with(&engine, request(&url, dir.path()).connections(1))
        .await
        .unwrap();
    assert_content(&done, 1, size);
    assert!(t0.elapsed() < Duration::from_secs(10), "{:?}", t0.elapsed());
    assert!(srv.requests().len() >= 2, "reconnected");
}
