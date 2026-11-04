//! The protocol end to end: a real engine behind a real pipe (a Unix socket
//! on Linux), the local test server as the internet.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boltget_engine::{DownloadId, DownloadSummary, Engine, EngineConfig};
use boltget_testserver::TestServer;

use crate::frame::{read_message, write_frame, write_message};
use crate::proto::{INVALID_REQUEST_KEY, VERSION_ERROR_KEY};
use crate::{AddSpec, Client, Endpoint, Error, Request, Response, ResumeSpec, Server, transport};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

fn cfg() -> EngineConfig {
    let mut c = EngineConfig::default();
    c.min_split_size = 64 * KIB;
    c.segment_align = 4 * KIB;
    c.worker_stagger = Duration::from_millis(5);
    c.flush_interval = Duration::from_millis(50);
    c.backoff_base = Duration::from_millis(10);
    c.backoff_max = Duration::from_millis(100);
    c.disk_margin = 0;
    c.initial_connections = 2;
    c
}

/// What the owner was told.
#[derive(Default)]
struct Recorder {
    changed: Mutex<Vec<(u64, String)>>,
    removed: Mutex<Vec<u64>>,
    limits: Mutex<Vec<Option<u64>>>,
}

impl crate::Owner for Recorder {
    fn changed(&self, s: &DownloadSummary) {
        self.changed
            .lock()
            .unwrap()
            .push((s.id.0, s.state.as_str().to_owned()));
    }
    fn removed(&self, id: DownloadId) {
        self.removed.lock().unwrap().push(id.0);
    }
    fn limit_changed(&self, bps: Option<u64>) {
        self.limits.lock().unwrap().push(bps);
    }
}

struct Owner {
    _data: tempfile::TempDir,
    downloads: tempfile::TempDir,
    endpoint: Endpoint,
    engine: Engine,
    recorder: Arc<Recorder>,
    task: tokio::task::JoinHandle<()>,
}

impl Owner {
    async fn start() -> Self {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let engine = Engine::open(cfg(), data.path()).unwrap();
        let endpoint = Endpoint::for_data_dir(data.path()).unwrap();
        let server = Server::bind(endpoint.clone()).await.unwrap();
        let recorder = Arc::new(Recorder::default());
        let task =
            tokio::spawn(server.serve(engine.clone(), recorder.clone(), "Boltget test".into()));
        Self {
            _data: data,
            downloads,
            endpoint,
            engine,
            recorder,
            task,
        }
    }

    async fn client(&self) -> Client {
        Client::connect(&self.endpoint, "test")
            .await
            .unwrap()
            .expect("the owner is serving")
    }

    fn dir(&self) -> PathBuf {
        self.downloads.path().to_path_buf()
    }

    async fn stop(self) {
        self.task.abort();
        self.engine.shutdown().await;
    }
}

async fn timed<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), f)
        .await
        .expect("in time")
}

async fn add(client: &mut Client, url: String, dir: &Path) -> u64 {
    match client
        .request(&Request::Add(AddSpec::new(url, dir)))
        .await
        .unwrap()
    {
        Response::Added { id } => id,
        other => panic!("expected added, got {other:?}"),
    }
}

/// Watches until the download stops; returns every message.
async fn watch(client: &mut Client, id: u64) -> Vec<Response> {
    let mut seen = vec![client.request(&Request::Watch { id }).await.unwrap()];
    while !matches!(
        seen.last(),
        Some(Response::Finished(_) | Response::Error { .. })
    ) {
        seen.push(timed(client.next()).await.unwrap());
    }
    seen
}

fn finished(seen: &[Response]) -> &crate::Finished {
    match seen.last() {
        Some(Response::Finished(f)) => f,
        other => panic!("expected finished, got {other:?}"),
    }
}

#[tokio::test]
async fn nobody_serves_a_directory_without_an_owner() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::for_data_dir(dir.path()).unwrap();
    assert!(Client::connect(&endpoint, "test").await.unwrap().is_none());
}

#[tokio::test]
async fn one_owner_per_endpoint() {
    let owner = Owner::start().await;
    assert!(matches!(
        Server::bind(owner.endpoint.clone()).await,
        Err(Error::InUse(_))
    ));
    owner.stop().await;
}

