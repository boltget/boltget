//! Win32 Job Objects for sidecars. Each `unsafe` block is a Win32 call with
//! handles owned by this module (or by a live child process) and properly
//! sized, initialised structures; see the SAFETY comments.
#![allow(unsafe_code)] // Justified: FFI to the Win32 job and thread APIs.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};

/// Process creation flags: start suspended (so the process cannot start
/// children before it is in the job) and without a console window.
pub(crate) const CREATION_FLAGS: u32 = CREATE_SUSPENDED | CREATE_NO_WINDOW;

/// A Job Object that kills every process in it when it is closed, so a
/// sidecar (and anything it starts) cannot outlive Boltget.
#[derive(Debug)]
pub(crate) struct Job(OwnedHandle);

impl Job {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: null security attributes and a null name are documented as
        // valid (an unnamed job with a default security descriptor).
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a fresh handle we own and nothing else will close.
        let job = Job(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        // SAFETY: the job handle is valid; `info` is an initialised structure of
        // the class named, and the size passed is its size.
        let ok = unsafe {
            SetInformationJobObject(
                job.handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    fn handle(&self) -> HANDLE {
        self.0.as_raw_handle()
    }

    /// Puts a process into the job.
    pub(crate) fn assign(&self, process: RawHandle) -> io::Result<()> {
        // SAFETY: both handles are valid: the job is ours, `process` belongs to
        // a child that has not been waited for.
        let ok = unsafe { AssignProcessToJobObject(self.handle(), process) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Ends every process in the job now.
    pub(crate) fn terminate(&self) {
        // SAFETY: the job handle is valid for `self`'s lifetime.
        unsafe {
            TerminateJobObject(self.handle(), 1);
        }
    }
}

/// Resumes the threads of a process created with `CREATE_SUSPENDED` (std
/// does not hand out its main thread's handle). Returns how many it resumed.
pub(crate) fn resume_threads(pid: u32) -> io::Result<usize> {
    // SAFETY: plain call; the result is checked before use.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `snap` is a fresh valid handle we own.
    let snap = unsafe { OwnedHandle::from_raw_handle(snap) };
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = 0;
    // SAFETY: valid snapshot handle; `entry` is initialised with its size.
    let mut more = unsafe { Thread32First(snap.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: plain call; the result is checked before use.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                // SAFETY: `thread` is a fresh valid handle we own.
                let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
                // SAFETY: valid thread handle opened with THREAD_SUSPEND_RESUME.
                if unsafe { ResumeThread(thread.as_raw_handle()) } != u32::MAX {
                    resumed += 1;
                }
            }
        }
        // SAFETY: as for Thread32First.
        more = unsafe { Thread32Next(snap.as_raw_handle(), &raw mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other(
            "no thread of the process could be resumed",
        ));
    }
    Ok(resumed)
}
