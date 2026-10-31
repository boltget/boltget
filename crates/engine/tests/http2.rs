//! HTTPS and HTTP/2 against the local TLS test server, whose certificate comes
//! from a CA generated at start-up and trusted through
//! `EngineConfig::extra_root_certificates`: one connection per worker (never
//! multiplexed), resume, connections taken down mid-download (`GOAWAY`, a
//! closed socket, a reset stream) and `503` with `Retry-After`. Every download
//! is compared with the generated content by SHA-256.

// Helpers outside `#[test]` functions panic on failure just like the tests do.
#![allow(clippy::unwrap_used)]

mod support;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use boltget_engine::{DownloadHandle, Engine, EngineConfig, Error, ResumeOptions, State};
use boltget_testserver::{Alpn, TestServer};
use http::Version;
use support::{KIB, MIB, assert_content, cfg, get_with, request};

/// The test tunables, trusting `srv`'s CA.
fn tls_cfg(srv: &TestServer) -> EngineConfig {
    let mut c = cfg();
    c.extra_root_certificates = vec![srv.ca_cert_der().expect("TLS server").to_vec()];
    c
}

fn connections_used(srv: &TestServer) -> usize {
    srv.requests()
        .iter()
        .map(|r| r.connection)
        .collect::<HashSet<_>>()
        .len()
}

fn assert_all_http2(srv: &TestServer) {
    let reqs = srv.requests();
    assert!(!reqs.is_empty());
    for r in &reqs {
        assert_eq!(r.version, Version::HTTP_2, "{r:?}");
    }
}

#[tokio::test]
async fn segmented_download_over_http2_uses_one_connection_per_worker() {
    let srv = TestServer::start_tls(Alpn::Both).await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = tls_cfg(&srv);
    c.adaptive = false;
    let engine = Engine::new(c).unwrap();
    let size = 8 * MIB;
    // 4 MiB/s per stream: the four workers overlap for about half a second.
    let url = srv.url(&format!("/data?size={size}&seed=7&throttle=4194304"));
    let done = get_with(&engine, request(&url, dir.path()).connections(4))
        .await
        .unwrap();
    assert_content(&done, 7, size);
    assert_eq!(done.max_connections, 4);
    assert_all_http2(&srv);
    // Four workers, four TCP + TLS connections open at once ...
    assert!(
        srv.max_concurrent_connections() >= 4,
        "{} connections at once",
        srv.max_concurrent_connections()
    );
    assert!(connections_used(&srv) >= 4);
    // ... and never several workers' streams on one of them. (A worker that
    // abandons a split segment's stream and reuses its connection can overlap
    // the cancelled stream for a moment, hence 2, not 1.)
    assert!(
        srv.max_streams_per_connection() <= 2,
        "{} streams on one connection",
        srv.max_streams_per_connection()
    );
}

#[tokio::test]
async fn https_without_http2_uses_http1() {
    let srv = TestServer::start_tls(Alpn::Http1).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(tls_cfg(&srv)).unwrap();
    let size = 2 * MIB;
    let url = srv.url(&format!("/data?size={size}&seed=8"));
    let done = get_with(&engine, request(&url, dir.path()).connections(2))
        .await
        .unwrap();
    assert_content(&done, 8, size);
    for r in srv.requests() {
        assert_eq!(r.version, Version::HTTP_11, "{r:?}");
    }
}

#[tokio::test]
async fn untrusted_test_certificate_is_rejected() {
    let srv = TestServer::start_tls(Alpn::Both).await;
    let dir = tempfile::tempdir().unwrap();
    // Without the extra root, only the system's trust store applies.
    let engine = Engine::new(cfg()).unwrap();
    let url = srv.url("/data?size=1024");
    let err = get_with(&engine, request(&url, dir.path()))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Tls(_)), "{err:?}");
    assert!(srv.requests().is_empty());
}

#[tokio::test]
async fn invalid_extra_root_fails_engine_creation() {
    let mut c = cfg();
    c.extra_root_certificates = vec![b"not a certificate".to_vec()];
    // Rejected when the engine builds its TLS configuration, never ignored.
    let err = Engine::new(c).unwrap_err();
    assert!(matches!(err, Error::Tls(_)), "{err:?}");
}

#[test]
fn certificates_are_read_from_pem_and_der() {
    let ca = boltget_testserver::TestCa::generate().unwrap();
    let pem = boltget_engine::parse_certificates(ca.pem().as_bytes()).unwrap();
    assert_eq!(pem, vec![ca.der().to_vec()]);
    let der = boltget_engine::parse_certificates(ca.der()).unwrap();
    assert_eq!(der, vec![ca.der().to_vec()]);
    assert!(boltget_engine::parse_certificates(b"hello").is_err());
}

