//! Win32 calls. Each `unsafe` block passes a valid handle owned by a live `File`
//! (or a NUL-terminated wide string we own) and buffers whose sizes we pass in.
#![allow(unsafe_code)] // Justified: FFI to Win32 file APIs; see the SAFETY comments.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeInformationByHandleW};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;

pub(super) fn set_sparse(file: &File) -> io::Result<()> {
    let handle = file.as_raw_handle();
    let mut returned = 0u32;
    // SAFETY: `handle` is a valid open file handle for the lifetime of `file`.
    // FSCTL_SET_SPARSE with no input buffer means "set sparse = TRUE"; no output
    // buffer is used; `returned` is a valid out pointer; no OVERLAPPED (synchronous).
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_SPARSE,
            ptr::null(),
            0,
            ptr::null_mut(),
            0,
            &raw mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn filesystem_name(file: &File) -> Option<String> {
    let handle = file.as_raw_handle();
    let mut name = [0u16; 64];
    // SAFETY: valid handle as above; `name` is a writable buffer whose length in
    // u16 units we pass; all other out-parameters are optional and passed as null.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            handle,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    Some(String::from_utf16_lossy(&name[..len]))
}

pub(super) fn available_space(dir: &Path) -> Option<u64> {
    let wide: Vec<u16> = OsStr::new(dir).encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; `free` is a valid out
    // pointer; the two other out-parameters are optional and passed as null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &raw mut free,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}
