//! Video and audio from web pages (docs/VIDEO.md; the window's side is in
//! docs/APP-DESIGN.md, "Video as built"): the commands of the add flow's
//! picker, the first-use install sheet and Settings, Video. Media downloads
//! themselves run in the bridge (`bridge::media`), as rows of the list.

use std::time::{SystemTime, UNIX_EPOCH};

use boltget_engine::{ContainerPreference, NewMediaJob, SubtitleChoice};
use boltget_media::{
    Channel as ToolChannel, Format, InstallEvent, MediaInfo, Program, ResolvedPreset, Tool,
    codec_label, presets,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::bridge::dto::DownloadRow;
use crate::bridge::{Bridge, media};
use crate::error::UiError;
use crate::settings::{MediaContainer, SettingsPatch, VideoPatch};

fn media_error(e: &boltget_media::Error) -> UiError {
    UiError::with_message(e.message_key(), e.to_string())
}

/// A video tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaTool {
    YtDlp,
    Ffmpeg,
}

impl From<Tool> for MediaTool {
    fn from(t: Tool) -> Self {
        match t {
            Tool::YtDlp => Self::YtDlp,
            Tool::Ffmpeg => Self::Ffmpeg,
        }
    }
}

impl From<MediaTool> for Tool {
    fn from(t: MediaTool) -> Self {
        match t {
            MediaTool::YtDlp => Self::YtDlp,
            MediaTool::Ffmpeg => Self::Ffmpeg,
        }
    }
}

/// yt-dlp's release channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaChannel {
    Stable,
    Nightly,
}

/// One tool, for Settings, Video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaToolInfo {
    pub(crate) tool: MediaTool,
    /// The installed version in use.
    pub(crate) version: Option<String>,
    /// The version kept for rollback.
    pub(crate) previous: Option<String>,
    /// When updates were last checked (Unix milliseconds).
    pub(crate) last_check: Option<u64>,
    /// The program the user chose instead, if any.
    pub(crate) custom_path: Option<String>,
    /// What that program reports as its version (`null`: it did not start).
    pub(crate) custom_version: Option<String>,
    /// Whether Boltget has a build of it for this system.
    pub(crate) supported: bool,
}

/// The video tools, for the install sheet and Settings, Video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaToolsInfo {
    /// Both tools are there (installed or chosen): video downloads work.
    pub(crate) ready: bool,
    pub(crate) channel: MediaChannel,
    pub(crate) tools: Vec<MediaToolInfo>,
}

/// What installing a missing tool downloads, from its release metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaToolSize {
    pub(crate) tool: MediaTool,
    pub(crate) version: String,
    /// Bytes.
    pub(crate) size: u64,
}

/// A step of an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaInstallStep {
    Checking,
    Downloading,
    Verifying,
    Unpacking,
    Testing,
    Done,
}

/// Progress of an install, streamed while `install_media_tools` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaInstallEvent {
    pub(crate) tool: MediaTool,
    pub(crate) step: MediaInstallStep,
    /// Bytes downloaded, while downloading.
    pub(crate) done: u64,
    pub(crate) total: Option<u64>,
}

impl MediaInstallEvent {
    fn from_event(e: &InstallEvent) -> Option<Self> {
        let (tool, step, done, total) = match e {
            InstallEvent::Checking { tool } => (*tool, MediaInstallStep::Checking, 0, None),
            InstallEvent::Downloading {
                tool, done, total, ..
            } => {
                // Release metadata, checksums and signatures are small; only
                // the tool itself is worth a bar.
                if total.is_none_or(|t| t < 1024 * 1024) {
                    return None;
                }
                (*tool, MediaInstallStep::Downloading, *done, *total)
            }
            InstallEvent::Verifying { tool } => (*tool, MediaInstallStep::Verifying, 0, None),
            InstallEvent::Unpacking { tool } => (*tool, MediaInstallStep::Unpacking, 0, None),
            InstallEvent::SmokeTesting { tool, .. } => (*tool, MediaInstallStep::Testing, 0, None),
            InstallEvent::Activated { tool, .. } => (*tool, MediaInstallStep::Done, 0, None),
            _ => return None,
        };
        Some(Self {
            tool: tool.into(),
            step,
            done,
            total,
        })
    }
}

