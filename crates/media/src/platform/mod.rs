//! OS-specific process containment for sidecars.
//!
//! On Windows a sidecar starts suspended, goes into a Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and only then runs: it and every
//! process it starts end when Boltget closes the job (or exits, or crashes).
//! Elsewhere (post-1.0 platforms) the sidecar gets its own process group and
//! only the direct child is stopped. This module and `windows.rs` are the only
//! places in this crate that may contain `unsafe` (Win32 calls).

use std::io;

#[cfg(windows)]
mod windows;

/// Keeps a started sidecar contained; dropping it ends the sidecar's process
/// tree on Windows.
#[derive(Debug)]
pub(crate) struct Containment {
    #[cfg(windows)]
    job: windows::Job,
}

/// Prepares a command so that [`contain`] can take the process over.
pub(crate) fn prepare(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    cmd.creation_flags(windows::CREATION_FLAGS);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(not(any(windows, unix)))]
    let _ = cmd;
}

/// Contains a child started from a command passed through [`prepare`], and
/// lets it run.
#[cfg(windows)]
pub(crate) fn contain(child: &tokio::process::Child) -> io::Result<Containment> {
    let job = windows::Job::new()?;
    let handle = child
        .raw_handle()
        .ok_or_else(|| io::Error::other("the process has already exited"))?;
    job.assign(handle)?;
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("the process has already exited"))?;
    windows::resume_threads(pid)?;
    Ok(Containment { job })
}

/// Contains a child started from a command passed through [`prepare`], and
/// lets it run.
#[cfg(not(windows))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the Windows version, which can fail"
)]
pub(crate) fn contain(child: &tokio::process::Child) -> io::Result<Containment> {
    let _ = child;
    Ok(Containment {})
}

impl Containment {
    /// Ends the contained processes now (Windows: the whole job).
    #[cfg_attr(
        not(windows),
        expect(
            clippy::unused_self,
            reason = "a no-op where there is no job; the caller kills the child"
        )
    )]
    pub(crate) fn terminate(&self) {
        #[cfg(windows)]
        self.job.terminate();
    }
}
