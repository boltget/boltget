//! Video and audio from web pages (docs/VIDEO.md; the window's side is in
//! docs/APP-DESIGN.md, "Video as built"): the commands of the add flow's
//! picker, the first-use install sheet and Settings, Video. Media downloads
//! themselves run in the bridge (`bridge::media`), as rows of the list.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use boltget_engine::{ContainerPreference, NewMediaJob, NewMediaPlaylist, SubtitleChoice};
use boltget_ipc::browser::{MediaKind, MediaPage};
use boltget_media::{
    Channel as ToolChannel, Format, InstallEvent, ItemRange, MediaInfo, PlaylistEntry,
    PlaylistMode, Preset, Program, ResolvedPreset, Tool, codec_label, playlist, presets,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::bridge::dto::DownloadRow;
use crate::bridge::media::MediaAuth;
use crate::bridge::{Bridge, media};
use crate::browser::Captured;
use crate::error::UiError;
use crate::settings::{MediaContainer, SettingsPatch, VideoPatch};

// Pages from the browser extension ---------------------------------------------

/// How long a page the extension sent keeps its browser context (cookies,
/// headers, hints) for the media sheet: its probe and its Download button.
const PAGE_KEEP: Duration = Duration::from_secs(30 * 60);

/// At most this many pages wait at once; the oldest goes first.
const MAX_PAGES: usize = 16;

/// Pages the extension sent (`media_page`, docs/VIDEO.md, "The extension's
/// media panel: as built"), with their cookies and headers, for the media
/// sheet. Held in memory only, never saved: cookies are secrets.
#[derive(Debug, Default)]
pub(crate) struct MediaPages(Mutex<Pages>);

#[derive(Debug, Default)]
struct Pages {
    next: u32,
    list: VecDeque<PageEntry>,
}

#[derive(Debug)]
struct PageEntry {
    id: u32,
    page: MediaPage,
    /// The window has opened the media sheet for it.
    taken: bool,
    at: Instant,
}

impl MediaPages {
    /// Keeps a page the extension sent; returns its number.
    pub(crate) fn add(&self, page: MediaPage) -> u32 {
        let mut pages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        pages.next = pages.next.wrapping_add(1).max(1);
        let id = pages.next;
        pages.list.retain(|p| p.at.elapsed() < PAGE_KEEP);
        while pages.list.len() >= MAX_PAGES {
            pages.list.pop_front();
        }
        pages.list.push_back(PageEntry {
            id,
            page,
            taken: false,
            at: Instant::now(),
        });
        id
    }

    /// The pages the window has not opened yet, oldest first, now marked
    /// opened.
    pub(crate) fn take_new(&self) -> Vec<BrowserMediaPage> {
        let mut pages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        pages
            .list
            .iter_mut()
            .filter(|p| !p.taken && p.at.elapsed() < PAGE_KEEP)
            .map(|p| {
                p.taken = true;
                BrowserMediaPage::new(p.id, &p.page)
            })
            .collect()
    }

    /// The page numbered `id`, while it is kept.
    pub(crate) fn get(&self, id: u32) -> Option<MediaPage> {
        let pages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        pages
            .list
            .iter()
            .find(|p| p.id == id && p.at.elapsed() < PAGE_KEEP)
            .map(|p| p.page.clone())
    }

    /// What yt-dlp gets of page `id`'s browser context, if it is kept.
    fn auth(&self, id: Option<u32>) -> Option<MediaAuth> {
        self.get(id?).map(|p| auth_of(&p))
    }
}

/// The browser's cookies, `User-Agent` and `Referer` (else the page) for
/// yt-dlp.
fn auth_of(page: &MediaPage) -> MediaAuth {
    MediaAuth {
        cookies: page.cookie_jar(),
        user_agent: page.header("user-agent").map(str::to_owned),
        referer: Some(page.header("referer").unwrap_or(&page.page_url).to_owned()),
    }
}

/// A page the browser extension sent: "Download video from this page".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserMediaPage {
    /// Names the page's browser context (cookies, headers) in
    /// `probe_media`, `add_media_download` and `media_hint_link`.
    pub(crate) page: u32,
    pub(crate) url: String,
    pub(crate) title: Option<String>,
    /// What the extension saw the page load, for when yt-dlp finds nothing:
    /// the element the user pointed at first.
    pub(crate) hints: Vec<MediaHintInfo>,
}