async fn until_durable(h: &DownloadHandle, bytes: u64) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let p = h.progress();
        assert!(!p.state.is_terminal(), "stopped early: {:?}", p.state);
        if p.durable >= bytes {
            return;
        }
        assert!(Instant::now() < deadline, "no progress");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn http2_download_resumes_from_its_checkpoint() {
    let srv = TestServer::start_tls(Alpn::Both).await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut c = tls_cfg(&srv);
    c.stall_timeout = Duration::from_secs(5);
    let engine = Engine::open(c, state.path()).unwrap();
    let size = 4 * MIB + 333;
    let url = srv.url(&format!("/data?size={size}&seed=9&throttle=262144"));
    let h = engine.add(request(&url, dir.path()).connections(4));
    let id = h.id();
    until_durable(&h, 256 * KIB).await;
    assert_eq!(engine.pause(id).await.unwrap(), State::Paused);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(30), h.wait())
            .await
            .unwrap(),
        Err(Error::Cancelled)
    ));

    let before = srv.requests().len();
    let sent_before = srv.body_bytes_sent();
    let h = engine.resume(id, ResumeOptions::default()).await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(90), h.wait())
        .await
        .unwrap()
        .unwrap();
    assert_content(&done, 9, size);
    assert!(done.resumed_from > 0, "resumed from 0");
    // Only the rest was fetched, over HTTP/2, every request validated.
    let resumed = &srv.requests()[before..];
    assert!(!resumed.is_empty());
    for r in resumed {
        assert_eq!(r.version, Version::HTTP_2);
        assert!(r.if_range.is_some(), "{r:?}");
        assert_ne!(r.range.as_deref(), Some("bytes=0-"), "{r:?}");
    }
    assert!(srv.body_bytes_sent() - sent_before < size);
    engine.shutdown().await;
}

/// A download whose first `count` requests lose their connection after 512 KiB.
async fn survives(behaviour: &str) {
    let srv = TestServer::start_tls(Alpn::Both).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(tls_cfg(&srv)).unwrap();
    let size = 4 * MIB;
    let url = srv.url(&format!(
        "/data?size={size}&seed=10&throttle=8388608&{behaviour}_after=524288&{behaviour}_count=2&key={behaviour}"
    ));
    let done = get_with(&engine, request(&url, dir.path()).connections(4))
        .await
        .unwrap();
    assert_content(&done, 10, size);
    assert_all_http2(&srv);
    // The interrupted segments were fetched again from where they stopped.
    assert!(srv.requests().len() >= 4 + 2, "{:?}", srv.requests());
}

#[tokio::test]
async fn goaway_mid_download_is_a_connection_error() {
    survives("goaway").await;
}

#[tokio::test]
async fn closed_http2_connection_mid_download_is_retried() {
    survives("drop").await;
}

#[tokio::test]
async fn reset_http2_stream_is_retried() {
    survives("reset").await;
}

#[tokio::test]
async fn http2_503_with_retry_after_is_honoured() {
    let srv = TestServer::start_tls(Alpn::Both).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(tls_cfg(&srv)).unwrap();
    let size = 2 * MIB;
    let t = Instant::now();
    let url = srv.url(&format!(
        "/data?size={size}&seed=11&fail_status=503&fail_count=1&retry_after=1&key=h2-503"
    ));
    let done = get_with(&engine, request(&url, dir.path())).await.unwrap();
    assert_content(&done, 11, size);
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "Retry-After ignored"
    );
    assert!(srv.requests().iter().any(|r| r.status == 503));
    assert_all_http2(&srv);
}

#[tokio::test]
async fn plain_lab_url_reaches_http2_through_tls_redirect() {
    let mut srv = TestServer::start().await;
    srv.listen_tls(([127, 0, 0, 1], 0).into(), Alpn::Both)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(tls_cfg(&srv)).unwrap();
    let size = 2 * MIB;
    let url = srv.url(&format!("/tls/data?size={size}&seed=12"));
    let done = get_with(&engine, request(&url, dir.path()).connections(2))
        .await
        .unwrap();
    assert_content(&done, 12, size);
    let reqs = srv.requests();
    assert_eq!(reqs[0].status, 307);
    assert!(
        reqs.iter()
            .any(|r| r.version == Version::HTTP_2 && r.status == 206),
        "{reqs:?}"
    );
}