/// What "Look for media on this page" found.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum MediaProbe {
    /// yt-dlp or FFmpeg is missing: the install sheet comes first. Nothing
    /// was run.
    NotInstalled,
    /// yt-dlp described the page.
    Found {
        /// What it found.
        media: Box<MediaSummary>,
    },
}

/// A page's media, as the picker shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSummary {
    /// The page, as yt-dlp names it (else as given).
    pub(crate) url: String,
    /// The site, for remembering the last preset (`youtube.com`).
    pub(crate) site: String,
    pub(crate) title: Option<String>,
    pub(crate) uploader: Option<String>,
    /// Seconds.
    pub(crate) duration: Option<f64>,
    /// The thumbnail's address; `media_thumbnail` fetches it.
    pub(crate) thumbnail: Option<String>,
    /// The presets for "Prefer MP4", best first; empty when there is
    /// nothing to download (no media, DRM, or a playlist).
    pub(crate) presets: Vec<MediaPreset>,
    /// The same for "Prefer original".
    pub(crate) original_presets: Vec<MediaPreset>,
    /// Every format, for the advanced table, best last as yt-dlp lists them.
    pub(crate) formats: Vec<MediaFormat>,
    pub(crate) subtitles: Vec<MediaSubtitle>,
    pub(crate) is_playlist: bool,
    pub(crate) entry_count: u32,
    /// Everything on the page is DRM-protected.
    pub(crate) drm: bool,
    /// The preset last used on this site, if any.
    pub(crate) remembered: Option<String>,
}

/// One preset with what it resolves to.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaPreset {
    /// `best`, `1080p`, `720p`, `small` or `audio`.
    pub(crate) preset: String,
    /// The yt-dlp format expression.
    pub(crate) expression: String,
    pub(crate) audio_only: bool,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) fps: Option<f64>,
    /// The file's extension (`mp4`, `m4a`, ...).
    pub(crate) container: String,
    /// Everyday codec names (`H.264`, `AAC`).
    pub(crate) video_codec: Option<String>,
    pub(crate) audio_codec: Option<String>,
    /// Bytes, when known.
    pub(crate) size: Option<u64>,
    /// The size is an estimate ("about").
    pub(crate) size_approximate: bool,
}

/// One format of the page, for the advanced table.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaFormat {
    /// yt-dlp's format id.
    pub(crate) id: String,
    pub(crate) ext: Option<String>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) fps: Option<f64>,
    /// Everyday names; `null` when the format has no such stream.
    pub(crate) video_codec: Option<String>,
    pub(crate) audio_codec: Option<String>,
    /// Kilobits per second.
    pub(crate) bitrate: Option<f64>,
    pub(crate) size: Option<u64>,
    pub(crate) size_approximate: bool,
    /// `https`, `m3u8_native`, `http_dash_segments`, ...
    pub(crate) protocol: Option<String>,
    pub(crate) note: Option<String>,
}

/// A subtitle language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSubtitle {
    pub(crate) code: String,
    pub(crate) name: Option<String>,
    /// Automatic captions rather than subtitles.
    pub(crate) automatic: bool,
}

/// Subtitles to download with a video.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSubtitlesChoice {
    /// Language codes as the page names them.
    pub(crate) languages: Vec<String>,
    /// Automatic captions count for languages without subtitles.
    pub(crate) automatic: bool,
    /// In the video rather than next to it.
    pub(crate) embed: bool,
}