/// Media the extension saw a page load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaHintInfo {
    pub(crate) url: String,
    pub(crate) kind: MediaHintKind,
    pub(crate) mime: Option<String>,
    /// Bytes, when the response said.
    pub(crate) size: Option<u64>,
}

/// What a hint is: a stream yt-dlp reads from its manifest, or a file the
/// engine downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaHintKind {
    Hls,
    Dash,
    File,
}

/// A link's kind from its path: `.m3u8` is HLS, `.mpd` DASH, anything else
/// a file.
fn kind_of_link(url: &str) -> MediaHintKind {
    let path = url::Url::parse(url)
        .map(|u| u.path().to_owned())
        .unwrap_or_default();
    let ext = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("m3u8") => MediaHintKind::Hls,
        Some("mpd") => MediaHintKind::Dash,
        _ => MediaHintKind::File,
    }
}

impl BrowserMediaPage {
    fn new(id: u32, page: &MediaPage) -> Self {
        let mut hints: Vec<MediaHintInfo> = Vec::new();
        if let Some(url) = &page.media_url {
            hints.push(MediaHintInfo {
                url: url.clone(),
                kind: kind_of_link(url),
                mime: None,
                size: None,
            });
        }
        for h in &page.hints {
            if hints.iter().any(|x| x.url == h.url) {
                continue;
            }
            hints.push(MediaHintInfo {
                url: h.url.clone(),
                kind: match h.kind {
                    MediaKind::Hls => MediaHintKind::Hls,
                    MediaKind::Dash => MediaHintKind::Dash,
                    _ => MediaHintKind::File,
                },
                mime: h.mime.clone(),
                size: h.size,
            });
        }
        Self {
            page: id,
            url: page.page_url.clone(),
            title: page.title.clone(),
            hints,
        }
    }
}

/// The pages the extension sent that the window has not opened yet,
/// oldest first. The window asks when the extension's request arrives
/// (`ShellRequest::MediaPage`) and when it starts, since the request may
/// come before it listens (the extension started the app).
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn take_media_pages(pages: State<'_, MediaPages>) -> Vec<BrowserMediaPage> {
    pages.take_new()
}

/// A direct file the extension saw on page `page`: its request (the
/// browser's cookies and headers, the page as referrer) waits for the add
/// sheet, which the window opens with `url` next. Only the page's own
/// hints.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub(crate) fn media_hint_link(
    pages: State<'_, MediaPages>,
    captured: State<'_, Captured>,
    page: u32,
    url: String,
) -> Result<(), UiError> {
    hint_link(&pages, &captured, page, url.trim())
}

fn hint_link(pages: &MediaPages, captured: &Captured, id: u32, url: &str) -> Result<(), UiError> {
    let page = pages
        .get(id)
        .ok_or_else(|| UiError::key("error.media.page_gone"))?;
    let known = page.media_url.as_deref() == Some(url) || page.hints.iter().any(|h| h.url == url);
    if !known {
        return Err(UiError::key("error.media.page_gone"));
    }
    let ctx = page
        .context(url)
        .map_err(|e| UiError::with_message("error.invalid_url", e.to_string()))?;
    captured.remember(url.to_owned(), ctx, page.incognito);
    Ok(())
}

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
    /// A playlist's entries (a page of them), when the page is a playlist.
    pub(crate) playlist: Option<MediaPlaylistPage>,
    /// One video whose address names a list it belongs to (`list=`): the
    /// sheet offers "this video" or "the whole playlist".
    pub(crate) in_playlist: bool,
    /// Everything on the page is DRM-protected.
    pub(crate) drm: bool,
    /// The preset last used on this site, if any.
    pub(crate) remembered: Option<String>,
}

/// A page of a playlist's entries, as yt-dlp listed them with
/// `--flat-playlist` (not probed one by one).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaPlaylistPage {
    /// The site's id for the playlist (the folder's name when it has no
    /// title).
    pub(crate) id: Option<String>,
    /// The place of the first entry asked for, from 1.
    pub(crate) start: u32,
    pub(crate) entries: Vec<MediaEntry>,
    /// More entries may follow: "Load more" probes the next page from
    /// `start + PAGE_SIZE`.
    pub(crate) more: bool,
    /// The folder the entries go to, in the Video folder.
    pub(crate) folder: String,
}

