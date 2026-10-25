//! Embedded, forward-only schema migrations tracked with `PRAGMA user_version`.
//!
//! Each migration is a plain SQL file applied in its own transaction together
//! with the `user_version` bump, so a failed migration leaves the database at the
//! previous version. Before upgrading an existing database, a copy is written to
//! `boltget.db.bak-<old version>` (see `docs/ARCHITECTURE.md`, "Persistence").

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::{Error, Result};

/// One schema step: applying `sql` to a database at `version - 1` yields `version`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) sql: &'static str,
}

/// All migrations, in order. Never edit a released migration; add a new one.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("migrations/0002_speed_limit.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("migrations/0003_checksum.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("migrations/0004_download_source.sql"),
    },
];

/// The schema version this build writes.
pub(crate) fn latest(migrations: &[Migration]) -> u32 {
    migrations.last().map_or(0, |m| m.version)
}

/// The schema version of an open database.
pub(crate) fn current(conn: &Connection) -> Result<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Brings the database up to the latest version. Refuses databases written by a
/// newer schema. Returns the resulting version.
pub(crate) fn migrate(
    conn: &mut Connection,
    db_path: &Path,
    migrations: &[Migration],
) -> Result<u32> {
    debug_assert!(
        migrations
            .iter()
            .enumerate()
            .all(|(i, m)| m.version as usize == i + 1),
        "migrations must be numbered 1, 2, 3, ..."
    );
    let from = current(conn)?;
    let to = latest(migrations);
    if from > to {
        return Err(Error::Database(format!(
            "{} has schema version {from}, but this BoltGet only knows up to {to}; \
             it was written by a newer BoltGet",
            db_path.display()
        )));
    }
    if from == to {
        return Ok(from);
    }
    if from > 0 {
        backup(conn, db_path, from)?;
    }
    for m in migrations.iter().filter(|m| m.version > from) {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)
            .map_err(|e| Error::Database(format!("migration {} failed: {e}", m.version)))?;
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
        tracing::debug!(version = m.version, "applied database migration");
    }
    Ok(to)
}

fn backup_path(db_path: &Path, version: u32) -> PathBuf {
    let mut s = db_path.as_os_str().to_owned();
    s.push(format!(".bak-{version}"));
    PathBuf::from(s)
}

/// Writes a consistent copy of the database (including WAL content) next to it.
fn backup(conn: &Connection, db_path: &Path, version: u32) -> Result<()> {
    let bak = backup_path(db_path, version);
    match std::fs::remove_file(&bak) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::disk(&bak, e)),
    }
    let target = bak
        .to_str()
        .ok_or_else(|| Error::Database(format!("backup path {} is not UTF-8", bak.display())))?;
    conn.execute("VACUUM INTO ?1", [target])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &Path) -> (Connection, PathBuf) {
        let p = dir.join("t.db");
        (Connection::open(&p).unwrap(), p)
    }

    #[test]
    fn embedded_migrations_are_numbered_and_apply() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1);
        }
        let dir = tempfile::tempdir().unwrap();
        let (mut c, p) = open(dir.path());
        assert_eq!(migrate(&mut c, &p, MIGRATIONS).unwrap(), latest(MIGRATIONS));
        // Idempotent.
        assert_eq!(migrate(&mut c, &p, MIGRATIONS).unwrap(), latest(MIGRATIONS));
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('download', 'segment', 'setting')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 3);
        // A fresh database needs no backup.
        assert!(!backup_path(&p, 0).exists());
    }

    #[test]
    fn upgrade_backs_up_and_applies_only_new_steps() {
        const V1: Migration = Migration {
            version: 1,
            sql: "CREATE TABLE a (x INTEGER); INSERT INTO a VALUES (7);",
        };
        const V2: Migration = Migration {
            version: 2,
            sql: "ALTER TABLE a ADD COLUMN y TEXT;",
        };
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, db) = open(dir.path());
        assert_eq!(migrate(&mut conn, &db, &[V1]).unwrap(), 1);
        assert_eq!(migrate(&mut conn, &db, &[V1, V2]).unwrap(), 2);
        let bak = backup_path(&db, 1);
        assert!(bak.exists(), "backup of version 1 expected");
        let backup = Connection::open(&bak).unwrap();
        assert_eq!(current(&backup).unwrap(), 1);
        let kept: i64 = backup
            .query_row("SELECT x FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 7);
        // The upgraded database kept its data and gained the column.
        let added: Option<String> = conn.query_row("SELECT y FROM a", [], |r| r.get(0)).unwrap();
        assert_eq!(added, None);
    }

    #[test]
    fn failed_migration_rolls_back() {
        const V1: Migration = Migration {
            version: 1,
            sql: "CREATE TABLE a (x INTEGER);",
        };
        const BAD: Migration = Migration {
            version: 2,
            sql: "CREATE TABLE b (x INTEGER); THIS IS NOT SQL;",
        };
        let dir = tempfile::tempdir().unwrap();
        let (mut c, p) = open(dir.path());
        migrate(&mut c, &p, &[V1]).unwrap();
        let e = migrate(&mut c, &p, &[V1, BAD]).unwrap_err();
        assert!(matches!(e, Error::Database(_)), "{e}");
        assert_eq!(current(&c).unwrap(), 1);
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "partial migration must not persist");
    }

    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut c, p) = open(dir.path());
        c.pragma_update(None, "user_version", 99).unwrap();
        let e = migrate(&mut c, &p, MIGRATIONS).unwrap_err();
        assert!(e.to_string().contains("newer BoltGet"), "{e}");
        assert_eq!(current(&c).unwrap(), 99);
    }
}
