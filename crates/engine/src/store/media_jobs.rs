//! Rows of the `media_job` table (schema version 7; `file_path` and `size`
//! since version 8).

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::records::now_ms;
use crate::error::{Error, Result};
use crate::events::DownloadId;
use crate::media_job::{
    ContainerPreference, MediaJob, MediaJobError, MediaJobId, MediaJobState, MediaJobUpdate,
    NewMediaJob, SubtitleChoice,
};

const COLUMNS: &str = "id, download_id, state, page_url, title, format_expression, preset, \
                       container, subtitles, output_template, dest_dir, error_key, error_detail, \
                       created_at, updated_at, file_path, size";

fn int(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| Error::Database(format!("value {v} does not fit the database")))
}

fn subtitles_json(s: Option<&SubtitleChoice>) -> Result<Option<String>> {
    s.map(|s| serde_json::to_string(s).map_err(|e| Error::Internal(e.to_string())))
        .transpose()
}

fn path_text(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

fn row(r: &Row<'_>) -> rusqlite::Result<MediaJob> {
    let bad = |i: usize, what: String| {
        rusqlite::Error::FromSqlConversionFailure(
            i,
            rusqlite::types::Type::Text,
            Box::new(Error::Database(what)),
        )
    };
    let state: String = r.get(2)?;
    let container: String = r.get(7)?;
    let subtitles: Option<String> = r.get(8)?;
    let error_key: Option<String> = r.get(11)?;
    let error_detail: Option<String> = r.get(12)?;
    let dest: String = r.get(10)?;
    Ok(MediaJob {
        id: MediaJobId(r.get::<_, i64>(0)?.max(0) as u64),
        download: r
            .get::<_, Option<i64>>(1)?
            .map(|d| DownloadId(d.max(0) as u64)),
        state: MediaJobState::parse(&state)
            .ok_or_else(|| bad(2, format!("unknown media job state {state:?}")))?,
        page_url: r.get(3)?,
        title: r.get(4)?,
        format_expression: r.get(5)?,
        preset: r.get(6)?,
        container: ContainerPreference::parse(&container)
            .ok_or_else(|| bad(7, format!("unknown container preference {container:?}")))?,
        // A value this build cannot read is dropped rather than failing the list.
        subtitles: subtitles.and_then(|s| serde_json::from_str(&s).ok()),
        output_template: r.get(9)?,
        dest_dir: PathBuf::from(dest),
        error: error_key.map(|key| MediaJobError {
            key,
            message: error_detail.unwrap_or_default(),
        }),
        created_at: r.get::<_, i64>(13)?.max(0) as u64,
        updated_at: r.get::<_, i64>(14)?.max(0) as u64,
        file_path: r.get::<_, Option<String>>(15)?.map(PathBuf::from),
        size: r.get::<_, Option<i64>>(16)?.map(|s| s.max(0) as u64),
    })
}

/// Inserts a job (state `queued`) and returns its id.
pub(crate) fn insert(conn: &Connection, job: &NewMediaJob) -> Result<MediaJobId> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO media_job (download_id, state, page_url, title, format_expression, preset,
                                container, subtitles, output_template, dest_dir,
                                created_at, updated_at, size)
         VALUES (?1, 'queued', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11)",
        params![
            job.download.map(|d| int(d.0)).transpose()?,
            job.page_url,
            job.title,
            job.format_expression,
            job.preset,
            job.container.as_str(),
            subtitles_json(job.subtitles.as_ref())?,
            job.output_template,
            path_text(&job.dest_dir),
            now,
            job.size.map(int).transpose()?
        ],
    )?;
    Ok(MediaJobId(conn.last_insert_rowid().max(0) as u64))
}

/// The job with this id.
pub(crate) fn get(conn: &Connection, id: MediaJobId) -> Result<Option<MediaJob>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM media_job WHERE id = ?1"),
            [int(id.0)?],
            row,
        )
        .optional()?)
}