/// One entry of a playlist.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaEntry {
    /// Its place in the playlist, from 1.
    pub(crate) index: u32,
    pub(crate) id: Option<String>,
    pub(crate) title: Option<String>,
    /// Its page, which its download fetches.
    pub(crate) url: Option<String>,
    /// Seconds.
    pub(crate) duration: Option<f64>,
    pub(crate) uploader: Option<String>,
    /// A thumbnail's address, when the flat entry carries one;
    /// `media_thumbnail` fetches it.
    pub(crate) thumbnail: Option<String>,
    /// It can be downloaded (not private or deleted, and it has a page).
    pub(crate) available: bool,
}

impl From<PlaylistEntry> for MediaEntry {
    fn from(e: PlaylistEntry) -> Self {
        Self {
            index: e.index,
            id: e.id,
            title: e.title,
            url: e.url,
            duration: e.duration,
            uploader: e.uploader,
            thumbnail: e.thumbnail,
            available: e.available,
        }
    }
}

/// Whether to look at the video an address names or at the playlist it
/// belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaPlaylistMode {
    /// The video (`--no-playlist`); a page that is only a playlist still
    /// lists its entries.
    Single,
    /// The playlist the address belongs to (`--yes-playlist`).
    Entries,
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
    /// The page the browser extension sent ([`BrowserMediaPage::page`]):
    /// yt-dlp gets the browser's cookies and headers for as long as the job
    /// runs in this session (never saved).
    #[specta(optional)]
    pub(crate) page: Option<u32>,
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
    /// A page's summary, a playlist's entries listed from place `start`.
    fn listed(info: &MediaInfo, url: &str, remembered: Option<String>, start: u32) -> Self {
        let page = info.webpage_url.clone().unwrap_or_else(|| url.to_owned());
        let list = info.is_playlist().then(|| {
            let entries: Vec<MediaEntry> = playlist::entries(info, start)
                .into_iter()
                .map(MediaEntry::from)
                .collect();
            let listed = u64::from(start) - 1 + entries.len() as u64;
            MediaPlaylistPage {
                id: info.id.clone(),
                start,
                more: entries.len() >= playlist::PAGE_SIZE as usize
                    || info.playlist_count.is_some_and(|n| n > listed),
                entries,
                folder: playlist::folder_name(info.title.as_deref(), info.id.as_deref()),
            }
        });
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
            playlist: list,
            in_playlist: !info.is_playlist() && playlist::names_a_list(url),
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
/// `error.media.extraction`, with its message. With `page`, a page the
/// browser extension sent (or one of its hints), yt-dlp gets the browser's
/// cookies, `User-Agent` and `Referer`. `playlist` chooses between the
/// video an address names and the playlist it belongs to (`single` when
/// absent); a playlist's entries are listed a page at a time
/// (`boltget_media::playlist::PAGE_SIZE`) from place `start` (1 when
/// absent).
#[tauri::command]
#[specta::specta]
pub(crate) async fn probe_media(
    bridge: State<'_, Bridge>,
    pages: State<'_, MediaPages>,
    url: String,
    page: Option<u32>,
    playlist: Option<MediaPlaylistMode>,
    start: Option<u32>,
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
    let start = start.unwrap_or(1).max(1);
    let mut opts = boltget_media::ProbeOptions {
        proxy: media::proxy_for(&settings, url),
        playlist: match playlist {
            Some(MediaPlaylistMode::Entries) => PlaylistMode::Entries,
            _ => PlaylistMode::Single,
        },
        items: Some(ItemRange::page(start)),
        ..boltget_media::ProbeOptions::default()
    };
    if let Some(auth) = pages.auth(page) {
        opts.cookies = (!auth.cookies.is_empty()).then_some(auth.cookies);
        opts.user_agent = auth.user_agent;
        opts.referer = auth.referer;
    }
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
        media: Box::new(MediaSummary::listed(&info, url, remembered, start)),
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
    pages: State<'_, MediaPages>,
    request: MediaRequest,
) -> Result<DownloadRow, UiError> {
    let expression = request.expression.trim();
    if expression.is_empty()
        || expression.len() > MAX_EXPRESSION
        || expression.chars().any(char::is_control)
    {
        return Err(UiError::key("error.media.expression"));
    }
    let subtitles = subtitle_choice(request.subtitles)?;
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
    let row = bridge.add_media(job, pages.auth(request.page)).await?;
    if let (Some(site), Some(preset)) = (request.site.as_deref(), preset.as_deref()) {
        remember_preset(&bridge, site, preset).await;
    }
    Ok(row)
}

/// The most entries of a playlist one request may start.
const MAX_ENTRIES: usize = 5000;

/// An entry of a playlist to download.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaEntryRequest {
    /// Its place in the playlist, from 1.
    pub(crate) index: u32,
    /// Its page.
    pub(crate) url: String,
    #[specta(optional)]
    pub(crate) title: Option<String>,
}

