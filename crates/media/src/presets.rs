//! The picker's presets (`docs/VIDEO.md`, "Format and quality picker UX"),
//! computed from a page's formats.
//!
//! Each preset is a yt-dlp format expression built from a few selectors, and
//! is resolved here the way yt-dlp would resolve it: yt-dlp lists formats
//! worst first, so a selector picks the **last** format that matches; `A+B`
//! needs both; `/` tries the next alternative. The selectors are yt-dlp's
//! own: `bv*` (any format with video), `ba` (audio only) and `b` (video and
//! audio together), with the filters `[height<=N]` and `[ext=...]`. A
//! format's codec counts as present unless yt-dlp says `none`, as in yt-dlp.

use std::fmt::Write as _;

use boltget_engine::ContainerPreference;
use serde::Serialize;

use crate::info::{Format, MediaInfo};

/// A preset of the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// The best video and audio.
    Best,
    /// At most 1080 lines.
    #[serde(rename = "1080p")]
    P1080,
    /// At most 720 lines.
    #[serde(rename = "720p")]
    P720,
    /// "Smaller file": at most 480 lines.
    Small,
    /// The best audio, without video.
    Audio,
}

impl Preset {
    /// Every preset, in the picker's order.
    pub const ALL: [Preset; 5] = [
        Preset::Best,
        Preset::P1080,
        Preset::P720,
        Preset::Small,
        Preset::Audio,
    ];

    /// The stored name (`best`, `1080p`, `720p`, `small`, `audio`), as in the
    /// `media_job` table.
    pub fn id(self) -> &'static str {
        match self {
            Preset::Best => "best",
            Preset::P1080 => "1080p",
            Preset::P720 => "720p",
            Preset::Small => "small",
            Preset::Audio => "audio",
        }
    }

    /// The inverse of [`Preset::id`].
    pub fn parse(s: &str) -> Option<Self> {
        Preset::ALL.into_iter().find(|p| p.id() == s)
    }

    /// The height limit, for the height presets.
    pub fn max_height(self) -> Option<u32> {
        match self {
            Preset::P1080 => Some(1080),
            Preset::P720 => Some(720),
            Preset::Small => Some(480),
            Preset::Best | Preset::Audio => None,
        }
    }

    /// Whether the result is audio only.
    pub fn audio_only(self) -> bool {
        self == Preset::Audio
    }

    /// The yt-dlp format expression (`-f`) for this preset.
    pub fn expression(self, container: ContainerPreference) -> String {
        render(&self.alternatives(container))
    }

    fn alternatives(self, container: ContainerPreference) -> Vec<Vec<Selector>> {
        let mp4 = container == ContainerPreference::Mp4;
        if self.audio_only() {
            let mut alts = Vec::new();
            if mp4 {
                alts.push(vec![Selector::new(Kind::BestAudio).ext("m4a")]);
            }
            alts.push(vec![Selector::new(Kind::BestAudio)]);
            alts.push(vec![Selector::new(Kind::Best)]);
            return alts;
        }
        let h = self.max_height();
        let video = Selector::new(Kind::AnyVideo).height(h);
        let both = Selector::new(Kind::Best).height(h);
        let audio = Selector::new(Kind::BestAudio);
        let mut alts = Vec::new();
        if mp4 {
            alts.push(vec![video.clone().ext("mp4"), audio.clone().ext("m4a")]);
            alts.push(vec![both.clone().ext("mp4")]);
        }
        alts.push(vec![video, audio]);
        alts.push(vec![both]);
        alts
    }
}

/// What a selector matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `bv*`: any format with video (with or without audio).
    AnyVideo,
    /// `ba`: audio only.
    BestAudio,
    /// `b`: video and audio in one format.
    Best,
}

/// One selector with its filters, such as `bv*[height<=720][ext=mp4]`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Selector {
    kind: Kind,
    max_height: Option<u32>,
    ext: Option<&'static str>,
}

impl Selector {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            max_height: None,
            ext: None,
        }
    }

    fn height(mut self, h: Option<u32>) -> Self {
        self.max_height = h;
        self
    }

    fn ext(mut self, ext: &'static str) -> Self {
        self.ext = Some(ext);
        self
    }

    fn matches(&self, f: &Format) -> bool {
        let kind = match self.kind {
            Kind::AnyVideo => f.has_video(),
            Kind::BestAudio => f.audio_only(),
            Kind::Best => f.has_video() && f.has_audio(),
        };
        // yt-dlp drops formats with DRM before selecting; a filter on a
        // value the format lacks does not match (`<=` without `?`).
        kind && !f.has_drm
            && self
                .max_height
                .is_none_or(|max| f.height.is_some_and(|h| h <= max))
            && self.ext.is_none_or(|e| f.ext.as_deref() == Some(e))
    }

    /// The last (best) matching format.
    fn pick<'a>(&self, formats: &'a [Format]) -> Option<&'a Format> {
        formats.iter().rev().find(|f| self.matches(f))
    }
}

