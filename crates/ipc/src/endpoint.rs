//! Where an owner listens: one endpoint per user and data directory.

use std::io;
use std::path::{Path, PathBuf};

/// The address of the owner of one data directory.
///
/// On Windows a named pipe, `\\.\pipe\boltget-<user SID>-<key>`; elsewhere a
/// Unix domain socket, `boltget-<key>.sock` in `$XDG_RUNTIME_DIR` (or in the
/// data directory when that is unset). `<key>` is a hash of the data
/// directory, so the app and a CLI given `--state` elsewhere never meet, and
/// a client always reaches the process whose database it would open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    #[cfg(windows)]
    name: String,
    #[cfg(not(windows))]
    path: PathBuf,
}

impl Endpoint {
    /// The endpoint for the database in `dir`.
    pub fn for_data_dir(dir: &Path) -> io::Result<Self> {
        let key = dir_key(dir);
        #[cfg(windows)]
        {
            let sid = crate::platform::user_sid()?;
            Ok(Self {
                name: format!(r"\\.\pipe\boltget-{sid}-{key}"),
            })
        }
        #[cfg(not(windows))]
        {
            let base = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .filter(|d| d.is_absolute() && d.is_dir())
                .unwrap_or_else(|| absolute(dir));
            Ok(Self {
                path: base.join(format!("boltget-{key}.sock")),
            })
        }
    }

    /// The pipe name or socket path, for messages.
    pub fn display(&self) -> String {
        #[cfg(windows)]
        {
            self.name.clone()
        }
        #[cfg(not(windows))]
        {
            self.path.display().to_string()
        }
    }

    #[cfg(windows)]
    pub(crate) fn pipe_name(&self) -> &str {
        &self.name
    }

    #[cfg(not(windows))]
    pub(crate) fn socket_path(&self) -> &Path {
        &self.path
    }
}

fn absolute(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir)
        .or_else(|_| std::path::absolute(dir))
        .unwrap_or_else(|_| dir.to_path_buf())
}

/// 16 hex digits identifying a data directory: FNV-1a over its canonical
/// path (case-folded on Windows, whose file names are case-insensitive).
/// Stable across builds and platforms, unlike `std`'s hashers.
pub(crate) fn dir_key(dir: &Path) -> String {
    let path = absolute(dir).to_string_lossy().into_owned();
    let path = if cfg!(windows) {
        path.to_lowercase()
    } else {
        path
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_key_per_directory() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(dir_key(a.path()), dir_key(a.path()));
        assert_ne!(dir_key(a.path()), dir_key(b.path()));
        assert_eq!(dir_key(a.path()).len(), 16);
        // Another spelling of the same directory.
        let dotted = a.path().join("x").join("..");
        std::fs::create_dir(a.path().join("x")).unwrap();
        assert_eq!(dir_key(&dotted), dir_key(a.path()));
        #[cfg(windows)]
        {
            let upper = PathBuf::from(a.path().to_string_lossy().to_uppercase());
            assert_eq!(dir_key(&upper), dir_key(a.path()));
        }
    }

    #[test]
    fn endpoints_differ_per_directory() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ea = Endpoint::for_data_dir(a.path()).unwrap();
        assert_eq!(ea, Endpoint::for_data_dir(a.path()).unwrap());
        assert_ne!(ea, Endpoint::for_data_dir(b.path()).unwrap());
        #[cfg(windows)]
        assert!(
            ea.display().starts_with(r"\\.\pipe\boltget-S-1-"),
            "{}",
            ea.display()
        );
    }
}
