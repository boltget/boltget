//! The named pipe's security on Windows (docs/ARCHITECTURE.md, "Named pipe
//! hardening"):
//!
//! - its name holds the current user's SID;
//! - it is created with a protected DACL that grants the current user alone
//!   any access, owned by that user, refusing remote clients, and its first
//!   instance must be the first one (so a pipe someone else created first is
//!   noticed instead of joined);
//! - the server drops a client from another logon session;
//! - the client accepts a server only in its own session, and a pipe owned
//!   by its own user (another user could have created the name first).
#![expect(
    unsafe_code,
    reason = "Win32 security and named pipe FFI; every unsafe block states why it is sound"
)]

use std::io;
use std::os::windows::io::AsRawHandle;
use std::ptr;

use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientSessionId, GetNamedPipeServerSessionId};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcessToken,
};

/// Frees memory the system allocated with `LocalAlloc` when dropped.
struct Local(*mut core::ffi::c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a Win32 call documented to allocate
            // with LocalAlloc, and is freed exactly once, here.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// Closes a handle when dropped.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by us and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

/// The current user's `TOKEN_USER`, in a buffer that keeps its SID alive.
struct TokenUserBuf(Vec<u64>);

impl TokenUserBuf {
    fn current() -> io::Result<Self> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no
        // closing; `token` is a valid out pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let mut len = 0u32;
        // SAFETY: asking for the size with an empty buffer is the documented
        // way to size it; the call fails with ERROR_INSUFFICIENT_BUFFER and
        // sets `len`.
        unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &raw mut len) };
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        // u64 elements: TOKEN_USER is pointer-aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: `buf` has at least `len` writable bytes, aligned for
        // TOKEN_USER; `len` is a valid out pointer.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buf.as_mut_ptr().cast(),
                len,
                &raw mut len,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buf))
    }

    fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation(TokenUser) succeeded, so the buffer
        // starts with a TOKEN_USER whose SID points into the same buffer,
        // which lives as long as `self`.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

/// The current user's SID in string form (`S-1-5-21-…`).
pub(crate) fn user_sid() -> io::Result<String> {
    let user = TokenUserBuf::current()?;
    let mut wide: *mut u16 = ptr::null_mut();
    // SAFETY: the SID is valid while `user` lives; `wide` is a valid out
    // pointer and receives a LocalAlloc'd, NUL-terminated string.
    if unsafe { ConvertSidToStringSidW(user.sid(), &raw mut wide) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _free = Local(wide.cast());
    let mut len = 0;
    // SAFETY: `wide` is NUL-terminated, so every index up to the NUL is in
    // bounds, and the slice covers exactly the characters before it.
    let text = unsafe {
        while *wide.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(wide, len)
    };
    Ok(String::from_utf16_lossy(text))
}

/// Creates one instance of the pipe: owned by and open only to the current
/// user, local clients only. `first` makes it fail if the pipe exists.
pub(crate) fn create_pipe(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let sid = user_sid()?;
    // Owner: the user. DACL, protected from inheritance: the user may do
    // anything (GA); nobody else may do anything.
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; `descriptor` is a valid out pointer
    // and receives a LocalAlloc'd descriptor, freed by `_free`.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let _free = Local(descriptor);
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: `attributes` is a valid SECURITY_ATTRIBUTES whose descriptor
    // stays alive for the call; the system copies it into the pipe object.
    unsafe { options.create_with_security_attributes_raw(name, (&raw mut attributes).cast()) }
}

/// This process's logon session.
fn own_session() -> io::Result<u32> {
    let mut session = 0u32;
    // SAFETY: `session` is a valid out pointer.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &raw mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(session)
}

/// Whether the client connected to `pipe` runs in this process's session.
pub(crate) fn client_in_session(pipe: &NamedPipeServer) -> io::Result<bool> {
    let mut session = 0u32;
    // SAFETY: the handle belongs to `pipe`, which is alive and connected;
    // `session` is a valid out pointer.
    let ok = unsafe { GetNamedPipeClientSessionId(pipe.as_raw_handle(), &raw mut session) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(session == own_session()?)
}

/// Checks the server end of `pipe`: in this session, and the pipe owned by
/// the current user.
pub(crate) fn verify_server(pipe: &NamedPipeClient) -> io::Result<()> {
    let handle = pipe.as_raw_handle();
    let mut session = 0u32;
    // SAFETY: the handle belongs to `pipe`, which is alive; `session` is a
    // valid out pointer.
    if unsafe { GetNamedPipeServerSessionId(handle, &raw mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if session != own_session()? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pipe is served from another logon session",
        ));
    }
    let mut owner: PSID = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: the handle is alive; the out pointers are valid. The owner SID
    // points into `descriptor`, which is LocalAlloc'd and freed by `_free`
    // after the last use of `owner`.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status.cast_signed()));
    }
    let _free = Local(descriptor);
    let user = TokenUserBuf::current()?;
    // SAFETY: both SIDs are valid: `owner` while `descriptor` lives, the
    // user's while `user` lives.
    if owner.is_null() || unsafe { EqualSid(owner, user.sid()) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pipe belongs to another user",
        ));
    }
    Ok(())
}
