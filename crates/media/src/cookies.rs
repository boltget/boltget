//! Cookies for yt-dlp: a temporary Netscape `cookies.txt` per job.
//!
//! yt-dlp reads cookies only from a file (`--cookies`); Boltget never lets it
//! read browser profiles. The file holds secrets, so it lives only as long as
//! the job: it is created in the tools directory's `.jobs` folder, which is
//! inside the user's local application data (on Windows that folder's
//! inherited permissions admit only the user, `SYSTEM` and administrators;
//! elsewhere the file itself is made readable by its owner only), and the
//! [`CookieFile`] guard deletes it when dropped, whether the job succeeded or
//! not. Files a crashed process left behind are deleted the next time a
//! cookie file is created. See `docs/VIDEO.md`, "Probing and presets: as
//! built".

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use boltget_engine::CookieJar;

use crate::{Error, Result};

/// The folder for per-job files, inside the tools directory.
pub(crate) const JOBS_DIR: &str = ".jobs";

/// Cookie files older than this are left over from a process that died.
const STALE: Duration = Duration::from_secs(3600);

const PREFIX: &str = "cookies-";

/// A temporary `cookies.txt`, deleted when this is dropped.
#[derive(Debug)]
pub struct CookieFile {
    path: PathBuf,
}

impl CookieFile {
    /// Writes `jar` (cookies that have not expired) into a new file in `dir`,
    /// creating `dir` if needed.
    pub fn create(dir: &Path, jar: &CookieJar) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(dir).map_err(|e| Error::io(format!("creating {}", dir.display()), e))?;
        sweep(dir);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let name = format!(
            "{PREFIX}{}-{}-{nanos:08x}.txt",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = dir.join(name);
        let mut file = open_private(&path)
            .map_err(|e| Error::io(format!("creating {}", path.display()), e))?;
        // From here on the guard deletes the file, also when writing fails.
        let guard = CookieFile { path };
        file.write_all(jar.to_netscape(SystemTime::now()).as_bytes())
            .and_then(|()| file.flush())
            .map_err(|e| Error::io(format!("writing {}", guard.path.display()), e))?;
        Ok(guard)
    }

    /// The file's path, for `--cookies`.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CookieFile {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), error = %e, "could not delete a cookie file");
        }
    }
}

/// A new file only the current user can read (Unix: mode 0600; Windows: the
/// folder's inherited permissions, see the module documentation).
fn open_private(path: &Path) -> std::io::Result<fs::File> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// Deletes cookie files older than [`STALE`] in `dir`.
fn sweep(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let stale = entry.file_name().to_string_lossy().starts_with(PREFIX)
            && entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > STALE));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boltget_engine::Cookie;

    #[test]
    fn written_for_yt_dlp_and_deleted_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = dir.path().join(JOBS_DIR);
        let mut jar = CookieJar::new();
        jar.insert(Cookie::from_browser(
            "SID",
            "s3cr3t",
            ".video.test",
            false,
            "/",
            true,
            None,
        ));
        jar.insert(
            Cookie::from_browser(
                "pref",
                "a b=c",
                "www.video.test",
                true,
                "/watch",
                false,
                Some(SystemTime::UNIX_EPOCH + Duration::from_secs(4_102_444_800)),
            )
            .with_http_only(true),
        );
        let file = CookieFile::create(&jobs, &jar).unwrap();
        let path = file.path().to_path_buf();
        assert!(path.starts_with(&jobs));
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# Netscape HTTP Cookie File\n\
             .video.test\tTRUE\t/\tTRUE\t0\tSID\ts3cr3t\n\
             #HttpOnly_www.video.test\tFALSE\t/watch\tFALSE\t4102444800\tpref\ta b=c\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Two jobs at once get two files.
        let other = CookieFile::create(&jobs, &jar).unwrap();
        assert_ne!(other.path(), path);
        drop(file);
        assert!(!path.exists());
        drop(other);
        assert_eq!(fs::read_dir(&jobs).unwrap().count(), 0);
    }

    #[test]
    fn leftovers_of_a_dead_process_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("cookies-1-0-0.txt");
        let unrelated = dir.path().join("keep.txt");
        fs::write(&old, "x").unwrap();
        fs::write(&unrelated, "x").unwrap();
        let long_ago = SystemTime::now() - Duration::from_secs(2 * 3600);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let fresh = CookieFile::create(dir.path(), &CookieJar::new()).unwrap();
        assert!(!old.exists());
        assert!(unrelated.exists());
        assert!(fresh.path().exists());
    }
}