fn render(alts: &[Vec<Selector>]) -> String {
    let mut out = String::new();
    for (i, alt) in alts.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        for (j, s) in alt.iter().enumerate() {
            if j > 0 {
                out.push('+');
            }
            out.push_str(match s.kind {
                Kind::AnyVideo => "bv*",
                Kind::BestAudio => "ba",
                Kind::Best => "b",
            });
            if let Some(h) = s.max_height {
                let _ = write!(out, "[height<={h}]");
            }
            if let Some(e) = s.ext {
                let _ = write!(out, "[ext={e}]");
            }
        }
    }
    out
}

/// A size estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SizeEstimate {
    /// Bytes.
    pub bytes: u64,
    /// Estimated rather than reported exactly (shown as "about").
    pub approximate: bool,
}

/// A preset as it resolves for one page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ResolvedPreset {
    /// The preset.
    pub preset: Preset,
    /// Its yt-dlp format expression.
    pub expression: String,
    /// The formats the expression selects (video first).
    pub format_ids: Vec<String>,
    /// Audio only.
    pub audio_only: bool,
    /// Width of the video, in pixels.
    pub width: Option<u32>,
    /// Height of the video, in pixels.
    pub height: Option<u32>,
    /// Frames per second.
    pub fps: Option<f64>,
    /// `HDR10`, `HLG`, ... (`None` for SDR or unknown).
    pub dynamic_range: Option<String>,
    /// The video codec (yt-dlp's name, such as `avc1.640028`).
    pub video_codec: Option<String>,
    /// The audio codec (such as `opus`).
    pub audio_codec: Option<String>,
    /// The file's extension after merging or extracting (`mp4`, `webm`,
    /// `mkv`, `m4a`, `opus`, ...).
    pub container: String,
    /// The estimated size: the formats' sizes added up (or their bit rates
    /// times the duration); `None` when unknown.
    pub size: Option<SizeEstimate>,
}

/// The presets for a page, in the picker's order: presets that select the
/// same formats into the same file as an earlier one are left out, and so
/// are height presets above the page's highest video and presets nothing
/// matches. A page whose media is all DRM-protected gets none
/// ([`MediaInfo::drm_only`]).
pub fn presets(info: &MediaInfo, container: ContainerPreference) -> Vec<ResolvedPreset> {
    if info.drm_only() {
        return Vec::new();
    }
    let max = info.max_height();
    let mut out: Vec<ResolvedPreset> = Vec::new();
    for preset in Preset::ALL {
        if let Some(limit) = preset.max_height()
            && max.is_none_or(|m| limit > m)
        {
            continue;
        }
        let Some(r) = resolve(info, preset, container) else {
            continue;
        };
        let same = out.iter().any(|o| {
            o.audio_only == r.audio_only
                && o.format_ids == r.format_ids
                && o.container == r.container
        });
        if !same {
            out.push(r);
        }
    }
    out
}

/// Resolves one preset against the page's formats; `None` if no alternative
/// of its expression matches.
pub fn resolve(
    info: &MediaInfo,
    preset: Preset,
    container: ContainerPreference,
) -> Option<ResolvedPreset> {
    let alts = preset.alternatives(container);
    let chosen: Vec<&Format> = alts.iter().find_map(|alt| {
        alt.iter()
            .map(|s| s.pick(&info.formats))
            .collect::<Option<Vec<_>>>()
    })?;
    let audio_only = preset.audio_only();
    let video = (!audio_only).then(|| chosen.first().copied()).flatten();
    let audio = chosen.iter().rev().find(|f| f.has_audio()).copied();
    let ext = if audio_only {
        audio_container(audio, container)
    } else {
        video_container(&chosen, container)
    };
    Some(ResolvedPreset {
        preset,
        expression: render(&alts),
        format_ids: chosen.iter().map(|f| f.format_id.clone()).collect(),
        audio_only,
        width: video.and_then(|v| v.width),
        height: video.and_then(|v| v.height),
        fps: video.and_then(|v| v.fps),
        dynamic_range: video
            .and_then(|v| v.dynamic_range.clone())
            .filter(|r| !r.eq_ignore_ascii_case("SDR")),
        video_codec: video.and_then(|v| v.vcodec.name().map(str::to_owned)),
        audio_codec: audio.and_then(|a| a.acodec.name().map(str::to_owned)),
        container: ext,
        size: estimate(&chosen, info.duration),
    })
}

/// The container of a video result: MP4 when preferred (yt-dlp merges or
/// remuxes into it); otherwise the format's own, or for two formats the one
/// yt-dlp picks when merging: theirs if they share a family, else MKV.
fn video_container(chosen: &[&Format], container: ContainerPreference) -> String {
    if container == ContainerPreference::Mp4 {
        return "mp4".into();
    }
    let exts: Vec<&str> = chosen
        .iter()
        .map(|f| f.ext.as_deref().unwrap_or("mp4"))
        .collect();
    match exts[..] {
        [one] => one.to_owned(),
        [v, a] => {
            const MP4: &[&str] = &["mp4", "m4a", "m4v", "mov"];
            const WEBM: &[&str] = &["webm", "weba"];
            if MP4.contains(&v) && MP4.contains(&a) {
                "mp4".into()
            } else if WEBM.contains(&v) && WEBM.contains(&a) {
                "webm".into()
            } else {
                "mkv".into()
            }
        }
        _ => "mkv".into(),
    }
}

