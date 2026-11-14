//! Engine commands the page invokes. Each returns `Result<_, UiError>`; the
//! effects also reach every subscribed page through the hub, so a page that
//! did not make the call stays in sync.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use boltget_engine::{AddRequest, Checksum, Destination, RequestContext, redact_url};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use super::Bridge;
use super::dto::{
    AddDefaults, AddDownload, AppSettings, ChecksumInfo, DestinationCheck, DownloadDetails,
    DownloadId, DownloadRow, DownloadState, EngineMessage, EngineStatus, LimitScope, LinkSource,
    ProbePreview, QueueId, QueueInfo, QueuePatch, SegmentSpan, ShiftDirection, path_string,
};
use super::hub::new_row;
use crate::browser::Captured;
use crate::error::UiError;
use crate::settings::{self, SettingsPatch, SystemProxyInfo};

/// Whether the engine is running, and if not, why.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn engine_status(bridge: State<'_, Bridge>) -> EngineStatus {
    bridge.status()
}

/// Streams the engine to this page: first a snapshot, then one batch per
/// tick while anything changes (docs/ARCHITECTURE.md, "UI to backend").
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn subscribe_engine(bridge: State<'_, Bridge>, on_message: Channel<EngineMessage>) {
    bridge.subscribe(Box::new(on_message));
}

/// The downloads, oldest first, with live progress.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_downloads(bridge: State<'_, Bridge>) -> Result<Vec<DownloadRow>, UiError> {
    bridge.engine()?;
    Ok(bridge.list().await)
}

/// Looks at a link without downloading it, for the add sheet's preview.
#[tauri::command]
#[specta::specta]
pub(crate) async fn probe_url(
    bridge: State<'_, Bridge>,
    captured: State<'_, Captured>,
    url: String,
) -> Result<ProbePreview, UiError> {
    let engine = bridge.engine()?;
    // A link the browser handed over is probed as the browser asked for it.
    let ctx = match captured.get(url.trim()) {
        Some(ctx) => ctx,
        None => RequestContext::new(url.trim())?,
    };
    let probe = engine.probe(&ctx).await?;
    Ok(ProbePreview::from(&probe))
}

/// Adds a download to its queue (the default queue unless the page names
/// one): it starts when its turn comes, or at `startAt`, or stays paused
/// with `paused`. Returns its row at once, so the list can show it before
/// the engine's first event arrives.
#[tauri::command]
#[specta::specta]
pub(crate) async fn add_download(
    app: AppHandle,
    bridge: State<'_, Bridge>,
    captured: State<'_, Captured>,
    request: AddDownload,
) -> Result<DownloadRow, UiError> {
    let engine = bridge.engine()?;
    // A link the browser handed over keeps its cookies and headers (and
    // gives them up once it is added).
    let ctx = match captured.get(request.url.trim()) {
        Some(ctx) => ctx,
        None => RequestContext::new(request.url.trim())?,
    };
    let dir = match request.directory.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => default_folder(&app, &bridge, request.file_name.as_deref())?,
    };
    let destination = match request.file_name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => Destination::File(dir.join(plain_file_name(name)?)),
        _ => Destination::Directory(dir.clone()),
    };
    let url = redact_url(ctx.url());
    let mut req = AddRequest::new(ctx, destination).speed_limit(request.speed_limit);
    if let Some(n) = request.connections {
        req = req.connections(n);
    }
    if let Some(c) = request.checksum.as_deref().map(str::trim)
        && !c.is_empty()
    {
        let checksum =
            Checksum::parse(c).map_err(|e| UiError::with_message("error.invalid_checksum", e))?;
        req = req.checksum(checksum);
    }
    let queue = request
        .queue
        .unwrap_or(QueueId(boltget_engine::DEFAULT_QUEUE.0));
    req = req.queue(queue.into());
    let start_at = request
        .start_at
        .map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
    if let Some(at) = start_at {
        req = req.start_at(at);
    }
    let paused = request.paused.unwrap_or(false);
    let id = if paused {
        engine.add_paused(req)?
    } else {
        engine.enqueue(req).await?
    };
    let mut row = new_row(id.into(), url);
    row.speed_limit = request.speed_limit.filter(|&r| r > 0);
    row.queue = queue;
    if paused {
        row.state = DownloadState::Paused;
    } else {
        row.start_at = request.start_at;
    }
    bridge.upsert(row.clone());
    captured.take(request.url.trim());
    remember_folder(engine, &dir).await;
    Ok(row)
}