/// A playlist's entries to download, one preset for all (docs/VIDEO.md,
/// "Playlists: as built").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaPlaylistRequest {
    /// The playlist's page.
    pub(crate) url: String,
    #[specta(optional)]
    pub(crate) title: Option<String>,
    #[specta(optional)]
    pub(crate) uploader: Option<String>,
    /// The site's id for the playlist (the folder's name without a title).
    #[specta(optional)]
    pub(crate) id: Option<String>,
    /// `best`, `1080p`, `720p`, `small` or `audio`: a preset is an
    /// expression with its own fallbacks, so each entry gets what it has.
    pub(crate) preset: String,
    pub(crate) container: MediaContainer,
    /// Subtitles in these languages where an entry has them (its own, else
    /// automatic captions).
    #[specta(optional)]
    pub(crate) subtitles: Option<MediaSubtitlesChoice>,
    /// The playlist's size, for numbering the files with as many digits.
    pub(crate) entry_count: u32,
    pub(crate) entries: Vec<MediaEntryRequest>,
    /// The page the browser extension sent: every entry's runs get the
    /// browser's cookies and headers in this session (never saved).
    #[specta(optional)]
    pub(crate) page: Option<u32>,
}

fn subtitle_choice(
    subtitles: Option<MediaSubtitlesChoice>,
) -> Result<Option<SubtitleChoice>, UiError> {
    match subtitles {
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
            Ok(Some(SubtitleChoice {
                languages: s.languages,
                automatic: s.automatic,
                embed: s.embed,
            }))
        }
        _ => Ok(None),
    }
}

/// The jobs for a playlist's chosen entries into `folder`: each its own
/// page with the preset's expression, numbered by its place.
pub(crate) fn playlist_jobs(
    request: &MediaPlaylistRequest,
    folder: &std::path::Path,
) -> Result<(NewMediaPlaylist, Vec<NewMediaJob>), UiError> {
    let preset =
        Preset::parse(&request.preset).ok_or_else(|| UiError::key("error.media.expression"))?;
    if request.entries.is_empty() || request.entries.len() > MAX_ENTRIES {
        return Err(UiError::key("error.media.entries"));
    }
    let container: ContainerPreference = request.container.into();
    let subtitles = subtitle_choice(request.subtitles.clone())?;
    let last = request
        .entries
        .iter()
        .map(|e| e.index)
        .max()
        .unwrap_or(1)
        .max(request.entry_count);
    let title = request.title.clone().filter(|t| !t.trim().is_empty());
    let dir = folder.join(playlist::folder_name(
        title.as_deref(),
        request.id.as_deref(),
    ));
    let mut seen = std::collections::HashSet::new();
    let mut jobs = Vec::with_capacity(request.entries.len());
    for e in &request.entries {
        if e.index == 0 || !seen.insert(e.index) {
            return Err(UiError::key("error.media.entries"));
        }
        let mut job = NewMediaJob::new(e.url.trim(), preset.expression(container), &dir);
        job.title = e.title.clone().filter(|t| !t.trim().is_empty());
        job.preset = Some(preset.id().to_owned());
        job.container = container;
        job.subtitles.clone_from(&subtitles);
        job.output_template = playlist::entry_template(e.index, last);
        job.playlist_index = Some(e.index);
        jobs.push(job);
    }
    let list = NewMediaPlaylist {
        page_url: request.url.trim().to_owned(),
        title,
        uploader: request.uploader.clone().filter(|t| !t.trim().is_empty()),
        dest_dir: dir,
    };
    Ok((list, jobs))
}