/// A media download to start.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaRequest {
    /// The page.
    pub(crate) url: String,
    /// The yt-dlp format expression.
    pub(crate) expression: String,
    /// The preset it came from; absent for the advanced table's choice.
    #[specta(optional)]
    pub(crate) preset: Option<String>,
    pub(crate) container: MediaContainer,
    #[specta(optional)]
    pub(crate) subtitles: Option<MediaSubtitlesChoice>,
    /// The title the probe found.
    #[specta(optional)]
    pub(crate) title: Option<String>,
    /// The expected size in bytes.
    #[specta(optional)]
    pub(crate) size: Option<u64>,
    /// The site, to remember the preset for.
    #[specta(optional)]
    pub(crate) site: Option<String>,
}

impl From<&ResolvedPreset> for MediaPreset {
    fn from(r: &ResolvedPreset) -> Self {
        Self {
            preset: r.preset.id().to_owned(),
            expression: r.expression.clone(),
            audio_only: r.audio_only,
            width: r.width,
            height: r.height,
            fps: r.fps,
            container: r.container.clone(),
            video_codec: r.video_codec.as_deref().map(codec_label),
            audio_codec: r.audio_codec.as_deref().map(codec_label),
            size: r.size.map(|s| s.bytes),
            size_approximate: r.size.is_some_and(|s| s.approximate),
        }
    }
}

impl From<&Format> for MediaFormat {
    fn from(f: &Format) -> Self {
        let size = f.size();
        Self {
            id: f.format_id.clone(),
            ext: f.ext.clone(),
            width: f.width,
            height: f.height,
            fps: f.fps,
            video_codec: f
                .has_video()
                .then(|| f.vcodec.name().map_or_else(|| "?".to_owned(), codec_label)),
            audio_codec: f
                .has_audio()
                .then(|| f.acodec.name().map_or_else(|| "?".to_owned(), codec_label)),
            bitrate: f.tbr.or_else(|| match (f.vbr, f.abr) {
                (None, None) => None,
                (v, a) => Some(v.unwrap_or(0.0) + a.unwrap_or(0.0)),
            }),
            size: size.map(|(b, _)| b),
            size_approximate: size.is_some_and(|(_, approx)| approx),
            protocol: f.protocol.clone(),
            note: f.format_note.clone(),
        }
    }
}

/// The site of a page, for remembering presets: its host without `www.`.
pub(crate) fn site_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .map(|h| h.strip_prefix("www.").map(str::to_owned).unwrap_or(h))
        .unwrap_or_default()
}

impl MediaSummary {
    fn new(info: &MediaInfo, url: &str, remembered: Option<String>) -> Self {
        let page = info.webpage_url.clone().unwrap_or_else(|| url.to_owned());
        Self {
            site: site_of(&page),
            url: page,
            title: info.title.clone(),
            uploader: info.publisher().map(str::to_owned),
            duration: info.duration,
            thumbnail: info.best_thumbnail().map(str::to_owned),
            presets: presets(info, ContainerPreference::Mp4)
                .iter()
                .map(MediaPreset::from)
                .collect(),
            original_presets: presets(info, ContainerPreference::Original)
                .iter()
                .map(MediaPreset::from)
                .collect(),
            formats: info
                .formats
                .iter()
                // Storyboards and the like carry neither video nor audio.
                .filter(|f| !f.has_drm && (f.has_video() || f.has_audio()))
                .filter(|f| f.ext.as_deref() != Some("mhtml"))
                .map(MediaFormat::from)
                .collect(),
            subtitles: info
                .subtitle_languages()
                .into_iter()
                .map(|l| MediaSubtitle {
                    code: l.code,
                    name: l.name,
                    automatic: l.automatic,
                })
                .collect(),
            is_playlist: info.is_playlist(),
            entry_count: u32::try_from(info.entry_count()).unwrap_or(u32::MAX),
            drm: info.drm_only(),
            remembered,
        }
    }
}

/// Where the last preset per site is kept (an `app.` setting).
const PRESETS_KEY: &str = "media_presets";
/// Sites remembered, most recent first.
const MAX_SITES: usize = 200;

async fn remembered_presets(bridge: &Bridge) -> Vec<(String, String)> {
    let Ok(engine) = bridge.engine() else {
        return Vec::new();
    };
    match engine.app_setting(PRESETS_KEY).await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_default(),
        _ => Vec::new(),
    }
}

