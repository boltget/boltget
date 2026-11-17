//! The sidecar manager against a fake release server: verified installs,
//! refused downloads, smoke tests and rollback, updates, pruning.
#![allow(clippy::unwrap_used)] // Test helpers outside #[test] functions.

mod support;

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use boltget_engine::{EngineConfig, ErrorKind};
use boltget_media::{Error, InstallEvent, Outcome, Program, Sidecars, Tool};
use support::{Fake, Route, Server, TestKey, publish_ffmpeg, publish_yt_dlp, sources};

fn manager(dir: &std::path::Path, server: &Server, key: &TestKey) -> Sidecars {
    Sidecars::new(
        dir.to_path_buf(),
        sources(server, key, None),
        EngineConfig::default(),
    )
    .smoke_timeout(Duration::from_secs(60))
}

fn quiet(_: InstallEvent) {}

fn version_dirs(dir: &std::path::Path, tool: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir.join(tool))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[tokio::test]
async fn installs_a_verified_yt_dlp_and_smoke_tests_it() {
    let server = Server::start().await;
    let key = TestKey::load();
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &Fake::default());
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);

    let events = Mutex::new(Vec::new());
    let record = |e: InstallEvent| events.lock().unwrap().push(e);
    let out = m.install(Tool::YtDlp, &record).await.unwrap();
    assert_eq!(
        out,
        Outcome::Installed {
            tool: Tool::YtDlp,
            version: "2026.01.01".into(),
            previous: None
        }
    );
    let events = events.into_inner().unwrap();
    assert!(events.contains(&InstallEvent::Verifying { tool: Tool::YtDlp }));
    assert_eq!(
        events.last(),
        Some(&InstallEvent::Activated {
            tool: Tool::YtDlp,
            version: "2026.01.01".into()
        })
    );

    let exe = m.program(Program::YtDlp).unwrap();
    assert!(exe.is_file());
    assert!(exe.starts_with(tmp.path().join("yt-dlp").join("2026.01.01")));
    // The smoke test ran with the safety prefix, and never with -U.
    let args = std::fs::read_to_string(exe.with_file_name("fake-args.txt")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-update",
            "--version"
        ]
    );
    let status = m.status().unwrap();
    assert_eq!(status.tools[0].active.as_deref(), Some("2026.01.01"));
    assert!(status.tools[0].last_check.is_some());
    // Nothing left behind but the version and the manifest.
    assert_eq!(version_dirs(tmp.path(), "yt-dlp"), ["2026.01.01"]);
    assert!(!tmp.path().join(".downloads").exists());

    // Installing again is a no-op (no network).
    let before = server.hits().len();
    let again = m.install(Tool::YtDlp, &quiet).await.unwrap();
    assert!(matches!(again, Outcome::UpToDate { .. }));
    assert_eq!(server.hits().len(), before);

    // Removing it deletes every version; the next use installs it again.
    m.remove(Tool::YtDlp).unwrap();
    assert_eq!(m.status().unwrap().tools[0].active, None);
    assert_eq!(version_dirs(tmp.path(), "yt-dlp"), Vec::<String>::new());
    assert!(matches!(
        m.program(Program::YtDlp),
        Err(Error::NotInstalled(Tool::YtDlp))
    ));
    let back = m.install(Tool::YtDlp, &quiet).await.unwrap();
    assert!(matches!(back, Outcome::Installed { .. }));
}

#[tokio::test]
async fn a_wrong_hash_is_refused() {
    let server = Server::start().await;
    let key = TestKey::load();
    let fake = Fake {
        wrong_hash: true,
        ..Fake::default()
    };
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &fake);
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);
    let err = m.install(Tool::YtDlp, &quiet).await.unwrap_err();
    assert!(
        matches!(&err, Error::Download(e) if e.kind() == ErrorKind::Integrity)
            || matches!(err, Error::Checksum { .. }),
        "{err}"
    );
    assert!(matches!(
        m.program(Program::YtDlp),
        Err(Error::NotInstalled(_))
    ));
    assert_eq!(version_dirs(tmp.path(), "yt-dlp"), Vec::<String>::new());
}

