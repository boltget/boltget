//! Media downloads in the list (docs/VIDEO.md, "Picker and media downloads:
//! as built"): a media job is a row of the download list, run by yt-dlp
//! rather than by the engine.
//!
//! - **Rows.** A job shows as a [`DownloadRow`] with `media` set, whose id
//!   is [`ROW_BASE`] plus the job's id, so the page keeps one list keyed by
//!   one kind of id and the commands that take a download's id (pause,
//!   resume, remove, open, show in folder, details) tell the two apart by
//!   the number. The job's own `download_id` column stays empty: a
//!   `download` row is something the engine itself would run.
//! - **Running.** At most [`MAX_RUNNING`] jobs run at once; more wait
//!   (`queued`) and start as others end. They do not take slots of the
//!   engine's queues: those count connections of the engine's downloads,
//!   and a yt-dlp process is not one of them.
//! - **Progress** goes through the hub like the engine's
//!   ([`Request::MediaSample`]), so speeds, the tray and the taskbar treat
//!   both alike; merging or converting is the `processing` state.
//! - **Pause** ends the yt-dlp process (its Job Object); resume starts it
//!   again with the same arguments and it continues from its `.part` files.
//! - **On start**, jobs that were running or waiting run again; quitting
//!   leaves them as they were, so they continue on the next start.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use boltget_engine::{
    Engine, EngineConfig, MediaJob, MediaJobError, MediaJobId, MediaJobState, MediaJobUpdate,
    NewMediaJob,
};
use boltget_media::{
    DownloadOptions, DownloadOutcome, Downloader, MediaProgress, Phase, Prober, Program, Sidecars,
    Sources,
};
use tokio::sync::watch;

use super::Bridge;
use super::dto::{
    DownloadError, DownloadId, DownloadRow, DownloadState, ErrorKind, MediaRowInfo, QueueId,
};
use super::hub::{HubHandle, Request};
use super::model::Sample;
use crate::error::UiError;
use crate::settings::{self, AppSettings, VideoSettings};

/// Row ids of media jobs start here (2^50, far above any download id and
/// still exact as a JavaScript number).
pub(crate) const ROW_BASE: u64 = 1 << 50;

/// Media jobs that run at once.
pub(crate) const MAX_RUNNING: usize = 2;

/// The row id of a job.
pub(crate) fn row_id(id: MediaJobId) -> DownloadId {
    DownloadId(ROW_BASE + id.0)
}

/// The job a row id names, if it names one.
pub(crate) fn job_id(id: DownloadId) -> Option<MediaJobId> {
    id.0.checked_sub(ROW_BASE)
        .filter(|&n| n > 0)
        .map(MediaJobId)
}

fn media_error(e: &boltget_media::Error) -> UiError {
    UiError::with_message(e.message_key(), e.to_string())
}

/// The sidecar manager for the tools in `dir`; its downloads use the app's
/// proxy settings.
pub(crate) fn sidecars(dir: &Path, settings: &AppSettings) -> Sidecars {
    let mut config = EngineConfig::default();
    config.proxy = settings::rules(&settings.proxy, crate::platform::system_proxy);
    Sidecars::new(dir.to_path_buf(), Sources::official(), config)
}

/// yt-dlp and FFmpeg's folder: the programs the user chose in Settings,
/// else the installed ones. FFmpeg is `None` when neither is there.
fn tool_paths(
    sidecars: &Sidecars,
    video: &VideoSettings,
) -> Result<(PathBuf, Option<PathBuf>), boltget_media::Error> {
    let yt_dlp = match &video.yt_dlp_path {
        Some(p) => PathBuf::from(p),
        None => sidecars.program(Program::YtDlp)?,
    };
    let ffmpeg = match &video.ffmpeg_path {
        Some(p) => Some(PathBuf::from(p)),
        None => sidecars.program(Program::Ffmpeg).ok(),
    };
    Ok((
        yt_dlp,
        ffmpeg.and_then(|p| p.parent().map(Path::to_path_buf)),
    ))
}

/// Whether both tools are there (installed, or chosen in Settings).
pub(crate) fn tools_ready(sidecars: &Sidecars, video: &VideoSettings) -> bool {
    tool_paths(sidecars, video).is_ok_and(|(_, ffmpeg)| ffmpeg.is_some())
}

/// A prober with the tools Settings names.
pub(crate) fn prober(sidecars: &Sidecars, video: &VideoSettings) -> Result<Prober, UiError> {
    let (yt_dlp, ffmpeg) = tool_paths(sidecars, video).map_err(|e| media_error(&e))?;
    Ok(Prober::new(yt_dlp, ffmpeg, sidecars.dir().join(".jobs")))
}