/// Where a download goes when the page names no folder: its category's
/// folder, the default download folder, or Windows' Downloads folder.
fn default_folder(
    app: &AppHandle,
    bridge: &Bridge,
    name: Option<&str>,
) -> Result<PathBuf, UiError> {
    let downloads = app.path().download_dir().ok();
    settings::folder_for(&bridge.settings(), name, downloads.as_deref())
        .ok_or_else(|| UiError::key("error.no_download_folder"))
}

/// How many folders the add sheet offers (APP-DESIGN #6).
const RECENT_FOLDERS: usize = 3;
/// Where they are kept (an `app.` setting in the engine's database).
const RECENT_FOLDERS_KEY: &str = "recent_folders";

async fn recent_folders(engine: &boltget_engine::Engine) -> Vec<String> {
    match engine.app_setting(RECENT_FOLDERS_KEY).await {
        Ok(Some(value)) => serde_json::from_value(value).unwrap_or_default(),
        Ok(None) => Vec::new(),
        Err(e) => {
            tracing::warn!(error = %e, "could not read the recent folders");
            Vec::new()
        }
    }
}

/// Puts `dir` first among the recent folders. A failure only costs the
/// suggestion, so it is logged, not reported.
async fn remember_folder(engine: &boltget_engine::Engine, dir: &Path) {
    let dir = path_string(&std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf()));
    let mut folders = recent_folders(engine).await;
    folders.retain(|f| !f.eq_ignore_ascii_case(&dir));
    folders.insert(0, dir);
    folders.truncate(RECENT_FOLDERS);
    if let Err(e) = engine
        .set_app_setting(RECENT_FOLDERS_KEY, Some(serde_json::json!(folders)))
        .await
    {
        tracing::warn!(error = %e, "could not remember the folder");
    }
}

/// The Downloads folder and the recent folders, for the add sheet.
#[tauri::command]
#[specta::specta]
pub(crate) async fn add_defaults(
    app: AppHandle,
    bridge: State<'_, Bridge>,
) -> Result<AddDefaults, UiError> {
    let engine = bridge.engine()?;
    Ok(AddDefaults {
        downloads_folder: default_folder(&app, &bridge, None)
            .ok()
            .map(|p| path_string(&p)),
        recent_folders: recent_folders(engine).await,
    })
}

/// The name a download would be saved under in `directory` (the Downloads
/// folder when absent), and the free space there, for the add sheet.
#[tauri::command]
#[specta::specta]
pub(crate) async fn check_destination(
    app: AppHandle,
    bridge: State<'_, Bridge>,
    directory: Option<String>,
    file_name: String,
) -> Result<DestinationCheck, UiError> {
    let engine = bridge.engine()?;
    let dir = match directory.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => default_folder(&app, &bridge, Some(file_name.trim()))?,
    };
    plain_file_name(file_name.trim())?;
    let path = engine
        .free_path(&dir, file_name.trim())
        .ok_or_else(|| UiError::key("error.invalid_file_name"))?;
    let saved_as = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The nearest folder that exists tells which drive the file lands on.
    let free_bytes = dir
        .ancestors()
        .find(|d| d.is_dir())
        .and_then(boltget_engine::available_space);
    Ok(DestinationCheck {
        renamed: saved_as != file_name.trim(),
        file_name: saved_as,
        folder_exists: dir.is_dir(),
        free_bytes,
    })
}

/// Asks for a folder with the system's folder picker, starting in `start`;
/// `null` if the user cancels.
#[tauri::command]
#[specta::specta]
pub(crate) async fn choose_folder(
    window: tauri::WebviewWindow,
    start: Option<String>,
) -> Result<Option<String>, UiError> {
    use tauri_plugin_dialog::DialogExt as _;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = window.dialog().file().set_parent(&window);
    if let Some(dir) = start.map(PathBuf::from).filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = rx
        .await
        .map_err(|e| UiError::with_message("error.unknown", e.to_string()))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| path_string(&p)))
}

