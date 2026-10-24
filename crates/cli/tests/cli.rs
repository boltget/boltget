//! End-to-end tests of the `boltget` binary against the local test server.

use std::process::{Command, Output};

use boltget_testserver::{TestServer, content};
use sha2::{Digest, Sha256};

async fn boltget(args: Vec<String>) -> Output {
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_boltget"))
            .args(&args)
            .output()
            .expect("run boltget")
    })
    .await
    .expect("join")
}

fn lines(out: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad JSON line {l:?}: {e}")))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn json_download_to_file() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.bin");
    let size = 3 * 1024 * 1024 + 5;
    let out = boltget(vec![
        "get".into(),
        srv.url(&format!("/data?size={size}&seed=11&throttle=4194304")),
        "-o".into(),
        out_path.display().to_string(),
        "-c".into(),
        "4".into(),
        "--json".into(),
    ])
    .await;
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events = lines(&out);
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.first(), Some(&"added"));
    assert!(kinds.contains(&"probed"));
    assert!(kinds.contains(&"progress"));
    assert_eq!(kinds.last(), Some(&"completed"));
    let done = events.last().unwrap();
    assert_eq!(done["bytes"], size);
    assert!(done["max_connections"].as_u64().unwrap() <= 4);
    let data = std::fs::read(&out_path).unwrap();
    assert_eq!(Sha256::digest(&data), Sha256::digest(content(11, size)));
}

#[tokio::test(flavor = "multi_thread")]
async fn human_output_into_directory() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let out = boltget(vec![
        "get".into(),
        srv.url("/data?size=1000&cd=attachment%3Bfilename%3Dhello.txt"),
        "-o".into(),
        dir.path().display().to_string(),
    ])
    .await;
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("Saved "), "{stdout}");
    assert!(dir.path().join("hello.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_codes() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().display().to_string();
    let not_found = boltget(vec![
        "get".into(),
        srv.url("/nope"),
        "-o".into(),
        d.clone(),
        "--json".into(),
    ])
    .await;
    assert_eq!(not_found.status.code(), Some(4));
    let last = lines(&not_found).pop().unwrap();
    assert_eq!(last["event"], "failed");
    assert_eq!(last["error"]["key"], "error.http.not_found");
    assert_eq!(last["exit_code"], 4);

    let changed = boltget(vec![
        "get".into(),
        srv.url("/data?size=4194304&change_after=2"),
        "-o".into(),
        d.clone(),
        "-q".into(),
    ])
    .await;
    assert_eq!(changed.status.code(), Some(6));

    let bad_url = boltget(vec![
        "get".into(),
        "ftp://example.test/x".into(),
        "-q".into(),
    ])
    .await;
    assert_eq!(bad_url.status.code(), Some(4));

    let bad_header = boltget(vec![
        "get".into(),
        srv.url("/data"),
        "-H".into(),
        "Range: bytes=0-".into(),
    ])
    .await;
    assert_eq!(bad_header.status.code(), Some(2));

    let usage = boltget(vec!["get".into()]).await;
    assert_eq!(usage.status.code(), Some(2));
}
