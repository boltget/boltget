//! Integration tests: real downloads from the local misbehaving test server,
//! verified byte for byte (SHA-256) against the generated content.

use std::path::Path;
use std::time::Duration;

use boltget_engine::{
    AddRequest, Completed, ConflictPolicy, Destination, Engine, EngineConfig, EngineEvent, Error,
    ErrorKind, RangeSupport, RequestContext, State, header,
};
use boltget_testserver::{TestServer, content};
use sha2::{Digest, Sha256};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

/// Small, fast tunables so tests exercise splitting and retries quickly.
fn cfg() -> EngineConfig {
    let mut c = EngineConfig::default();
    c.min_split_size = 64 * KIB;
    c.segment_align = 4 * KIB;
    c.coalesce_size = 32 * 1024;
    c.worker_stagger = Duration::from_millis(5);
    c.flush_interval = Duration::from_millis(50);
    c.backoff_base = Duration::from_millis(10);
    c.backoff_max = Duration::from_millis(100);
    c.stall_timeout = Duration::from_millis(800);
    c.first_byte_timeout = Duration::from_secs(10);
    c.connect_timeout = Duration::from_secs(5);
    c.disk_margin = 0;
    c
}

fn sha(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

fn sha_file(p: &Path) -> Vec<u8> {
    sha(&std::fs::read(p).expect("read downloaded file"))
}

async fn get(engine: &Engine, url: &str, dir: &Path) -> Result<Completed, Error> {
    get_with(
        engine,
        AddRequest::new(
            RequestContext::new(url)?,
            Destination::Directory(dir.to_path_buf()),
        ),
    )
    .await
}

async fn get_with(engine: &Engine, req: AddRequest) -> Result<Completed, Error> {
    tokio::time::timeout(Duration::from_secs(60), engine.add(req).wait())
        .await
        .expect("download timed out")
}

fn assert_content(done: &Completed, seed: u64, size: u64) {
    assert_eq!(done.bytes, size);
    assert_eq!(
        std::fs::metadata(&done.path).expect("stat result").len(),
        size
    );
    assert_eq!(
        sha_file(&done.path),
        sha(&content(seed, size)),
        "content mismatch"
    );
    let partial = format!("{}{}", done.path.display(), boltget_engine::PARTIAL_SUFFIX);
    assert!(!Path::new(&partial).exists(), "partial file left behind");
}

#[tokio::test]
async fn segmented_download_is_byte_identical() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 8 * MIB + 12_345;
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&seed=3")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 3, size);
    assert_eq!(done.info.probe.ranges, RangeSupport::Yes);
    assert_eq!(done.info.initial_connections, 8);
    assert!(done.max_connections > 1, "expected parallel connections");
    assert!(done.segments >= 8);
    // The probe used GET with an open range, never HEAD.
    let reqs = srv.requests();
    assert!(reqs.iter().all(|r| r.method == "GET"));
    assert_eq!(reqs[0].range.as_deref(), Some("bytes=0-"));
    // Every later range request carries the strong ETag as If-Range.
    for r in &reqs[1..] {
        assert!(r.range.is_some());
        assert_eq!(
            r.if_range.as_deref(),
            Some(format!("\"3-{size}-v1\"").as_str())
        );
    }
    assert_eq!(done.path.file_name().unwrap(), "data");
}

#[tokio::test]
async fn sparse_preallocation_on_windows() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let done = get(&engine, &srv.url("/data?size=1048576"), dir.path())
        .await
        .unwrap();
    if cfg!(windows) {
        assert_eq!(
            done.info.preallocation,
            boltget_engine::Preallocation::Sparse
        );
    }
}

#[tokio::test]
async fn dynamic_splitting_rebalances_throttled_connections() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.initial_connections = 4;
    // A long stagger makes the later workers fall behind worker 0, so the
    // first worker to finish must split someone else's remaining bytes.
    c.worker_stagger = Duration::from_millis(200);
    let engine = Engine::new(c).unwrap();
    let size = 2 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&throttle=524288")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert!(
        done.segments > 4,
        "expected dynamic splits, got {} segments",
        done.segments
    );
}