async fn remember_preset(bridge: &Bridge, site: &str, preset: &str) {
    if site.is_empty() || boltget_media::Preset::parse(preset).is_none() {
        return;
    }
    let Ok(engine) = bridge.engine() else { return };
    let mut list = remembered_presets(bridge).await;
    list.retain(|(s, _)| s != site);
    list.insert(0, (site.to_owned(), preset.to_owned()));
    list.truncate(MAX_SITES);
    if let Err(e) = engine
        .set_app_setting(PRESETS_KEY, Some(serde_json::json!(list)))
        .await
    {
        tracing::warn!(error = %e, "could not remember the preset");
    }
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The tools' state, with the user's own programs asked for their version.
async fn tools_info(bridge: &Bridge) -> Result<MediaToolsInfo, UiError> {
    let sidecars = bridge.media_sidecars()?;
    let status = sidecars.status().map_err(|e| media_error(&e))?;
    let video = bridge.settings().video;
    let mut tools = Vec::new();
    let mut channel = MediaChannel::Stable;
    for t in &status.tools {
        let (custom, program) = match t.tool {
            Tool::YtDlp => (video.yt_dlp_path.clone(), Program::YtDlp),
            Tool::Ffmpeg => (video.ffmpeg_path.clone(), Program::Ffmpeg),
        };
        let custom_version = match &custom {
            Some(p) => sidecars
                .check_custom(program, std::path::Path::new(p))
                .await
                .ok(),
            None => None,
        };
        if t.tool == Tool::YtDlp && t.channel == ToolChannel::Nightly {
            channel = MediaChannel::Nightly;
        }
        tools.push(MediaToolInfo {
            tool: t.tool.into(),
            version: t.active.clone(),
            previous: t.previous.clone(),
            last_check: t.last_check.map(unix_ms),
            custom_path: custom,
            custom_version,
            supported: t.supported,
        });
    }
    Ok(MediaToolsInfo {
        ready: media::tools_ready(&sidecars, &video),
        channel,
        tools,
    })
}

/// The video tools: what is installed, what the user chose, the channel.
#[tauri::command]
#[specta::specta]
pub(crate) async fn media_tools(bridge: State<'_, Bridge>) -> Result<MediaToolsInfo, UiError> {
    tools_info(&bridge).await
}

/// What installing the missing tools downloads: each tool's version and
/// size, read from its release metadata now (yt-dlp: GitHub's release
/// API; FFmpeg: the build Boltget pins).
#[tauri::command]
#[specta::specta]
pub(crate) async fn media_install_plan(
    bridge: State<'_, Bridge>,
) -> Result<Vec<MediaToolSize>, UiError> {
    let sidecars = bridge.media_sidecars()?;
    let status = sidecars.status().map_err(|e| media_error(&e))?;
    let video = bridge.settings().video;
    let mut plan = Vec::new();
    for t in status.tools.iter().filter(|t| t.active.is_none()) {
        let custom = match t.tool {
            Tool::YtDlp => video.yt_dlp_path.is_some(),
            Tool::Ffmpeg => video.ffmpeg_path.is_some(),
        };
        if custom {
            continue;
        }
        let release = sidecars.latest(t.tool).await.map_err(|e| media_error(&e))?;
        plan.push(MediaToolSize {
            tool: t.tool.into(),
            version: release.version,
            size: release.size,
        });
    }
    Ok(plan)
}

/// Installs the missing tools (downloaded with the engine, verified,
/// smoke-tested), streaming progress to `on_event`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn install_media_tools(
    bridge: State<'_, Bridge>,
    on_event: Channel<MediaInstallEvent>,
) -> Result<MediaToolsInfo, UiError> {
    let sidecars = bridge.media_sidecars()?;
    let video = bridge.settings().video;
    let send = |e: InstallEvent| {
        if let Some(event) = MediaInstallEvent::from_event(&e) {
            let _ = on_event.send(event);
        }
    };
    for tool in Tool::ALL {
        let custom = match tool {
            Tool::YtDlp => video.yt_dlp_path.is_some(),
            Tool::Ffmpeg => video.ffmpeg_path.is_some(),
        };
        if !custom {
            sidecars
                .install(tool, &send)
                .await
                .map_err(|e| media_error(&e))?;
        }
    }
    tools_info(&bridge).await
}

