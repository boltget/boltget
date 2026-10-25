//! Checksum verification against the local test server: user checksums, server
//! digests, the `Corrupt` state and what resuming a corrupt download does.

mod support;

use std::time::Duration;

use boltget_engine::{
    Checksum, ChecksumSource, DownloadId, Engine, Error, ErrorKind, HashAlgo, ResumeOptions, State,
};
use boltget_testserver::{TestServer, content};
use md5::Digest as _;
use support::{MIB, assert_content, cfg, get_with, request};

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn sha256_of(seed: u64, size: u64) -> Checksum {
    Checksum::new(
        HashAlgo::Sha256,
        &hex(&sha2::Sha256::digest(content(seed, size))),
    )
    .expect("valid digest")
}

#[tokio::test]
async fn user_checksums_of_every_algorithm_are_verified() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 3 * MIB + 77;
    let data = content(5, size);
    for c in [
        Checksum::new(HashAlgo::Md5, &hex(&md5::Md5::digest(&data))).unwrap(),
        Checksum::new(HashAlgo::Sha1, &hex(&sha1::Sha1::digest(&data))).unwrap(),
        sha256_of(5, size),
        Checksum::new(HashAlgo::Sha512, &hex(&sha2::Sha512::digest(&data))).unwrap(),
    ] {
        let url = srv.url(&format!(
            "/data?size={size}&seed=5&key={}&throttle=8388608",
            c.algo
        ));
        let done = get_with(
            &engine,
            request(&url, dir.path()).connections(4).checksum(c.clone()),
        )
        .await
        .unwrap();
        assert_content(&done, 5, size);
        assert_eq!(done.verified.as_ref(), Some(&c));
        assert!(
            done.segments > 1,
            "hashing must cope with out-of-order segments"
        );
        std::fs::remove_file(&done.path).unwrap();
    }
}

#[tokio::test]
async fn mismatch_leaves_a_corrupt_download_that_can_be_kept_or_redone() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let size = 2 * MIB;
    let wrong = sha256_of(99, size);
    let h = engine.add(
        request(&srv.url(&format!("/data?size={size}&seed=6")), dir.path()).checksum(wrong.clone()),
    );
    let id = h.id();
    let err = h.wait().await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Integrity);
    let Error::ChecksumMismatch {
        expected,
        actual,
        origin,
    } = &err
    else {
        panic!("expected a checksum mismatch, got {err}");
    };
    assert_eq!(expected, &wrong);
    assert_eq!(actual, &sha256_of(6, size).hex);
    assert_eq!(origin, &ChecksumSource::User);
    // Never renamed to the final name; the partial file is kept.
    assert!(!dir.path().join("data").exists());
    assert!(dir.path().join("data.boltget").exists());
    let list = engine.list().await.unwrap();
    assert_eq!(list[0].state, State::Corrupt);
    let status = list[0].checksum.as_ref().unwrap();
    assert_eq!(status.expected, wrong);
    assert_eq!(status.actual.as_deref(), Some(actual.as_str()));
    assert_eq!(list[0].error.as_ref().unwrap().kind, ErrorKind::Integrity);

    // Resuming verifies again (from disk, no network) and fails the same way.
    let before = srv.requests().len();
    let err = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Integrity);
    assert_eq!(srv.requests().len(), before, "nothing to download again");

    // "Keep anyway": resume without a checksum.
    let done = engine
        .resume(id, ResumeOptions::default().checksum(None))
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_content(&done, 6, size);
    assert_eq!(done.verified, None);
    assert_eq!(srv.requests().len(), before);
    engine.shutdown().await;
}

