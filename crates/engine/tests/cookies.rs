//! Cookies against the local test server: `cookies.txt` files, per-URL
//! matching across redirects, `Set-Cookie` on a redirect hop, and what a
//! resumed download does with its cookie file.

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use boltget_engine::{
    AddRequest, Cookie, DEFAULT_QUEUE, Destination, DownloadId, Engine, ErrorKind, RequestContext,
    ResumeOptions, State, SystemClock,
};
use boltget_testserver::{RequestRecord, TestServer};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use support::{MIB, assert_content, cfg, get_with};

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn cookie_file(dir: &Path, lines: &[&str]) -> std::path::PathBuf {
    let p = dir.join("cookies.txt");
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    for l in lines {
        text.push_str(l);
        text.push('\n');
    }
    std::fs::write(&p, text).expect("write cookie file");
    p
}

fn req(ctx: RequestContext, dir: &Path) -> AddRequest {
    AddRequest::new(ctx, Destination::Directory(dir.to_path_buf()))
}

#[tokio::test]
async fn cookie_file_is_sent_only_where_it_matches() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = cookie_file(
        dir.path(),
        &["127.0.0.1\tFALSE\t/\tFALSE\t0\tsession\tabc123"],
    );
    let engine = Engine::new(cfg()).unwrap();
    let size = 2 * MIB;
    let url = srv.url(&format!(
        "/cookie?need={}&size={size}&seed=7",
        enc("session=abc123")
    ));
    let ctx = RequestContext::new(&url)
        .unwrap()
        .with_cookie_file(&file)
        .unwrap();
    let done = get_with(&engine, req(ctx, dir.path()).connections(4))
        .await
        .unwrap();
    assert_content(&done, 7, size);
    let reqs = srv.requests();
    assert!(reqs.len() > 1);
    assert!(
        reqs.iter()
            .all(|r| r.cookie.as_deref() == Some("session=abc123"))
    );

    // A redirect to another host does not receive the cookie.
    let elsewhere = format!(
        "http://localhost:{}/data?size=1000&key=x",
        srv.addr().port()
    );
    let start = srv.requests().len();
    let url = srv.url(&format!("/redirect?to={}", enc(&elsewhere)));
    let ctx = RequestContext::new(&url)
        .unwrap()
        .with_cookie_file(&file)
        .unwrap();
    get_with(&engine, req(ctx, dir.path())).await.unwrap();
    let reqs = &srv.requests()[start..];
    assert_eq!(reqs[0].cookie.as_deref(), Some("session=abc123"));
    assert!(reqs[1..].iter().all(|r| r.cookie.is_none()), "{reqs:?}");
}

#[tokio::test]
async fn set_cookie_on_a_redirect_hop_is_used() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = MIB;
    let target = srv.url(&format!("/cookie?need={}&size={size}", enc("sid=42")));
    let url = srv.url(&format!(
        "/setcookie?set={}&to={}",
        enc("sid=42; Path=/; HttpOnly"),
        enc(&target)
    ));
    // Without a jar, Set-Cookie is not kept and the file is refused.
    let err = get_with(&engine, req(RequestContext::new(&url).unwrap(), dir.path()))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Permanent, "{err}");
    // With a jar (here from --cookie style cookies), it is.
    let mut ctx = RequestContext::new(&url).unwrap();
    ctx.add_cookie(Cookie::for_host("127.0.0.1", "lang", "en"));
    let done = get_with(&engine, req(ctx, dir.path()).connections(2))
        .await
        .unwrap();
    assert_content(&done, 1, size);
    let last = srv.requests().pop().unwrap();
    let cookie = last.cookie.unwrap();
    assert!(
        cookie.contains("sid=42") && cookie.contains("lang=en"),
        "{cookie}"
    );
}

