//! The process runner: allowlisted arguments, no shell, timeouts.
#![allow(clippy::unwrap_used)] // Test helpers outside #[test] functions.

mod support;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use boltget_media::runner::Invocation;
use boltget_media::{Error, Program};

/// A copy of the fake sidecar named `name` in a fresh directory.
fn fake(name: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join(support::exe(name));
    std::fs::write(&exe, support::fake_exe()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (tmp, exe)
}

#[tokio::test]
async fn runs_with_the_prefix_and_the_ffmpeg_location() {
    let (tmp, exe) = fake("yt-dlp");
    let inv = Invocation::yt_dlp(&exe, Some(tmp.path()), &["--version".to_owned()]).unwrap();
    let out = inv.run().await.unwrap();
    assert!(out.status.success(), "{}", out.stderr_text());
    let args = std::fs::read_to_string(tmp.path().join("fake-args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(
        &args[..3],
        ["--ignore-config", "--no-plugin-dirs", "--no-update"]
    );
    assert_eq!(args[3], "--ffmpeg-location");
    assert_eq!(args.last(), Some(&"--version"));
}

#[tokio::test]
async fn ffmpeg_inputs_are_limited_to_local_files() {
    let (_tmp, exe) = fake("ffmpeg");
    let args: Vec<String> = ["-i", "a.mp4", "-i", "b.m4a", "-c", "copy", "out.mp4"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let inv = Invocation::ffmpeg(Program::Ffmpeg, &exe, &args).unwrap();
    assert_eq!(
        inv.args(),
        [
            "-protocol_whitelist",
            "file",
            "-i",
            "a.mp4",
            "-protocol_whitelist",
            "file",
            "-i",
            "b.m4a",
            "-c",
            "copy",
            "out.mp4"
        ]
    );
}

#[tokio::test]
async fn refused_arguments_never_start_anything() {
    let (tmp, exe) = fake("yt-dlp");
    for bad in [["--exec", "calc"], ["-U", "--"], ["--netrc-cmd", "x"]] {
        let args: Vec<String> = bad.iter().map(|s| (*s).to_owned()).collect();
        assert!(matches!(
            Invocation::yt_dlp(&exe, None, &args),
            Err(Error::Argument { .. })
        ));
    }
    assert!(!tmp.path().join("fake-args.txt").exists());
}

#[tokio::test]
async fn only_absolute_executables_and_on_windows_only_exe() {
    let v = vec!["--version".to_owned()];
    assert!(Invocation::yt_dlp(std::path::Path::new("yt-dlp"), None, &v).is_err());
    if cfg!(windows) {
        for name in [r"C:\t\yt-dlp.bat", r"C:\t\yt-dlp.cmd", r"C:\t\yt-dlp"] {
            assert!(Invocation::yt_dlp(std::path::Path::new(name), None, &v).is_err());
        }
    }
}

#[tokio::test]
async fn a_hanging_sidecar_is_stopped_at_its_timeout() {
    let (tmp, exe) = fake("yt-dlp");
    std::fs::write(tmp.path().join("fake-hang"), b"").unwrap();
    let inv = Invocation::yt_dlp(&exe, None, &["--version".to_owned()])
        .unwrap()
        .timeout(Duration::from_millis(500));
    let start = Instant::now();
    let err = inv.run().await.unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err}");
    assert!(start.elapsed() < Duration::from_secs(30));
    // The process is gone: its file can be deleted (Windows keeps running
    // executables locked).
    wait_deletable(&exe).await;
}

/// Waits until `path` can be deleted, which on Windows means no process runs
/// from it any more.
async fn wait_deletable(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while std::fs::remove_file(path).is_err() {
        assert!(Instant::now() < deadline, "{} still runs", path.display());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn the_job_object_ends_the_whole_process_tree() {
    let (tmp, exe) = fake("yt-dlp");
    let gc_dir = tmp.path().join("gc");
    std::fs::create_dir(&gc_dir).unwrap();
    let gc = gc_dir.join(support::exe("yt-dlp"));
    std::fs::write(&gc, support::fake_exe()).unwrap();
    std::fs::write(gc_dir.join("fake-hang"), b"").unwrap();
    std::fs::write(tmp.path().join("fake-spawn"), b"").unwrap();
    std::fs::write(tmp.path().join("fake-hang"), b"").unwrap();

    let running = Invocation::yt_dlp(&exe, None, &["--version".to_owned()])
        .unwrap()
        .spawn()
        .unwrap();
    // The grandchild has started once it wrote its arguments.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !gc_dir.join("fake-args.txt").exists() {
        assert!(Instant::now() < deadline, "the grandchild did not start");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        std::fs::remove_file(&gc).is_err(),
        "the grandchild should be running"
    );
    drop(running);
    wait_deletable(&gc).await;
    wait_deletable(&exe).await;
}

#[tokio::test]
async fn the_environment_is_reduced() {
    assert!(std::env::var_os("PATH").is_some());
    let (tmp, exe) = fake("ffmpeg");
    let out = Invocation::ffmpeg(Program::Ffmpeg, &exe, &["-version".to_owned()])
        .unwrap()
        .run()
        .await
        .unwrap();
    assert!(out.stdout_text().starts_with("ffmpeg version "));
    let env = std::fs::read_to_string(tmp.path().join("fake-env.txt")).unwrap();
    let names: Vec<String> = env.lines().map(str::to_ascii_uppercase).collect();
    assert!(!names.contains(&"PATH".to_owned()), "{names:?}");
    if cfg!(windows) {
        assert!(names.contains(&"SYSTEMROOT".to_owned()), "{names:?}");
    }
}
