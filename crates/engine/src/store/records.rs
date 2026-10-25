//! Rows of the download database and the queries the engine runs on them.
//!
//! Every function takes the connection owned by the DB thread; callers reach it
//! through [`Store::call`](super::Store::call) or [`Store::send`](super::Store::send).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::error::{Error, ErrorKind, Result};
use crate::events::{DownloadId, ErrorInfo, State};
use crate::headers::Validators;
use crate::integrity::{Checksum, ChecksumSource, HashAlgo};
use crate::request::redact_url;
use crate::segment::SegmentCheckpoint;

/// Unix time in milliseconds.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn int(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| Error::Database(format!("value {v} does not fit the database")))
}

fn uint(v: i64) -> u64 {
    v.max(0) as u64
}

/// `u64::MAX` (unknown end of an open segment) is stored as NULL.
fn end_to_sql(end: u64) -> Result<Option<i64>> {
    if end == u64::MAX {
        Ok(None)
    } else {
        int(end).map(Some)
    }
}

/// What is known when a download is added, before the probe.
#[derive(Debug, Clone)]
pub(crate) struct NewRecord {
    pub(crate) id: DownloadId,
    pub(crate) url: String,
    pub(crate) request_ctx: String,
    /// `"dir"` or `"file"`.
    pub(crate) requested_kind: &'static str,
    pub(crate) requested_dest: String,
    pub(crate) conflict: &'static str,
    pub(crate) max_connections: Option<u32>,
    pub(crate) speed_limit: Option<u64>,
}

/// What the probe resolved: destination, size, validators and the initial split.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub(crate) file_name: String,
    pub(crate) dest_dir: String,
    pub(crate) temp_path: String,
    pub(crate) total_size: Option<u64>,
    pub(crate) mime: Option<String>,
    pub(crate) validators: Validators,
    pub(crate) resumable: bool,
    pub(crate) encoded: bool,
    pub(crate) final_url: String,
    pub(crate) segments: Vec<SegmentCheckpoint>,
}

/// One checkpoint: the segment table built from durable offsets only.
#[derive(Debug, Clone)]
pub(crate) struct Checkpoint {
    pub(crate) segments: Vec<SegmentCheckpoint>,
    pub(crate) downloaded: u64,
    /// Known once an unknown-size download reached its end.
    pub(crate) total: Option<u64>,
    pub(crate) resumable: bool,
    pub(crate) final_url: String,
    /// Set just before the final rename: the name the file is about to get.
    pub(crate) file_name: Option<String>,
}

/// A download as stored, for resuming it.
#[derive(Debug, Clone)]
pub(crate) struct Saved {
    pub(crate) id: DownloadId,
    pub(crate) state: State,
    pub(crate) url: String,
    pub(crate) final_url: Option<String>,
    pub(crate) request_ctx: String,
    pub(crate) requested_kind: String,
    pub(crate) requested_dest: String,
    pub(crate) conflict: String,
    pub(crate) file_name: Option<String>,
    pub(crate) dest_dir: Option<String>,
    pub(crate) temp_path: Option<String>,
    pub(crate) total_size: Option<u64>,
    pub(crate) mime: Option<String>,
    pub(crate) validators: Validators,
    pub(crate) resumable: bool,
    pub(crate) encoded: bool,
    pub(crate) max_connections: Option<u32>,
    pub(crate) speed_limit: Option<u64>,
    pub(crate) checksum: Option<(Checksum, ChecksumSource)>,
    pub(crate) segments: Vec<SegmentCheckpoint>,
}

impl Saved {
    /// The resolved final path, once the probe has run.
    pub(crate) fn path(&self) -> Option<PathBuf> {
        match (&self.dest_dir, &self.file_name) {
            (Some(d), Some(n)) => Some(PathBuf::from(d).join(n)),
            _ => None,
        }
    }
}

/// The error a download stopped with, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedError {
    /// Classification.
    pub kind: ErrorKind,
    /// Message-catalog key.
    pub key: String,
    /// English diagnostic text.
    pub message: String,
}

/// A download's checksum, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChecksumStatus {
    /// What the file must hash to.
    pub expected: Checksum,
    /// Where that came from.
    pub source: ChecksumSource,
    /// What the file hashed to, once verified (lowercase hex).
    pub actual: Option<String>,
}