/// A downloader with the tools Settings names; FFmpeg is required.
fn downloader(sidecars: &Sidecars, video: &VideoSettings) -> Result<Downloader, UiError> {
    let (yt_dlp, ffmpeg) = tool_paths(sidecars, video).map_err(|e| media_error(&e))?;
    let ffmpeg = ffmpeg.ok_or_else(|| UiError::key("error.media.not_installed"))?;
    Ok(Downloader::new(
        yt_dlp,
        Some(ffmpeg),
        sidecars.dir().join(".jobs"),
    ))
}

/// The `--proxy` value for a page under the app's settings.
pub(crate) fn proxy_for(settings: &AppSettings, url: &str) -> Option<String> {
    boltget_media::proxy_for(
        &settings::rules(&settings.proxy, crate::platform::system_proxy),
        url,
    )
}

fn row_state(s: MediaJobState) -> DownloadState {
    match s {
        MediaJobState::Queued => DownloadState::Queued,
        MediaJobState::Downloading => DownloadState::Downloading,
        MediaJobState::Processing => DownloadState::Processing,
        MediaJobState::Paused => DownloadState::Paused,
        MediaJobState::Completed => DownloadState::Completed,
        MediaJobState::Cancelled => DownloadState::Cancelled,
        _ => DownloadState::Failed,
    }
}

fn row_error(e: &MediaJobError) -> DownloadError {
    DownloadError {
        kind: ErrorKind::Permanent,
        key: e.key.clone(),
        message: e.message.clone(),
    }
}

/// The name a job shows before yt-dlp names its file: the title with the
/// extension it will most likely have.
fn expected_name(job: &MediaJob) -> Option<String> {
    let title = job.title.as_deref()?.trim();
    if title.is_empty() {
        return None;
    }
    let ext = match (job.preset.as_deref(), job.container) {
        (Some("audio"), _) => "m4a",
        (_, boltget_engine::ContainerPreference::Original) => "mkv",
        _ => "mp4",
    };
    Some(format!("{title}.{ext}"))
}

/// A job as a row of the list.
pub(crate) fn row(job: &MediaJob) -> DownloadRow {
    let state = row_state(job.state);
    let completed = state == DownloadState::Completed;
    let file_name = job
        .file_path
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| expected_name(job));
    DownloadRow {
        id: row_id(job.id),
        state,
        url: redacted_page(&job.page_url),
        file_name,
        path: job.file_path.as_deref().map(super::dto::path_string),
        total: job.size,
        downloaded: if completed { job.size.unwrap_or(0) } else { 0 },
        bps: 0,
        connections: 0,
        resumable: true,
        speed_limit: None,
        error: job.error.as_ref().map(row_error),
        created_at: job.created_at,
        completed_at: completed.then_some(job.updated_at),
        // No queue of the engine's: media jobs keep their own order.
        queue: QueueId(0),
        queue_position: i64::MAX,
        start_at: None,
        private: false,
        media: Some(MediaRowInfo {
            title: job.title.clone(),
            preset: job.preset.clone(),
        }),
    }
}

/// A page's address as rows show links: no query string or credentials.
fn redacted_page(url: &str) -> String {
    url::Url::parse(url).map_or_else(|_| String::new(), |u| boltget_engine::redact_url(&u))
}

/// What a stopped run does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    /// Becomes paused.
    Pause,
    /// Is being removed: nothing.
    Remove,
}

#[derive(Debug)]
struct Running {
    stop: watch::Sender<bool>,
    then: Then,
}

#[derive(Debug, Default)]
struct Runs {
    running: HashMap<MediaJobId, Running>,
    waiting: VecDeque<MediaJobId>,
    /// Files each job wrote in this session, for removing with its file.
    files: HashMap<MediaJobId, Vec<PathBuf>>,
    /// The app is quitting: runs end without changing their jobs.
    closing: bool,
}

/// The media jobs that run now. Bridge state.
#[derive(Debug, Default)]
pub(crate) struct MediaRuns {
    inner: Arc<Mutex<Runs>>,
}

fn lock(runs: &Mutex<Runs>) -> MutexGuard<'_, Runs> {
    runs.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a run needs, owned, for its task.
#[derive(Clone)]
struct Ctx {
    engine: Engine,
    hub: HubHandle,
    tools: PathBuf,
    settings: watch::Receiver<AppSettings>,
    runs: Arc<Mutex<Runs>>,
}

impl Bridge {
    /// Where the video tools live (`tools` in the data directory). Set once,
    /// at start.
    pub(crate) fn set_tools_dir(&self, dir: PathBuf) {
        let _ = self.tools_dir.set(dir);
    }

