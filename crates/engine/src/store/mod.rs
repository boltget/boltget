//! Persistence: the SQLite download database, owned by one process at a time.
//!
//! - The database lives in the data directory as `boltget.db`, in WAL mode with
//!   `synchronous=NORMAL` (`docs/ARCHITECTURE.md`, "Persistence").
//! - Ownership is an exclusive OS lock on `boltget.db.lock`, held for as long as
//!   the store is open ("Single owner rule"). The lock is released by the OS when
//!   the process dies, so a crash never leaves a stale lock behind.
//! - The connection lives on a dedicated thread (the "DB actor"). Async code sends
//!   it closures: [`Store::call`] awaits the result, [`Store::send`] does not.
//!   Jobs run in submission order, which is what lets checkpoint writes be
//!   fire-and-forget: a later job always sees the effects of earlier ones.

mod migrations;
pub(crate) mod records;

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use tokio::sync::oneshot;

use crate::error::{Error, Result};

pub use records::{ChecksumStatus, DownloadSummary, SavedError};

/// Name of the database file in the data directory.
pub const DB_FILE: &str = "boltget.db";
/// Name of the owner lock file in the data directory.
pub const LOCK_FILE: &str = "boltget.db.lock";

/// The default data directory: `%LOCALAPPDATA%\BoltGet` on Windows,
/// `~/Library/Application Support/app.boltget` on macOS and
/// `$XDG_DATA_HOME/boltget` elsewhere. `None` if the OS reports no home directory.
pub fn default_data_dir() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    #[cfg(windows)]
    let dir = base.data_local_dir().join("BoltGet");
    #[cfg(target_os = "macos")]
    let dir = base.data_dir().join("app.boltget");
    #[cfg(not(any(windows, target_os = "macos")))]
    let dir = base.data_local_dir().join("boltget");
    Some(dir)
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

struct Inner {
    tx: Mutex<Option<mpsc::Sender<Job>>>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
    dir: PathBuf,
}

/// Handle to the DB thread. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Store {
    inner: Arc<Inner>,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("dir", &self.inner.dir)
            .finish_non_exhaustive()
    }
}

/// The result of [`Store::open`].
#[derive(Debug)]
pub(crate) struct Opened {
    pub(crate) store: Store,
    /// The first id to hand out.
    pub(crate) next_id: u64,
    /// The stored settings.
    pub(crate) settings: crate::config::Settings,
    /// Learned per-host connection ceilings.
    pub(crate) ceilings: records::Ceilings,
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|e| Error::Database(format!("opening {}: {e}", path.display())))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    if !read_only {
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::Database(format!(
                "{} cannot use WAL mode (got {mode})",
                path.display()
            )));
        }
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
    }
    Ok(conn)
}

/// Takes the exclusive owner lock, or reports that another process holds it.
fn take_lock(dir: &Path) -> Result<File> {
    let path = dir.join(LOCK_FILE);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| Error::disk(&path, e))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(Error::DatabaseLocked {
            dir: dir.to_path_buf(),
        }),
        Err(TryLockError::Error(e)) => Err(Error::disk(&path, e)),
    }
}

impl Store {
    /// Opens (creating if needed) the database in `dir`: takes the owner lock,
    /// applies migrations, and marks downloads a dead process left running as
    /// paused. Blocking; quick.
    pub(crate) fn open(dir: &Path) -> Result<Opened> {
        std::fs::create_dir_all(dir).map_err(|e| Error::disk(dir, e))?;
        let lock = take_lock(dir)?;
        let db_path = dir.join(DB_FILE);
        let mut conn = open_connection(&db_path, false)?;
        migrations::migrate(&mut conn, &db_path, migrations::MIGRATIONS)?;
        let recovered = records::recover_interrupted(&conn)?;
        let next_id = records::next_id(&conn)?;
        let settings = records::load_settings(&conn)?;
        let ceilings = records::load_host_ceilings(&conn)?;
        if recovered > 0 {
            tracing::info!(
                recovered,
                "downloads interrupted by a previous run are paused"
            );
        }
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = thread::Builder::new()
            .name("boltget-db".into())
            .spawn(move || {
                let mut conn = conn;
                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
                // Close the database before giving up ownership.
                drop(conn);
                drop(lock);
            })
            .map_err(|e| Error::Internal(format!("spawning database thread: {e}")))?;
        Ok(Opened {
            store: Store {
                inner: Arc::new(Inner {
                    tx: Mutex::new(Some(tx)),
                    thread: Mutex::new(Some(thread)),
                    dir: dir.to_path_buf(),
                }),
            },
            next_id,
            settings,
            ceilings,
        })
    }

    /// Reads the download list without taking ownership (read-only connection),
    /// so it works while another process owns the database. A missing database
    /// is an empty list.
    pub(crate) fn peek(dir: &Path) -> Result<Vec<DownloadSummary>> {
        Ok(Self::peek_with(dir, records::list)?.unwrap_or_default())
    }