/// A row of [`Engine::list`](crate::Engine::list).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct DownloadSummary {
    /// Download id.
    pub id: DownloadId,
    /// State at the last update.
    pub state: State,
    /// The original URL, redacted (no query string or credentials).
    pub url: String,
    /// Resolved file name, once probed.
    pub file_name: Option<String>,
    /// Final path, once probed.
    pub path: Option<PathBuf>,
    /// The in-progress file, while not completed.
    pub partial_path: Option<PathBuf>,
    /// Total size, if known.
    pub total: Option<u64>,
    /// Durable bytes at the last checkpoint.
    pub downloaded: u64,
    /// Whether the server supports resuming (byte ranges).
    pub resumable: bool,
    /// The error, for failed downloads.
    pub error: Option<SavedError>,
    /// The expected checksum and, once checked, the actual digest.
    pub checksum: Option<ChecksumStatus>,
    /// Creation time, Unix milliseconds.
    pub created_at: u64,
    /// Last update, Unix milliseconds.
    pub updated_at: u64,
    /// Completion time, Unix milliseconds.
    pub completed_at: Option<u64>,
}

pub(crate) fn insert(conn: &Connection, r: &NewRecord) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO download (id, state, url, request_ctx, requested_kind, requested_dest,
                               conflict, max_connections, speed_limit_bps, created_at, updated_at)
         VALUES (?1, 'queued', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            int(r.id.0)?,
            r.url,
            r.request_ctx,
            r.requested_kind,
            r.requested_dest,
            r.conflict,
            r.max_connections,
            r.speed_limit.map(int).transpose()?,
            now
        ],
    )?;
    Ok(())
}

fn replace_segments(conn: &Connection, id: DownloadId, segs: &[SegmentCheckpoint]) -> Result<()> {
    let id = int(id.0)?;
    conn.execute("DELETE FROM segment WHERE download_id = ?1", [id])?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO segment (download_id, idx, start, \"end\", done) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for s in segs {
        stmt.execute(params![
            id,
            s.idx,
            int(s.start)?,
            end_to_sql(s.end)?,
            int(s.done)?
        ])?;
    }
    Ok(())
}

pub(crate) fn save_resolved(conn: &mut Connection, id: DownloadId, r: &Resolved) -> Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    tx.execute(
        "UPDATE download SET file_name = ?2, dest_dir = ?3, temp_path = ?4, total_size = ?5,
                downloaded = 0, mime = ?6, etag = ?7, last_modified = ?8, validator_date = ?9,
                resumable = ?10, encoded = ?11, final_url = ?12,
                started_at = coalesce(started_at, ?13), updated_at = ?13
         WHERE id = ?1",
        params![
            int(id.0)?,
            r.file_name,
            r.dest_dir,
            r.temp_path,
            r.total_size.map(int).transpose()?,
            r.mime,
            r.validators.etag,
            r.validators.last_modified,
            r.validators.date,
            r.resumable,
            r.encoded,
            r.final_url,
            now
        ],
    )?;
    replace_segments(&tx, id, &r.segments)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn save_checkpoint(conn: &mut Connection, id: DownloadId, c: &Checkpoint) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE download SET downloaded = ?2, resumable = ?3, final_url = ?4,
                file_name = coalesce(?5, file_name), total_size = coalesce(?6, total_size),
                updated_at = ?7
         WHERE id = ?1",
        params![
            int(id.0)?,
            int(c.downloaded)?,
            c.resumable,
            c.final_url,
            c.file_name,
            c.total.map(int).transpose()?,
            now_ms()
        ],
    )?;
    replace_segments(&tx, id, &c.segments)?;
    tx.commit()?;
    Ok(())
}