#[tokio::test]
async fn a_checksum_file_with_a_bad_signature_is_refused() {
    let server = Server::start().await;
    let key = TestKey::load();
    let fake = Fake {
        tamper_sums: true,
        ..Fake::default()
    };
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &fake);
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);
    let err = m.install(Tool::YtDlp, &quiet).await.unwrap_err();
    assert!(matches!(err, Error::Signature { .. }), "{err}");
    // The binary itself was never downloaded.
    assert!(!server.hits().iter().any(|h| h.ends_with(support::YT_ASSET)));
    assert_eq!(version_dirs(tmp.path(), "yt-dlp"), Vec::<String>::new());
}

#[tokio::test]
async fn a_signature_by_another_key_is_refused() {
    let server = Server::start().await;
    let key = TestKey::load();
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &Fake::default());
    let tmp = tempfile::tempdir().unwrap();
    // The manager trusts yt-dlp's real key, not the test key.
    let mut s = sources(&server, &key, None);
    s.yt_dlp_key = boltget_media::sources::yt_dlp_key();
    let m = Sidecars::new(tmp.path().to_path_buf(), s, EngineConfig::default());
    let err = m.install(Tool::YtDlp, &quiet).await.unwrap_err();
    assert!(matches!(err, Error::Signature { .. }), "{err}");
}

#[tokio::test]
async fn downloads_stay_on_the_pinned_hosts() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();

    // An asset URL on another host is refused before anything is fetched.
    let fake = Fake {
        asset_url: Some(format!("http://localhost:{}/x.zip", server.port())),
        ..Fake::default()
    };
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &fake);
    let m = manager(tmp.path(), &server, &key);
    let err = m.install(Tool::YtDlp, &quiet).await.unwrap_err();
    assert!(matches!(err, Error::Release { .. }), "{err}");

    // A redirect to another host is refused by the engine.
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &Fake::default());
    let path = format!("/yt/stable/download/2026.01.01/{}", support::YT_ASSET);
    server.set(
        &path,
        Route::Redirect(format!("http://localhost:{}{path}", server.port())),
    );
    let err = m.install(Tool::YtDlp, &quiet).await.unwrap_err();
    assert!(
        matches!(&err, Error::Download(e) if e.kind() == ErrorKind::Permanent),
        "{err}"
    );
    assert_eq!(version_dirs(tmp.path(), "yt-dlp"), Vec::<String>::new());
}

#[tokio::test]
async fn a_failed_smoke_test_keeps_the_previous_version() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &Fake::default());
    m.install(Tool::YtDlp, &quiet).await.unwrap();

    // 2026.02.02 crashes on start.
    let broken = Fake {
        extra: vec![("fake-fail", Vec::new())],
        ..Fake::default()
    };
    publish_yt_dlp(&server, &key, "stable", "2026.02.02", &broken);
    let err = m.update(Tool::YtDlp, &quiet).await.unwrap_err();
    assert!(matches!(err, Error::SmokeTest { .. }), "{err}");
    let st = &m.status().unwrap().tools[0];
    assert_eq!(st.active.as_deref(), Some("2026.01.01"));
    assert_eq!(version_dirs(tmp.path(), "yt-dlp"), ["2026.01.01"]);

    // 2026.03.03 prints the wrong version.
    let lying = Fake {
        extra: vec![("fake-version.txt", b"1999.01.01".to_vec())],
        ..Fake::default()
    };
    publish_yt_dlp(&server, &key, "stable", "2026.03.03", &lying);
    let err = m.update(Tool::YtDlp, &quiet).await.unwrap_err();
    assert!(matches!(err, Error::SmokeTest { .. }), "{err}");
    assert_eq!(
        m.status().unwrap().tools[0].active.as_deref(),
        Some("2026.01.01")
    );
}

#[tokio::test]
async fn updates_keep_one_previous_version_and_prune_the_rest() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);
    for v in ["2026.01.01", "2026.02.02", "2026.03.03"] {
        publish_yt_dlp(&server, &key, "stable", v, &Fake::default());
        m.update(Tool::YtDlp, &quiet).await.unwrap();
    }
    let st = &m.status().unwrap().tools[0];
    assert_eq!(st.active.as_deref(), Some("2026.03.03"));
    assert_eq!(st.previous.as_deref(), Some("2026.02.02"));
    assert_eq!(
        version_dirs(tmp.path(), "yt-dlp"),
        ["2026.02.02", "2026.03.03"]
    );

    // The same release again: up to date, nothing downloaded but metadata.
    let out = m.update(Tool::YtDlp, &quiet).await.unwrap();
    assert_eq!(
        out,
        Outcome::UpToDate {
            tool: Tool::YtDlp,
            version: "2026.03.03".into()
        }
    );

    // Leftovers of an interrupted install are pruned too.
    std::fs::create_dir_all(tmp.path().join("yt-dlp").join(".staging-x-1")).unwrap();
    std::fs::create_dir_all(tmp.path().join("yt-dlp").join("2025.12.12")).unwrap();
    let removed = m.prune(Tool::YtDlp).unwrap();
    assert_eq!(removed, ["2025.12.12"]);
    assert_eq!(
        version_dirs(tmp.path(), "yt-dlp"),
        ["2026.02.02", "2026.03.03"]
    );
}

