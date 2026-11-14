//! Starting the app when the browser has a download for it and it is not
//! running (docs/ARCHITECTURE.md, "The native messaging host").

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Names the app to start instead of the one next to the host (tests, and
/// development setups where the two are built apart).
pub const APP_ENV: &str = "BOLTGET_NATIVE_HOST_APP";

/// The app's file names next to the host, in order: the installed name,
/// then the development build's.
#[cfg(windows)]
const APP_NAMES: [&str; 2] = ["Boltget.exe", "boltget-desktop.exe"];
#[cfg(not(windows))]
const APP_NAMES: [&str; 2] = ["boltget-desktop", "Boltget"];

/// The app to start: [`APP_ENV`] if set, else the first of the app's names
/// that exists in `dir` (the host's own folder, which is the install
/// folder). `None` when there is none.
///
/// Names match exactly, case included: Windows would otherwise take the
/// command-line tool, `boltget.exe`, for `Boltget.exe` in a build folder
/// where the two sit side by side (`target/debug`), and start that.
pub fn find_app(dir: Option<&Path>, env: Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = env.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let dir = dir?;
    let names: Vec<OsString> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.file_name()))
        .collect();
    APP_NAMES
        .iter()
        .filter(|name| {
            names
                .iter()
                .any(|n| n.as_os_str() == std::ffi::OsStr::new(name))
        })
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

/// Starts `app` in the tray (`--background`), detached from the host: the
/// browser ends the host when the port closes, and the app must outlive
/// it. Its standard streams are closed, since the host's stdout belongs to
/// the browser.
pub fn start(app: &Path) -> io::Result<()> {
    let mut cmd = Command::new(app);
    cmd.arg("--background")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = app.parent() {
        cmd.current_dir(dir);
    }
    spawn_detached(&mut cmd)
}

#[cfg(windows)]
fn spawn_detached(cmd: &mut Command) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    // Browsers may run hosts in a job object that kills its processes when
    // the host ends (Firefox does); breaking away keeps the app alive.
    // A job that forbids breaking away refuses the flag: then the app starts
    // inside the job, which is still better than not at all.
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const ERROR_ACCESS_DENIED: i32 = 5;

    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    match cmd.creation_flags(base | CREATE_BREAKAWAY_FROM_JOB).spawn() {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
            cmd.creation_flags(base).spawn().map(drop)
        }
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
fn spawn_detached(cmd: &mut Command) -> io::Result<()> {
    use std::os::unix::process::CommandExt;

    // Its own process group, so a signal to the browser's group spares it.
    cmd.process_group(0).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_wins_then_the_installed_name() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_app(Some(dir.path()), None), None);
        assert_eq!(find_app(None, None), None);
        std::fs::write(dir.path().join(APP_NAMES[1]), b"").unwrap();
        assert_eq!(
            find_app(Some(dir.path()), None),
            Some(dir.path().join(APP_NAMES[1]))
        );
        std::fs::write(dir.path().join(APP_NAMES[0]), b"").unwrap();
        assert_eq!(
            find_app(Some(dir.path()), None),
            Some(dir.path().join(APP_NAMES[0]))
        );
        assert_eq!(
            find_app(Some(dir.path()), Some("other-app".into())),
            Some(PathBuf::from("other-app"))
        );
        assert_eq!(
            find_app(Some(dir.path()), Some(OsString::new())),
            Some(dir.path().join(APP_NAMES[0])),
            "an empty variable is unset"
        );
    }

    #[test]
    fn the_command_line_tool_is_not_the_app() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("boltget.exe"), b"").unwrap();
        std::fs::write(dir.path().join("boltget"), b"").unwrap();
        assert_eq!(find_app(Some(dir.path()), None), None);
        std::fs::write(dir.path().join(APP_NAMES[1]), b"").unwrap();
        assert_eq!(
            find_app(Some(dir.path()), None),
            Some(dir.path().join(APP_NAMES[1]))
        );
    }

    #[test]
    fn a_missing_app_fails_to_start() {
        let dir = tempfile::tempdir().unwrap();
        assert!(start(&dir.path().join("no-such-app")).is_err());
    }
}