/// The extension of an extracted audio file: M4A when MP4 is preferred
/// (converted if needed); otherwise the codec's own container, as yt-dlp's
/// audio extraction keeps the stream as it is.
fn audio_container(audio: Option<&Format>, container: ContainerPreference) -> String {
    if container == ContainerPreference::Mp4 {
        return "m4a".into();
    }
    let codec = audio
        .and_then(|a| a.acodec.name())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let by_codec = [
        ("opus", "opus"),
        ("mp4a", "m4a"),
        ("aac", "m4a"),
        ("alac", "m4a"),
        ("vorbis", "ogg"),
        ("mp3", "mp3"),
        ("flac", "flac"),
    ]
    .iter()
    .find(|(prefix, _)| codec.starts_with(prefix))
    .map(|(_, ext)| (*ext).to_owned());
    by_codec
        .or_else(|| audio.and_then(|a| a.ext.clone()))
        .unwrap_or_else(|| "m4a".into())
}

/// The formats' sizes added up: each format's exact size, else its estimate,
/// else its bit rate times the duration. `None` if any is unknown.
fn estimate(chosen: &[&Format], duration: Option<f64>) -> Option<SizeEstimate> {
    let mut total = SizeEstimate {
        bytes: 0,
        approximate: false,
    };
    for f in chosen {
        let (bytes, approximate) = f.size().or_else(|| {
            let kbps = f.tbr.or_else(|| match (f.vbr, f.abr) {
                (Some(v), Some(a)) => Some(v + a),
                (v, a) => v.or(a),
            })?;
            let secs = duration.filter(|d| *d > 0.0)?;
            Some(((kbps * 1000.0 / 8.0 * secs) as u64, true))
        })?;
        total.bytes = total.bytes.saturating_add(bytes);
        total.approximate |= approximate;
    }
    Some(total)
}

/// A codec's everyday name (`avc1.640028` is H.264, `mp4a.40.2` is AAC), or
/// yt-dlp's name when it is not a common one.
pub fn codec_label(codec: &str) -> String {
    let c = codec.to_ascii_lowercase();
    let known = [
        ("avc", "H.264"),
        ("h264", "H.264"),
        ("hev", "H.265"),
        ("hvc", "H.265"),
        ("h265", "H.265"),
        ("vp09", "VP9"),
        ("vp9", "VP9"),
        ("vp08", "VP8"),
        ("vp8", "VP8"),
        ("av01", "AV1"),
        ("mp4a", "AAC"),
        ("aac", "AAC"),
        ("opus", "Opus"),
        ("vorbis", "Vorbis"),
        ("mp3", "MP3"),
        ("flac", "FLAC"),
        ("ac-3", "AC-3"),
        ("ec-3", "E-AC-3"),
    ];
    known
        .iter()
        .find(|(prefix, _)| c.starts_with(prefix))
        .map_or_else(|| codec.to_owned(), |(_, label)| (*label).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_follow_the_design() {
        use ContainerPreference::{Mp4, Original};
        assert_eq!(Preset::Best.expression(Original), "bv*+ba/b");
        assert_eq!(
            Preset::P1080.expression(Original),
            "bv*[height<=1080]+ba/b[height<=1080]"
        );
        assert_eq!(
            Preset::Small.expression(Original),
            "bv*[height<=480]+ba/b[height<=480]"
        );
        assert_eq!(Preset::Audio.expression(Original), "ba/b");
        assert_eq!(
            Preset::Best.expression(Mp4),
            "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b"
        );
        assert_eq!(
            Preset::P720.expression(Mp4),
            "bv*[height<=720][ext=mp4]+ba[ext=m4a]/b[height<=720][ext=mp4]/bv*[height<=720]+ba/b[height<=720]"
        );
        assert_eq!(Preset::Audio.expression(Mp4), "ba[ext=m4a]/ba/b");
        for p in Preset::ALL {
            assert_eq!(Preset::parse(p.id()), Some(p));
            // Every expression passes the argument allowlist.
            crate::args::check_yt_dlp(&["-f".into(), p.expression(Mp4)]).unwrap();
        }
    }

    #[test]
    fn codec_labels() {
        assert_eq!(codec_label("avc1.640028"), "H.264");
        assert_eq!(codec_label("vp09.00.40.08"), "VP9");
        assert_eq!(codec_label("av01.0.08M.08"), "AV1");
        assert_eq!(codec_label("mp4a.40.2"), "AAC");
        assert_eq!(codec_label("opus"), "Opus");
        assert_eq!(codec_label("dvhe.05.06"), "dvhe.05.06");
    }
}