    /// The stored settings, read without taking ownership (defaults if there is
    /// no database).
    pub(crate) fn peek_settings(dir: &Path) -> Result<crate::config::Settings> {
        Ok(Self::peek_with(dir, records::load_settings)?.unwrap_or_default())
    }

    /// Runs `f` on a read-only connection; `None` if there is no database yet.
    fn peek_with<T>(dir: &Path, f: impl FnOnce(&Connection) -> Result<T>) -> Result<Option<T>> {
        let db_path = dir.join(DB_FILE);
        if !db_path.exists() {
            return Ok(None);
        }
        let conn = open_connection(&db_path, true)?;
        let version = migrations::current(&conn)?;
        let latest = migrations::latest(migrations::MIGRATIONS);
        if version == 0 {
            return Ok(None);
        }
        if version != latest {
            return Err(Error::Database(format!(
                "{} has schema version {version}, this BoltGet expects {latest}",
                db_path.display()
            )));
        }
        f(&conn).map(Some)
    }

    /// The data directory.
    pub(crate) fn dir(&self) -> &Path {
        &self.inner.dir
    }

    fn submit(&self, job: Job) -> Result<()> {
        let g = self
            .inner
            .tx
            .lock()
            .map_err(|_| Error::Internal("store lock poisoned".into()))?;
        match g.as_ref() {
            Some(tx) => tx
                .send(job)
                .map_err(|_| Error::Database("the database thread has stopped".into())),
            None => Err(Error::Database("the database is closed".into())),
        }
    }

    /// Runs `f` on the DB thread and waits for its result.
    pub(crate) async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.submit(Box::new(move |c| {
            let _ = tx.send(f(c));
        }))?;
        rx.await
            .map_err(|_| Error::Database("the database thread has stopped".into()))?
    }

    /// Queues `f` without waiting. Failures are logged: everything sent this way
    /// is progress bookkeeping where lagging behind is safe.
    pub(crate) fn send<F>(&self, what: &'static str, f: F)
    where
        F: FnOnce(&mut Connection) -> Result<()> + Send + 'static,
    {
        let job: Job = Box::new(move |c| {
            if let Err(e) = f(c) {
                tracing::warn!(error = %e, "could not save {what}");
            }
        });
        if let Err(e) = self.submit(job) {
            tracing::warn!(error = %e, "could not save {what}");
        }
    }

    /// Finishes queued jobs, closes the database and releases the owner lock.
    /// Later calls fail with a database error.
    pub(crate) async fn close(&self) {
        let tx = self.inner.tx.lock().ok().and_then(|mut g| g.take());
        drop(tx);
        let thread = self.inner.thread.lock().ok().and_then(|mut g| g.take());
        if let Some(t) = thread {
            let _ = tokio::task::spawn_blocking(move || t.join()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{DownloadId, State};

    fn new_record(id: u64) -> records::NewRecord {
        records::NewRecord {
            id: DownloadId(id),
            url: "https://example.com/f.bin?sig=secret".into(),
            request_ctx: "{}".into(),
            requested_kind: "dir",
            requested_dest: "/tmp".into(),
            conflict: "rename",
            max_connections: None,
            speed_limit: None,
        }
    }

    #[tokio::test]
    async fn second_owner_is_refused_until_close() {
        let dir = tempfile::tempdir().unwrap();
        let a = Store::open(dir.path()).unwrap();
        let e = Store::open(dir.path()).unwrap_err();
        assert!(matches!(e, Error::DatabaseLocked { .. }), "{e}");
        // Reading without ownership still works.
        assert!(Store::peek(dir.path()).unwrap().is_empty());
        a.store.close().await;
        let b = Store::open(dir.path()).unwrap();
        b.store.close().await;
        // Closed stores refuse work instead of hanging.
        assert!(b.store.call(|_| Ok(())).await.is_err());
    }

    #[tokio::test]
    async fn uses_wal_and_recovers_interrupted_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let o = Store::open(dir.path()).unwrap();
        assert_eq!(o.next_id, 1);
        let mode: String = o
            .store
            .call(|c| Ok(c.pragma_query_value(None, "journal_mode", |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "wal");
        o.store
            .call(|c| {
                records::insert(c, &new_record(1))?;
                records::insert(c, &new_record(2))?;
                records::set_state(c, DownloadId(1), State::Downloading, None)?;
                records::delete(c, DownloadId(2))?;
                Ok(())
            })
            .await
            .unwrap();
        o.store.close().await;

        let o = Store::open(dir.path()).unwrap();
        // Ids are never reused, even after a delete.
        assert_eq!(o.next_id, 3);
        let list = o.store.call(|c| records::list(c)).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].state, State::Paused);
        assert_eq!(list[0].url, "https://example.com/f.bin?…");
        o.store.close().await;
    }

    #[test]
    fn default_data_dir_is_absolute() {
        if let Some(d) = default_data_dir() {
            assert!(d.is_absolute(), "{}", d.display());
        }
    }
}
