//! Client mode: with an owner serving the data directory's pipe (here an
//! engine in the test process, standing in for the desktop app), the
//! `boltget` binary sends its commands there instead of opening the
//! database, so several terminals work at once and `pause` reaches a
//! download running elsewhere.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::Duration;

use boltget_engine::{DownloadId, Engine, EngineConfig, State};
use boltget_ipc::{Endpoint, Ownership, Server};
use boltget_testserver::{TestServer, content};

const MIB: u64 = 1024 * 1024;

struct App {
    engine: Engine,
    task: tokio::task::JoinHandle<()>,
}

impl App {
    async fn start(state: &Path) -> Self {
        let engine = Engine::open(EngineConfig::default(), state).expect("the engine opens");
        let endpoint = Endpoint::for_data_dir(state).expect("an endpoint");
        let server = Server::bind(endpoint).await.expect("the pipe binds");
        let (_, ownership) = tokio::sync::watch::channel(Ownership::Owner(engine.clone()));
        let task = tokio::spawn(server.serve(ownership, Arc::new(()), "Boltget test".into()));
        Self { engine, task }
    }

    async fn stop(self) {
        self.task.abort();
        self.engine.shutdown().await;
    }
}

async fn boltget(state: &Path, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_boltget"));
    c.args(args).env("BOLTGET_DATA_DIR", state);
    tokio::task::spawn_blocking(move || c.output().expect("run boltget"))
        .await
        .expect("join")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn lines(out: &Output) -> Vec<serde_json::Value> {
    stdout(out)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad JSON line {l:?}: {e}")))
        .collect()
}

async fn state_of(engine: &Engine, id: u64) -> State {
    engine
        .list()
        .await
        .expect("the list")
        .into_iter()
        .find(|s| s.id == DownloadId(id))
        .expect("the download")
        .state
}

#[tokio::test(flavor = "multi_thread")]
async fn get_runs_in_the_app_and_streams_json() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(state.path()).await;
    let size = 3 * MIB;
    let out_path = dir.path().join("out.bin");
    let out = boltget(
        state.path(),
        &[
            "get",
            &srv.url(&format!("/data?size={size}&seed=5&throttle={}", 4 * MIB)),
            "-o",
            &out_path.display().to_string(),
            "--json",
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let events = lines(&out);
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.first(), Some(&"added"), "{kinds:?}");
    assert!(kinds.contains(&"progress"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"completed"), "{kinds:?}");
    let done = events.last().unwrap();
    assert_eq!(done["bytes"], size);
    assert_eq!(std::fs::read(&out_path).unwrap(), content(5, size));
    // The app recorded it: the CLI never opened the database.
    let id = done["id"].as_u64().unwrap();
    assert_eq!(state_of(&app.engine, id).await, State::Completed);

    let out = boltget(state.path(), &["list"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("out.bin"), "{}", stdout(&out));
    app.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_terminals_download_at_once() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(state.path()).await;
    let get = |seed: u64| {
        let url = srv.url(&format!(
            "/data?size={}&seed={seed}&throttle={}",
            2 * MIB,
            2 * MIB
        ));
        let out = dir.path().join(format!("{seed}.bin"));
        let state = state.path().to_path_buf();
        async move {
            boltget(
                &state,
                &["get", &url, "-o", &out.display().to_string(), "-q"],
            )
            .await
        }
    };
    let (a, b) = tokio::join!(get(1), get(2));
    for out in [&a, &b] {
        assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(out));
        assert!(stdout(out).starts_with("Saved "), "{}", stdout(out));
    }
    assert_eq!(
        std::fs::read(dir.path().join("1.bin")).unwrap(),
        content(1, 2 * MIB)
    );
    assert_eq!(
        std::fs::read(dir.path().join("2.bin")).unwrap(),
        content(2, 2 * MIB)
    );
    app.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_reaches_a_download_running_in_the_app() {
    let srv = TestServer::start().await;
    let state = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(state.path()).await;
    let url = srv.url(&format!("/data?size={}&seed=9&throttle={}", 8 * MIB, MIB));
    let out = boltget(
        state.path(),
        &[
            "get",
            &url,
            "-o",
            &dir.path().display().to_string(),
            "--detach",
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).starts_with("Added download 1 to Boltget"),
        "{}",
        stdout(&out)
    );
    let mut tries = 0;
    while state_of(&app.engine, 1).await != State::Downloading {
        tries += 1;
        assert!(tries < 200, "it starts");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let out = boltget(state.path(), &["pause", "1"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "Paused download 1");
    assert_eq!(state_of(&app.engine, 1).await, State::Paused);

    let out = boltget(state.path(), &["resume", "1", "--detach"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    // And another terminal's `get`-style watch sees it pause from elsewhere.
    let watcher = tokio::spawn({
        let state = state.path().to_path_buf();
        async move { boltget(&state, &["resume", "1", "--json"]).await }
    });
    tokio::time::sleep(Duration::from_millis(800)).await;
    let out = boltget(state.path(), &["pause", "1"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let watched = watcher.await.unwrap();
    // Resuming a running download is refused (8), or it was watched until
    // the pause (130): either way the CLI says so in JSON.
    let last = lines(&watched).pop().expect("a last line");
    assert!(
        matches!(watched.status.code(), Some(8 | 130)),
        "{:?} {last}",
        watched.status.code()
    );
    assert!(
        matches!(last["event"].as_str(), Some("paused" | "failed")),
        "{last}"
    );

    let out = boltget(state.path(), &["remove", "1", "--delete-file"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(app.engine.list().await.unwrap().is_empty());
    app.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn config_changes_the_apps_limit_at_once() {
    let state = tempfile::tempdir().unwrap();
    let app = App::start(state.path()).await;
    let out = boltget(state.path(), &["config", "set", "limit", "5M"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).starts_with("limit = 5.00 MiB/s"),
        "{}",
        stdout(&out)
    );
    assert_eq!(
        app.engine.limit(boltget_engine::LimitScope::Global),
        Some(5 * MIB)
    );
    let out = boltget(state.path(), &["config", "unset", "limit"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert_eq!(app.engine.limit(boltget_engine::LimitScope::Global), None);
    app.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn options_the_app_cannot_apply_are_refused() {
    let state = tempfile::tempdir().unwrap();
    let app = App::start(state.path()).await;
    let out = boltget(
        state.path(),
        &["get", "https://example.invalid/a", "--max-connections", "4"],
    )
    .await;
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("--max-connections"),
        "{}",
        stderr(&out)
    );
    assert!(
        app.engine.list().await.unwrap().is_empty(),
        "nothing was added"
    );
    app.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_app_detach_is_a_usage_error_and_commands_run_standalone() {
    let state = tempfile::tempdir().unwrap();
    let out = boltget(
        state.path(),
        &["get", "https://example.invalid/a", "--detach"],
    )
    .await;
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("needs the Boltget app"),
        "{}",
        stderr(&out)
    );
    let out = boltget(state.path(), &["list"]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "No downloads.");
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_skips_the_app() {
    let state = tempfile::tempdir().unwrap();
    let app = App::start(state.path()).await;
    // The app owns the database, so a standalone boltget finds it locked.
    let out = boltget(
        state.path(),
        &["--standalone", "config", "set", "limit", "1M"],
    )
    .await;
    assert_eq!(out.status.code(), Some(7), "stderr: {}", stderr(&out));
    app.stop().await;
}
