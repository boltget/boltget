//! `tools/manifest.json`: which versions are installed and which one is
//! active, written atomically (a new file renamed over the old one), so the
//! "active" pointer switches in one step.

use std::collections::BTreeMap;
use std::fs::{self, File, TryLockError};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sources::Channel;
use crate::{Error, Result, Tool};

/// The manifest's file name inside the tools directory.
pub(crate) const MANIFEST: &str = "manifest.json";
/// The lock file held while installing.
pub(crate) const LOCK: &str = ".lock";
/// The current manifest format.
const FORMAT: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) format: u32,
    #[serde(default)]
    pub(crate) tools: BTreeMap<String, ToolState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ToolState {
    /// The version in use.
    #[serde(default)]
    pub(crate) active: Option<String>,
    /// The version before it, kept for rollback.
    #[serde(default)]
    pub(crate) previous: Option<String>,
    /// yt-dlp's release channel.
    #[serde(default)]
    pub(crate) channel: Channel,
    /// The last successful update check (seconds since the Unix epoch).
    #[serde(default)]
    pub(crate) last_check: Option<u64>,
    /// The last check caused by an extraction failure.
    #[serde(default)]
    pub(crate) last_failure_check: Option<u64>,
    /// Versions rolled back from: never installed again automatically.
    #[serde(default)]
    pub(crate) rejected: Vec<String>,
    /// Installed versions.
    #[serde(default)]
    pub(crate) versions: BTreeMap<String, Installed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Installed {
    /// Where it was downloaded from.
    pub(crate) url: String,
    /// The download's SHA-256.
    pub(crate) sha256: String,
    /// The download's size.
    pub(crate) size: u64,
    /// When it was installed (seconds since the Unix epoch).
    pub(crate) added_at: u64,
    /// Program name to path relative to the version directory.
    pub(crate) programs: BTreeMap<String, String>,
}

impl Manifest {
    pub(crate) fn tool(&self, tool: Tool) -> ToolState {
        self.tools.get(tool.id()).cloned().unwrap_or_default()
    }

    pub(crate) fn tool_mut(&mut self, tool: Tool) -> &mut ToolState {
        self.tools.entry(tool.id().to_owned()).or_default()
    }

    /// Reads the manifest in `dir`; a missing one is empty. One that cannot be
    /// parsed is moved aside (`manifest.json.bad`) and treated as empty: the
    /// tools are then installed again rather than never.
    pub(crate) fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(MANIFEST);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Manifest::fresh()),
            Err(e) => return Err(Error::io(format!("reading {}", path.display()), e)),
        };
        match serde_json::from_slice::<Manifest>(&bytes) {
            Ok(m) if m.format == FORMAT => Ok(m),
            Ok(m) => Err(Error::Manifest {
                path,
                detail: format!("format {} is newer than this Boltget understands", m.format),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "tools manifest unreadable; starting over");
                let _ = fs::rename(&path, dir.join(format!("{MANIFEST}.bad")));
                Ok(Manifest::fresh())
            }
        }
    }

    fn fresh() -> Self {
        Manifest {
            format: FORMAT,
            tools: BTreeMap::new(),
        }
    }

    /// Writes the manifest: to a temporary file, flushed to disk, then renamed
    /// over the old one.
    pub(crate) fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(MANIFEST);
        let tmp = dir.join(format!("{MANIFEST}.tmp"));
        let json = serde_json::to_vec_pretty(self).map_err(|e| Error::Manifest {
            path: path.clone(),
            detail: e.to_string(),
        })?;
        let write = || -> std::io::Result<()> {
            let mut f = File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp, &path)
        };
        write().map_err(|e| Error::io(format!("writing {}", path.display()), e))
    }
}

/// The install lock: one process at a time changes the tools directory.
#[derive(Debug)]
pub(crate) struct LockGuard {
    _file: File,
}

pub(crate) fn lock(dir: &Path) -> Result<LockGuard> {
    fs::create_dir_all(dir).map_err(|e| Error::io(format!("creating {}", dir.display()), e))?;
    let path: PathBuf = dir.join(LOCK);
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| Error::io(format!("opening {}", path.display()), e))?;
    match file.try_lock() {
        Ok(()) => Ok(LockGuard { _file: file }),
        Err(TryLockError::WouldBlock) => Err(Error::Busy(dir.to_path_buf())),
        Err(TryLockError::Error(e)) => Err(Error::io(format!("locking {}", path.display()), e)),
    }
}