#[tokio::test]
async fn small_file_uses_one_connection() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let done = get(&engine, &srv.url("/data?size=1000&seed=9"), dir.path())
        .await
        .unwrap();
    assert_content(&done, 9, 1000);
    assert_eq!(done.segments, 1);
    assert_eq!(srv.requests().len(), 1);
}

#[tokio::test]
async fn no_range_support_falls_back_to_single_connection() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 3 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&ranges=none")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert_eq!(done.info.probe.ranges, RangeSupport::No);
    assert_eq!(done.segments, 1);
}

#[tokio::test]
async fn server_ignoring_zero_range_is_verified_and_segmented() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 4 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&ranges=ignore-zero")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert_eq!(done.info.probe.ranges, RangeSupport::Yes, "verified");
    assert!(done.segments > 1, "verification should enable segmentation");
    let reqs = srv.requests();
    assert_eq!(reqs[0].status, 200);
    assert!(
        reqs.iter()
            .any(|r| r.range.as_deref() == Some("bytes=2097152-") && r.status == 206)
    );
}

#[tokio::test]
async fn unadvertised_range_support_is_still_used() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 4 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!(
            "/data?size={size}&ranges=ignore-zero&accept_ranges=0"
        )),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert!(done.segments > 1);
}

#[tokio::test]
async fn wrong_content_length_is_retried() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 2 * MIB;
    // The first three responses declare the full length but close after half.
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&short_count=3")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
}

#[tokio::test]
async fn wrong_content_length_without_ranges_restarts_from_zero() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = MIB;
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&ranges=none&short_count=2")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    // Probe, verification request, and at least one full restart.
    assert!(srv.requests().len() >= 3);
}

#[tokio::test]
async fn connection_resets_are_retried() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 6 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!(
            "/data?size={size}&reset_after=200000&reset_count=12&seed=5"
        )),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 5, size);
}

#[tokio::test]
async fn stalled_connections_are_detected_and_resumed() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 3 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!(
            "/data?size={size}&stall_after=100000&stall_count=3"
        )),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
}

#[tokio::test]
async fn etag_change_mid_download_is_detected() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    // From the third request on the content and ETag change; If-Range then
    // yields 200 instead of 206 and the engine must stop.
    let err = get(
        &engine,
        &srv.url("/data?size=4194304&change_after=2"),
        dir.path(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ResourceChanged, "{err}");
}

#[tokio::test]
async fn weak_etag_change_is_detected_without_if_range() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let err = get(
        &engine,
        &srv.url("/data?size=4194304&etag=weak&change_after=2"),
        dir.path(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ResourceChanged, "{err}");
    // Weak ETags are never sent in If-Range.
    assert!(srv.requests().iter().all(|r| r.if_range.is_none()));
}

#[tokio::test]
async fn last_modified_is_used_as_if_range() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let done = get(
        &engine,
        &srv.url("/data?size=2097152&etag=none&lastmod=1"),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, 2 * MIB);
    let reqs = srv.requests();
    assert!(reqs.len() > 1);
    assert!(
        reqs[1..]
            .iter()
            .all(|r| r.if_range.as_deref().is_some_and(|v| v.ends_with("GMT")))
    );
}

#[tokio::test]
async fn ignored_if_range_with_changed_content_is_detected_by_etag() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    // The server serves 206 with the *new* content despite If-Range.
    let err = get(
        &engine,
        &srv.url("/data?size=4194304&change_after=2&ifrange=ignore"),
        dir.path(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ResourceChanged, "{err}");
}

#[tokio::test]
async fn redirects_are_followed_and_recorded() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let target = srv.url("/data?size=2097152&seed=4");
    let enc = percent_encode(&target);
    let done = get(
        &engine,
        &srv.url(&format!("/redirect?hops=3&code=307&to={enc}")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 4, 2 * MIB);
    assert_eq!(done.info.probe.redirects, 3);
    // Segment requests go straight to the final URL.
    let redirect_hits = srv
        .requests()
        .iter()
        .filter(|r| r.path.starts_with("/redirect"))
        .count();
    assert_eq!(redirect_hits, 3);
}