#[tokio::test]
async fn a_client_of_another_version_is_told_why() {
    let owner = Owner::start().await;
    let mut io = transport::connect(&owner.endpoint).await.unwrap().unwrap();
    let hello = Request::Hello {
        v: 99,
        client: "boltget-cli 9.9".into(),
    };
    write_message(&mut io, &hello).await.unwrap();
    let reply: Response = read_message(&mut io).await.unwrap().unwrap();
    let Response::Error { error } = reply else {
        panic!("expected an error, got {reply:?}")
    };
    assert_eq!(error.key, VERSION_ERROR_KEY);
    assert!(
        error.message.contains("Boltget test") && error.message.contains("99"),
        "{}",
        error.message
    );
    // And then it hangs up.
    assert!(
        read_message::<Response, _>(&mut io)
            .await
            .unwrap()
            .is_none()
    );
    owner.stop().await;
}

#[tokio::test]
async fn hello_comes_first_and_bad_requests_are_refused() {
    let owner = Owner::start().await;
    let mut io = transport::connect(&owner.endpoint).await.unwrap().unwrap();
    write_message(&mut io, &Request::List).await.unwrap();
    let reply: Response = read_message(&mut io).await.unwrap().unwrap();
    assert!(
        matches!(&reply, Response::Error { error } if error.key == INVALID_REQUEST_KEY),
        "{reply:?}"
    );

    let mut client = owner.client().await;
    assert_eq!(client.server(), "Boltget test");
    // An option this version does not define is refused, not ignored.
    write_frame(
        &mut client.io,
        br#"{"type":"pause","id":1,"everywhere":true}"#,
    )
    .await
    .unwrap();
    let reply = client.next().await.unwrap();
    assert!(
        matches!(&reply, Response::Error { error } if error.key == INVALID_REQUEST_KEY),
        "{reply:?}"
    );
    assert!(matches!(client.next().await, Err(Error::Closed)));
    owner.stop().await;
}

#[tokio::test]
async fn adds_and_streams_a_download_to_completion() {
    let server = TestServer::start().await;
    let owner = Owner::start().await;
    let mut client = owner.client().await;
    assert_eq!(
        client.request(&Request::List).await.unwrap(),
        Response::Downloads { downloads: vec![] }
    );

    let size = 3 * MIB;
    let id = add(
        &mut client,
        server.url(&format!("/data?size={size}&seed=7&throttle={}", 4 * MIB)),
        &owner.dir(),
    )
    .await;
    let seen = timed(watch(&mut client, id)).await;
    let done = finished(&seen);
    assert_eq!(done.state, "completed", "{seen:?}");
    assert_eq!((done.downloaded, done.total), (size, Some(size)));
    let path = done.path.clone().expect("a path");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        boltget_testserver::content(7, size)
    );
    assert!(
        seen.iter()
            .any(|m| matches!(m, Response::Progress(p) if p.id == id)),
        "progress was reported"
    );
    assert!(
        seen.iter().any(
            |m| matches!(m, Response::Event { event } if event["event"] == "probed" && event["id"] == id)
        ),
        "engine events are forwarded as the CLI prints them"
    );

    // Watching a finished download answers at once; list shows it.
    let again = timed(watch(&mut client, id)).await;
    assert_eq!(again.len(), 1);
    assert_eq!(finished(&again).path.as_deref(), Some(path.as_path()));
    let Response::Downloads { downloads } = client.request(&Request::List).await.unwrap() else {
        panic!("expected downloads")
    };
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0]["state"], "completed");
    assert_eq!(downloads[0]["id"], id);
    owner.stop().await;
}