    /// The sidecar manager, with the app's proxy.
    pub(crate) fn media_sidecars(&self) -> Result<Sidecars, UiError> {
        let dir = self
            .tools_dir
            .get()
            .ok_or_else(|| UiError::key("error.no_data_dir"))?;
        Ok(sidecars(dir, &self.settings()))
    }

    fn media_ctx(&self) -> Result<Ctx, UiError> {
        Ok(Ctx {
            engine: self.engine()?.clone(),
            hub: self.hub.clone(),
            tools: self
                .tools_dir
                .get()
                .cloned()
                .ok_or_else(|| UiError::key("error.no_data_dir"))?,
            settings: self.watch_settings(),
            runs: self.media.inner.clone(),
        })
    }

    /// Whether a media job is running or waiting to.
    pub(crate) fn media_busy(&self) -> bool {
        let runs = lock(&self.media.inner);
        !runs.running.is_empty() || !runs.waiting.is_empty()
    }

    /// Records a media job, shows its row and starts it (or lets it wait
    /// for a free slot).
    pub(crate) async fn add_media(&self, job: NewMediaJob) -> Result<DownloadRow, UiError> {
        let ctx = self.media_ctx()?;
        let job = ctx.engine.add_media_job(job).await?;
        let row = row(&job);
        self.upsert(row.clone());
        launch(&ctx, job.id);
        Ok(row)
    }

    /// Stops a media job and leaves it paused (its partial files stay).
    pub(crate) async fn pause_media(&self, id: MediaJobId) -> Result<DownloadState, UiError> {
        let ctx = self.media_ctx()?;
        let job = ctx.engine.media_job(id).await?;
        {
            let mut runs = lock(&ctx.runs);
            if let Some(r) = runs.running.get_mut(&id) {
                r.then = Then::Pause;
                r.stop.send_replace(true);
                // The run reports the state once yt-dlp has stopped.
                return Ok(DownloadState::Paused);
            }
            runs.waiting.retain(|w| *w != id);
        }
        if matches!(job.state, MediaJobState::Completed | MediaJobState::Paused) {
            return Ok(row_state(job.state));
        }
        set_state(&ctx, id, MediaJobState::Paused, None).await;
        Ok(DownloadState::Paused)
    }

    /// Starts a stopped media job again (or lets it wait for a free slot);
    /// yt-dlp continues from its partial files.
    pub(crate) async fn resume_media(&self, id: MediaJobId) -> Result<(), UiError> {
        let ctx = self.media_ctx()?;
        let job = ctx.engine.media_job(id).await?;
        if job.state == MediaJobState::Completed {
            return Err(UiError::key("error.invalid_state"));
        }
        if lock(&ctx.runs).running.contains_key(&id) {
            return Ok(());
        }
        set_state(&ctx, id, MediaJobState::Queued, None).await;
        launch(&ctx, id);
        Ok(())
    }