/// A file name the user typed, without any folder in it.
fn plain_file_name(name: &str) -> Result<&str, UiError> {
    let ok = Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name)
        && !name.contains(['/', '\\', ':']);
    if ok {
        Ok(name)
    } else {
        Err(UiError::key("error.invalid_file_name"))
    }
}

/// Pauses a download after its final checkpoint; returns the state it is in.
#[tauri::command]
#[specta::specta]
pub(crate) async fn pause_download(
    bridge: State<'_, Bridge>,
    id: DownloadId,
) -> Result<DownloadState, UiError> {
    let state: DownloadState = bridge.engine()?.pause(id.into()).await?.into();
    // A download that was not running changes state without an event.
    bridge.set_state(id, state);
    Ok(state)
}

/// Starts a stopped or waiting download now, whatever its queue does (it
/// does not wait for a free slot); `restart` throws its progress away
/// first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn resume_download(
    bridge: State<'_, Bridge>,
    id: DownloadId,
    restart: bool,
) -> Result<(), UiError> {
    let engine = bridge.engine()?;
    engine
        .resume(
            id.into(),
            boltget_engine::ResumeOptions::default().restart(restart),
        )
        .await?;
    Ok(())
}

/// Lets a stopped download (paused, failed, cancelled) wait in its queue
/// again instead of starting it now; returns the state it is in.
#[tauri::command]
#[specta::specta]
pub(crate) async fn requeue_download(
    bridge: State<'_, Bridge>,
    id: DownloadId,
) -> Result<DownloadState, UiError> {
    let state: DownloadState = bridge.engine()?.requeue(id.into()).await?.into();
    bridge.set_state(id, state);
    Ok(state)
}

/// Sets when a download starts on its own (Unix milliseconds; `null`: when
/// its turn comes). A stopped download given a time waits for it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_start_at(
    bridge: State<'_, Bridge>,
    id: DownloadId,
    at: Option<u64>,
) -> Result<DownloadState, UiError> {
    let at = at.map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
    let state: DownloadState = bridge.engine()?.set_start_at(id.into(), at).await?.into();
    bridge.set_state(id, state);
    Ok(state)
}

/// Creates a queue (last, started, with the default limit).
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_queue(
    bridge: State<'_, Bridge>,
    name: String,
) -> Result<QueueInfo, UiError> {
    let q = bridge.engine()?.create_queue(&name).await?;
    Ok(QueueInfo::from(&q))
}

/// Changes a queue's settings; refused changes (a name taken, a limit out
/// of range) return an `error.queue.*` key and change nothing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_queue(
    bridge: State<'_, Bridge>,
    id: QueueId,
    patch: QueuePatch,
) -> Result<QueueInfo, UiError> {
    let q = bridge
        .engine()?
        .update_queue(id.into(), (&patch).into())
        .await?;
    Ok(QueueInfo::from(&q))
}

/// Deletes a queue; its downloads move to the end of the default queue.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_queue(bridge: State<'_, Bridge>, id: QueueId) -> Result<(), UiError> {
    bridge.engine()?.delete_queue(id.into()).await?;
    Ok(())
}

/// Starts a queue: it starts its waiting downloads as slots free.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_queue(bridge: State<'_, Bridge>, id: QueueId) -> Result<(), UiError> {
    bridge.engine()?.start_queue(id.into()).await?;
    Ok(())
}

/// Stops a queue: its running downloads go back to waiting in their place.
#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_queue(bridge: State<'_, Bridge>, id: QueueId) -> Result<(), UiError> {
    bridge.engine()?.stop_queue(id.into()).await?;
    Ok(())
}

/// Moves downloads into a queue, before `before` or to the end, in the
/// order given (dragging rows, "Move to queue").
#[tauri::command]
#[specta::specta]
pub(crate) async fn move_downloads(
    bridge: State<'_, Bridge>,
    ids: Vec<DownloadId>,
    queue: QueueId,
    before: Option<DownloadId>,
) -> Result<(), UiError> {
    let ids: Vec<boltget_engine::DownloadId> = ids.into_iter().map(Into::into).collect();
    bridge
        .engine()?
        .move_downloads(&ids, queue.into(), before.map(Into::into))
        .await?;
    Ok(())
}

