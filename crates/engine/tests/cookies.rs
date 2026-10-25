//! Cookies against the local test server: `cookies.txt` files, per-URL
//! matching across redirects, `Set-Cookie` on a redirect hop, and what a
//! resumed download does with its cookie file.

mod support;

use std::path::Path;

use boltget_engine::{
    AddRequest, Cookie, Destination, Engine, ErrorKind, RequestContext, ResumeOptions, State,
};
use boltget_testserver::TestServer;
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
