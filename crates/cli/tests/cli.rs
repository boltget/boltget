//! End-to-end tests of the `boltget` binary against the local test server.
//! Every invocation gets its own state directory (`BOLTGET_DATA_DIR`), never the
//! user's real one.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use boltget_testserver::{TestServer, content};
use sha2::{Digest, Sha256};

fn command(state: &Path, args: &[String]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_boltget"));
    c.args(args).env("BOLTGET_DATA_DIR", state);
    c
}

async fn boltget(state: &Path, args: Vec<String>) -> Output {
    let mut c = command(state, &args);
    tokio::task::spawn_blocking(move || c.output().expect("run boltget"))
        .await
        .expect("join")
}

fn s(v: &str) -> String {
    v.to_owned()
}

fn lines(out: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad JSON line {l:?}: {e}")))
        .collect()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

async fn list_json(state: &Path) -> Vec<serde_json::Value> {
    let out = boltget(state, vec![s("list"), s("--json")]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout).expect("JSON array")
}

#[tokio::test(flavor = "multi_thread")]
async fn json_download_to_file_is_recorded() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.bin");
    let size = 3 * 1024 * 1024 + 5;
    let out = boltget(
        state.path(),
        vec![
            s("get"),
            srv.url(&format!("/data?size={size}&seed=11&throttle=4194304")),
            s("-o"),
            out_path.display().to_string(),
            s("-c"),
            s("4"),
            s("--json"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let events = lines(&out);
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.first(), Some(&"added"));
    assert_eq!(events[0]["id"], 1);
    assert!(kinds.contains(&"probed"));
    assert!(kinds.contains(&"progress"));
    assert_eq!(kinds.last(), Some(&"completed"));
    for e in &events {
        assert_eq!(e["id"], 1, "every line carries the id: {e}");
    }
    let done = events.last().unwrap();
    assert_eq!(done["bytes"], size);
    assert!(done["max_connections"].as_u64().unwrap() <= 4);
    let data = std::fs::read(&out_path).unwrap();
    assert_eq!(Sha256::digest(&data), Sha256::digest(content(11, size)));

    let list = list_json(state.path()).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], 1);
    assert_eq!(list[0]["state"], "completed");
    assert_eq!(list[0]["downloaded"], size);
    assert_eq!(list[0]["file_name"], "out.bin");
    assert!(list[0]["url"].as_str().unwrap().ends_with("/data?…"));
}