/// Checks both installed tools for a newer release now, and installs it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_media_tools(
    bridge: State<'_, Bridge>,
) -> Result<MediaToolsInfo, UiError> {
    let sidecars = bridge.media_sidecars()?;
    let status = sidecars.status().map_err(|e| media_error(&e))?;
    for t in status.tools.iter().filter(|t| t.active.is_some()) {
        sidecars
            .update(t.tool, &|_| {})
            .await
            .map_err(|e| media_error(&e))?;
    }
    tools_info(&bridge).await
}

/// Removes both installed tools (not the user's own programs). Refused
/// while media downloads run.
#[tauri::command]
#[specta::specta]
pub(crate) async fn remove_media_tools(
    bridge: State<'_, Bridge>,
) -> Result<MediaToolsInfo, UiError> {
    if bridge.media_busy() {
        return Err(UiError::key("error.media.in_use"));
    }
    let sidecars = bridge.media_sidecars()?;
    for tool in Tool::ALL {
        sidecars.remove(tool).map_err(|e| media_error(&e))?;
    }
    tools_info(&bridge).await
}

/// Chooses yt-dlp's release channel; the next update check follows it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_media_channel(
    bridge: State<'_, Bridge>,
    channel: MediaChannel,
) -> Result<MediaToolsInfo, UiError> {
    let sidecars = bridge.media_sidecars()?;
    std::fs::create_dir_all(sidecars.dir())
        .map_err(|e| UiError::with_message("error.media.disk", e.to_string()))?;
    sidecars
        .set_channel(match channel {
            MediaChannel::Stable => ToolChannel::Stable,
            MediaChannel::Nightly => ToolChannel::Nightly,
        })
        .map_err(|e| media_error(&e))?;
    tools_info(&bridge).await
}

/// Uses a program the user chose instead of the installed tool (`null`:
/// back to the installed one). The program must start and report its
/// version first; it is not verified or updated by Boltget.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_media_tool_path(
    bridge: State<'_, Bridge>,
    tool: MediaTool,
    path: Option<String>,
) -> Result<MediaToolsInfo, UiError> {
    let path = path.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
    if let Some(p) = &path {
        crate::settings::parse_program_path(p)?;
        let program = match tool {
            MediaTool::YtDlp => Program::YtDlp,
            MediaTool::Ffmpeg => Program::Ffmpeg,
        };
        bridge
            .media_sidecars()?
            .check_custom(program, std::path::Path::new(p))
            .await
            .map_err(|e| media_error(&e))?;
    }
    let value = Some(path.unwrap_or_default());
    let patch = SettingsPatch {
        video: Some(match tool {
            MediaTool::YtDlp => VideoPatch {
                yt_dlp_path: value,
                ..VideoPatch::default()
            },
            MediaTool::Ffmpeg => VideoPatch {
                ffmpeg_path: value,
                ..VideoPatch::default()
            },
        }),
        ..SettingsPatch::default()
    };
    bridge.update_settings(&patch).await?;
    tools_info(&bridge).await
}