#[tokio::test]
async fn a_broken_active_version_rolls_back_and_is_not_reinstalled() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);
    for v in ["2026.01.01", "2026.02.02"] {
        publish_yt_dlp(&server, &key, "stable", v, &Fake::default());
        m.update(Tool::YtDlp, &quiet).await.unwrap();
    }
    assert_eq!(
        m.verify_active(Tool::YtDlp, &quiet).await.unwrap(),
        "2026.02.02"
    );

    // The active version starts failing (a damaged file, say).
    let active = m.program(Program::YtDlp).unwrap();
    std::fs::write(active.with_file_name("fake-fail"), b"").unwrap();
    let events = Mutex::new(Vec::new());
    let record = |e: InstallEvent| events.lock().unwrap().push(e);
    let now = m.verify_active(Tool::YtDlp, &record).await.unwrap();
    assert_eq!(now, "2026.01.01");
    assert!(
        events
            .into_inner()
            .unwrap()
            .contains(&InstallEvent::RolledBack {
                tool: Tool::YtDlp,
                from: "2026.02.02".into(),
                to: "2026.01.01".into()
            })
    );
    let st = &m.status().unwrap().tools[0];
    assert_eq!(st.active.as_deref(), Some("2026.01.01"));
    assert_eq!(st.previous, None);
    assert_eq!(version_dirs(tmp.path(), "yt-dlp"), ["2026.01.01"]);

    // The latest release is still the bad one: the update stays put.
    let out = m.update(Tool::YtDlp, &quiet).await.unwrap();
    assert!(matches!(out, Outcome::UpToDate { ref version, .. } if version == "2026.01.01"));

    // Nothing to go back to now.
    assert!(matches!(m.rollback(Tool::YtDlp), Err(Error::NoPrevious(_))));
}

#[tokio::test]
async fn update_checks_are_daily_and_on_extractor_failure() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);

    // Not installed: nothing is checked or installed.
    let out = m.update_if_due(SystemTime::now(), &quiet).await.unwrap();
    assert!(
        out.iter()
            .all(|o| matches!(o, Outcome::NotInstalled { .. }))
    );
    assert!(matches!(
        m.on_extractor_failure(SystemTime::now(), &quiet)
            .await
            .unwrap(),
        Outcome::NotInstalled { .. }
    ));

    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &Fake::default());
    m.install(Tool::YtDlp, &quiet).await.unwrap();
    publish_yt_dlp(&server, &key, "stable", "2026.02.02", &Fake::default());

    let now = SystemTime::now();
    let soon = now + Duration::from_secs(3600);
    let out = m.update_if_due(soon, &quiet).await.unwrap();
    assert_eq!(out[0], Outcome::NotDue { tool: Tool::YtDlp });

    let tomorrow = now + Duration::from_secs(25 * 3600);
    let out = m.update_if_due(tomorrow, &quiet).await.unwrap();
    assert!(matches!(&out[0], Outcome::Installed { version, .. } if version == "2026.02.02"));

    // An extractor failure checks at once, then not again within the hour.
    publish_yt_dlp(&server, &key, "stable", "2026.03.03", &Fake::default());
    let out = m.on_extractor_failure(soon, &quiet).await.unwrap();
    assert!(matches!(out, Outcome::Installed { ref version, .. } if version == "2026.03.03"));
    let out = m
        .on_extractor_failure(soon + Duration::from_secs(600), &quiet)
        .await
        .unwrap();
    assert_eq!(out, Outcome::NotDue { tool: Tool::YtDlp });
}