/// Every job, oldest first.
pub(crate) fn list(conn: &Connection) -> Result<Vec<MediaJob>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM media_job ORDER BY id"))?;
    let rows = stmt.query_map([], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Applies `change` to the job; `false` if there is no such job.
pub(crate) fn update(conn: &Connection, id: MediaJobId, change: &MediaJobUpdate) -> Result<bool> {
    let Some(mut job) = get(conn, id)? else {
        return Ok(false);
    };
    let c = change.clone();
    if let Some(v) = c.state {
        job.state = v;
    }
    if let Some(v) = c.title {
        job.title = v;
    }
    if let Some(v) = c.format_expression {
        job.format_expression = v;
    }
    if let Some(v) = c.preset {
        job.preset = v;
    }
    if let Some(v) = c.container {
        job.container = v;
    }
    if let Some(v) = c.subtitles {
        job.subtitles = v;
    }
    if let Some(v) = c.output_template {
        job.output_template = v;
    }
    if let Some(v) = c.dest_dir {
        job.dest_dir = v;
    }
    if let Some(v) = c.download {
        job.download = v;
    }
    if let Some(v) = c.error {
        job.error = v;
    }
    if let Some(v) = c.file_path {
        job.file_path = v;
    }
    if let Some(v) = c.size {
        job.size = v;
    }
    crate::media_job::validate(&job.page_url, &job.format_expression, &job.output_template)?;
    conn.execute(
        "UPDATE media_job SET download_id = ?2, state = ?3, title = ?4, format_expression = ?5,
                preset = ?6, container = ?7, subtitles = ?8, output_template = ?9,
                dest_dir = ?10, error_key = ?11, error_detail = ?12, updated_at = ?13,
                file_path = ?14, size = ?15
         WHERE id = ?1",
        params![
            int(id.0)?,
            job.download.map(|d| int(d.0)).transpose()?,
            job.state.as_str(),
            job.title,
            job.format_expression,
            job.preset,
            job.container.as_str(),
            subtitles_json(job.subtitles.as_ref())?,
            job.output_template,
            path_text(&job.dest_dir),
            job.error.as_ref().map(|e| e.key.as_str()),
            job.error.as_ref().map(|e| e.message.as_str()),
            now_ms(),
            job.file_path.as_deref().map(path_text),
            job.size.map(int).transpose()?
        ],
    )?;
    Ok(true)
}

/// Deletes the job; `false` if there was none.
pub(crate) fn delete(conn: &Connection, id: MediaJobId) -> Result<bool> {
    Ok(conn.execute("DELETE FROM media_job WHERE id = ?1", [int(id.0)?])? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Store, records};

    #[tokio::test]
    async fn insert_get_update_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let o = Store::open(dir.path()).unwrap();
        o.store
            .call(|c| {
                let mut job = NewMediaJob::new(
                    "https://video.test/watch?v=1",
                    "bv*+ba/b",
                    PathBuf::from("/media"),
                );
                job.title = Some("A video".into());
                job.preset = Some("best".into());
                let a = insert(c, &job)?;
                job.container = ContainerPreference::Original;
                job.subtitles = Some(SubtitleChoice {
                    languages: vec!["en".into(), "pt-BR".into()],
                    automatic: true,
                    embed: true,
                });
                let b = insert(c, &job)?;
                assert_eq!((a, b), (MediaJobId(1), MediaJobId(2)));

                let got = get(c, a)?.unwrap();
                assert_eq!(got.state, MediaJobState::Queued);
                assert_eq!(got.title.as_deref(), Some("A video"));
                assert_eq!(got.container, ContainerPreference::Mp4);
                assert_eq!(got.output_template, "%(title)s [%(id)s].%(ext)s");
                assert_eq!(got.subtitles, None);
                assert_eq!(get(c, b)?.unwrap().subtitles, job.subtitles);

                let change = MediaJobUpdate {
                    state: Some(MediaJobState::Failed),
                    error: Some(Some(MediaJobError {
                        key: "error.media.extraction".into(),
                        message: "Video unavailable".into(),
                    })),
                    preset: Some(None),
                    ..MediaJobUpdate::default()
                };
                assert!(update(c, a, &change)?);
                let got = get(c, a)?.unwrap();
                assert_eq!(got.state, MediaJobState::Failed);
                assert_eq!(got.preset, None);
                assert_eq!(got.error.unwrap().message, "Video unavailable");
                assert!(!update(c, MediaJobId(9), &change)?);
                let done = MediaJobUpdate {
                    state: Some(MediaJobState::Completed),
                    file_path: Some(Some(PathBuf::from("/media/A video [1].mp4"))),
                    size: Some(Some(1234)),
                    ..MediaJobUpdate::default()
                };
                assert!(update(c, b, &done)?);
                let got = get(c, b)?.unwrap();
                assert_eq!(got.file_path, Some(PathBuf::from("/media/A video [1].mp4")));
                assert_eq!(got.size, Some(1234));
                // An update cannot store what an insert would refuse.
                let blank = MediaJobUpdate {
                    format_expression: Some(" ".into()),
                    ..MediaJobUpdate::default()
                };
                assert!(update(c, a, &blank).is_err());

                assert_eq!(list(c)?.iter().map(|j| j.id).collect::<Vec<_>>(), [a, b]);
                assert!(delete(c, a)?);
                assert!(!delete(c, a)?);
                assert_eq!(list(c)?.len(), 1);
                Ok(())
            })
            .await
            .unwrap();
        o.store.close().await;
    }

    #[tokio::test]
    async fn a_job_goes_with_its_download() {
        let dir = tempfile::tempdir().unwrap();
        let o = Store::open(dir.path()).unwrap();
        o.store
            .call(|c| {
                records::insert(
                    c,
                    &records::NewRecord {
                        id: DownloadId(1),
                        url: "https://video.test/watch?v=1".into(),
                        request_ctx: "{}".into(),
                        requested_kind: "dir",
                        requested_dest: "/media".into(),
                        conflict: "rename",
                        max_connections: None,
                        speed_limit: None,
                        queue: crate::queue::DEFAULT_QUEUE,
                        start_at: None,
                        private: false,
                    },
                )?;
                let mut job = NewMediaJob::new("https://video.test/watch?v=1", "b", "/media");
                job.download = Some(DownloadId(1));
                let id = insert(c, &job)?;
                assert_eq!(get(c, id)?.unwrap().download, Some(DownloadId(1)));
                records::delete(c, DownloadId(1))?;
                assert!(get(c, id)?.is_none());
                Ok(())
            })
            .await
            .unwrap();
        o.store.close().await;
    }
}
