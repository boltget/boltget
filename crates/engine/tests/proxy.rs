//! Downloads through the local test proxies: HTTP forward proxy with Basic
//! auth, SOCKS5 with and without a password, `NO_PROXY`, and what is saved.

mod support;

use boltget_engine::{
    Engine, Error, ErrorKind, NoProxy, Proxy, ProxyChoice, ProxyRules, RequestContext,
    ResumeOptions, State,
};
use boltget_testserver::{ProxyMode, TestProxy, TestServer};
use support::{MIB, assert_content, cfg, get_with, request};

fn via(proxy: &str) -> ProxyChoice {
    ProxyChoice::Via(Proxy::parse(proxy).expect("valid proxy URL"))
}

fn proxied(url: &str, dir: &std::path::Path, proxy: ProxyChoice) -> boltget_engine::AddRequest {
    boltget_engine::AddRequest::new(
        RequestContext::new(url)
            .expect("valid URL")
            .with_proxy(proxy),
        boltget_engine::Destination::Directory(dir.to_path_buf()),
    )
}

#[tokio::test]
async fn http_proxy_with_basic_auth() {
    let srv = TestServer::start().await;
    let proxy = TestProxy::start(ProxyMode::Http, Some(("alice", "s3cret"))).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 2 * MIB + 5;
    let url = srv.url(&format!("/data?size={size}&seed=3"));
    let done = get_with(
        &engine,
        proxied(&url, dir.path(), via(&proxy.url())).connections(4),
    )
    .await
    .unwrap();
    assert_content(&done, 3, size);
    let recs = proxy.records();
    assert!(recs.len() >= 2, "{recs:?}");
    assert!(
        recs.iter().all(|r| r.kind == "absolute" && r.authorized),
        "{recs:?}"
    );
    assert!(done.max_connections > 1);

    // Wrong credentials: the proxy's 407 is a permanent error.
    let bad = proxy.url_with(Some(("alice", "wrong")));
    let err = get_with(&engine, proxied(&url, dir.path(), via(&bad)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 407, .. }), "{err}");
    assert_eq!(err.kind(), ErrorKind::Permanent);
    assert!(
        !err.to_string().contains("wrong"),
        "no password in errors: {err}"
    );
}

#[tokio::test]
async fn socks5_proxy_resolves_names_remotely() {
    let srv = TestServer::start().await;
    let proxy = TestProxy::start(ProxyMode::Socks5, Some(("bob", "pw"))).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 2 * MIB;
    // A name, not an address: the proxy must receive it as a name.
    let url = format!(
        "http://localhost:{}/data?size={size}&seed=4",
        srv.addr().port()
    );
    let done = get_with(
        &engine,
        proxied(&url, dir.path(), via(&proxy.url())).connections(3),
    )
    .await
    .unwrap();
    assert_content(&done, 4, size);
    let recs = proxy.records();
    assert_ne!(recs.len(), 0);
    for r in &recs {
        assert_eq!(r.target, format!("localhost:{}", srv.addr().port()));
        assert!(r.authorized);
    }

    let err = get_with(
        &engine,
        proxied(
            &url,
            dir.path(),
            via(&proxy.url_with(Some(("bob", "nope")))),
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::ProxyAuth(_)), "{err}");
    let err = get_with(
        &engine,
        proxied(&url, dir.path(), via(&proxy.url_with(None))),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::ProxyAuth(_)), "{err}");
}

#[tokio::test]
async fn engine_rules_and_no_proxy() {
    let srv = TestServer::start().await;
    let proxy = TestProxy::start(ProxyMode::Socks5, None).await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.proxy = ProxyRules::all(Proxy::parse(&proxy.url()).unwrap());
    let engine = Engine::new(c.clone()).unwrap();
    let url = srv.url("/data?size=300000&seed=5");
    assert_content(
        &get_with(&engine, request(&url, dir.path())).await.unwrap(),
        5,
        300_000,
    );
    let n = proxy.records().len();
    assert!(n >= 1);
    std::fs::remove_file(dir.path().join("data")).unwrap();
    // A per-download choice wins over the rules.
    let done = get_with(&engine, proxied(&url, dir.path(), ProxyChoice::Direct))
        .await
        .unwrap();
    assert_content(&done, 5, 300_000);
    std::fs::remove_file(&done.path).unwrap();
    assert_eq!(proxy.records().len(), n);
    // NO_PROXY bypasses the rules.
    c.proxy.no_proxy = NoProxy::parse("127.0.0.1");
    let engine = Engine::new(c).unwrap();
    get_with(&engine, request(&url, dir.path())).await.unwrap();
    assert_eq!(proxy.records().len(), n);
}

#[tokio::test]
async fn saved_proxy_has_no_credentials() {
    let srv = TestServer::start().await;
    let proxy = TestProxy::start(ProxyMode::Http, Some(("carol", "hunter2-secret"))).await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let url = srv.url("/data?size=2097152&seed=6&throttle=1048576");
    let id = {
        let engine = Engine::open(cfg(), state.path()).unwrap();
        let h = engine.add(proxied(&url, dir.path(), via(&proxy.url())).connections(2));
        let id = h.id();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(engine.pause(id).await.unwrap(), State::Paused);
        engine.shutdown().await;
        id
    };
    for f in std::fs::read_dir(state.path()).unwrap() {
        let bytes = std::fs::read(f.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("hunter2-secret"),
            "proxy password saved to disk"
        );
    }
    let engine = Engine::open(cfg(), state.path()).unwrap();
    // The saved proxy is used again, but without its password: 407.
    let err = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 407, .. }), "{err}");
    assert!(proxy.records().iter().any(|r| !r.authorized));
    // Given again, it works.
    let done = engine
        .resume(id, ResumeOptions::default().proxy(via(&proxy.url())))
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_content(&done, 6, 2 * MIB);
    engine.shutdown().await;
}

/// `CONNECT` tunnels and SOCKS5 to a real HTTPS server (TLS through the
/// tunnel). Needs the internet: `cargo test --test proxy -- --ignored`.
#[tokio::test]
#[ignore = "downloads from the internet"]
async fn https_through_connect_and_socks5_real_world() {
    let url = std::env::var("BOLTGET_TEST_HTTPS_URL")
        .unwrap_or_else(|_| "https://proof.ovh.net/files/1Mb.dat".into());
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let mut digests = Vec::new();
    for mode in [ProxyMode::Http, ProxyMode::Socks5] {
        let proxy = TestProxy::start(mode, Some(("u", "p"))).await;
        let done = get_with(
            &engine,
            proxied(&url, dir.path(), via(&proxy.url())).connections(2),
        )
        .await
        .unwrap();
        let recs = proxy.records();
        assert_ne!(recs.len(), 0);
        let want = if mode == ProxyMode::Http {
            "connect"
        } else {
            "socks5"
        };
        assert!(
            recs.iter().all(|r| r.kind == want && r.authorized),
            "{recs:?}"
        );
        digests.push(support::sha(&std::fs::read(&done.path).unwrap()));
        std::fs::remove_file(&done.path).unwrap();
    }
    assert_eq!(digests[0], digests[1]);
}
