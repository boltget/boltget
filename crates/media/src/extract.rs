//! Unpacking a verified download into a staging directory.
//!
//! Zip entries are unpacked only under the target directory: names with `..`,
//! absolute paths, drive letters or other escapes are refused (as is the whole
//! archive), symbolic links are refused, and the unpacked size is bounded.

use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};

use crate::sources::Layout;

/// The most an archive may unpack to.
const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;
/// The most entries an archive may have.
const MAX_ENTRIES: usize = 20_000;

/// Puts `download` into `dir` (which exists and is empty) as `layout` says.
pub(crate) fn unpack(download: &Path, layout: &Layout, dir: &Path) -> Result<(), String> {
    match layout {
        Layout::Executable { file } => {
            let name = single_component(file).ok_or("bad executable name")?;
            let dest = dir.join(name);
            fs::copy(download, &dest).map_err(|e| format!("copy: {e}"))?;
            make_executable(&dest).map_err(|e| format!("set executable: {e}"))?;
            Ok(())
        }
        Layout::Zip { strip, keep } => unzip(download, *strip, keep.as_deref(), dir),
    }
}

fn single_component(name: &str) -> Option<&str> {
    let mut c = Path::new(name).components();
    match (c.next(), c.next()) {
        (Some(Component::Normal(_)), None) => Some(name),
        _ => None,
    }
}

fn unzip(archive: &Path, strip: usize, keep: Option<&str>, dir: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("open: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a zip archive: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{} entries (at most {MAX_ENTRIES})", zip.len()));
    }
    let mut total = 0u64;
    let mut unpacked = 0usize;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("entry {i}: {e}"))?;
        let name = entry
            .name()
            .map_err(|e| format!("entry {i}: {e}"))?
            .into_owned();
        if entry.is_symlink() {
            return Err(format!("{name:?} is a symbolic link"));
        }
        let Some(relative) = relative_path(&name, strip)? else {
            continue;
        };
        if let Some(keep) = keep {
            let keep = Path::new(keep.trim_end_matches('/'));
            if !relative.starts_with(keep) || relative == keep {
                continue;
            }
        }
        let dest = dir.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&dest).map_err(|e| format!("{name}: {e}"))?;
            continue;
        }
        let size = entry.size();
        total = total.saturating_add(size);
        if total > MAX_UNPACKED {
            return Err("unpacks to more than 2 GiB".into());
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{name}: {e}"))?;
        }
        let mut out = File::create(&dest).map_err(|e| format!("{name}: {e}"))?;
        // Bounded by the declared size: a lying header cannot unpack more.
        let written =
            io::copy(&mut (&mut entry).take(size), &mut out).map_err(|e| format!("{name}: {e}"))?;
        if written != size {
            return Err(format!("{name}: truncated entry"));
        }
        drop(out);
        if entry.unix_mode().is_some_and(|m| m & 0o111 != 0) {
            make_executable(&dest).map_err(|e| format!("{name}: {e}"))?;
        }
        unpacked += 1;
    }
    if unpacked == 0 {
        return Err("nothing to unpack".into());
    }
    Ok(())
}

/// The entry's path inside the target after dropping `strip` components;
/// `Ok(None)` for entries that disappear entirely (the stripped directories
/// themselves). Refuses any name that could escape the target.
fn relative_path(name: &str, strip: usize) -> Result<Option<PathBuf>, String> {
    if name.contains(':') || name.contains('\0') {
        return Err(format!("unsafe entry name {name:?}"));
    }
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(format!("absolute entry name {name:?}"));
    }
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(format!("entry name {name:?} leaves the archive")),
            p => parts.push(p),
        }
    }
    if parts.len() <= strip {
        return Ok(None);
    }
    Ok(Some(parts[strip..].iter().collect()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = fs::metadata(path)?.permissions();
    perm.set_mode(perm.mode() | 0o755);
    fs::set_permissions(path, perm)
}

#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the Unix version, which can fail"
)]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names() {
        assert_eq!(
            relative_path("ffmpeg-n9/bin/ffmpeg.exe", 1).unwrap(),
            Some(PathBuf::from("bin").join("ffmpeg.exe"))
        );
        assert_eq!(relative_path("ffmpeg-n9/", 1).unwrap(), None);
        assert_eq!(
            relative_path("_internal/x.pyd", 0).unwrap(),
            Some(PathBuf::from("_internal").join("x.pyd"))
        );
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "C:/x",
            "a\\..\\x",
            "a\0b",
        ] {
            assert!(relative_path(bad, 0).is_err(), "{bad}");
        }
    }

    #[test]
    fn executable_names() {
        assert!(single_component("yt-dlp").is_some());
        assert!(single_component("../yt-dlp").is_none());
        assert!(single_component("a/yt-dlp").is_none());
    }
}