#[tokio::test]
async fn too_many_redirects_fail() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let target = percent_encode(&srv.url("/data"));
    let err = get(
        &engine,
        &srv.url(&format!("/redirect?hops=11&to={target}")),
        dir.path(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::TooManyRedirects(10)), "{err}");
}

#[tokio::test]
async fn authorization_is_dropped_on_cross_origin_redirect() {
    let a = TestServer::start().await;
    let b = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let to = percent_encode(&b.url("/auth?size=1000"));
    let ctx = RequestContext::new(&a.url(&format!("/redirect?to={to}")))
        .unwrap()
        .with_header(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
    let err = get_with(
        &engine,
        AddRequest::new(ctx, Destination::Directory(dir.path().into())),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::Status { status: 401, .. }), "{err}");
    assert!(a.requests()[0].authorization, "origin A gets the header");
    assert!(
        b.requests().iter().all(|r| !r.authorization),
        "origin B must not"
    );
}

#[tokio::test]
async fn expiring_signed_url_is_re_resolved_from_original() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.initial_connections = 2;
    let engine = Engine::new(c).unwrap();
    // Each signed URL works for 2 requests; the original URL mints a new one.
    let size = 4 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!("/signed?valid=2&size={size}&seed=8&key=signed")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 8, size);
    let signs = srv
        .requests()
        .iter()
        .filter(|r| r.path.starts_with("/signed"))
        .count();
    assert!(
        signs >= 2,
        "expected a re-resolve, saw {signs} /signed requests"
    );
}

#[tokio::test]
async fn content_disposition_variants() {
    let srv = TestServer::start().await;
    let engine = Engine::new(cfg()).unwrap();
    let cases: &[(&str, &str)] = &[
        ("attachment%3B%20filename%3D%22report.pdf%22", "report.pdf"),
        (
            "attachment%3B%20filename%3D%22fallback.txt%22%3B%20filename*%3DUTF-8''na%25C3%25AFve%2520file.txt",
            "naïve file.txt",
        ),
        // Raw UTF-8 bytes in the quoted filename.
        (
            "attachment%3B%20filename%3D%22r%C3%A9sum%C3%A9.doc%22",
            "résumé.doc",
        ),
        // Path traversal is stripped.
        (
            "attachment%3B%20filename%3D%22..%2F..%2Fevil.exe%22",
            "evil.exe",
        ),
        // Reserved device name.
        ("attachment%3B%20filename%3DCON.txt", "_CON.txt"),
    ];
    for (cd, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let done = get(
            &engine,
            &srv.url(&format!("/data?size=100&cd={cd}")),
            dir.path(),
        )
        .await
        .unwrap();
        assert_eq!(done.path.file_name().unwrap().to_string_lossy(), *expected);
        assert_eq!(done.path.parent().unwrap(), dir.path());
    }
}

#[tokio::test]
async fn content_type_adds_extension() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let done = get(
        &engine,
        &srv.url("/data?size=10&ctype=application/pdf"),
        dir.path(),
    )
    .await
    .unwrap();
    assert_eq!(done.path.file_name().unwrap(), "data.pdf");
}

#[tokio::test]
async fn name_collisions_get_a_suffix() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data"), b"existing").unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let done = get(&engine, &srv.url("/data?size=10"), dir.path())
        .await
        .unwrap();
    assert_eq!(done.path.file_name().unwrap(), "data (1)");
    assert_eq!(std::fs::read(dir.path().join("data")).unwrap(), b"existing");

    let ctx = RequestContext::new(&srv.url("/data?size=10&seed=2")).unwrap();
    let req = AddRequest::new(ctx, Destination::File(dir.path().join("data")))
        .conflict(ConflictPolicy::Overwrite);
    let done = get_with(&engine, req).await.unwrap();
    assert_eq!(done.path, dir.path().join("data"));
    assert_content(&done, 2, 10);
}

