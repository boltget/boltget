//! A stand-in for yt-dlp, ffmpeg and ffprobe in the media crate's tests. Never
//! shipped. It answers `--version` / `-version` like the real programs, with
//! the version taken from `fake-version.txt` next to it or, failing that, from
//! the name of its version directory (`tools/<tool>/<version>/...`). It writes
//! its arguments to `fake-args.txt` and its environment's names to
//! `fake-env.txt`, fails if `fake-fail` exists next to it, and
//! hangs if `fake-hang` does. With `fake-spawn` it first starts `gc/<its own
//! name>`. As yt-dlp with `-J` it copies the `--cookies` file (if any) to
//! `fake-cookies.txt`, then prints `fake-probe-error.txt` to stderr and
//! fails if that exists, or prints `fake-info.json`.
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
        if let Ok(json) = std::fs::read_to_string(dir.join("fake-info.json")) {
            print!("{json}");
            return ExitCode::SUCCESS;
        }
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