/// Moves a download one place up or down in its queue, or to the top or
/// the bottom (Alt+arrow keys).
#[tauri::command]
#[specta::specta]
pub(crate) async fn shift_download(
    bridge: State<'_, Bridge>,
    id: DownloadId,
    direction: ShiftDirection,
) -> Result<(), UiError> {
    bridge.engine()?.shift(id.into(), direction.into()).await?;
    Ok(())
}

/// Cancels a finished queue's sleep or shut down while it counts down.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn cancel_finish_action(bridge: State<'_, Bridge>) {
    bridge.cancel_finish_action();
}

/// Removes a download from the list (pausing it first), and with
/// `delete_file` its partial or completed file too.
#[tauri::command]
#[specta::specta]
pub(crate) async fn remove_download(
    bridge: State<'_, Bridge>,
    id: DownloadId,
    delete_file: bool,
) -> Result<(), UiError> {
    let result = bridge.engine()?.remove(id.into(), delete_file).await;
    match result {
        Ok(_) => {
            bridge.removed(id);
            Ok(())
        }
        Err(e) => {
            if matches!(e, boltget_engine::Error::UnknownDownload(_)) {
                bridge.removed(id);
            }
            Err(e.into())
        }
    }
}

/// Gives a stopped download a new link, after checking it is the same file.
/// The download stays paused; resume it to continue.
#[tauri::command]
#[specta::specta]
pub(crate) async fn refresh_link(
    bridge: State<'_, Bridge>,
    id: DownloadId,
    url: String,
) -> Result<(), UiError> {
    let engine = bridge.engine()?;
    let ctx = RequestContext::new(url.trim())?;
    engine.refresh_link(id.into(), ctx).await?;
    // The link, state and error changed in the database only.
    let list = engine.list().await?;
    if let Some(s) = list.iter().find(|s| DownloadId::from(s.id) == id) {
        bridge.upsert(DownloadRow::from(s));
    }
    Ok(())
}

/// Changes the global or one download's speed limit; `null` or 0 removes it.
/// Takes effect at once, without reconnecting.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_speed_limit(
    bridge: State<'_, Bridge>,
    scope: LimitScope,
    bytes_per_second: Option<u64>,
) -> Result<(), UiError> {
    let engine = bridge.engine()?;
    engine.set_limit(scope.into(), bytes_per_second).await?;
    let limit = bytes_per_second.filter(|&r| r > 0);
    match scope {
        LimitScope::Global => bridge.limit_changed(limit),
        LimitScope::Download { id } => bridge.speed_limit(id, limit),
    }
    Ok(())
}

/// Every setting, as stored (docs/APP-DESIGN.md, "Settings as built").
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn get_settings(bridge: State<'_, Bridge>) -> Result<AppSettings, UiError> {
    bridge.engine()?;
    Ok(bridge.settings())
}

/// Changes some settings; returns them all as stored, or what is wrong
/// with the change (a `settings.problem.*` key) without changing anything.
/// The change is in effect when this returns. The global speed limit is
/// changed with `set_speed_limit`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_settings(
    bridge: State<'_, Bridge>,
    patch: SettingsPatch,
) -> Result<AppSettings, UiError> {
    bridge.update_settings(&patch).await
}

/// What Windows' proxy settings say now, for the Network section.
#[tauri::command]
#[specta::specta]
pub(crate) fn system_proxy() -> SystemProxyInfo {
    SystemProxyInfo::from(&crate::platform::system_proxy())
}

/// Opens the default download folder in Explorer (the palette, the tray).
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn open_downloads_folder(
    app: AppHandle,
    bridge: State<'_, Bridge>,
) -> Result<(), UiError> {
    open_default_folder(&app, &bridge)
}

/// Opens the default download folder, creating it if it is missing.
pub(crate) fn open_default_folder(app: &AppHandle, bridge: &Bridge) -> Result<(), UiError> {
    let dir = default_folder(app, bridge, None)?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| UiError::with_message("error.open_failed", e.to_string()))?;
    tauri_plugin_opener::open_path(&dir, None::<&str>)
        .map_err(|e| UiError::with_message("error.open_failed", e.to_string()))
}

/// The file map of a running download, now (`null` if it is not running).
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn download_segments(
    bridge: State<'_, Bridge>,
    id: DownloadId,
) -> Result<Option<Vec<SegmentSpan>>, UiError> {
    let progress = bridge.engine()?.progress(&[id.into()]);
    Ok(progress
        .first()
        .map(|p| p.segments.iter().map(SegmentSpan::from).collect()))
}

