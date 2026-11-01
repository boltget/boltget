//! Engine commands the page invokes. Each returns `Result<_, UiError>`; the
//! effects also reach every subscribed page through the hub, so a page that
//! did not make the call stays in sync.

use std::path::{Path, PathBuf};

use boltget_engine::{AddRequest, Checksum, Destination, RequestContext, redact_url};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use super::Bridge;
use super::dto::{
    AddDownload, AppSettings, ChecksumInfo, DownloadDetails, DownloadId, DownloadRow,
    DownloadState, EngineMessage, EngineStatus, LimitScope, LinkSource, ProbePreview, SegmentSpan,
    path_string,
};
use super::hub::new_row;
use crate::error::UiError;

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
    url: String,
) -> Result<ProbePreview, UiError> {
    let engine = bridge.engine()?;
    let ctx = RequestContext::new(url.trim())?;
    let probe = engine.probe(&ctx).await?;
    Ok(ProbePreview::from(&probe))
}

/// Starts a download. Returns its row at once, so the list can show it
/// before the engine's first event arrives.
#[tauri::command]
#[specta::specta]
pub(crate) async fn add_download(
    app: AppHandle,
    bridge: State<'_, Bridge>,
    request: AddDownload,
) -> Result<DownloadRow, UiError> {
    let engine = bridge.engine()?;
    let ctx = RequestContext::new(request.url.trim())?;
    let dir = match request.directory.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => app
            .path()
            .download_dir()
            .map_err(|e| UiError::with_message("error.no_download_folder", e.to_string()))?,
    };
    let destination = match request.file_name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => Destination::File(dir.join(plain_file_name(name)?)),
        _ => Destination::Directory(dir),
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
    let handle = engine.add(req);
    let mut row = new_row(handle.id().into(), url);
    row.speed_limit = request.speed_limit.filter(|&r| r > 0);
    bridge.upsert(row.clone());
    Ok(row)
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

/// Continues a stopped download; `restart` throws its progress away first.
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
        LimitScope::Global => bridge.settings_changed(AppSettings { speed_limit: limit }),
        LimitScope::Download { id } => bridge.speed_limit(id, limit),
    }
    Ok(())
}

/// The settings the engine keeps.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_settings(bridge: State<'_, Bridge>) -> Result<AppSettings, UiError> {
    Ok(AppSettings::from(&bridge.engine()?.settings().await?))
}

/// Replaces the settings; returns them as stored.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_settings(
    bridge: State<'_, Bridge>,
    settings: AppSettings,
) -> Result<AppSettings, UiError> {
    let engine = bridge.engine()?;
    engine
        .set_limit(boltget_engine::LimitScope::Global, settings.speed_limit)
        .await?;
    let stored = AppSettings::from(&engine.settings().await?);
    bridge.settings_changed(stored.clone());
    Ok(stored)
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
    let summary = summary(bridge.engine()?, id).await?;
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
    let summary = summary(bridge.engine()?, id).await?;
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
