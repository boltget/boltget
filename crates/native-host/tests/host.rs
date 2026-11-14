//! The host as the browser runs it: a child process fed framed messages on
//! stdin, with a stand-in app serving the pipe in this process.
// Helpers outside `#[test]` functions unwrap too; a panic fails the test.
#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use boltget_engine::{Engine, EngineConfig};
use boltget_ipc::browser::{Accepted, CaptureRules, HelloReply, Message, Reply, StatusReply};
use boltget_ipc::{BrowserReply, Endpoint, Ownership, Server};
use serde_json::{Value, json};

const HOST: &str = env!("CARGO_BIN_EXE_boltget-native-host");

/// What the app was asked.
#[derive(Default)]
struct App {
    seen: Mutex<Vec<Message>>,
}

impl boltget_ipc::Owner for App {
    fn browser(&self, _engine: Engine, _v: u32, message: Message) -> BrowserReply {
        let reply = match &message {
            Message::Hello(_) => Reply::Hello(HelloReply::new(
                Some("Boltget test".into()),
                Some(CaptureRules::default()),
            )),
            Message::Status(_) => Reply::Status(StatusReply::new(1, 2, 3)),
            _ => Reply::Accepted(Accepted::added(42)),
        };
        self.seen.lock().unwrap().push(message);
        Box::pin(async move { reply })
    }
}

/// A stand-in app serving the pipe of `dir`.
async fn serve(dir: &std::path::Path, app: Arc<App>) -> (Engine, tokio::task::JoinHandle<()>) {
    let engine = Engine::open(EngineConfig::default(), dir).unwrap();
    let server = Server::bind(Endpoint::for_data_dir(dir).unwrap())
        .await
        .unwrap();
    let (tx, ownership) = tokio::sync::watch::channel(Ownership::Owner(engine.clone()));
    let task = tokio::spawn(async move {
        let _keep = tx;
        server.serve(ownership, app, "Boltget test".into()).await;
    });
    (engine, task)
}

struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
}

impl Host {
    fn start(data: &std::path::Path, app: &str) -> Self {
        let mut child = Command::new(HOST)
            .arg("chrome-extension://fckefagfbiinebmgfpgjiielmmmaggbd/")
            .env("BOLTGET_DATA_DIR", data)
            .env("BOLTGET_NATIVE_HOST_APP", app)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: &Value) {
        let bytes = serde_json::to_vec(message).unwrap();
        let stdin = self.stdin.as_mut().unwrap();
        stdin
            .write_all(&u32::try_from(bytes.len()).unwrap().to_ne_bytes())
            .unwrap();
        stdin.write_all(&bytes).unwrap();
        stdin.flush().unwrap();
    }