#[tokio::test]
async fn empty_resource_416() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let done = get(&engine, &srv.url("/data?size=0"), dir.path())
        .await
        .unwrap();
    assert_eq!(done.bytes, 0);
    assert_eq!(std::fs::metadata(&done.path).unwrap().len(), 0);
}

#[tokio::test]
async fn throttled_429_with_retry_after_is_honoured() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let t = std::time::Instant::now();
    let done = get(
        &engine,
        &srv.url("/data?size=2097152&fail_status=429&fail_count=1&retry_after=1&key=t429"),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, 2 * MIB);
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "Retry-After ignored"
    );
}

#[tokio::test]
async fn service_unavailable_503_is_retried() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let done = get(
        &engine,
        &srv.url("/data?size=2097152&fail_status=503&fail_count=3&key=t503"),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, 2 * MIB);
}

#[tokio::test]
async fn throttling_mid_download_reduces_connections_and_completes() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    // Requests 4..14 are refused with 503: those workers give their segments
    // back (fewer connections) and the remaining ones finish the job.
    let done = get(
        &engine,
        &srv.url("/data?size=4194304&fail_status=503&fail_from=4&fail_count=10&key=tmid&throttle=8388608"),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, 4 * MIB);
}

#[tokio::test]
async fn permanent_errors_fail_fast() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let err = get(&engine, &srv.url("/missing"), dir.path())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 404, .. }), "{err}");
    assert_eq!(err.kind(), ErrorKind::Permanent);
    assert_eq!(srv.requests().len(), 1, "404 is not retried");
    let err = get(&engine, &srv.url("/data?fail_status=403"), dir.path())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Permanent);
}

#[tokio::test]
async fn unsatisfiable_range_416_on_nonempty_is_permanent() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let err = get(&engine, &srv.url("/data?fail_status=416"), dir.path())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 416, .. }), "{err}");
}

#[tokio::test]
async fn unknown_size_chunked_download() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 3 * MIB + 7;
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&ranges=none&chunked=1&seed=6")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 6, size);
    assert_eq!(done.info.probe.size, None);
}

#[tokio::test]
async fn unknown_total_in_content_range_is_resumable_single_connection() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 2 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!(
            "/data?size={size}&star_total=1&reset_after=300000&reset_count=2"
        )),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert_eq!(done.segments, 1);
    // The retry continued from the cursor instead of restarting.
    assert!(
        srv.requests()
            .iter()
            .any(|r| r.range.as_deref().is_some_and(|v| v != "bytes=0-"))
    );
}

#[tokio::test]
async fn content_encoding_forces_single_connection() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 2 * MIB;
    let done = get(
        &engine,
        &srv.url(&format!("/data?size={size}&ranges=none&encoding=gzip")),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert_eq!(srv.requests().len(), 1);
}

#[tokio::test]
async fn per_host_cap_is_respected() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.max_per_host = 3;
    c.initial_connections = 8;
    let engine = Engine::new(c).unwrap();
    let size = 4 * MIB;
    // Two concurrent downloads share the host's three slots.
    let url_a = srv.url(&format!("/data?size={size}&key=a&throttle=4194304"));
    let url_b = srv.url(&format!("/data?size={size}&key=b&seed=2&throttle=4194304"));
    let (a, b) = tokio::join!(
        get(&engine, &url_a, dir.path()),
        get(&engine, &url_b, dir.path()),
    );
    assert_content(&a.unwrap(), 1, size);
    assert_content(&b.unwrap(), 2, size);
    // Without the cap this would be up to 16. The server notices a closed
    // socket slightly after the client released its slot, so allow a little
    // overlap here; the cap itself is unit-tested exactly.
    assert!(
        srv.max_concurrent_connections() <= 5,
        "saw {} concurrent connections",
        srv.max_concurrent_connections()
    );
}