#[tokio::test]
async fn resume_reads_the_cookie_file_again() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let file = cookie_file(
        dir.path(),
        &["127.0.0.1\tFALSE\t/\tFALSE\t0\tsession\tsupersecretvalue"],
    );
    let size = 2 * MIB;
    let url = srv.url(&format!(
        "/cookie?need={}&size={size}&seed=8&throttle=1048576",
        enc("session=supersecretvalue")
    ));
    let id = {
        let engine = Engine::open(cfg(), state.path()).unwrap();
        let ctx = RequestContext::new(&url)
            .unwrap()
            .with_cookie_file(&file)
            .unwrap();
        let h = engine.add(req(ctx, dir.path()).connections(2));
        let id = h.id();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(engine.pause(id).await.unwrap(), State::Paused);
        engine.shutdown().await;
        id
    };
    for f in std::fs::read_dir(state.path()).unwrap() {
        let bytes = std::fs::read(f.unwrap().path()).unwrap();
        assert!(
            // (The URL holds the value percent-encoded, as the test server needs it.)
            !String::from_utf8_lossy(&bytes).contains("session=supersecretvalue"),
            "cookie value saved to disk"
        );
    }
    let engine = Engine::open(cfg(), state.path()).unwrap();
    // The file went away: the server refuses, and the download needs a new link.
    let moved = dir.path().join("moved.txt");
    std::fs::rename(&file, &moved).unwrap();
    let err = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LinkExpired, "{err}");
    // Back in place: resuming reads it and finishes.
    std::fs::rename(&moved, &file).unwrap();
    let done = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_content(&done, 8, size);
    engine.shutdown().await;
}

/// Cookies the way the browser extension reports them: a domain cookie for
/// the CDN host the link redirects to, and a host-only one for the page's.
fn browser_cookies(ctx: &mut RequestContext) {
    ctx.add_cookie(Cookie::from_browser(
        "sid",
        "s3cret",
        "localhost",
        false,
        "/",
        false,
        None,
    ));
    ctx.add_cookie(Cookie::from_browser(
        "page",
        "1",
        "127.0.0.1",
        true,
        "/",
        false,
        None,
    ));
}

/// A link on `srv`'s `127.0.0.1` that redirects to another host
/// (`localhost`) and path, where every request needs the `sid` cookie.
fn cdn_link(srv: &TestServer, query: &str) -> String {
    let file = format!(
        "http://localhost:{}/cookie?need={}&{query}",
        srv.addr().port(),
        enc("sid=s3cret")
    );
    srv.url(&format!("/redirect?code=302&to={}", enc(&file)))
}

async fn wait_state(engine: &Engine, id: DownloadId, want: impl Fn(State) -> bool) -> State {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let s = engine
            .list()
            .await
            .expect("list")
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| s.state)
            .expect("listed");
        if want(s) {
            return s;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "download {id} is still {s}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn error_of(engine: &Engine, id: DownloadId) -> Option<String> {
    engine
        .list()
        .await
        .expect("list")
        .into_iter()
        .find(|s| s.id == id)
        .and_then(|s| s.error)
        .map(|e| e.message)
}

/// Every request for the file carried the cookie, and every request on the
/// page's host carried only its own.
fn assert_cookies_everywhere(srv: &TestServer) -> Vec<RequestRecord> {
    let reqs = srv.requests();
    let (file, page): (Vec<_>, Vec<_>) = reqs
        .into_iter()
        .partition(|r| r.path.starts_with("/cookie"));
    assert!(!file.is_empty());
    for r in &file {
        assert_eq!(r.cookie.as_deref(), Some("sid=s3cret"), "{r:?}");
        assert!(r.status < 400, "{r:?}");
    }
    for r in &page {
        assert_eq!(r.cookie.as_deref(), Some("page=1"), "{r:?}");
    }
    file
}

/// The app's path for a captured download: it goes into a queue and the
/// scheduler starts it. Its browser cookies must reach every request,
/// across a redirect to another host and path, on the probe, on the
/// connections the adaptive controller adds and on the tail splits.
#[tokio::test(flavor = "multi_thread")]
async fn browser_cookies_reach_every_request_of_a_queued_download() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.adaptive = true;
    c.initial_connections = 2;
    c.min_split_size = 256 * 1024;
    c.tail_split_size = 256 * 1024;
    c.slow_window = Duration::ZERO;
    let engine = Engine::open(c, state.path()).unwrap();
    engine.start_queues(Arc::new(SystemClock));
    let size = 8 * MIB;
    // 1 MiB/s per connection (more connections pay off), and the third
    // request crawls (its tail is split off).
    let url = cdn_link(
        &srv,
        &format!("size={size}&seed=11&throttle=1048576&slow_index=2&slow_bps=131072&key=cdn"),
    );
    let mut ctx = RequestContext::new(&url).unwrap();
    browser_cookies(&mut ctx);
    let id = engine
        .enqueue(req(ctx, dir.path()).queue(DEFAULT_QUEUE))
        .await
        .unwrap();
    let s = wait_state(&engine, id, |s| !s.is_active() && s != State::Queued).await;
    assert_eq!(s, State::Completed, "{:?}", error_of(&engine, id).await);
    let file = assert_cookies_everywhere(&srv);
    let connections: std::collections::HashSet<_> = file.iter().map(|r| r.connection).collect();
    assert!(
        file.len() >= 4 && connections.len() >= 3,
        "{} requests on {} connections",
        file.len(),
        connections.len()
    );
    let path = engine
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == id)
        .and_then(|s| s.path)
        .unwrap();
    support::assert_content_at(&path, 11, size);
    engine.shutdown().await;
}