#[tokio::test]
async fn the_nightly_channel_reads_its_own_releases() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);
    publish_yt_dlp(&server, &key, "stable", "2026.01.01", &Fake::default());
    publish_yt_dlp(
        &server,
        &key,
        "nightly",
        "2026.01.05.101010",
        &Fake::default(),
    );
    m.set_channel(boltget_media::Channel::Nightly).unwrap();
    assert_eq!(
        m.latest(Tool::YtDlp).await.unwrap().version,
        "2026.01.05.101010"
    );
    let out = m.install(Tool::YtDlp, &quiet).await.unwrap();
    assert!(
        matches!(out, Outcome::Installed { ref version, .. } if version == "2026.01.05.101010")
    );
}

#[tokio::test]
async fn ffmpeg_installs_from_its_pin_and_updates_when_the_pin_changes() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let ffplay = support::exe("ffplay");
    let pin = publish_ffmpeg(&server, "n9.9-test", &[(ffplay.as_str(), b"player")]);
    let m = Sidecars::new(
        tmp.path().to_path_buf(),
        sources(&server, &key, Some(pin.clone())),
        EngineConfig::default(),
    );
    assert_eq!(m.latest(Tool::Ffmpeg).await.unwrap().size, pin.size);
    let out = m.install(Tool::Ffmpeg, &quiet).await.unwrap();
    assert!(matches!(out, Outcome::Installed { ref version, .. } if version == "n9.9-test"));
    let ffmpeg = m.program(Program::Ffmpeg).unwrap();
    let ffprobe = m.program(Program::Ffprobe).unwrap();
    assert!(ffmpeg.is_file() && ffprobe.is_file());
    // Only bin/ was unpacked, without the player.
    let vdir = tmp.path().join("ffmpeg").join("n9.9-test");
    assert!(!vdir.join("doc").exists());
    assert!(vdir.join("bin").is_dir());
    assert!(!vdir.join("bin").join(&ffplay).exists());
    assert_eq!(
        std::fs::read_to_string(ffmpeg.with_file_name("fake-args.txt")).unwrap(),
        "-version"
    );

    // Same pin: not due. A new pin (a Boltget update): installed.
    let out = m.update_if_due(SystemTime::now(), &quiet).await.unwrap();
    assert_eq!(out[1], Outcome::NotDue { tool: Tool::Ffmpeg });
    let pin2 = publish_ffmpeg(&server, "n9.10-test", &[]);
    let m2 = Sidecars::new(
        tmp.path().to_path_buf(),
        sources(&server, &key, Some(pin2)),
        EngineConfig::default(),
    );
    let out = m2.update_if_due(SystemTime::now(), &quiet).await.unwrap();
    assert!(
        matches!(&out[1], Outcome::Installed { version, previous, .. }
        if version == "n9.10-test" && previous.as_deref() == Some("n9.9-test"))
    );
}

#[tokio::test]
async fn ffmpeg_with_a_wrong_pinned_hash_is_refused() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let mut pin = publish_ffmpeg(&server, "n9.9-test", &[]);
    pin.sha256 = support::sha256_hex(b"not it");
    let m = Sidecars::new(
        tmp.path().to_path_buf(),
        sources(&server, &key, Some(pin)),
        EngineConfig::default(),
    );
    let err = m.install(Tool::Ffmpeg, &quiet).await.unwrap_err();
    assert!(
        matches!(&err, Error::Download(e) if e.kind() == ErrorKind::Integrity),
        "{err}"
    );
    assert_eq!(version_dirs(tmp.path(), "ffmpeg"), Vec::<String>::new());
}

#[tokio::test]
async fn an_ffmpeg_whose_ffprobe_fails_is_not_activated() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    // `fake-fail` in bin/ fails both programs.
    let pin = publish_ffmpeg(&server, "n9.9-test", &[("fake-fail", b"")]);
    let m = Sidecars::new(
        tmp.path().to_path_buf(),
        sources(&server, &key, Some(pin)),
        EngineConfig::default(),
    );
    let err = m.install(Tool::Ffmpeg, &quiet).await.unwrap_err();
    assert!(matches!(err, Error::SmokeTest { .. }), "{err}");
    assert!(matches!(
        m.program(Program::Ffmpeg),
        Err(Error::NotInstalled(_))
    ));
    assert_eq!(version_dirs(tmp.path(), "ffmpeg"), Vec::<String>::new());
}

#[tokio::test]
async fn unsupported_platforms_say_so() {
    let server = Server::start().await;
    let key = TestKey::load();
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path(), &server, &key);
    assert!(matches!(
        m.install(Tool::Ffmpeg, &quiet).await,
        Err(Error::Unsupported(Tool::Ffmpeg))
    ));
    assert!(!m.status().unwrap().tools[1].supported);
}
