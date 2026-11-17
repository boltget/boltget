//! A stand-in for yt-dlp, ffmpeg and ffprobe in the media crate's tests. Never
//! shipped. It answers `--version` / `-version` like the real programs, with
//! the version taken from `fake-version.txt` next to it or, failing that, from
//! the name of its version directory (`tools/<tool>/<version>/...`). It writes
//! its arguments to `fake-args.txt` and its environment's names to
//! `fake-env.txt`, fails if `fake-fail` exists next to it, and
//! hangs if `fake-hang` does. With `fake-spawn` it first starts `gc/<its own
//! name>`. As yt-dlp with `-J` it copies the `--cookies` file (if any) to
//! `fake-cookies.txt`, then prints `fake-probe-error.txt` to stderr and
//! fails if that exists, or prints `fake-info.json` (with `--yes-playlist`,
//! `fake-info-playlist.json` when that exists). As yt-dlp with
//! `--progress-template` (a download) it copies the cookies the same way,
//! prints `fake-download-error.txt` to stderr and fails if that exists,
//! otherwise prints the lines of `fake-progress.txt` (if any; 50 ms apart
//! with `fake-slow`), hangs with `fake-hang-download` (leaving
//! `fake video [v1].mp4.part` in the `-P` folder), then writes
//! `fake video [v1].mp4` there (the `-o` template with `%(title)s` as
//! `fake video`, `%(id)s` as `v1`, `%(ext)s` as `mp4`; `fake-resumed.txt`
//! records whether a `.part` file was there to continue from) and reports
//! it as yt-dlp's `--print after_move:` template would. While a download
//! runs it keeps a `fake-running-<pid>` file next to it, and appends how
//! many there were when it started to `fake-overlap.txt`, so tests can see
//! how many ran at once.
#![allow(clippy::print_stdout)]

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().unwrap_or(Path::new("."));
    let name = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_owned();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = std::fs::write(dir.join("fake-args.txt"), args.join("\n"));
    let env: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    let _ = std::fs::write(dir.join("fake-env.txt"), env.join("\n"));
    if dir.join("fake-spawn").exists()
        && let Some(file) = exe.file_name()
    {
        // A grandchild for the Job Object test: `gc/<same name>`.
        let _ = std::process::Command::new(dir.join("gc").join(file))
            .arg("--version")
            .spawn();
    }
    if dir.join("fake-hang").exists() {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
    if dir.join("fake-fail").exists() {
        eprintln!("fake failure");
        return ExitCode::from(1);
    }
    let version = std::fs::read_to_string(dir.join("fake-version.txt"))
        .ok()
        .map(|s| s.trim().to_owned())
        .or_else(|| version_dir(dir))
        .unwrap_or_else(|| "unknown".into());
    if name == "yt-dlp" && args.iter().any(|a| a == "-J") {
        if let Some(i) = args.iter().position(|a| a == "--cookies")
            && let Some(file) = args.get(i + 1)
        {
            let _ = std::fs::copy(file, dir.join("fake-cookies.txt"));
        }
        if let Ok(message) = std::fs::read_to_string(dir.join("fake-probe-error.txt")) {
            eprint!("{message}");
            return ExitCode::from(1);
        }
        let playlist = dir.join("fake-info-playlist.json");
        if args.iter().any(|a| a == "--yes-playlist")
            && let Ok(json) = std::fs::read_to_string(&playlist)
        {
            print!("{json}");
            return ExitCode::SUCCESS;
        }
        if let Ok(json) = std::fs::read_to_string(dir.join("fake-info.json")) {
            print!("{json}");
            return ExitCode::SUCCESS;
        }
    }
    if name == "yt-dlp" && args.iter().any(|a| a == "--progress-template") {
        let marker = dir.join(format!("fake-running-{}", std::process::id()));
        let _ = std::fs::write(&marker, b"");
        let running = std::fs::read_dir(dir).map_or(0, |d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("fake-running-"))
                .count()
        });
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("fake-overlap.txt"))
        {
            use std::io::Write as _;
            let _ = writeln!(f, "{running}");
        }
        let code = fake_download(dir, &args);
        let _ = std::fs::remove_file(&marker);
        return code;
    }
    if name == "yt-dlp" && args.last().is_some_and(|a| a == "--version") {
        println!("{version}");
        return ExitCode::SUCCESS;
    }
    if (name == "ffmpeg" || name == "ffprobe") && args.iter().any(|a| a == "-version") {
        println!("{name} version {version} Copyright (c) the FFmpeg developers");
        println!("built with a fake");
        return ExitCode::SUCCESS;
    }
    eprintln!("fake {name}: unexpected arguments {args:?}");
    ExitCode::from(2)
}

/// The directory below `yt-dlp` or `ffmpeg` among `dir` and its ancestors.
fn version_dir(dir: &Path) -> Option<String> {
    let mut cur = dir;
    loop {
        let parent = cur.parent()?;
        let parent_name = parent.file_name()?.to_str()?;
        if parent_name == "yt-dlp" || parent_name == "ffmpeg" {
            return cur.file_name()?.to_str().map(str::to_owned);
        }
        cur = parent;
    }
}

/// yt-dlp downloading: see the module documentation.
fn fake_download(dir: &Path, args: &[String]) -> ExitCode {
    use std::io::Write as _;
    let value = |opt: &str| {
        args.iter()
            .position(|a| a == opt)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if let Some(file) = value("--cookies") {
        let _ = std::fs::copy(file, dir.join("fake-cookies.txt"));
    }
    if let Ok(message) = std::fs::read_to_string(dir.join("fake-download-error.txt")) {
        eprint!("{message}");
        return ExitCode::from(1);
    }
    let dest = std::path::PathBuf::from(value("-P").unwrap_or_default());
    let name = value("-o")
        .unwrap_or_else(|| "%(title)s [%(id)s].%(ext)s".into())
        .replace("%(title)s", "fake video")
        .replace("%(id)s", "v1")
        .replace("%(ext)s", "mp4");
    let out = dest.join(&name);
    let part = dest.join(format!("{name}.part"));
    let _ = std::fs::write(
        dir.join("fake-resumed.txt"),
        if part.exists() { "yes" } else { "no" },
    );
    let slow = dir.join("fake-slow").exists();
    let mut stdout = std::io::stdout();
    if let Ok(lines) = std::fs::read_to_string(dir.join("fake-progress.txt")) {
        for line in lines.lines() {
            let _ = writeln!(stdout, "{line}");
            let _ = stdout.flush();
            if slow {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    if dir.join("fake-hang-download").exists() {
        let _ = std::fs::write(&part, b"partial");
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
    let _ = std::fs::remove_file(&part);
    if std::fs::write(&out, b"fake media").is_err() {
        eprintln!("ERROR: cannot write {}", out.display());
        return ExitCode::from(1);
    }
    let path = serde_json::to_string(&out.to_string_lossy()).unwrap_or_default();
    let _ = writeln!(stdout, "[boltget] file {path}");
    ExitCode::SUCCESS
}