/// A download added paused, started, paused and resumed in the same run
/// keeps the browser's cookies and its `Authorization` every time, and the
/// database holds neither.
#[tokio::test(flavor = "multi_thread")]
async fn pausing_and_resuming_keeps_the_browsers_cookies_and_authorization() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let size = 2 * MIB;
    let url = cdn_link(&srv, &format!("size={size}&seed=12&throttle=1048576&key=p"));
    let mut ctx = RequestContext::new(&url).unwrap().with_header(
        boltget_engine::header::AUTHORIZATION,
        boltget_engine::HeaderValue::from_static("Bearer t0ken"),
    );
    browser_cookies(&mut ctx);
    let id = engine
        .add_paused(req(ctx, dir.path()).connections(3))
        .unwrap();
    engine.resume(id, ResumeOptions::default()).await.unwrap();
    wait_state(&engine, id, |s| s == State::Downloading).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(engine.pause(id).await.unwrap(), State::Paused);
    let done = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_content(&done, 12, size);
    let file = assert_cookies_everywhere(&srv);
    assert!(file.len() >= 4, "{file:?}");
    // `Authorization` goes to the link's own origin only.
    for r in srv.requests() {
        let page = !r.path.starts_with("/cookie");
        assert_eq!(r.authorization, page, "{r:?}");
    }
    engine.shutdown().await;
    for f in std::fs::read_dir(state.path()).unwrap() {
        let text = String::from_utf8_lossy(&std::fs::read(f.unwrap().path()).unwrap()).into_owned();
        assert!(!text.contains("t0ken"), "Authorization saved to disk");
    }
}

/// The browser's cookies are held in memory only: a download resumed by an
/// engine opened later runs without them, the server refuses, and the
/// download asks for a new link, saying why. A new link from the browser,
/// with its cookies, finishes it.
#[tokio::test(flavor = "multi_thread")]
async fn after_a_restart_the_browsers_cookies_are_gone_and_a_new_link_is_asked_for() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let size = 2 * MIB;
    let url = cdn_link(&srv, &format!("size={size}&seed=13&throttle=1048576&key=r"));
    let id = {
        let engine = Engine::open(cfg(), state.path()).unwrap();
        let mut ctx = RequestContext::new(&url).unwrap();
        browser_cookies(&mut ctx);
        let h = engine.add(req(ctx, dir.path()).connections(2));
        let id = h.id();
        wait_state(&engine, id, |s| s == State::Downloading).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(engine.pause(id).await.unwrap(), State::Paused);
        engine.shutdown().await;
        id
    };
    for f in std::fs::read_dir(state.path()).unwrap() {
        let text = String::from_utf8_lossy(&std::fs::read(f.unwrap().path()).unwrap()).into_owned();
        // (The URL holds it percent-encoded, as the test server needs it.)
        assert!(!text.contains("sid=s3cret"), "cookie value saved to disk");
    }
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let err = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LinkExpired, "{err}");
    assert!(err.to_string().contains("browser's cookies"), "{err}");
    let saved = engine.list().await.unwrap();
    assert_eq!(saved[0].state, State::NeedsNewLink);
    assert!(
        saved[0]
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("browser's cookies"),
        "{saved:?}"
    );

    let mut ctx = RequestContext::new(&url).unwrap();
    browser_cookies(&mut ctx);
    engine.refresh_link(id, ctx).await.unwrap();
    let done = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_content(&done, 13, size);
    engine.shutdown().await;
}