#[tokio::test(flavor = "multi_thread")]
async fn human_output_into_directory() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let out = boltget(
        state.path(),
        vec![
            s("get"),
            srv.url("/data?size=1000&cd=attachment%3Bfilename%3Dhello.txt"),
            s("-o"),
            dir.path().display().to_string(),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("Saved "), "{stdout}");
    assert!(dir.path().join("hello.txt").exists());

    let out = boltget(state.path(), vec![s("list")]).await;
    let table = String::from_utf8_lossy(&out.stdout);
    assert!(table.starts_with("ID"), "{table}");
    assert!(table.contains("completed"), "{table}");
    assert!(table.contains("hello.txt"), "{table}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_state_downloads_are_not_recorded() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let out = boltget(
        state.path(),
        vec![
            s("get"),
            srv.url("/data?size=2000"),
            s("-o"),
            dir.path().display().to_string(),
            s("--no-state"),
            s("-q"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert_eq!(
        list_json(state.path()).await,
        Vec::<serde_json::Value>::new()
    );
    // Neither the download nor the listing created a database.
    assert!(!state.path().join(boltget_engine::DB_FILE).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_commands_and_their_exit_codes() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let st = state.path();
    let d = dir.path().display().to_string();
    let out = boltget(
        st,
        vec![
            s("get"),
            srv.url("/data?size=5000&seed=2"),
            s("-o"),
            d.clone(),
            s("-q"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));

    // Completed downloads cannot be resumed or paused.
    let out = boltget(st, vec![s("resume"), s("1")]).await;
    assert_eq!(out.status.code(), Some(8), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("already completed"),
        "{}",
        stderr(&out)
    );
    let out = boltget(st, vec![s("pause"), s("1")]).await;
    assert_eq!(out.status.code(), Some(8));
    // Unknown ids.
    for cmd in ["resume", "pause", "remove"] {
        let out = boltget(st, vec![s(cmd), s("9")]).await;
        assert_eq!(out.status.code(), Some(8), "{cmd}: {}", stderr(&out));
    }
    let out = boltget(st, vec![s("resume"), s("9"), s("--json")]).await;
    let last = lines(&out).pop().unwrap();
    assert_eq!(last["event"], "failed");
    assert_eq!(last["id"], 9);
    assert_eq!(last["error"]["key"], "error.unknown_download");
    assert_eq!(last["exit_code"], 8);

    // A failed download: pause leaves it alone, resume retries it.
    let out = boltget(
        st,
        vec![s("get"), srv.url("/nope"), s("-o"), d.clone(), s("-q")],
    )
    .await;
    assert_eq!(out.status.code(), Some(4));
    let out = boltget(st, vec![s("pause"), s("2")]).await;
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("not running (failed)"));
    let list = list_json(st).await;
    assert_eq!(list[1]["state"], "failed");
    assert_eq!(list[1]["error"]["key"], "error.http.not_found");
    let out = boltget(st, vec![s("resume"), s("2"), s("-q")]).await;
    assert_eq!(out.status.code(), Some(4));

    // Remove, keeping and deleting the file.
    let saved = dir.path().join("data");
    assert!(saved.exists());
    let out = boltget(st, vec![s("remove"), s("2")]).await;
    assert_eq!(out.status.code(), Some(0));
    let out = boltget(st, vec![s("remove"), s("1"), s("--delete-file")]).await;
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Deleted"));
    assert!(!saved.exists());
    assert_eq!(list_json(st).await, Vec::<serde_json::Value>::new());
    // Ids are never reused.
    let out = boltget(
        st,
        vec![s("get"), srv.url("/data?size=10"), s("-o"), d, s("--json")],
    )
    .await;
    assert_eq!(lines(&out)[0]["id"], 3);
}

/// Reads a running child's stdout lines until `pred` matches one.
fn wait_for_line(
    child: &mut Child,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> BufReader<std::process::ChildStdout> {
    let mut reader = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read child stdout");
        assert!(n > 0, "child exited before the expected line");
        let v: serde_json::Value = serde_json::from_str(line.trim()).expect("JSON line");
        if pred(&v) {
            return reader;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn second_process_gets_a_clear_error_and_list_still_works() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let st = state.path().to_path_buf();
    let d = dir.path().display().to_string();
    let size = 4 * 1024 * 1024;
    // Only the first request (the owner's single connection) is slow, so the
    // resume at the end runs at full speed.
    let url = srv.url(&format!(
        "/data?size={size}&seed=12&slow_index=0&slow_bps=131072&key=owner"
    ));
    // A slow download that owns the database.
    let mut owner = command(
        &st,
        &[
            s("get"),
            url.clone(),
            s("-o"),
            d.clone(),
            s("--json"),
            s("-c"),
            s("1"),
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let reader = tokio::task::spawn_blocking(move || {
        let r = wait_for_line(&mut owner, |v| {
            v["event"] == "state_changed" && v["state"] == "downloading"
        });
        (owner, r)
    });
    let (mut owner, reader) = tokio::time::timeout(Duration::from_secs(60), reader)
        .await
        .expect("owner started")
        .unwrap();

    let out = boltget(&st, vec![s("pause"), s("1")]).await;
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    assert!(stderr(&out).contains("Ctrl-C"), "{}", stderr(&out));
    let out = boltget(
        &st,
        vec![s("get"), url.clone(), s("-o"), d.clone(), s("-q")],
    )
    .await;
    assert_eq!(out.status.code(), Some(7));
    assert!(stderr(&out).contains("--no-state"), "{}", stderr(&out));
    let out = boltget(&st, vec![s("resume"), s("1"), s("--json")]).await;
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(
        lines(&out).pop().unwrap()["error"]["key"],
        "error.database_locked"
    );
    // Listing reads the owner's last checkpoint.
    let list = list_json(&st).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["state"], "downloading");
    // Unrecorded downloads still work alongside.
    let out = boltget(
        &st,
        vec![
            s("get"),
            srv.url("/data?size=1000"),
            s("-o"),
            d.clone(),
            s("--no-state"),
            s("-q"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));

    // Kill the owner; the download continues from its checkpoint.
    owner.kill().unwrap();
    owner.wait().unwrap();
    drop(reader);
    let out = boltget(&st, vec![s("resume"), s("1"), s("--json")]).await;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let events = lines(&out);
    let resumed = events.iter().find(|e| e["event"] == "resumed");
    let done = events.last().unwrap();
    assert_eq!(done["event"], "completed");
    let data = std::fs::read(done["path"].as_str().unwrap()).unwrap();
    assert_eq!(Sha256::digest(&data), Sha256::digest(content(12, size)));
    // Usually there was a checkpoint to continue from (unless the kill came
    // before the first flush).
    if let Some(r) = resumed {
        assert!(r["downloaded"].as_u64().unwrap() > 0);
        assert_eq!(done["resumed_from"], r["downloaded"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_codes() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let st = state.path();
    let d = dir.path().display().to_string();
    let not_found = boltget(
        st,
        vec![s("get"), srv.url("/nope"), s("-o"), d.clone(), s("--json")],
    )
    .await;
    assert_eq!(not_found.status.code(), Some(4));
    let last = lines(&not_found).pop().unwrap();
    assert_eq!(last["event"], "failed");
    assert_eq!(last["error"]["key"], "error.http.not_found");
    assert_eq!(last["exit_code"], 4);

    let changed = boltget(
        st,
        vec![
            s("get"),
            srv.url("/data?size=4194304&change_after=2"),
            s("-o"),
            d.clone(),
            s("-q"),
        ],
    )
    .await;
    assert_eq!(changed.status.code(), Some(6));
    assert!(
        stderr(&changed).contains("--restart"),
        "{}",
        stderr(&changed)
    );

    let bad_url = boltget(st, vec![s("get"), s("ftp://example.test/x"), s("-q")]).await;
    assert_eq!(bad_url.status.code(), Some(4));

    let bad_header = boltget(
        st,
        vec![s("get"), srv.url("/data"), s("-H"), s("Range: bytes=0-")],
    )
    .await;
    assert_eq!(bad_header.status.code(), Some(2));
    let bad_header = boltget(st, vec![s("resume"), s("1"), s("-H"), s("nonsense")]).await;
    assert_eq!(bad_header.status.code(), Some(2));

    let usage = boltget(st, vec![s("get")]).await;
    assert_eq!(usage.status.code(), Some(2));
    let usage = boltget(st, vec![s("resume"), s("x")]).await;
    assert_eq!(usage.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn help_documents_exit_codes_and_events() {
    let state = tempfile::tempdir().unwrap();
    for cmd in ["get", "resume"] {
        let out = boltget(state.path(), vec![s("help"), s(cmd)]).await;
        let text = String::from_utf8_lossy(&out.stdout);
        for needle in ["130", "resumed", "paused", "exit_code", "  7 ", "  8 "] {
            assert!(
                text.contains(needle),
                "`help {cmd}` lacks {needle:?}:\n{text}"
            );
        }
    }
    let out = boltget(state.path(), vec![s("help"), s("list")]).await;
    assert!(String::from_utf8_lossy(&out.stdout).contains("JSON array"));
}

/// Ctrl-C (SIGINT) pauses with a final checkpoint; `resume` continues from it.
/// Unix only: on Windows, Ctrl-C cannot be sent to one child process from a test.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sigint_pauses_and_resume_continues() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let st = state.path().to_path_buf();
    let size = 4 * 1024 * 1024;
    let url = srv.url(&format!(
        "/data?size={size}&seed=13&slow_index=0&slow_bps=262144&key=sigint"
    ));
    let mut child = command(
        &st,
        &[
            s("get"),
            url,
            s("-o"),
            dir.path().display().to_string(),
            s("--json"),
            s("-c"),
            s("1"),
            s("--flush-interval-ms"),
            s("50"),
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let pid = child.id().to_string();
    let run = tokio::task::spawn_blocking(move || {
        let reader = wait_for_line(&mut child, |v| {
            v["event"] == "progress" && v["downloaded"].as_u64().unwrap_or(0) > 0
        });
        let status = Command::new("kill").args(["-INT", &pid]).status().unwrap();
        assert!(status.success());
        let rest: Vec<String> = reader.lines().map_while(Result::ok).collect();
        (child.wait().unwrap(), rest)
    });
    let (status, rest) = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .expect("child paused")
        .unwrap();
    assert_eq!(status.code(), Some(130));
    let last: serde_json::Value = serde_json::from_str(rest.last().unwrap()).unwrap();
    assert_eq!(last["event"], "paused");
    assert_eq!(last["exit_code"], 130);
    let saved = last["downloaded"].as_u64().unwrap();
    assert!(saved > 0 && saved < size);
    let list = list_json(&st).await;
    assert_eq!(list[0]["state"], "paused");
    assert_eq!(list[0]["downloaded"], saved);

    let out = boltget(&st, vec![s("resume"), s("1"), s("--json")]).await;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let events = lines(&out);
    let resumed = events.iter().find(|e| e["event"] == "resumed").unwrap();
    assert_eq!(resumed["downloaded"], saved);
    let done = events.last().unwrap();
    assert_eq!(done["resumed_from"], saved);
    let data = std::fs::read(done["path"].as_str().unwrap()).unwrap();
    assert_eq!(Sha256::digest(&data), Sha256::digest(content(13, size)));
}

#[tokio::test(flavor = "multi_thread")]
async fn speed_limit_flag_and_global_setting() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();

    let out = boltget(state.path(), vec![s("config"), s("get"), s("limit")]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "unlimited");
    let out = boltget(
        state.path(),
        vec![s("config"), s("set"), s("limit"), s("8M")],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let out = boltget(state.path(), vec![s("config"), s("list")]).await;
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("8388608 bytes per second"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let bad = boltget(
        state.path(),
        vec![s("config"), s("set"), s("limit"), s("fast")],
    )
    .await;
    assert_eq!(bad.status.code(), Some(2));
    let bad = boltget(
        state.path(),
        vec![s("get"), srv.url("/data"), s("--limit"), s("fast")],
    )
    .await;
    assert_eq!(bad.status.code(), Some(2));

    // 2 MiB at 1 MiB/s (the per-download limit is lower than the global 8M).
    let size = 2 * 1024 * 1024;
    let t0 = std::time::Instant::now();
    let out = boltget(
        state.path(),
        vec![
            s("get"),
            srv.url(&format!("/data?size={size}&seed=4")),
            s("-o"),
            dir.path().join("limited.bin").display().to_string(),
            s("--limit"),
            s("1M"),
            s("-q"),
        ],
    )
    .await;
    let secs = t0.elapsed().as_secs_f64();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(secs > 1.5 && secs < 4.5, "took {secs:.2} s");
    let got = std::fs::read(dir.path().join("limited.bin")).unwrap();
    assert_eq!(Sha256::digest(&got), Sha256::digest(content(4, size)));

    let out = boltget(state.path(), vec![s("config"), s("unset"), s("limit")]).await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("unlimited"));
}

fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(data).iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_match_mismatch_and_keep() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let size = 1024 * 1024 + 9;
    let url = srv.url(&format!("/data?size={size}&seed=21"));
    let good = sha256_hex(&content(21, size));

    let out = boltget(
        state.path(),
        vec![
            s("get"),
            url.clone(),
            s("-o"),
            dir.path().join("ok.bin").display().to_string(),
            s("--checksum"),
            format!("sha256:{good}"),
            s("--json"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let last = lines(&out).pop().unwrap();
    assert_eq!(last["event"], "completed");
    assert_eq!(last["verified"], format!("sha256:{good}"));
    let events = lines(&out);
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "state_changed" && e["state"] == "verifying")
    );

    // A wrong checksum: exit 9, file kept under its partial name, hints shown.
    let wrong = "0".repeat(64);
    let target = dir.path().join("bad.bin");
    let out = boltget(
        state.path(),
        vec![
            s("get"),
            url.clone(),
            s("-o"),
            target.display().to_string(),
            s("--checksum"),
            wrong,
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(9), "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("checksum mismatch"), "{err}");
    assert!(err.contains("--no-checksum"), "{err}");
    assert!(!target.exists());
    assert!(dir.path().join("bad.bin.boltget").exists());
    let list = list_json(state.path()).await;
    assert_eq!(list[1]["state"], "corrupt");
    assert_eq!(list[1]["checksum"]["actual"], good.as_str());

    // Keep it anyway.
    let out = boltget(
        state.path(),
        vec![s("resume"), s("2"), s("--no-checksum"), s("-q")],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert_eq!(sha256_hex(&std::fs::read(&target).unwrap()), good);

    // A malformed checksum is a usage error.
    let out = boltget(
        state.path(),
        vec![s("get"), url, s("--checksum"), s("sha256:xyz")],
    )
    .await;
    assert_eq!(out.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_link_is_refreshed() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let size = 4 * 1024 * 1024;
    let target = dir.path().join("big.bin");
    let out = boltget(
        state.path(),
        vec![
            s("get"),
            srv.url(&format!(
                "/data?size={size}&seed=31&throttle=2097152&slow_index=1&slow_bps=131072&fail_status=403&fail_from=4"
            )),
            s("-o"),
            target.display().to_string(),
            s("-c"),
            s("4"),
            s("--min-split-size"),
            s("65536"),
            s("-q"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(6), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("boltget refresh 1"),
        "{}",
        stderr(&out)
    );
    assert_eq!(list_json(state.path()).await[0]["state"], "needs_new_link");

    let out = boltget(
        state.path(),
        vec![
            s("refresh"),
            s("1"),
            srv.url(&format!("/data?size={}&seed=31&key=x", size + 1)),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(6), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("size differs"), "{}", stderr(&out));

    let out = boltget(
        state.path(),
        vec![
            s("refresh"),
            s("1"),
            srv.url(&format!("/data?size={size}&seed=31&etag_salt=b")),
            s("--json"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let events = lines(&out);
    assert_eq!(events[0]["event"], "refreshed");
    assert_eq!(events[0]["evidence"]["kind"], "samples");
    assert_eq!(events.last().unwrap()["event"], "completed");
    let got = std::fs::read(&target).unwrap();
    assert_eq!(Sha256::digest(&got), Sha256::digest(content(31, size)));
}

#[tokio::test(flavor = "multi_thread")]
async fn proxy_flag_environment_and_no_proxy() {
    use boltget_testserver::{ProxyMode, TestProxy};
    let srv = TestServer::start().await;
    let proxy = TestProxy::start(ProxyMode::Http, Some(("u", "topsecret"))).await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let url = srv.url("/data?size=200000&seed=41");
    let run = |args: Vec<String>, env: Vec<(&'static str, String)>| {
        let mut c = command(state.path(), &args);
        for k in [
            "http_proxy",
            "HTTP_PROXY",
            "https_proxy",
            "HTTPS_PROXY",
            "all_proxy",
            "ALL_PROXY",
            "no_proxy",
            "NO_PROXY",
        ] {
            c.env_remove(k);
        }
        for (k, v) in env {
            c.env(k, v);
        }
        tokio::task::spawn_blocking(move || c.output().expect("run boltget"))
    };
    let out = run(
        vec![
            s("get"),
            url.clone(),
            s("-o"),
            dir.path().join("a").display().to_string(),
            s("--proxy"),
            proxy.url(),
            s("-q"),
        ],
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let n = proxy.records().len();
    assert!(n >= 1);
    // From the environment.
    let out = run(
        vec![
            s("get"),
            url.clone(),
            s("-o"),
            dir.path().join("b").display().to_string(),
            s("-q"),
        ],
        vec![("http_proxy", proxy.url())],
    )
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let m = proxy.records().len();
    assert!(m > n);
    // --no-proxy ignores it.
    let out = run(
        vec![
            s("get"),
            url.clone(),
            s("-o"),
            dir.path().join("c").display().to_string(),
            s("--no-proxy"),
            s("-q"),
        ],
        vec![("http_proxy", proxy.url())],
    )
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert_eq!(proxy.records().len(), m);
    // Bad credentials: 407 is exit code 4; the password is never printed.
    let out = run(
        vec![
            s("get"),
            url.clone(),
            s("--proxy"),
            proxy.url_with(Some(("u", "badsecret"))),
            s("--no-state"),
        ],
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(4), "stderr: {}", stderr(&out));
    assert!(!stderr(&out).contains("badsecret"), "{}", stderr(&out));
    // A malformed proxy URL is a usage error, also without echoing it.
    let out = run(
        vec![s("get"), url, s("--proxy"), s("gopher://u:leak@x")],
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!stderr(&out).contains("leak"), "{}", stderr(&out));
}
