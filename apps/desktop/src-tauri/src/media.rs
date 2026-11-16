//! Video and audio from web pages (docs/VIDEO.md): the add sheet's "Look for
//! media on this page". v0.4 step 2 probes only; the picker, the first-use
//! install sheet and media downloads arrive in step 3.

use std::path::PathBuf;
use std::time::SystemTime;

use boltget_engine::{ContainerPreference, EngineConfig};
use boltget_media::{
    MediaInfo, ProbeOptions, ResolvedPreset, Sidecars, Sources, codec_label, presets,
};
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::bridge::Bridge;
use crate::error::UiError;
use crate::settings;

/// Where the video tools live: `tools` in the data directory. Tauri state.
#[derive(Debug)]
pub(crate) struct MediaTools {
    dir: Option<PathBuf>,
}

impl MediaTools {
    /// The tools of the data directory `data_dir` (`None`: there is none).
    pub(crate) fn new(data_dir: Option<PathBuf>) -> Self {
        Self {
            dir: data_dir.map(|d| d.join("tools")),
        }
    }

    /// The sidecar manager; its downloads use the app's proxy settings.
    fn sidecars(&self, bridge: &Bridge) -> Result<Sidecars, UiError> {
        let dir = self
            .dir
            .clone()
            .ok_or_else(|| UiError::key("error.no_data_dir"))?;
        let mut config = EngineConfig::default();
        config.proxy = settings::rules(&bridge.settings().proxy, crate::platform::system_proxy);
        Ok(Sidecars::new(dir, Sources::official(), config))
    }
}

fn media_error(e: &boltget_media::Error) -> UiError {
    UiError::with_message(e.message_key(), e.to_string())
}

/// What "Look for media on this page" found.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum MediaProbe {
    /// yt-dlp and FFmpeg are not installed yet (the install sheet is v0.4
    /// step 3); nothing was run.
    NotInstalled,
    /// yt-dlp described the page.
    Found {
        /// What it found.
        media: MediaSummary,
    },
}

/// A page's media, as the add sheet shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSummary {
    pub(crate) title: Option<String>,
    pub(crate) uploader: Option<String>,
    /// Seconds.
    pub(crate) duration: Option<f64>,
    pub(crate) thumbnail: Option<String>,
    /// The presets for "Prefer MP4", best first; empty when there is
    /// nothing to download (no media, DRM, or a playlist).
    pub(crate) presets: Vec<MediaPreset>,
    pub(crate) subtitles: Vec<MediaSubtitle>,
    pub(crate) is_playlist: bool,
    pub(crate) entry_count: u32,
    /// Everything on the page is DRM-protected.
    pub(crate) drm: bool,
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

/// A subtitle language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSubtitle {
    pub(crate) code: String,
    pub(crate) name: Option<String>,
    /// Automatic captions rather than subtitles.
    pub(crate) automatic: bool,
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

impl From<&MediaInfo> for MediaSummary {
    fn from(info: &MediaInfo) -> Self {
        Self {
            title: info.title.clone(),
            uploader: info.publisher().map(str::to_owned),
            duration: info.duration,
            thumbnail: info.best_thumbnail().map(str::to_owned),
            presets: presets(info, ContainerPreference::Mp4)
                .iter()
                .map(MediaPreset::from)
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
        }
    }
}

/// Looks for video or audio on a web page with yt-dlp. Answers
/// `notInstalled` without running anything while the tools are missing;
/// otherwise installs due updates first (yt-dlp at most once a day; a failed
/// check keeps the installed version), then probes. yt-dlp's own failure is
/// `error.media.extraction`, with its message.
#[tauri::command]
#[specta::specta]
pub(crate) async fn probe_media(
    bridge: State<'_, Bridge>,
    tools: State<'_, MediaTools>,
    url: String,
) -> Result<MediaProbe, UiError> {
    let sidecars = tools.sidecars(&bridge)?;
    let status = sidecars.status().map_err(|e| media_error(&e))?;
    if status.tools.iter().any(|t| t.active.is_none()) {
        return Ok(MediaProbe::NotInstalled);
    }
    if let Err(e) = sidecars.update_if_due(SystemTime::now(), &|_| {}).await {
        tracing::warn!(error = %e, "the video tools' update check failed");
    }
    let prober = sidecars.prober().map_err(|e| media_error(&e))?;
    let info = prober
        .probe(url.trim(), &ProbeOptions::default())
        .await
        .map_err(|e| media_error(&e))?;
    Ok(MediaProbe::Found {
        media: MediaSummary::from(&info),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_shows_presets_with_everyday_codec_names() {
        let info = MediaInfo::from_json(
            br#"{"title": "Clip", "channel": "Chan", "duration": 10, "formats": [
                {"format_id": "a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "filesize": 100},
                {"format_id": "v", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 720, "filesize_approx": 900}
            ], "subtitles": {"en": [{"ext": "vtt", "name": "English"}]}}"#,
        )
        .unwrap();
        let s = MediaSummary::from(&info);
        assert_eq!(s.uploader.as_deref(), Some("Chan"));
        assert_eq!(s.presets.len(), 2);
        let best = &s.presets[0];
        assert_eq!(best.preset, "best");
        assert_eq!(best.video_codec.as_deref(), Some("H.264"));
        assert_eq!(best.audio_codec.as_deref(), Some("AAC"));
        assert_eq!((best.size, best.size_approximate), (Some(1000), true));
        assert_eq!(s.presets[1].preset, "audio");
        assert_eq!(s.subtitles[0].code, "en");
        assert!(!s.drm && !s.is_playlist);
    }

    #[tokio::test]
    async fn without_tools_nothing_runs() {
        let dir = tempfile::tempdir().unwrap();
        let tools = MediaTools::new(Some(dir.path().to_path_buf()));
        let (bridge, _hub) = Bridge::new(std::time::Duration::from_millis(50));
        let sidecars = tools.sidecars(&bridge).unwrap();
        let status = sidecars.status().unwrap();
        assert!(status.tools.iter().all(|t| t.active.is_none()));
        assert!(MediaTools::new(None).sidecars(&bridge).is_err());
    }
}