    fn reply(&mut self) -> Value {
        let mut len = [0u8; 4];
        self.stdout.read_exact(&mut len).unwrap();
        let mut buf = vec![0; u32::from_ne_bytes(len) as usize];
        self.stdout.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    fn ask(&mut self, message: &Value) -> Value {
        self.send(message);
        self.reply()
    }

    /// Closes stdin, as the browser does, and waits for the exit.
    fn close(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "the host did not exit"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn capture(id: u32) -> Value {
    json!({
        "v": 1, "id": id, "type": "capture",
        "url": "https://dl.example.com/a.iso", "filename": "a.iso", "size": 1_000_000,
        "cookies": [{"name": "sid", "value": "s", "domain": ".example.com"}],
        "headers": [["Referer", "https://example.com/"]]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relays_to_a_running_app() {
    let data = tempfile::tempdir().unwrap();
    let app = Arc::new(App::default());
    let (engine, task) = serve(data.path(), app.clone()).await;
    let seen = tokio::task::spawn_blocking({
        let dir = data.path().to_path_buf();
        move || {
            let mut host = Host::start(&dir, "no-app-needed");
            let hello = host.ask(&json!({"v": 1, "id": 1, "type": "hello", "extension": "0.3.0", "browser": "chrome"}));
            let accepted = host.ask(&capture(2));
            let status = host.ask(&json!({"v": 1, "id": 3, "type": "status"}));
            let invalid = host.ask(&json!({"v": 1, "id": 4, "type": "capture", "url": "file:///etc/passwd"}));
            let accepted_again = host.ask(&capture(5));
            assert!(host.close().success());
            (hello, accepted, status, invalid, accepted_again)
        }
    })
    .await
    .unwrap();
    let (hello, accepted, status, invalid, again) = seen;
    assert_eq!(hello["type"], "hello", "{hello}");
    assert_eq!(hello["id"], 1);
    assert_eq!(hello["app"], "Boltget test");
    assert_eq!(hello["host"], env!("CARGO_PKG_VERSION"));
    assert_eq!(hello["rules"]["min_size"], 1_048_576);
    assert_eq!(
        accepted,
        json!({"v": 1, "id": 2, "type": "accepted", "download": 42, "confirm": false})
    );
    assert_eq!(status["active"], 1);
    assert_eq!(
        (invalid["type"].as_str(), invalid["reason"].as_str()),
        (Some("error"), Some("invalid"))
    );
    assert_eq!(again["type"], "accepted", "a fresh connection per capture");
    let seen = app.seen.lock().unwrap().clone();
    let kinds: Vec<_> = seen.iter().map(Message::kind).collect();
    assert_eq!(
        kinds,
        ["hello", "capture", "status", "capture"],
        "the invalid one never reached the app"
    );
    let Message::Capture(c) = &seen[1] else {
        panic!()
    };
    assert_eq!(c.cookies[0].value, "s");
    assert_eq!(c.headers[0].1, "https://example.com/");
    task.abort();
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starts_the_app_and_waits_for_it() {
    let data = tempfile::tempdir().unwrap();
    // "Starting the app" runs the host itself, which exits at once (stdin
    // is closed); the stand-in app appears a little later, as a real one
    // would after opening its engine.
    let mut host = Host::start(data.path(), HOST);
    let status = tokio::task::block_in_place(|| host.ask(&json!({"v": 1, "type": "status"})));
    assert_eq!(
        status["reason"], "not_running",
        "status never starts the app"
    );
    let hello = tokio::task::block_in_place(|| {
        host.ask(&json!({"v": 1, "type": "hello", "extension": "0.3.0", "browser": "firefox"}))
    });
    assert_eq!(
        (hello["type"].as_str(), &hello["app"]),
        (Some("hello"), &Value::Null)
    );

    let app = Arc::new(App::default());
    let dir = data.path().to_path_buf();
    let later = tokio::spawn({
        let app = app.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            serve(&dir, app).await
        }
    });
    let accepted = tokio::task::block_in_place(|| host.ask(&capture(9)));
    assert_eq!(accepted["type"], "accepted", "{accepted}");
    assert_eq!(accepted["id"], 9);
    assert!(tokio::task::block_in_place(|| host.close()).success());
    let (engine, task) = later.await.unwrap();
    task.abort();
    engine.shutdown().await;
}

#[test]
fn falls_back_when_the_app_cannot_start() {
    let data = tempfile::tempdir().unwrap();
    let missing = data.path().join("no-such-app.exe");
    let mut host = Host::start(data.path(), &missing.to_string_lossy());
    let reply = host.ask(&capture(1));
    assert_eq!(reply["type"], "unavailable");
    assert_eq!(reply["reason"], "not_installed");
    // Garbage is answered, and the session goes on.
    let stdin = host.stdin.as_mut().unwrap();
    stdin.write_all(&5u32.to_ne_bytes()).unwrap();
    stdin.write_all(b"hello").unwrap();
    stdin.flush().unwrap();
    assert_eq!(host.reply()["reason"], "invalid");
    assert_eq!(
        host.ask(&json!({"v": 1, "type": "status"}))["reason"],
        "not_running"
    );
    assert!(host.close().success());
}

#[test]
fn a_frame_cut_short_ends_the_host() {
    let data = tempfile::tempdir().unwrap();
    let mut host = Host::start(data.path(), "none");
    let stdin = host.stdin.as_mut().unwrap();
    stdin.write_all(&100u32.to_ne_bytes()).unwrap();
    stdin.write_all(b"{\"v\":").unwrap();
    assert!(!host.close().success());
}