#[tokio::test]
async fn corrupt_download_restarts_with_the_right_checksum() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let size = MIB + 3;
    let url = srv.url(&format!("/data?size={size}&seed=8"));
    let h = engine.add(request(&url, dir.path()).checksum(sha256_of(1, size)));
    let id = h.id();
    assert_eq!(h.wait().await.unwrap_err().kind(), ErrorKind::Integrity);
    let done = engine
        .resume(
            id,
            ResumeOptions::default()
                .restart(true)
                .checksum(Some(sha256_of(8, size))),
        )
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_content(&done, 8, size);
    assert_eq!(done.verified, Some(sha256_of(8, size)));
    let list = engine.list().await.unwrap();
    assert_eq!(list[0].state, State::Completed);
    assert_eq!(
        list[0].checksum.as_ref().unwrap().actual.as_deref(),
        Some(sha256_of(8, size).hex.as_str())
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn server_digests_are_verified() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = 2 * MIB + 1;
    // (query, expected source header, algorithm)
    let cases = [
        ("digest=repr", "repr-digest", HashAlgo::Sha256),
        ("digest=legacy", "digest", HashAlgo::Sha512),
        ("digest=goog", "x-goog-hash", HashAlgo::Md5),
        ("digest=amz", "x-amz-checksum-sha256", HashAlgo::Sha256),
        // Body digests count only because the probe's body is the whole file.
        ("digest=content", "content-digest", HashAlgo::Sha256),
        ("digest=md5&ranges=none", "content-md5", HashAlgo::Md5),
    ];
    for (i, (q, header, algo)) in cases.into_iter().enumerate() {
        let url = srv.url(&format!("/data?size={size}&seed={i}&{q}"));
        let done = get_with(&engine, request(&url, dir.path()))
            .await
            .unwrap_or_else(|e| panic!("{q}: {e}"));
        assert_content(&done, i as u64, size);
        let v = done.verified.unwrap_or_else(|| panic!("{q}: not verified"));
        assert_eq!(v.algo, algo, "{q} ({header})");
        std::fs::remove_file(&done.path).unwrap();
    }
}

#[tokio::test]
async fn wrong_server_digest_is_corrupt_unless_ignored() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(cfg()).unwrap();
    let size = MIB;
    let url = srv.url(&format!("/data?size={size}&digest=repr&digest_bad=1"));
    let err = get_with(&engine, request(&url, dir.path()))
        .await
        .unwrap_err();
    let Error::ChecksumMismatch { origin, .. } = &err else {
        panic!("{err}");
    };
    assert_eq!(origin, &ChecksumSource::Server("repr-digest".into()));
    assert!(err.to_string().contains("repr-digest"), "{err}");
    std::fs::remove_file(dir.path().join("data.boltget")).unwrap();
    let done = get_with(&engine, request(&url, dir.path()).server_digest(false))
        .await
        .unwrap();
    assert_content(&done, 1, size);
    assert_eq!(done.verified, None);
    // A user checksum takes precedence over the server's.
    std::fs::remove_file(&done.path).unwrap();
    let done = get_with(
        &engine,
        request(&url, dir.path()).checksum(sha256_of(1, size)),
    )
    .await
    .unwrap();
    assert_eq!(done.verified, Some(sha256_of(1, size)));
}

#[tokio::test]
async fn resumed_download_hashes_what_was_already_on_disk() {
    let srv = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let size = 4 * MIB;
    let url = srv.url(&format!("/data?size={size}&seed=12&throttle=262144"));
    let id: DownloadId;
    {
        let engine = Engine::open(cfg(), state.path()).unwrap();
        let h = engine.add(
            request(&url, dir.path())
                .connections(4)
                .checksum(sha256_of(12, size)),
        );
        id = h.id();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(engine.pause(id).await.unwrap(), State::Paused);
        let p = engine.list().await.unwrap()[0].downloaded;
        assert!(p > 0 && p < size, "paused mid-way at {p}");
        engine.shutdown().await;
    }
    let engine = Engine::open(cfg(), state.path()).unwrap();
    let done = engine
        .resume(id, ResumeOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_content(&done, 12, size);
    assert!(done.resumed_from > 0);
    assert_eq!(done.verified, Some(sha256_of(12, size)));
    engine.shutdown().await;
}
