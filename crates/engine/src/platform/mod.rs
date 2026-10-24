//! OS-specific file operations: sparse preallocation, positional writes,
//! file system and free-space queries, Mark of the Web.
//!
//! This is the only place in the engine that may contain `unsafe` (Win32 calls).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(windows)]
mod windows;

/// How the output file was prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Preallocation {
    /// Marked sparse, then extended (NTFS, ReFS): no zero-fill stalls.
    Sparse,
    /// Extended with `set_len` only. On Unix this is sparse by default; on
    /// Windows it means the file system refused `FSCTL_SET_SPARSE` (FAT, exFAT, SMB).
    SetLen,
    /// Size unknown: the file grows as data arrives.
    None,
}

/// Creates (truncating) the partial file and preallocates `size` bytes.
pub(crate) fn create_preallocated(
    path: &Path,
    size: Option<u64>,
) -> io::Result<(File, Preallocation)> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    let Some(size) = size else {
        return Ok((file, Preallocation::None));
    };
    let mode = if size > 0 && set_sparse(&file).is_ok() {
        Preallocation::Sparse
    } else {
        Preallocation::SetLen
    };
    file.set_len(size)?;
    Ok((file, mode))
}

/// Marks the file sparse. A no-op success on Unix, where `ftruncate` already
/// creates sparse files on the common file systems.
pub(crate) fn set_sparse(file: &File) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::set_sparse(file)
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        Ok(())
    }
}

/// Writes all of `buf` at `offset` without using a shared file cursor.
pub(crate) fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let (mut buf, mut offset) = (buf, offset);
        while !buf.is_empty() {
            match file.seek_write(buf, offset) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
                Ok(n) => {
                    buf = &buf[n..];
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(buf, offset)
    }
}

/// The name of the file system holding `file` ("NTFS", "FAT32", "exFAT", ...), if known.
pub(crate) fn filesystem_name(file: &File) -> Option<String> {
    #[cfg(windows)]
    {
        windows::filesystem_name(file)
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        None
    }
}

/// Bytes available to the current user in `dir`, if the OS can tell us.
pub(crate) fn available_space(dir: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        windows::available_space(dir)
    }
    #[cfg(not(windows))]
    {
        // Unix support is post-1.0; without a libc dependency we skip the check.
        let _ = dir;
        None
    }
}

/// Whether the file has the sparse attribute (always false off Windows).
pub fn is_sparse(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
        std::fs::metadata(path).is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Writes the `Zone.Identifier` alternate data stream (Mark of the Web), as
/// browsers do, so `SmartScreen` and Office Protected View keep working.
/// Silently does nothing where alternate data streams are unsupported.
pub(crate) fn mark_of_the_web(path: &Path, host_url: &str, referrer: Option<&str>) {
    #[cfg(windows)]
    {
        let mut content = String::from("[ZoneTransfer]\r\nZoneId=3\r\n");
        if let Some(r) = referrer {
            content.push_str("ReferrerUrl=");
            content.push_str(r);
            content.push_str("\r\n");
        }
        content.push_str("HostUrl=");
        content.push_str(host_url);
        content.push_str("\r\n");
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        if let Err(e) = std::fs::write(&ads, content) {
            tracing::debug!(error = %e, "could not write Zone.Identifier");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, host_url, referrer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn positional_writes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        let (f, _) = create_preallocated(&p, Some(10)).unwrap();
        write_all_at(&f, b"world", 5).unwrap();
        write_all_at(&f, b"hello", 0).unwrap();
        drop(f);
        assert_eq!(std::fs::read(&p).unwrap(), b"helloworld");
    }

    #[test]
    fn unknown_size_grows() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.bin");
        let (f, mode) = create_preallocated(&p, None).unwrap();
        assert_eq!(mode, Preallocation::None);
        write_all_at(&f, b"abc", 3).unwrap();
        drop(f);
        assert_eq!(std::fs::read(&p).unwrap(), b"\0\0\0abc");
    }

    /// The scenario from `docs/ENGINE.md`: the last segment writes first, far
    /// beyond the valid data length. On a sparse file this must not zero-fill.
    #[test]
    fn write_at_end_of_large_sparse_file_is_fast() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.bin");
        let size = 8u64 * 1024 * 1024 * 1024; // 8 GiB, nothing is allocated
        let (f, mode) = create_preallocated(&p, Some(size)).unwrap();
        let t = Instant::now();
        write_all_at(&f, &[1u8; 4096], size - 4096).unwrap();
        f.sync_data().unwrap();
        let elapsed = t.elapsed();
        assert_eq!(std::fs::metadata(&p).unwrap().len(), size);
        if cfg!(windows) {
            let fs = filesystem_name(&f).unwrap_or_default();
            if fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS") {
                assert_eq!(mode, Preallocation::Sparse);
                assert!(is_sparse(&p));
            }
        }
        if mode == Preallocation::Sparse || cfg!(unix) {
            assert!(
                elapsed < Duration::from_secs(5),
                "zero-fill stall: {elapsed:?}"
            );
        }
    }

    #[test]
    fn volume_queries_do_not_fail() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("q.bin");
        let (f, _) = create_preallocated(&p, Some(1)).unwrap();
        if cfg!(windows) {
            assert!(filesystem_name(&f).is_some());
            assert!(available_space(dir.path()).is_some_and(|n| n > 0));
        }
    }
}