#[tokio::test]
async fn another_client_pauses_and_resumes_a_running_download() {
    let server = TestServer::start().await;
    let owner = Owner::start().await;
    let mut watcher = owner.client().await;
    let mut other = owner.client().await;
    let size = 4 * MIB;
    let id = add(
        &mut watcher,
        server.url(&format!("/data?size={size}&seed=3&throttle={}", 512 * KIB)),
        &owner.dir(),
    )
    .await;
    watcher.request(&Request::Watch { id }).await.unwrap();

    // Wait until it runs, then pause it from the other connection.
    loop {
        let Response::Downloads { downloads } = other.request(&Request::List).await.unwrap() else {
            panic!("expected downloads")
        };
        if downloads[0]["downloaded"].as_u64().unwrap_or(0) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let paused = timed(other.request(&Request::Pause { id })).await.unwrap();
    assert_eq!(
        paused,
        Response::Paused {
            id,
            state: "paused".into()
        }
    );
    let done = loop {
        if let Response::Finished(f) = timed(watcher.next()).await.unwrap() {
            break f;
        }
    };
    assert_eq!(done.state, "paused");
    assert!(done.downloaded > 0 && done.downloaded < size, "{done:?}");
    assert!(
        owner
            .recorder
            .changed
            .lock()
            .unwrap()
            .contains(&(id, "paused".into())),
        "the owner heard about it"
    );

    let mut resume = ResumeSpec::new(id);
    resume.limit = Some(0);
    assert_eq!(
        other.request(&Request::Resume(resume)).await.unwrap(),
        Response::Ok
    );
    let seen = timed(watch(&mut watcher, id)).await;
    assert_eq!(finished(&seen).state, "completed", "{seen:?}");
    owner.stop().await;
}

#[tokio::test]
async fn settings_limits_and_removal() {
    let server = TestServer::start().await;
    let owner = Owner::start().await;
    let mut client = owner.client().await;
    assert_eq!(
        client.request(&Request::Settings).await.unwrap(),
        Response::Settings { limit: None }
    );
    assert_eq!(
        client
            .request(&Request::SetLimit {
                id: None,
                bps: Some(5 * MIB)
            })
            .await
            .unwrap(),
        Response::Settings {
            limit: Some(5 * MIB)
        }
    );
    assert_eq!(*owner.recorder.limits.lock().unwrap(), [Some(5 * MIB)]);

    let mut spec = AddSpec::new(server.url("/data?size=65536"), owner.dir());
    spec.paused = true;
    let Response::Added { id } = client.request(&Request::Add(spec)).await.unwrap() else {
        panic!("expected added")
    };
    let seen = timed(watch(&mut client, id)).await;
    assert_eq!(finished(&seen).state, "paused", "added paused, not started");

    client
        .request(&Request::SetLimit {
            id: Some(id),
            bps: Some(KIB),
        })
        .await
        .unwrap();
    assert!(
        owner
            .recorder
            .changed
            .lock()
            .unwrap()
            .iter()
            .any(|(i, _)| *i == id),
        "a download's limit is a change the owner hears about"
    );

    assert_eq!(
        client
            .request(&Request::Remove {
                id,
                delete_file: true
            })
            .await
            .unwrap(),
        Response::Removed {
            id,
            deleted: vec![]
        }
    );
    assert_eq!(*owner.recorder.removed.lock().unwrap(), [id]);
    let reply = client.request(&Request::Pause { id }).await.unwrap();
    let Response::Error { error } = reply else {
        panic!("expected an error")
    };
    assert_eq!(error.key, "error.unknown_download");
    assert_eq!(
        error.error_kind(),
        Some(boltget_engine::ErrorKind::NotFound)
    );
    // The connection is still usable after an error.
    assert!(matches!(
        client.request(&Request::List).await.unwrap(),
        Response::Downloads { .. }
    ));
    owner.stop().await;
}

#[tokio::test]
async fn requests_with_bad_values_are_refused_with_reasons() {
    let owner = Owner::start().await;
    let mut client = owner.client().await;
    let mut relative = AddSpec::new("https://x.test/a", "relative/dir");
    relative.paused = true;
    let mut bad_header = AddSpec::new("https://x.test/a", owner.dir());
    bad_header
        .request
        .headers
        .push(("Range".into(), "bytes=0-".into()));
    let mut bad_url = AddSpec::new("ftp://x.test/a", owner.dir());
    bad_url.paused = true;
    for (spec, key) in [
        (relative, INVALID_REQUEST_KEY),
        (bad_header, INVALID_REQUEST_KEY),
        (bad_url, "error.invalid_url"),
    ] {
        let reply = client.request(&Request::Add(spec)).await.unwrap();
        assert!(
            matches!(&reply, Response::Error { error } if error.key == key),
            "{reply:?}"
        );
    }
    owner.stop().await;
}
