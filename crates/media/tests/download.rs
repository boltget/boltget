//! Media downloads with a fake yt-dlp: progress from the template lines, the
//! finished file, failures, and stopping and continuing.
#![allow(clippy::unwrap_used)] // Test helpers outside #[test] functions.

mod support;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use boltget_engine::{Cookie, CookieJar};
use boltget_media::partial::{self, Scope};
use boltget_media::{
    DownloadOptions, DownloadOutcome, Downloader, Error, ErrorCategory, MediaProgress, Phase,
};
use tokio::sync::watch;

/// A fake yt-dlp in a fresh directory, a downloader for it and a folder for
/// the result.
fn fake() -> (tempfile::TempDir, PathBuf, Downloader, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join(support::exe("yt-dlp"));
    std::fs::write(&exe, support::fake_exe()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let downloader = Downloader::new(
        exe,
        Some(tmp.path().to_path_buf()),
        tmp.path().join(".jobs"),
    );
    let dir = tmp.path().to_path_buf();
    let out = tmp.path().join("Videos");
    (tmp, dir, downloader, out)
}

const PROGRESS: &str = r#"[youtube] Extracting URL: https://v.test/watch?v=v1
[boltget] download {"status": "downloading", "downloaded_bytes": 100, "total_bytes": 400, "filename": "a.f137.mp4", "speed": 50.0, "eta": 6}
[boltget] download {"status": "downloading", "downloaded_bytes": 300, "total_bytes": 400, "filename": "a.f137.mp4", "speed": 50.0, "eta": 2}
[boltget] download {"status": "finished", "downloaded_bytes": 400, "total_bytes": 400, "filename": "a.f137.mp4"}
[boltget] download {"status": "finished", "downloaded_bytes": 100, "total_bytes": 100, "filename": "a.f140.m4a"}
[boltget] postprocess {"status": "started", "postprocessor": "Merger"}
[boltget] postprocess {"status": "finished", "postprocessor": "Merger"}
"#;

#[tokio::test]
async fn a_download_reports_progress_and_its_file() {
    let (_tmp, dir, downloader, out) = fake();
    std::fs::write(dir.join("fake-progress.txt"), PROGRESS).unwrap();
    std::fs::write(dir.join("fake-slow"), "").unwrap();
    let mut jar = CookieJar::new();
    jar.insert(Cookie::from_browser(
        "SID", "secret", ".v.test", false, "/", true, None,
    ));
    let mut opts = DownloadOptions::new("bv*+ba/b", &out);
    opts.cookies = Some(jar);
    opts.expected_size = Some(450);
    let seen: Mutex<Vec<MediaProgress>> = Mutex::default();
    let (_stop, stop_rx) = watch::channel(false);
    let outcome = downloader
        .run(
            "https://v.test/watch?v=v1&secret=1",
            &opts,
            &|p| seen.lock().unwrap().push(p.clone()),
            stop_rx,
        )
        .await
        .unwrap();
    let file = out.join("fake video [v1].mp4");
    assert_eq!(
        outcome,
        DownloadOutcome::Completed {
            path: file.clone(),
            size: Some(10),
            kept_original: false,
        }
    );
    let seen = seen.into_inner().unwrap();
    let first = seen.first().unwrap();
    assert_eq!((first.downloaded, first.total), (100, Some(450)));
    assert!(seen.iter().any(|p| p.phase == Phase::Processing));
    let last = seen.last().unwrap();
    assert_eq!(
        (last.downloaded, last.total, last.estimated),
        (500, Some(500), false)
    );
    // The cookies reached yt-dlp through a file that is gone now.
    let cookies = std::fs::read_to_string(dir.join("fake-cookies.txt")).unwrap();
    assert!(cookies.contains("SID\tsecret"));
    assert_eq!(std::fs::read_dir(dir.join(".jobs")).unwrap().count(), 0);
    let args = std::fs::read_to_string(dir.join("fake-args.txt")).unwrap();
    assert!(
        args.starts_with("--ignore-config\n--no-plugin-dirs\n--no-update\n--ffmpeg-location\n")
    );
    assert!(args.ends_with("--\nhttps://v.test/watch?v=v1&secret=1"));
    #[cfg(windows)]
    {
        let mut ads = file.into_os_string();
        ads.push(":Zone.Identifier");
        let zone = std::fs::read_to_string(ads).unwrap();
        assert!(zone.contains("ZoneId=3"));
        assert!(zone.contains("HostUrl=https://v.test/watch\r\n"), "{zone}");
    }
}

#[tokio::test]
async fn a_failure_carries_yt_dlp_s_message() {
    let (_tmp, dir, downloader, out) = fake();
    std::fs::write(
        dir.join("fake-download-error.txt"),
        "WARNING: something\nERROR: [generic] Requested format is not available\n",
    )
    .unwrap();
    let (_stop, rx) = watch::channel(false);
    let e = downloader
        .run(
            "https://v.test/x",
            &DownloadOptions::new("b", &out),
            &|_| {},
            rx,
        )
        .await
        .unwrap_err();
    match &e {
        Error::Failed { category, message } => {
            assert_eq!(message, "[generic] Requested format is not available");
            assert_eq!(*category, ErrorCategory::Other);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(e.message_key(), "error.media.other");

    // Sorted into its category, with the key front ends show.
    std::fs::write(
        dir.join("fake-download-error.txt"),
        "ERROR: [youtube] v1: Sign in to confirm your age. This video may be inappropriate for some users.
",
    )
    .unwrap();
    let (_stop, rx) = watch::channel(false);
    let e = downloader
        .run(
            "https://v.test/x",
            &DownloadOptions::new("b", &out),
            &|_| {},
            rx,
        )
        .await
        .unwrap_err();
    assert_eq!(e.category(), Some(ErrorCategory::SignIn));
    assert_eq!(e.message_key(), "error.media.sign_in");
}

#[tokio::test]
async fn prefer_mp4_keeps_the_original_container_when_mp4_cannot_hold_it() {
    let (_tmp, dir, downloader, out) = fake();
    // A VP8 WebM (as on Wikimedia Commons): FFmpeg cannot remux it to MP4.
    std::fs::write(dir.join("fake-ext.txt"), "webm").unwrap();
    std::fs::write(dir.join("fake-remux-fails"), "").unwrap();
    let opts = DownloadOptions::new("bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b", &out);
    let (_stop, rx) = watch::channel(false);
    let outcome = downloader
        .run("https://v.test/x", &opts, &|_| {}, rx)
        .await
        .unwrap();
    let webm = out.join("fake video [v1].webm");
    assert_eq!(
        outcome,
        DownloadOutcome::Completed {
            path: webm.clone(),
            size: Some(10),
            kept_original: true,
        }
    );
    assert_eq!(std::fs::read(&webm).unwrap(), b"fake media");
    // FFmpeg's unfinished MP4s are gone; the second run asked for no MP4.
    assert!(!out.join("fake video [v1].temp.mp4").exists());
    assert!(!out.join("fake video [v1].mp4").exists());
    let args = std::fs::read_to_string(dir.join("fake-args.txt")).unwrap();
    assert!(!args.contains("--remux-video") && !args.contains("--merge-output-format"));
    // Audio is converted, not remuxed: no second run there.
    let mut audio = opts.clone();
    audio.audio_only = true;
    std::fs::write(
        dir.join("fake-download-error.txt"),
        "ERROR: Postprocessing: Conversion failed!
",
    )
    .unwrap();
    let (_stop, rx) = watch::channel(false);
    let e = downloader
        .run("https://v.test/x", &audio, &|_| {}, rx)
        .await
        .unwrap_err();
    assert_eq!(e.category(), Some(ErrorCategory::Processing));
}

#[tokio::test]
async fn a_stopped_download_keeps_its_part_file_and_continues() {
    let (_tmp, dir, downloader, out) = fake();
    std::fs::write(
        dir.join("fake-progress.txt"),
        PROGRESS.lines().nth(1).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("fake-hang-download"), "").unwrap();
    let opts = DownloadOptions::new("b", &out);
    let (stop, rx) = watch::channel(false);
    let started: Mutex<bool> = Mutex::default();
    let on_progress = |_: &MediaProgress| *started.lock().unwrap() = true;
    let run = downloader.run("https://v.test/x", &opts, &on_progress, rx);
    let stopper = async {
        while !*started.lock().unwrap() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // The fake writes its partial file right after its progress line.
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop.send_replace(true);
    };
    let (outcome, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(run, stopper)
    })
    .await
    .unwrap();
    assert_eq!(outcome.unwrap(), DownloadOutcome::Stopped);
    let part = out.join("fake video [v1].mp4.part");
    assert_eq!(std::fs::read(&part).unwrap(), b"fake ");
    assert!(!out.join("fake video [v1].mp4").exists());
    // The job's partial files are found by its stem, with nothing else.
    std::fs::write(out.join("fake video [v1] (mine).mp4"), b"keep").unwrap();
    let left = partial::files(&out, "fake video [v1]", Scope::Unfinished);
    let names: Vec<String> = left
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "fake video [v1].mp4.part",
            "fake video [v1].mp4.part-Frag1",
            "fake video [v1].mp4.ytdl"
        ]
    );
    // The same arguments again continue from it.
    std::fs::remove_file(dir.join("fake-hang-download")).unwrap();
    let (_stop, rx) = watch::channel(false);
    let outcome = downloader
        .run("https://v.test/x", &opts, &|_| {}, rx)
        .await
        .unwrap();
    assert!(matches!(outcome, DownloadOutcome::Completed { .. }));
    assert_eq!(
        std::fs::read_to_string(dir.join("fake-resumed.txt")).unwrap(),
        "yes"
    );
    // It continued from the fifth byte, and the file is whole.
    assert_eq!(
        std::fs::read_to_string(dir.join("fake-resumed-from.txt")).unwrap(),
        "5"
    );
    assert_eq!(
        std::fs::read(out.join("fake video [v1].mp4")).unwrap(),
        b"fake media"
    );
    assert_eq!(
        partial::files(&out, "fake video [v1]", Scope::Unfinished),
        Vec::<PathBuf>::new()
    );
}