    /// Removes a media job, stopping it first; with `delete_file`, its file
    /// too (or the partial files it wrote in this session).
    pub(crate) async fn remove_media(
        &self,
        id: MediaJobId,
        delete_file: bool,
    ) -> Result<(), UiError> {
        let ctx = self.media_ctx()?;
        let job = match ctx.engine.media_job(id).await {
            Ok(job) => Some(job),
            Err(boltget_engine::Error::UnknownMediaJob(_)) => None,
            Err(e) => return Err(e.into()),
        };
        let was_running = {
            let mut runs = lock(&ctx.runs);
            runs.waiting.retain(|w| *w != id);
            match runs.running.get_mut(&id) {
                Some(r) => {
                    r.then = Then::Remove;
                    r.stop.send_replace(true);
                    true
                }
                None => false,
            }
        };
        if was_running {
            // yt-dlp holds its files until its process has ended.
            for _ in 0..100 {
                if !lock(&ctx.runs).running.contains_key(&id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        let files = lock(&ctx.runs).files.remove(&id).unwrap_or_default();
        if let Some(job) = &job {
            if delete_file {
                delete_files(job, &files);
            }
            ctx.engine.remove_media_job(id).await?;
        }
        self.removed(row_id(id));
        Ok(())
    }

    /// The page a media job downloads from, redacted.
    pub(crate) async fn media_page(&self, id: MediaJobId) -> Result<String, UiError> {
        Ok(row(&self.engine()?.media_job(id).await?).url)
    }

    /// The rows of the stored media jobs, and those to run again: jobs that
    /// were running when the app ended (killed, or quit) or waited.
    pub(crate) async fn media_rows(engine: &Engine) -> (Vec<DownloadRow>, Vec<MediaJobId>) {
        let jobs = match engine.media_jobs().await {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::warn!(error = %e, "could not read the media jobs");
                return (Vec::new(), Vec::new());
            }
        };
        let rerun = jobs
            .iter()
            .filter(|j| {
                matches!(
                    j.state,
                    MediaJobState::Queued | MediaJobState::Downloading | MediaJobState::Processing
                )
            })
            .map(|j| j.id)
            .collect();
        (jobs.iter().map(row).collect(), rerun)
    }

    /// Runs the jobs [`Bridge::media_rows`] found unfinished.
    pub(crate) fn rerun_media(&self, ids: &[MediaJobId]) {
        let Ok(ctx) = self.media_ctx() else { return };
        for &id in ids {
            tracing::info!(job = id.0, "continuing a media download");
            launch(&ctx, id);
        }
    }

    /// The app is quitting: every yt-dlp stops, and the jobs stay as they
    /// are recorded, so the next start continues them.
    pub(crate) fn stop_media_for_exit(&self) {
        let mut runs = lock(&self.media.inner);
        runs.closing = true;
        runs.waiting.clear();
        for r in runs.running.values() {
            r.stop.send_replace(true);
        }
    }

    /// Pauses every running or waiting media job (the tray's "Pause all").
    pub(crate) async fn pause_all_media(&self) {
        let ids: Vec<MediaJobId> = {
            let runs = lock(&self.media.inner);
            runs.running
                .keys()
                .chain(runs.waiting.iter())
                .copied()
                .collect()
        };
        for id in ids {
            if let Err(e) = self.pause_media(id).await {
                tracing::warn!(job = id.0, error = ?e, "could not pause a media download");
            }
        }
    }

    /// Lets every paused media job run again (the tray's "Resume all").
    pub(crate) async fn resume_all_media(&self) {
        let Ok(engine) = self.engine() else { return };
        let Ok(jobs) = engine.media_jobs().await else {
            return;
        };
        for job in jobs.iter().filter(|j| j.state == MediaJobState::Paused) {
            if let Err(e) = self.resume_media(job.id).await {
                tracing::warn!(job = job.id.0, error = ?e, "could not resume a media download");
            }
        }
    }
}

/// Deletes a job's file, or the partial files it wrote in this session.
fn delete_files(job: &MediaJob, seen: &[PathBuf]) {
    let mut paths: Vec<PathBuf> = job.file_path.iter().cloned().collect();
    for f in seen {
        let mut part = f.clone().into_os_string();
        part.push(".part");
        let mut ytdl = f.clone().into_os_string();
        ytdl.push(".ytdl");
        paths.extend([f.clone(), part.into(), ytdl.into()]);
    }
    for p in paths {
        // Only inside the job's folder: the names came from yt-dlp.
        if p.starts_with(&job.dest_dir)
            && p.is_file()
            && let Err(e) = std::fs::remove_file(&p)
        {
            tracing::warn!(path = %p.display(), error = %e, "could not delete a media file");
        }
    }
}

/// Records a job's state (and error) and tells the page.
async fn set_state(ctx: &Ctx, id: MediaJobId, state: MediaJobState, error: Option<MediaJobError>) {
    let change = MediaJobUpdate {
        state: Some(state),
        error: Some(error.clone()),
        ..MediaJobUpdate::default()
    };
    if let Err(e) = ctx.engine.update_media_job(id, change).await {
        tracing::warn!(job = id.0, error = %e, "could not record a media job's state");
    }
    ctx.hub.send(Request::MediaState(
        row_id(id),
        row_state(state),
        error.as_ref().map(row_error),
    ));
}

/// Starts a job's run, or puts it in line when [`MAX_RUNNING`] run.
fn launch(ctx: &Ctx, id: MediaJobId) {
    let stop = {
        let mut runs = lock(&ctx.runs);
        if runs.closing || runs.running.contains_key(&id) {
            return;
        }
        if runs.running.len() >= MAX_RUNNING {
            if !runs.waiting.contains(&id) {
                runs.waiting.push_back(id);
            }
            return;
        }
        runs.waiting.retain(|w| *w != id);
        let (tx, rx) = watch::channel(false);
        runs.running.insert(
            id,
            Running {
                stop: tx,
                then: Then::Pause,
            },
        );
        rx
    };
    let ctx = ctx.clone();
    tokio::spawn(async move { run(ctx, id, stop).await });
}

/// One run of a job, from start to its end, then the next waiting job.
async fn run(ctx: Ctx, id: MediaJobId, stop: watch::Receiver<bool>) {
    let result = download(&ctx, id, stop).await;
    let (then, closing) = {
        let mut runs = lock(&ctx.runs);
        let then = runs.running.remove(&id).map_or(Then::Pause, |r| r.then);
        (then, runs.closing)
    };
    if !closing && then != Then::Remove {
        match result {
            Ok(DownloadOutcome::Completed { path, size }) => {
                let change = MediaJobUpdate {
                    state: Some(MediaJobState::Completed),
                    error: Some(None),
                    file_path: Some(Some(path.clone())),
                    size: Some(size),
                    ..MediaJobUpdate::default()
                };
                if let Err(e) = ctx.engine.update_media_job(id, change).await {
                    tracing::warn!(job = id.0, error = %e, "could not record a finished media job");
                }
                lock(&ctx.runs).files.remove(&id);
                ctx.hub
                    .send(Request::MediaCompleted(row_id(id), path, size));
            }
            Ok(DownloadOutcome::Stopped) => {
                set_state(&ctx, id, MediaJobState::Paused, None).await;
            }
            Err(e) => {
                tracing::info!(job = id.0, error = ?e, "a media download failed");
                let error = MediaJobError {
                    key: e.key,
                    message: e.message.unwrap_or_default(),
                };
                set_state(&ctx, id, MediaJobState::Failed, Some(error)).await;
            }
        }
    }
    let next = {
        let mut runs = lock(&ctx.runs);
        if runs.closing {
            None
        } else {
            runs.waiting.pop_front()
        }
    };
    if let Some(next) = next {
        launch(&ctx, next);
    }
}

/// Downloads a job's media with yt-dlp, reporting progress to the hub.
async fn download(
    ctx: &Ctx,
    id: MediaJobId,
    stop: watch::Receiver<bool>,
) -> Result<DownloadOutcome, UiError> {
    let job = ctx.engine.media_job(id).await?;
    let settings = ctx.settings.borrow().clone();
    let sidecars = sidecars(&ctx.tools, &settings);
    // yt-dlp is checked at most once a day while video is in use; a failed
    // check keeps the installed version.
    if let Err(e) = sidecars.update_if_due(SystemTime::now(), &|_| {}).await {
        tracing::debug!(error = %e, "the video tools' update check did not run");
    }
    let downloader = downloader(&sidecars, &settings.video)?;
    set_state(ctx, id, MediaJobState::Downloading, None).await;
    let mut opts = DownloadOptions::new(job.format_expression.clone(), job.dest_dir.clone());
    opts.container = job.container;
    opts.audio_only = job.preset.as_deref() == Some("audio");
    opts.subtitles.clone_from(&job.subtitles);
    opts.output_template.clone_from(&job.output_template);
    opts.expected_size = job.size;
    opts.proxy = proxy_for(&settings, &job.page_url);
    let row = row_id(id);
    let phase = Mutex::new(Phase::Downloading);
    let on_progress = |p: &MediaProgress| {
        ctx.hub.send(Request::MediaSample(Sample {
            id: row,
            downloaded: p.downloaded,
            total: p.total,
            connections: 0,
            segments: Vec::new(),
        }));
        if let Some(file) = &p.file {
            let path = job.dest_dir.join(file);
            let mut runs = lock(&ctx.runs);
            let files = runs.files.entry(id).or_default();
            if !files.contains(&path) {
                files.push(path);
            }
        }
        let mut current = phase.lock().unwrap_or_else(PoisonError::into_inner);
        if *current != p.phase {
            *current = p.phase;
            let state = match p.phase {
                Phase::Processing => MediaJobState::Processing,
                Phase::Downloading => MediaJobState::Downloading,
            };
            ctx.hub
                .send(Request::MediaState(row, row_state(state), None));
            let engine = ctx.engine.clone();
            tokio::spawn(async move {
                let change = MediaJobUpdate {
                    state: Some(state),
                    ..MediaJobUpdate::default()
                };
                let _ = engine.update_media_job(id, change).await;
            });
        }
    };
    downloader
        .run(&job.page_url, &opts, &on_progress, stop)
        .await
        .map_err(|e| media_error(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_ids_and_job_ids() {
        assert_eq!(row_id(MediaJobId(7)), DownloadId(ROW_BASE + 7));
        assert_eq!(job_id(DownloadId(ROW_BASE + 7)), Some(MediaJobId(7)));
        assert_eq!(job_id(DownloadId(7)), None);
        assert_eq!(job_id(DownloadId(ROW_BASE)), None);
        // Exact as a JavaScript number.
        assert!(row_id(MediaJobId(1_000_000)).0 < (1 << 53));
    }
}