/// Looks for video or audio on a web page with yt-dlp. Answers
/// `notInstalled` without running anything while a tool is missing (the
/// window offers the install sheet); otherwise installs due updates first
/// (yt-dlp at most once a day; a failed check keeps the installed
/// version), then probes through the app's proxy. yt-dlp's own failure is
/// `error.media.extraction`, with its message.
#[tauri::command]
#[specta::specta]
pub(crate) async fn probe_media(
    bridge: State<'_, Bridge>,
    url: String,
) -> Result<MediaProbe, UiError> {
    let sidecars = bridge.media_sidecars()?;
    let settings = bridge.settings();
    if !media::tools_ready(&sidecars, &settings.video) {
        return Ok(MediaProbe::NotInstalled);
    }
    if let Err(e) = sidecars.update_if_due(SystemTime::now(), &|_| {}).await {
        tracing::warn!(error = %e, "the video tools' update check failed");
    }
    let prober = media::prober(&sidecars, &settings.video)?;
    let url = url.trim();
    let opts = boltget_media::ProbeOptions {
        proxy: media::proxy_for(&settings, url),
        ..boltget_media::ProbeOptions::default()
    };
    let info = prober
        .probe(url, &opts)
        .await
        .map_err(|e| media_error(&e))?;
    let site = site_of(info.webpage_url.as_deref().unwrap_or(url));
    let remembered = remembered_presets(&bridge)
        .await
        .into_iter()
        .find(|(s, _)| *s == site)
        .map(|(_, p)| p);
    Ok(MediaProbe::Found {
        media: Box::new(MediaSummary::new(&info, url, remembered)),
    })
}

/// A page's thumbnail as a `data:` URL (JPEG, PNG or WebP, at most 2 MiB),
/// fetched through the app's proxy.
#[tauri::command]
#[specta::specta]
pub(crate) async fn media_thumbnail(
    bridge: State<'_, Bridge>,
    url: String,
) -> Result<String, UiError> {
    let image = bridge
        .media_sidecars()?
        .thumbnail(url.trim())
        .await
        .map_err(|e| media_error(&e))?;
    Ok(image.data_url())
}

/// Opens a video tool's project page in the browser (the install sheet's
/// links). Only these two addresses: the page never names one.
#[tauri::command]
#[specta::specta]
pub(crate) fn open_media_project(tool: MediaTool) -> Result<(), UiError> {
    let url = match tool {
        MediaTool::YtDlp => "https://github.com/yt-dlp/yt-dlp",
        MediaTool::Ffmpeg => "https://ffmpeg.org",
    };
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|e| UiError::with_message("error.open_failed", e.to_string()))
}

/// The most a format expression may be.
const MAX_EXPRESSION: usize = 512;

