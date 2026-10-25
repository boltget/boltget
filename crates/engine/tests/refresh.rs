//! Link expiry and refresh against the local test server: the `NeedsNewLink`
//! state, validating a new link (size, `ETag`, sample bytes) and continuing.

mod support;

use boltget_engine::{
    DownloadId, Engine, Error, ErrorKind, RefreshEvidence, RequestContext, ResumeOptions, State,
};
use boltget_testserver::TestServer;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use support::{MIB, assert_content, cfg, request};

const SIZE: u64 = 4 * MIB;

/// A link that works for the first four requests (the probe and three more
/// workers) and then expires, while worker 1 is slow, so a split request comes
/// after the expiry and the download stops part-way.
fn expiring(srv: &TestServer, fail: &str) -> String {
    srv.url(&format!(
        "/data?size={SIZE}&throttle=2097152&slow_index=1&slow_bps=131072&{fail}"
    ))
}

/// Starts a download of `url` and waits for it to need a new link.
async fn until_needs_new_link(engine: &Engine, url: &str, dir: &std::path::Path) -> DownloadId {
    let h = engine.add(request(url, dir).connections(4));
    let id = h.id();
    let err = tokio::time::timeout(std::time::Duration::from_secs(30), h.wait())
        .await
        .expect("download stopped")
        .expect_err("the link expires");
    assert_eq!(err.kind(), ErrorKind::LinkExpired, "{err}");
    let s = &engine.list().await.expect("list")[0];
    assert_eq!(s.state, State::NeedsNewLink);
    assert_eq!(
        s.error.as_ref().expect("error saved").kind,
        ErrorKind::LinkExpired
    );
    assert!(s.downloaded > 0 && s.downloaded < SIZE, "{}", s.downloaded);
    id
}

async fn finish(engine: &Engine, id: DownloadId) -> boltget_engine::Completed {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        engine
            .resume(id, ResumeOptions::default())
            .await
            .expect("resume starts")
            .wait(),
    )
    .await
    .expect("resume finished")
    .expect("resume completes")
}

#[tokio::test]
async fn expired_link_is_refreshed_by_etag() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let id = until_needs_new_link(
        &engine,
        &expiring(&srv, "fail_status=403&fail_from=4"),
        dir.path(),
    )
    .await;
    let fresh = srv.url(&format!("/data?size={SIZE}&key=fresh"));
    let out = engine
        .refresh_link(id, RequestContext::new(&fresh).unwrap())
        .await
        .unwrap();
    assert_eq!(out.evidence, RefreshEvidence::SameEtag);
    assert_eq!(engine.list().await.unwrap()[0].state, State::Paused);
    let sources = engine.sources(id).await.unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[1].1, "refreshed");
    let before = srv.requests().len();
    let done = finish(&engine, id).await;
    assert_content(&done, 1, SIZE);
    assert!(done.resumed_from > 0);
    // The rest came from the new link only.
    for r in &srv.requests()[before..] {
        assert!(r.path.contains("key=fresh"), "{}", r.path);
    }
    engine.shutdown().await;
}

#[tokio::test]
async fn new_link_with_another_etag_is_checked_by_samples() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let id = until_needs_new_link(
        &engine,
        &expiring(&srv, "fail_status=410&fail_from=4"),
        dir.path(),
    )
    .await;

    // Same size, different content: rejected by the samples.
    let other = srv.url(&format!("/data?size={SIZE}&seed=2&etag_salt=x"));
    let err = engine
        .refresh_link(id, RequestContext::new(&other).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::RefreshRejected(_)), "{err}");
    assert!(err.to_string().contains("different file"), "{err}");
    assert_eq!(err.kind(), ErrorKind::ResourceChanged);
    // Different size: rejected before any sample.
    let smaller = srv.url(&format!("/data?size={}&etag_salt=x", SIZE - 1));
    let err = engine
        .refresh_link(id, RequestContext::new(&smaller).unwrap())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("size differs"), "{err}");
    // Nothing changed.
    let s = &engine.list().await.unwrap()[0];
    assert_eq!(s.state, State::NeedsNewLink);
    assert!(
        s.url.contains("fail_status") || s.url.ends_with('…'),
        "{}",
        s.url
    );

    // Same bytes behind another ETag: accepted by the samples.
    let same = srv.url(&format!("/data?size={SIZE}&etag_salt=edge2"));
    let out = engine
        .refresh_link(id, RequestContext::new(&same).unwrap())
        .await
        .unwrap();
    let RefreshEvidence::Samples { ranges, bytes } = out.evidence else {
        panic!("expected samples, got {:?}", out.evidence);
    };
    assert!((1..=3).contains(&ranges) && bytes > 0, "{ranges} {bytes}");
    // The sample requests asked for closed ranges and carried no If-Range.
    let samples: Vec<_> = srv
        .requests()
        .into_iter()
        .filter(|r| r.path.contains("edge2") && r.range.as_deref() != Some("bytes=0-"))
        .collect();
    assert_eq!(samples.len(), ranges);
    assert!(samples.iter().all(|r| r.if_range.is_none()));
    let done = finish(&engine, id).await;
    assert_content(&done, 1, SIZE);
    engine.shutdown().await;
}

#[tokio::test]
async fn login_page_instead_of_the_file_needs_a_new_link() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let id = until_needs_new_link(&engine, &expiring(&srv, "html_from=4"), dir.path()).await;
    let msg = engine.list().await.unwrap()[0]
        .error
        .clone()
        .unwrap()
        .message;
    assert!(msg.contains("HTML page"), "{msg}");
    let fresh = srv.url(&format!("/data?size={SIZE}&key=fresh"));
    engine
        .refresh_link(id, RequestContext::new(&fresh).unwrap())
        .await
        .unwrap();
    assert_content(&finish(&engine, id).await, 1, SIZE);
    engine.shutdown().await;
}

#[tokio::test]
async fn redirect_to_an_error_elsewhere_needs_a_new_link() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let enc = |u: &str| utf8_percent_encode(u, NON_ALPHANUMERIC).to_string();
    // The original link redirects to the file at first, later to a 404 on
    // another host; the file's own URL expires in between.
    let target = expiring(&srv, "fail_status=403&fail_from=4");
    let elsewhere = format!("http://localhost:{}/missing", srv.addr().port());
    let original = srv.url(&format!(
        "/switch?after=1&to={}&then={}",
        enc(&target),
        enc(&elsewhere)
    ));
    let id = until_needs_new_link(&engine, &original, dir.path()).await;
    let msg = engine.list().await.unwrap()[0]
        .error
        .clone()
        .unwrap()
        .message;
    assert!(msg.contains("localhost"), "{msg}");
    engine.shutdown().await;
    let _ = id;
}

#[tokio::test]
async fn refresh_refuses_running_completed_and_unknown_downloads() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let url = srv.url("/data?size=1000");
    let ctx = || RequestContext::new(&url).unwrap();
    let err = engine.refresh_link(DownloadId(9), ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let h = engine.add(request(
        &srv.url("/data?size=4194304&throttle=262144"),
        dir.path(),
    ));
    let running = h.id();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let err = engine.refresh_link(running, ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidState);
    h.cancel();
    let _ = h.wait().await;
    let done = engine.add(request(&url, dir.path())).wait().await.unwrap();
    let err = engine.refresh_link(done.id, ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidState);
    engine.shutdown().await;
}