#[tokio::test]
async fn cancellation_keeps_partial_file() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let mut events = engine.subscribe();
    let ctx = RequestContext::new(&srv.url("/data?size=8388608&throttle=262144")).unwrap();
    let h = engine.add(AddRequest::new(
        ctx,
        Destination::Directory(dir.path().into()),
    ));
    loop {
        if let Ok(EngineEvent::StateChanged {
            state: State::Downloading,
            ..
        }) = events.recv().await
        {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let partial = h.info().unwrap().partial_path.clone();
    h.cancel();
    let err = h.wait().await.unwrap_err();
    assert!(matches!(err, Error::Cancelled));
    assert!(partial.exists(), "partial file is kept for resume");
}

#[tokio::test]
async fn progress_and_events_are_reported() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let mut events = engine.subscribe();
    let ctx = RequestContext::new(&srv.url("/data?size=4194304&throttle=262144")).unwrap();
    let h = engine.add(AddRequest::new(
        ctx,
        Destination::Directory(dir.path().into()),
    ));
    let id = h.id();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let p = h.progress();
    assert_eq!(p.total, Some(4 * MIB));
    assert!(p.connections >= 1);
    assert_ne!(p.segments, [] as [boltget_engine::segment::Segment; 0]);
    assert_eq!(engine.progress(&[id]).len(), 1);
    let done = h.wait().await.unwrap();
    assert_content(&done, 1, 4 * MIB);
    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        seen.push(e);
    }
    assert!(matches!(seen.first(), Some(EngineEvent::Added { .. })));
    assert!(seen.iter().any(|e| matches!(e, EngineEvent::Probed { .. })));
    assert!(
        seen.iter()
            .any(|e| matches!(e, EngineEvent::Completed { .. }))
    );
    let json = serde_json::to_string(&seen).unwrap();
    assert!(json.contains("\"event\":\"completed\""));
}

#[tokio::test]
async fn engine_probe_reports_without_downloading() {
    let srv = TestServer::start().await;
    let engine = Engine::new(cfg()).unwrap();
    let ctx =
        RequestContext::new(&srv.url("/data?size=5000&cd=attachment%3Bfilename%3Dx.bin")).unwrap();
    let p = engine.probe(&ctx).await.unwrap();
    assert_eq!(p.size, Some(5000));
    assert_eq!(p.ranges, RangeSupport::Yes);
    assert_eq!(p.filename, "x.bin");
    assert!(p.validators.strong_etag().is_some());
    let ctx = RequestContext::new(&srv.url("/data?size=5000&ranges=ignore-zero")).unwrap();
    assert_eq!(engine.probe(&ctx).await.unwrap().ranges, RangeSupport::Yes);
    let ctx = RequestContext::new(&srv.url("/data?size=5000&ranges=none")).unwrap();
    assert_eq!(engine.probe(&ctx).await.unwrap().ranges, RangeSupport::No);
}

#[tokio::test]
async fn endgame_duplicates_slow_tail() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.endgame = true;
    c.endgame_threshold = Duration::from_millis(200);
    c.initial_connections = 3;
    let engine = Engine::new(c).unwrap();
    let size = 3 * MIB;
    // Request #2 (the third segment's worker) crawls at 64 KiB/s; splitting
    // shrinks its share, and the endgame duplicate takes over its tail.
    let done = get(
        &engine,
        &srv.url(&format!(
            "/data?size={size}&throttle=4194304&slow_index=2&slow_bps=65536"
        )),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert!(done.endgame_takeovers >= 1, "endgame did not trigger");
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::test]
async fn slow_connection_is_reconnected() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.initial_connections = 4;
    c.stall_timeout = Duration::from_secs(30);
    c.slow_window = Duration::from_millis(300);
    let engine = Engine::new(c).unwrap();
    let size = 4 * MIB;
    // Request #2 crawls at 1 KiB/s but never stalls completely. Splitting alone
    // would leave it tens of KiB, i.e. tens of seconds; the slow-connection
    // detector reconnects it (the new request is fast).
    let t = std::time::Instant::now();
    let done = get(
        &engine,
        &srv.url(&format!(
            "/data?size={size}&throttle=1048576&slow_index=2&slow_bps=1024"
        )),
        dir.path(),
    )
    .await
    .unwrap();
    assert_content(&done, 1, size);
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "took {:?}",
        t.elapsed()
    );
}
