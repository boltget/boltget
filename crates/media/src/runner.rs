//! Starting sidecars: explicit argument vectors, allowlisted options, a fixed
//! working directory, a reduced environment, a Job Object and a timeout.
//!
//! There is no shell anywhere: the program is started directly with
//! [`tokio::process::Command`], and on Windows only `.exe` files are started
//! (the standard library would hand `.bat` and `.cmd` files to `cmd.exe`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

use crate::platform::{self, Containment};
use crate::{Error, Program, Result, args};

/// How long a sidecar may run unless the caller says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Output kept from each stream by [`Invocation::run`] (the rest is read and
/// dropped, so the sidecar never blocks on a full pipe).
pub const DEFAULT_MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// Environment variables passed through to sidecars; every other one is
/// dropped (so no `PYTHONPATH`, no proxy or configuration variables that
/// Boltget did not choose, no `FFREPORT`).
const KEPT_ENV: &[&str] = &[
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "TEMP",
    "TMP",
    "TMPDIR",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "HOME",
    "LANG",
    "LC_ALL",
    "TZ",
];

/// A checked sidecar command line, ready to run.
#[derive(Debug, Clone)]
pub struct Invocation {
    program: Program,
    exe: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    timeout: Duration,
    max_output: usize,
}

impl Invocation {
    /// yt-dlp with `args` (checked against the allowlist), after
    /// [`args::YT_DLP_PREFIX`] and, if given, `--ffmpeg-location <dir>`.
    pub fn yt_dlp(exe: &Path, ffmpeg_dir: Option<&Path>, user_args: &[String]) -> Result<Self> {
        args::check_yt_dlp(user_args)?;
        let mut argv: Vec<String> = args::YT_DLP_PREFIX
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        if let Some(dir) = ffmpeg_dir {
            let dir = dir.to_str().ok_or_else(|| Error::Argument {
                program: Program::YtDlp.name(),
                detail: "the FFmpeg directory is not valid Unicode".into(),
            })?;
            argv.push("--ffmpeg-location".into());
            argv.push(dir.to_owned());
        }
        argv.extend(user_args.iter().cloned());
        Self::new(Program::YtDlp, exe, argv)
    }

    /// ffmpeg or ffprobe with `args` (checked against the allowlist). Every
    /// input is restricted to local files (`-protocol_whitelist file` before
    /// each `-i`), so a playlist or concat file cannot reach the network.
    pub fn ffmpeg(program: Program, exe: &Path, user_args: &[String]) -> Result<Self> {
        if program == Program::YtDlp {
            return Err(Error::Argument {
                program: program.name(),
                detail: "not an FFmpeg program".into(),
            });
        }
        args::check_ffmpeg(program, user_args)?;
        let mut argv = Vec::with_capacity(user_args.len() + 2);
        for a in user_args {
            if a == "-i" {
                argv.push("-protocol_whitelist".to_owned());
                argv.push("file".to_owned());
            }
            argv.push(a.clone());
        }
        Self::new(program, exe, argv)
    }

    fn new(program: Program, exe: &Path, argv: Vec<String>) -> Result<Self> {
        let refuse = |detail: String| Error::Argument {
            program: program.name(),
            detail,
        };
        if !exe.is_absolute() {
            return Err(refuse(format!("{} is not an absolute path", exe.display())));
        }
        if cfg!(windows)
            && !exe
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            return Err(refuse(format!("{} is not an .exe", exe.display())));
        }
        let cwd = exe
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Ok(Invocation {
            program,
            exe: exe.to_path_buf(),
            args: argv,
            cwd,
            timeout: DEFAULT_TIMEOUT,
            max_output: DEFAULT_MAX_OUTPUT,
        })
    }

    /// Runs in `dir` instead of the program's own directory.
    #[must_use]
    pub fn current_dir(mut self, dir: &Path) -> Self {
        dir.clone_into(&mut self.cwd);
        self
    }

    /// Stops the sidecar after `timeout`.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Keeps at most `bytes` of each output stream in [`Invocation::run`].
    #[must_use]
    pub fn max_output(mut self, bytes: usize) -> Self {
        self.max_output = bytes;
        self
    }

    /// The full argument vector that will be passed (after the program).
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The program.
    pub fn program(&self) -> Program {
        self.program
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.args(&self.args)
            .current_dir(&self.cwd)
            .env_clear()
            .envs(kept_env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        platform::prepare(&mut cmd);
        cmd
    }

    /// Starts the sidecar with piped stdout and stderr. It is stopped when the
    /// returned [`Running`] is dropped.
    pub fn spawn(&self) -> Result<Running> {
        let mut child = self
            .command()
            .spawn()
            .map_err(|e| Error::io(format!("cannot start {}", self.exe.display()), e))?;
        match platform::contain(&child) {
            Ok(containment) => Ok(Running {
                child,
                containment,
                program: self.program,
                timeout: self.timeout,
            }),
            Err(e) => {
                let _ = child.start_kill();
                Err(Error::io(
                    format!("cannot contain {} in a job object", self.program),
                    e,
                ))
            }
        }
    }

    /// Runs the sidecar to completion (or its timeout) and collects its output.
    pub async fn run(&self) -> Result<Output> {
        let mut running = self.spawn()?;
        let stdout = running.child.stdout.take();
        let stderr = running.child.stderr.take();
        let max = self.max_output;
        let work = async {
            let (out, err, status) = tokio::join!(
                collect(stdout, max),
                collect(stderr, max),
                running.child.wait()
            );
            status.map(|status| Output {
                status,
                stdout: out,
                stderr: err,
            })
        };
        match tokio::time::timeout(self.timeout, work).await {
            Ok(Ok(out)) => Ok(out),
            Ok(Err(e)) => Err(Error::io(format!("waiting for {}", self.program), e)),
            Err(_) => {
                running.stop();
                Err(Error::Timeout {
                    program: self.program.name().to_owned(),
                    timeout: self.timeout,
                })
            }
        }
    }
}

/// The environment passed to sidecars: [`KEPT_ENV`] only.
fn kept_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(k, _)| {
            k.to_str()
                .is_some_and(|k| KEPT_ENV.iter().any(|kept| kept.eq_ignore_ascii_case(k)))
        })
        .collect()
}

async fn collect(stream: Option<impl AsyncRead + Unpin>, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(mut stream) = stream else {
        return out;
    };
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = max.saturating_sub(out.len());
                out.extend_from_slice(&buf[..n.min(room)]);
            }
        }
    }
    out
}

/// What a finished sidecar produced.
#[derive(Debug)]
pub struct Output {
    /// How it exited.
    pub status: ExitStatus,
    /// Its standard output (up to the limit).
    pub stdout: Vec<u8>,
    /// Its standard error (up to the limit).
    pub stderr: Vec<u8>,
}

impl Output {
    /// Standard output as text (lossy UTF-8).
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Standard error as text (lossy UTF-8).
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A started sidecar. Dropping it stops the sidecar and everything it
/// started (on Windows, by closing its Job Object).
#[derive(Debug)]
pub struct Running {
    child: Child,
    containment: Containment,
    program: Program,
    timeout: Duration,
}

impl Running {
    /// The child process, for its stdout and stderr pipes and to wait for it.
    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    /// The program.
    pub fn program(&self) -> Program {
        self.program
    }

    /// The timeout the invocation asked for (enforced by
    /// [`Invocation::run`]; a caller streaming output enforces it itself).
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Stops the sidecar and everything it started, now.
    pub fn stop(&mut self) {
        self.containment.terminate();
        let _ = self.child.start_kill();
    }
}