/// Records a state change. Entering any state other than `failed`, `retrying`
/// or `corrupt` clears the stored error; those record it when given.
pub(crate) fn set_state(
    conn: &Connection,
    id: DownloadId,
    state: State,
    error: Option<&ErrorInfo>,
) -> Result<()> {
    let keep_error = matches!(
        state,
        State::Failed | State::Retrying | State::Corrupt | State::NeedsNewLink
    );
    let (kind, key, detail) = match error.filter(|_| keep_error) {
        Some(e) => (
            Some(e.kind.as_str()),
            Some(e.key.to_owned()),
            Some(e.message.clone()),
        ),
        None => (None, None, None),
    };
    conn.execute(
        "UPDATE download SET state = ?2, error_kind = ?3, error_key = ?4, error_detail = ?5,
                updated_at = ?6
         WHERE id = ?1",
        params![int(id.0)?, state.as_str(), kind, key, detail, now_ms()],
    )?;
    Ok(())
}

/// The download finished: final name, no partial file, segments dropped.
pub(crate) fn mark_completed(
    conn: &mut Connection,
    id: DownloadId,
    file_name: &str,
    dest_dir: &str,
    total: u64,
) -> Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    tx.execute(
        "UPDATE download SET state = 'completed', file_name = ?2, dest_dir = ?3, temp_path = NULL,
                total_size = ?4, downloaded = ?4, error_kind = NULL, error_key = NULL,
                error_detail = NULL, completed_at = ?5, updated_at = ?5
         WHERE id = ?1",
        params![int(id.0)?, file_name, dest_dir, int(total)?, now],
    )?;
    tx.execute("DELETE FROM segment WHERE download_id = ?1", [int(id.0)?])?;
    tx.commit()?;
    Ok(())
}

/// Downloads left `probing`/`downloading`/`retrying` by a process that died are
/// paused: their checkpoint is valid (it never claims more than was durable).
pub(crate) fn recover_interrupted(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE download SET state = 'paused', updated_at = ?1
         WHERE state IN ('probing', 'downloading', 'retrying', 'verifying')",
        [now_ms()],
    )?)
}

/// The next unused id. `AUTOINCREMENT` keeps the high-water mark in
/// `sqlite_sequence`, so ids of removed downloads are never handed out again.
pub(crate) fn next_id(conn: &Connection) -> Result<u64> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'download'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let max: Option<i64> = conn.query_row("SELECT max(id) FROM download", [], |r| r.get(0))?;
    Ok(uint(seq.unwrap_or(0).max(max.unwrap_or(0))) + 1)
}