/// Starts downloading a playlist's chosen entries into a folder named
/// after it in the Video category's folder: each entry its own media job
/// and row, the same preset for all, run one at a time. Returns the rows,
/// in the playlist's order.
#[tauri::command]
#[specta::specta]
pub(crate) async fn add_media_playlist(
    app: AppHandle,
    bridge: State<'_, Bridge>,
    pages: State<'_, MediaPages>,
    request: MediaPlaylistRequest,
) -> Result<Vec<DownloadRow>, UiError> {
    let folder = crate::bridge::commands::default_folder(&app, &bridge, Some("video.mp4"))?;
    let (list, mut jobs) = playlist_jobs(&request, &folder)?;
    jobs.sort_by_key(|j| j.playlist_index);
    bridge
        .add_media_playlist(list, jobs, pages.auth(request.page))
        .await
}

/// Pauses every entry of a playlist that runs or waits.
#[tauri::command]
#[specta::specta]
pub(crate) async fn pause_media_playlist(
    bridge: State<'_, Bridge>,
    playlist: u64,
) -> Result<(), UiError> {
    bridge
        .pause_media_playlist(boltget_engine::MediaPlaylistId(playlist))
        .await
}

/// Lets every paused or failed entry of a playlist wait its turn again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn resume_media_playlist(
    bridge: State<'_, Bridge>,
    playlist: u64,
) -> Result<(), UiError> {
    bridge
        .resume_media_playlist(boltget_engine::MediaPlaylistId(playlist))
        .await
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
        let s = MediaSummary::listed(&info(), "https://video.test/w", Some("720p".into()), 1);
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
    fn a_playlist_summary_lists_a_page_of_entries() {
        let list = MediaInfo::from_json(
            br#"{"_type": "playlist", "id": "PL1", "title": "Weekend: projects",
                 "uploader": "Chan", "playlist_count": 450, "entries": [
                {"id": "a", "url": "https://v.test/w?v=a", "title": "A", "duration": 61},
                {"id": "b", "url": "https://v.test/w?v=b", "title": "[Private video]"}
            ]}"#,
        )
        .unwrap();
        let s = MediaSummary::listed(&list, "https://v.test/playlist?list=PL1", None, 201);
        assert!(s.is_playlist && !s.in_playlist);
        assert!(s.presets.is_empty(), "entries are not probed one by one");
        let list = s.playlist.unwrap();
        assert_eq!(list.start, 201);
        assert_eq!(list.entries[0].index, 201);
        assert!(list.entries[0].available && !list.entries[1].available);
        assert!(list.more, "450 entries, 202 listed");
        assert_eq!(list.folder, "Weekend_ projects");
        // A watch page in a list offers the playlist.
        let s = MediaSummary::listed(&info(), "https://v.test/w?v=1&list=PL1", None, 1);
        assert!(s.in_playlist && s.playlist.is_none());
    }

    #[test]
    fn a_playlist_request_becomes_one_job_per_entry() {
        let request = MediaPlaylistRequest {
            url: "https://v.test/playlist?list=PL1".into(),
            title: Some("Weekend: projects".into()),
            uploader: Some("Chan".into()),
            id: Some("PL1".into()),
            preset: "720p".into(),
            container: MediaContainer::Mp4,
            subtitles: Some(MediaSubtitlesChoice {
                languages: vec!["en".into()],
                automatic: true,
                embed: true,
            }),
            entry_count: 120,
            entries: vec![
                MediaEntryRequest {
                    index: 3,
                    url: "https://v.test/w?v=c".into(),
                    title: Some("C".into()),
                },
                MediaEntryRequest {
                    index: 1,
                    url: "https://v.test/w?v=a".into(),
                    title: None,
                },
            ],
            page: None,
        };
        let folder = std::path::Path::new("/videos");
        let (list, jobs) = playlist_jobs(&request, folder).unwrap();
        assert_eq!(list.dest_dir, folder.join("Weekend_ projects"));
        assert_eq!(list.title.as_deref(), Some("Weekend: projects"));
        assert_eq!(jobs.len(), 2);
        let c = &jobs[0];
        assert_eq!(c.page_url, "https://v.test/w?v=c");
        assert_eq!(c.output_template, "003 - %(title)s [%(id)s].%(ext)s");
        assert_eq!(c.playlist_index, Some(3));
        assert_eq!(c.dest_dir, list.dest_dir);
        assert_eq!(c.preset.as_deref(), Some("720p"));
        assert_eq!(
            c.format_expression,
            Preset::P720.expression(ContainerPreference::Mp4)
        );
        assert!(c.subtitles.as_ref().is_some_and(|s| s.automatic));
        // Refused: an unknown preset, no entries, a place twice or zero.
        for change in [
            |r: &mut MediaPlaylistRequest| r.preset = "bv*".into(),
            |r: &mut MediaPlaylistRequest| r.entries.clear(),
            |r: &mut MediaPlaylistRequest| r.entries[1].index = 3,
            |r: &mut MediaPlaylistRequest| r.entries[0].index = 0,
        ] {
            let mut bad = request.clone();
            change(&mut bad);
            assert!(playlist_jobs(&bad, folder).is_err());
        }
    }

    fn browser_page() -> MediaPage {
        serde_json::from_value(serde_json::json!({
            "page_url": "https://v.test/watch?v=1",
            "cookies": [{"name": "sid", "value": "s3cret", "domain": "v.test"}],
            "headers": [["User-Agent", "Browser UA"]],
            "incognito": true,
            "hints": [
                {"url": "https://cdn.v.test/a.mp4", "kind": "file"},
                {"url": "https://cdn.v.test/a.mpd", "kind": "dash"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn pages_from_the_browser_wait_with_their_context() {
        let pages = MediaPages::default();
        let id = pages.add(browser_page());
        // yt-dlp gets the cookies, the browser's User-Agent, and the page as
        // the Referer when the browser sent none.
        let auth = pages.auth(Some(id)).expect("kept");
        assert_eq!(auth.cookies.len(), 1);
        assert_eq!(auth.user_agent.as_deref(), Some("Browser UA"));
        assert_eq!(auth.referer.as_deref(), Some("https://v.test/watch?v=1"));
        assert!(pages.auth(None).is_none());
        assert!(pages.auth(Some(id + 1)).is_none());
        let opened = pages.take_new();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].hints[1].kind, MediaHintKind::Dash);
        assert_eq!(pages.take_new().len(), 0);
        assert!(pages.get(id).is_some(), "still there for the download");
        // Bounded: the oldest goes first.
        for _ in 0..MAX_PAGES {
            pages.add(browser_page());
        }
        assert!(pages.get(id).is_none());
    }

    #[test]
    fn a_direct_hint_goes_to_the_add_sheet_with_the_browsers_request() {
        let pages = MediaPages::default();
        let captured = Captured::default();
        let id = pages.add(browser_page());
        hint_link(&pages, &captured, id, "https://cdn.v.test/a.mp4").unwrap();
        let (ctx, private) = captured
            .get_link("https://cdn.v.test/a.mp4")
            .expect("waiting for the sheet");
        assert!(private, "from a private window");
        assert_eq!(ctx.headers()["user-agent"], "Browser UA");
        assert_eq!(ctx.headers()["referer"], "https://v.test/watch?v=1");
        // Only the page's own hints, and only while the page is kept.
        assert!(hint_link(&pages, &captured, id, "https://evil.test/x.mp4").is_err());
        assert!(hint_link(&pages, &captured, id + 1, "https://cdn.v.test/a.mp4").is_err());
    }

    #[test]
    fn link_kinds() {
        assert_eq!(
            kind_of_link("https://x.test/a/B.M3U8?x=1"),
            MediaHintKind::Hls
        );
        assert_eq!(kind_of_link("https://x.test/a.mpd"), MediaHintKind::Dash);
        assert_eq!(kind_of_link("https://x.test/a.mp4"), MediaHintKind::File);
        assert_eq!(kind_of_link("https://x.test/m3u8/a"), MediaHintKind::File);
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