#[tokio::test]
async fn a_download_that_goes_quiet_is_stopped() {
    let (_tmp, dir, downloader, out) = fake();
    std::fs::write(dir.join("fake-hang-download"), "").unwrap();
    let mut opts = DownloadOptions::new("b", &out);
    opts.stall_timeout = Duration::from_millis(500);
    let (_stop, rx) = watch::channel(false);
    let e = tokio::time::timeout(
        Duration::from_secs(30),
        downloader.run("https://v.test/x", &opts, &|_| {}, rx),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(matches!(e, Error::Timeout { .. }), "{e:?}");
}

#[tokio::test]
async fn custom_programs_must_start_and_report_a_version() {
    use boltget_media::{Program, Sidecars, Sources};
    let (_tmp, dir, _downloader, _out) = fake();
    let sidecars = Sidecars::new(
        dir.join("tools"),
        Sources::official(),
        boltget_engine::EngineConfig::default(),
    );
    let exe = dir.join(support::exe("yt-dlp"));
    std::fs::write(dir.join("fake-version.txt"), "2026.11.01").unwrap();
    assert_eq!(
        sidecars.check_custom(Program::YtDlp, &exe).await.unwrap(),
        "2026.11.01"
    );
    std::fs::write(dir.join("fake-version.txt"), "not a version!").unwrap();
    let e = sidecars
        .check_custom(Program::YtDlp, &exe)
        .await
        .unwrap_err();
    assert_eq!(e.message_key(), "error.media.custom_program");
    std::fs::write(dir.join("fake-fail"), "").unwrap();
    assert!(sidecars.check_custom(Program::YtDlp, &exe).await.is_err());
    assert!(
        sidecars
            .check_custom(Program::YtDlp, &dir.join(support::exe("missing")))
            .await
            .is_err()
    );
    // A tool that is not installed is removed without complaint.
    sidecars.remove(boltget_media::Tool::YtDlp).unwrap();
}