/// Forgets all progress (restart from zero): segments and byte count.
pub(crate) fn reset_progress(conn: &mut Connection, id: DownloadId) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM segment WHERE download_id = ?1", [int(id.0)?])?;
    tx.execute(
        "UPDATE download SET downloaded = 0, updated_at = ?2 WHERE id = ?1",
        params![int(id.0)?, now_ms()],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn delete(conn: &Connection, id: DownloadId) -> Result<bool> {
    Ok(conn.execute("DELETE FROM download WHERE id = ?1", [int(id.0)?])? > 0)
}

fn state_of(row: &Row<'_>) -> rusqlite::Result<State> {
    let s: String = row.get("state")?;
    Ok(State::parse(&s).unwrap_or(State::Failed))
}

fn segments(conn: &Connection, id: DownloadId) -> Result<Vec<SegmentCheckpoint>> {
    let mut stmt = conn.prepare_cached(
        "SELECT idx, start, \"end\", done FROM segment WHERE download_id = ?1 ORDER BY start",
    )?;
    let rows = stmt.query_map([int(id.0)?], |r| {
        Ok(SegmentCheckpoint {
            idx: r.get(0)?,
            start: uint(r.get(1)?),
            end: r.get::<_, Option<i64>>(2)?.map_or(u64::MAX, uint),
            done: uint(r.get(3)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn load(conn: &Connection, id: DownloadId) -> Result<Option<Saved>> {
    let saved = conn
        .query_row("SELECT * FROM download WHERE id = ?1", [int(id.0)?], |r| {
            Ok(Saved {
                id,
                state: state_of(r)?,
                url: r.get("url")?,
                final_url: r.get("final_url")?,
                request_ctx: r.get("request_ctx")?,
                requested_kind: r.get("requested_kind")?,
                requested_dest: r.get("requested_dest")?,
                conflict: r.get("conflict")?,
                file_name: r.get("file_name")?,
                dest_dir: r.get("dest_dir")?,
                temp_path: r.get("temp_path")?,
                total_size: r.get::<_, Option<i64>>("total_size")?.map(uint),
                mime: r.get("mime")?,
                validators: Validators {
                    etag: r.get("etag")?,
                    last_modified: r.get("last_modified")?,
                    date: r.get("validator_date")?,
                },
                resumable: r.get("resumable")?,
                encoded: r.get("encoded")?,
                max_connections: r.get("max_connections")?,
                speed_limit: r.get::<_, Option<i64>>("speed_limit_bps")?.map(uint),
                checksum: checksum_of(r)?,
                segments: Vec::new(),
            })
        })
        .optional()?;
    match saved {
        Some(mut s) => {
            s.segments = segments(conn, id)?;
            Ok(Some(s))
        }
        None => Ok(None),
    }
}

fn summary(r: &Row<'_>) -> rusqlite::Result<DownloadSummary> {
    let state = state_of(r)?;
    let url: String = r.get("url")?;
    let url = url::Url::parse(&url).map_or_else(|_| "<invalid URL>".to_owned(), |u| redact_url(&u));
    let file_name: Option<String> = r.get("file_name")?;
    let dest_dir: Option<String> = r.get("dest_dir")?;
    let path = match (&dest_dir, &file_name) {
        (Some(d), Some(n)) => Some(PathBuf::from(d).join(n)),
        _ => None,
    };
    let temp: Option<String> = r.get("temp_path")?;
    let error = match r.get::<_, Option<String>>("error_detail")? {
        Some(message) => Some(SavedError {
            kind: r
                .get::<_, Option<String>>("error_kind")?
                .and_then(|k| ErrorKind::parse(&k))
                .unwrap_or(ErrorKind::Internal),
            key: r
                .get::<_, Option<String>>("error_key")?
                .unwrap_or_else(|| "error.internal".into()),
            message,
        }),
        None => None,
    };
    Ok(DownloadSummary {
        id: DownloadId(uint(r.get("id")?)),
        state,
        url,
        file_name,
        path,
        partial_path: temp
            .filter(|_| state != State::Completed)
            .map(PathBuf::from),
        total: r.get::<_, Option<i64>>("total_size")?.map(uint),
        downloaded: uint(r.get("downloaded")?),
        resumable: r.get("resumable")?,
        error,
        checksum: checksum_of(r)?.map(|(expected, source)| ChecksumStatus {
            expected,
            source,
            actual: r.get("checksum_actual").ok().flatten(),
        }),
        created_at: uint(r.get("created_at")?),
        updated_at: uint(r.get("updated_at")?),
        completed_at: r.get::<_, Option<i64>>("completed_at")?.map(uint),
    })
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<DownloadSummary>> {
    let mut stmt = conn.prepare_cached("SELECT * FROM download ORDER BY id")?;
    let rows = stmt.query_map([], summary)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn get_summary(conn: &Connection, id: DownloadId) -> Result<Option<DownloadSummary>> {
    Ok(conn
        .query_row(
            "SELECT * FROM download WHERE id = ?1",
            [int(id.0)?],
            summary,
        )
        .optional()?)
}

/// Changes a download's own speed limit (`None`: no per-download limit).
pub(crate) fn set_speed_limit(conn: &Connection, id: DownloadId, bps: Option<u64>) -> Result<()> {
    conn.execute(
        "UPDATE download SET speed_limit_bps = ?2, updated_at = ?3 WHERE id = ?1",
        params![int(id.0)?, bps.map(int).transpose()?, now_ms()],
    )?;
    Ok(())
}

/// Setting key of the global bandwidth limit (bytes per second, JSON number).
pub(crate) const SETTING_SPEED_LIMIT: &str = "engine.speed_limit_bps";

fn get_setting(conn: &Connection, key: &str) -> Result<Option<serde_json::Value>> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    // A value this version cannot parse is treated as unset, not as an error.
    Ok(raw.and_then(|v| serde_json::from_str(&v).ok()))
}

/// Stores a setting; `None` deletes the row (the default applies).
pub(crate) fn put_setting(
    conn: &Connection,
    key: &str,
    value: Option<&serde_json::Value>,
) -> Result<()> {
    match value {
        Some(v) => conn.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, v.to_string()],
        )?,
        None => conn.execute("DELETE FROM setting WHERE key = ?1", [key])?,
    };
    Ok(())
}

/// The typed settings, with defaults for missing keys.
pub(crate) fn load_settings(conn: &Connection) -> Result<crate::config::Settings> {
    Ok(crate::config::Settings {
        speed_limit: get_setting(conn, SETTING_SPEED_LIMIT)?
            .and_then(|v| v.as_u64())
            .filter(|v| *v > 0),
    })
}

fn checksum_of(r: &Row<'_>) -> rusqlite::Result<Option<(Checksum, ChecksumSource)>> {
    let algo: Option<String> = r.get("checksum_algo")?;
    let hex: Option<String> = r.get("checksum_expected")?;
    let source: Option<String> = r.get("checksum_source")?;
    Ok(match (algo.as_deref().and_then(HashAlgo::parse), hex) {
        (Some(a), Some(h)) => Checksum::new(a, &h).ok().map(|c| {
            (
                c,
                ChecksumSource::from_db(source.as_deref().unwrap_or("user")),
            )
        }),
        _ => None,
    })
}

/// Sets (or with `None` clears) the expected checksum; clears the last result.
pub(crate) fn set_checksum(
    conn: &Connection,
    id: DownloadId,
    checksum: Option<&(Checksum, ChecksumSource)>,
) -> Result<()> {
    conn.execute(
        "UPDATE download SET checksum_algo = ?2, checksum_expected = ?3, checksum_source = ?4,
                checksum_actual = NULL, updated_at = ?5
         WHERE id = ?1",
        params![
            int(id.0)?,
            checksum.map(|(c, _)| c.algo.as_str()),
            checksum.map(|(c, _)| c.hex.clone()),
            checksum.map(|(_, s)| s.as_db()),
            now_ms()
        ],
    )?;
    Ok(())
}

/// Records the digest the file actually has (after verification).
pub(crate) fn set_checksum_actual(conn: &Connection, id: DownloadId, hex: &str) -> Result<()> {
    conn.execute(
        "UPDATE download SET checksum_actual = ?2, updated_at = ?3 WHERE id = ?1",
        params![int(id.0)?, hex, now_ms()],
    )?;
    Ok(())
}

/// A new link accepted by link refresh.
#[derive(Debug, Clone)]
pub(crate) struct Refreshed {
    pub(crate) url: String,
    pub(crate) request_ctx: String,
    pub(crate) final_url: String,
    pub(crate) validators: Validators,
    pub(crate) mime: Option<String>,
}

/// Switches a download to a new link: re-resolution starts from it, its
/// validators replace the old ones, and both links are kept in
/// `download_source`. A download waiting for a new link becomes paused.
pub(crate) fn apply_refresh(conn: &mut Connection, id: DownloadId, r: &Refreshed) -> Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    let old: String = tx.query_row(
        "SELECT url FROM download WHERE id = ?1",
        [int(id.0)?],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO download_source (download_id, url, role, added_at)
         VALUES (?1, ?2, 'primary', ?3)",
        params![int(id.0)?, old, now],
    )?;
    tx.execute(
        "INSERT INTO download_source (download_id, url, role, added_at) VALUES (?1, ?2, 'refreshed', ?3)
         ON CONFLICT (download_id, url) DO UPDATE SET added_at = excluded.added_at, last_error = NULL",
        params![int(id.0)?, r.url, now],
    )?;
    tx.execute(
        "UPDATE download SET url = ?2, request_ctx = ?3, final_url = ?4, etag = ?5,
                last_modified = ?6, validator_date = ?7, mime = coalesce(?8, mime),
                state = CASE WHEN state = 'needs_new_link' THEN 'paused' ELSE state END,
                error_kind = NULL, error_key = NULL, error_detail = NULL, updated_at = ?9
         WHERE id = ?1",
        params![
            int(id.0)?,
            r.url,
            r.request_ctx,
            r.final_url,
            r.validators.etag,
            r.validators.last_modified,
            r.validators.date,
            r.mime,
            now
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Links a download has used, oldest first: `(url, role)`.
pub(crate) fn sources(conn: &Connection, id: DownloadId) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT url, role FROM download_source WHERE download_id = ?1 ORDER BY added_at, role <> 'primary', url",
    )?;
    let rows = stmt.query_map([int(id.0)?], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