/// What the details pane shows beyond the row: checksum, links used and the
/// partial file.
#[tauri::command]
#[specta::specta]
pub(crate) async fn download_details(
    bridge: State<'_, Bridge>,
    id: DownloadId,
) -> Result<DownloadDetails, UiError> {
    let engine = bridge.engine()?;
    let summary = summary(engine, id).await?;
    let sources = engine
        .sources(id.into())
        .await?
        .into_iter()
        .map(|(url, role)| LinkSource { url, role })
        .collect();
    Ok(DownloadDetails {
        id,
        partial_path: summary.partial_path.as_deref().map(path_string),
        checksum: summary.checksum.as_ref().map(ChecksumInfo::from),
        sources,
    })
}

/// Opens a completed download with the program Windows associates with it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn open_download(
    bridge: State<'_, Bridge>,
    id: DownloadId,
) -> Result<(), UiError> {
    open_file(bridge.engine()?, id).await
}

/// Opens a completed download's file (the window and notifications).
pub(crate) async fn open_file(
    engine: &boltget_engine::Engine,
    id: DownloadId,
) -> Result<(), UiError> {
    let summary = summary(engine, id).await?;
    if summary.state != boltget_engine::State::Completed {
        return Err(UiError::key("error.invalid_state"));
    }
    let path = summary
        .path
        .filter(|p| p.is_file())
        .ok_or_else(|| UiError::key("error.file_missing"))?;
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|e| UiError::with_message("error.open_failed", e.to_string()))
}

/// Shows a download in its folder: the file selected once it exists (the
/// partial file while it is not completed), otherwise the folder itself.
#[tauri::command]
#[specta::specta]
pub(crate) async fn show_download_in_folder(
    bridge: State<'_, Bridge>,
    id: DownloadId,
) -> Result<(), UiError> {
    reveal_file(bridge.engine()?, id).await
}

/// Shows a download's file in its folder (the window and notifications).
pub(crate) async fn reveal_file(
    engine: &boltget_engine::Engine,
    id: DownloadId,
) -> Result<(), UiError> {
    let summary = summary(engine, id).await?;
    let file = [summary.path.as_deref(), summary.partial_path.as_deref()]
        .into_iter()
        .flatten()
        .find(|p| p.is_file());
    let result = if let Some(file) = file {
        tauri_plugin_opener::reveal_item_in_dir(file)
    } else {
        let folder = summary
            .path
            .as_deref()
            .and_then(Path::parent)
            .filter(|d| d.is_dir())
            .ok_or_else(|| UiError::key("error.file_missing"))?;
        tauri_plugin_opener::open_path(folder, None::<&str>)
    };
    result.map_err(|e| UiError::with_message("error.open_failed", e.to_string()))
}

/// One download as the database has it; the page only ever names an id, so
/// it can open and reveal nothing but downloads (docs/ARCHITECTURE.md,
/// Security).
async fn summary(
    engine: &boltget_engine::Engine,
    id: DownloadId,
) -> Result<boltget_engine::DownloadSummary, UiError> {
    engine
        .list()
        .await?
        .into_iter()
        .find(|s| DownloadId::from(s.id) == id)
        .ok_or_else(|| UiError::key("error.unknown_download"))
}

/// Which download's file map streams with its progress (the one in the
/// details pane); `null` for none.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn set_focused_download(bridge: State<'_, Bridge>, id: Option<DownloadId>) {
    bridge.focus(id);
}

/// The page reports whether it is visible: hidden pages get no batches, and
/// a snapshot when they are shown again (docs/APP-DESIGN.md #30).
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn set_ui_visible(bridge: State<'_, Bridge>, visible: bool) {
    bridge.visible(visible);
}

#[cfg(test)]
mod tests {
    use super::plain_file_name;

    #[test]
    fn typed_file_names_cannot_name_folders() {
        assert!(plain_file_name("ubuntu.iso").is_ok());
        assert!(plain_file_name("a b (1).tar.gz").is_ok());
        for bad in ["..", ".", "a/b", "a\\b", "C:x", "C:\\x", "/etc/passwd"] {
            assert!(plain_file_name(bad).is_err(), "{bad}");
        }
    }
}