/// Starts downloading a page's media into the Video category's folder, as
/// a row of the list. Returns the row.
#[tauri::command]
#[specta::specta]
pub(crate) async fn add_media_download(
    app: AppHandle,
    bridge: State<'_, Bridge>,
    request: MediaRequest,
) -> Result<DownloadRow, UiError> {
    let expression = request.expression.trim();
    if expression.is_empty()
        || expression.len() > MAX_EXPRESSION
        || expression.chars().any(char::is_control)
    {
        return Err(UiError::key("error.media.expression"));
    }
    let subtitles = match request.subtitles {
        Some(s) if !s.languages.is_empty() => {
            let ok = s.languages.iter().all(|l| {
                !l.is_empty()
                    && l.len() <= 32
                    && l.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            });
            if !ok {
                return Err(UiError::key("error.media.subtitles"));
            }
            Some(SubtitleChoice {
                languages: s.languages,
                automatic: s.automatic,
                embed: s.embed,
            })
        }
        _ => None,
    };
    let preset = request
        .preset
        .filter(|p| boltget_media::Preset::parse(p).is_some());
    let folder = crate::bridge::commands::default_folder(&app, &bridge, Some("video.mp4"))?;
    let mut job = NewMediaJob::new(request.url.trim(), expression, folder);
    job.title = request.title.filter(|t| !t.trim().is_empty());
    job.preset.clone_from(&preset);
    job.container = request.container.into();
    job.subtitles = subtitles;
    job.size = request.size;
    let row = bridge.add_media(job).await?;
    if let (Some(site), Some(preset)) = (request.site.as_deref(), preset.as_deref()) {
        remember_preset(&bridge, site, preset).await;
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> MediaInfo {
        MediaInfo::from_json(
            br#"{"title": "Clip", "channel": "Chan", "duration": 10,
                 "webpage_url": "https://www.video.test/watch?v=1", "formats": [
                {"format_id": "sb0", "ext": "mhtml", "vcodec": "none", "acodec": "none"},
                {"format_id": "a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "filesize": 100, "abr": 128, "protocol": "https"},
                {"format_id": "o", "ext": "webm", "vcodec": "none", "acodec": "opus", "filesize": 90},
                {"format_id": "v", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 720, "width": 1280, "fps": 30, "filesize_approx": 900, "tbr": 1500.5}
            ], "subtitles": {"en": [{"ext": "vtt", "name": "English"}]}}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_summary_shows_presets_formats_and_the_site() {
        let s = MediaSummary::new(&info(), "https://video.test/w", Some("720p".into()));
        assert_eq!(s.uploader.as_deref(), Some("Chan"));
        assert_eq!(s.site, "video.test");
        assert_eq!(s.url, "https://www.video.test/watch?v=1");
        assert_eq!(s.presets.len(), 2);
        let best = &s.presets[0];
        assert_eq!(best.preset, "best");
        assert_eq!(best.video_codec.as_deref(), Some("H.264"));
        assert_eq!(best.audio_codec.as_deref(), Some("AAC"));
        assert_eq!((best.size, best.size_approximate), (Some(1000), true));
        assert_eq!(s.presets[1].preset, "audio");
        // Prefer original: Opus audio is the best audio there.
        assert_eq!(s.original_presets[0].audio_codec.as_deref(), Some("Opus"));
        // The storyboard is not a format to pick.
        let ids: Vec<&str> = s.formats.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["a", "o", "v"]);
        let v = &s.formats[2];
        assert_eq!(
            (v.width, v.height, v.bitrate),
            (Some(1280), Some(720), Some(1500.5))
        );
        assert_eq!(v.audio_codec, None);
        assert_eq!(s.formats[0].video_codec, None);
        assert_eq!(s.formats[0].protocol.as_deref(), Some("https"));
        assert_eq!(s.subtitles[0].code, "en");
        assert_eq!(s.remembered.as_deref(), Some("720p"));
        assert!(!s.drm && !s.is_playlist);
    }

    #[test]
    fn sites() {
        assert_eq!(site_of("https://www.YouTube.com/watch?v=1"), "youtube.com");
        assert_eq!(site_of("https://m.example.org/x"), "m.example.org");
        assert_eq!(site_of("nonsense"), "");
    }

    #[test]
    fn install_events_for_the_window() {
        let big = InstallEvent::Downloading {
            tool: Tool::Ffmpeg,
            file: "ffmpeg.zip".into(),
            done: 5,
            total: Some(90 * 1024 * 1024),
        };
        let e = MediaInstallEvent::from_event(&big).unwrap();
        assert_eq!(
            (e.tool, e.step, e.done),
            (MediaTool::Ffmpeg, MediaInstallStep::Downloading, 5)
        );
        let small = InstallEvent::Downloading {
            tool: Tool::YtDlp,
            file: "SHA2-256SUMS".into(),
            done: 5,
            total: Some(900),
        };
        assert_eq!(MediaInstallEvent::from_event(&small), None);
        let done = InstallEvent::Activated {
            tool: Tool::YtDlp,
            version: "2026.10.26".into(),
        };
        assert_eq!(
            MediaInstallEvent::from_event(&done).map(|e| e.step),
            Some(MediaInstallStep::Done)
        );
    }

    #[tokio::test]
    async fn without_tools_nothing_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, _hub) = Bridge::new(std::time::Duration::from_millis(50));
        assert!(bridge.media_sidecars().is_err());
        bridge.set_tools_dir(dir.path().join("tools"));
        let info = tools_info(&bridge).await.unwrap();
        assert!(!info.ready);
        assert_eq!(info.channel, MediaChannel::Stable);
        assert!(info.tools.iter().all(|t| t.version.is_none()));
    }
}
