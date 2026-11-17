//! yt-dlp's info JSON (`yt-dlp -J`) as a typed model.
//!
//! Only the fields Boltget uses are read, and every one of them leniently:
//! unknown fields are ignored, missing fields and `null` are `None` (or
//! empty), a number may come as an integer or a float, and an element of a
//! list that does not have the expected shape is skipped rather than failing
//! the whole page. yt-dlp's extractors are many and not uniform; one odd
//! field must not hide a video. See `docs/VIDEO.md`, "Probing and presets:
//! as built".

use std::collections::BTreeMap;

use serde::de::{DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a page holds: one video (or audio), or a playlist.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MediaInfo {
    /// `video` (also for audio-only pages), `playlist`, `multi_video` or
    /// `url` (an unresolved entry). yt-dlp leaves it out for a video.
    #[serde(rename = "_type", default, deserialize_with = "string")]
    pub kind: Option<String>,
    /// The site's id for it.
    #[serde(default, deserialize_with = "string")]
    pub id: Option<String>,
    /// Its title.
    #[serde(default, deserialize_with = "string")]
    pub title: Option<String>,
    /// Who published it.
    #[serde(default, deserialize_with = "string")]
    pub uploader: Option<String>,
    /// The channel, where the site has one (a fallback for the uploader).
    #[serde(default, deserialize_with = "string")]
    pub channel: Option<String>,
    /// Length in seconds.
    #[serde(default, deserialize_with = "float")]
    pub duration: Option<f64>,
    /// The page's canonical URL.
    #[serde(default, deserialize_with = "string")]
    pub webpage_url: Option<String>,
    /// The extractor's name (`youtube`, `generic`, ...).
    #[serde(default, deserialize_with = "string")]
    pub extractor: Option<String>,
    /// The extractor's key (`Youtube`, `Generic`, ...).
    #[serde(default, deserialize_with = "string")]
    pub extractor_key: Option<String>,
    /// The thumbnail yt-dlp chose.
    #[serde(default, deserialize_with = "string")]
    pub thumbnail: Option<String>,
    /// Every thumbnail, worst first.
    #[serde(default, deserialize_with = "list")]
    pub thumbnails: Vec<Thumbnail>,
    /// The formats, in yt-dlp's order of preference: worst first, best last.
    #[serde(default, deserialize_with = "list")]
    pub formats: Vec<Format>,
    /// Subtitles by language.
    #[serde(default, deserialize_with = "tracks")]
    pub subtitles: BTreeMap<String, Vec<SubtitleTrack>>,
    /// Automatic captions by language.
    #[serde(default, deserialize_with = "tracks")]
    pub automatic_captions: BTreeMap<String, Vec<SubtitleTrack>>,
    /// A live stream.
    #[serde(default, deserialize_with = "flag")]
    pub is_live: bool,
    /// yt-dlp found formats but all of them are protected by DRM (it then
    /// reports no formats).
    #[serde(rename = "_has_drm", default, deserialize_with = "flag")]
    pub has_drm: bool,
    /// A playlist's entries (with `--flat-playlist`, unresolved).
    #[serde(default, deserialize_with = "list")]
    pub entries: Vec<Entry>,
    /// A playlist's number of entries, when the site says.
    #[serde(default, deserialize_with = "unsigned")]
    pub playlist_count: Option<u64>,
}

/// One format: a video stream, an audio stream, or both together.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Format {
    /// yt-dlp's id for it (what `-f` names).
    #[serde(default, deserialize_with = "string_or_empty")]
    pub format_id: String,
    /// The file extension (`mp4`, `webm`, `m4a`, ...).
    #[serde(default, deserialize_with = "string")]
    pub ext: Option<String>,
    /// How it is fetched (`https`, `m3u8_native`, `http_dash_segments`, ...).
    #[serde(default, deserialize_with = "string")]
    pub protocol: Option<String>,
    /// The video codec.
    #[serde(default, deserialize_with = "codec")]
    pub vcodec: Codec,
    /// The audio codec.
    #[serde(default, deserialize_with = "codec")]
    pub acodec: Codec,
    /// Width in pixels.
    #[serde(default, deserialize_with = "unsigned32")]
    pub width: Option<u32>,
    /// Height in pixels.
    #[serde(default, deserialize_with = "unsigned32")]
    pub height: Option<u32>,
    /// Frames per second.
    #[serde(default, deserialize_with = "float")]
    pub fps: Option<f64>,
    /// Total bit rate, kbit/s.
    #[serde(default, deserialize_with = "float")]
    pub tbr: Option<f64>,
    /// Video bit rate, kbit/s.
    #[serde(default, deserialize_with = "float")]
    pub vbr: Option<f64>,
    /// Audio bit rate, kbit/s.
    #[serde(default, deserialize_with = "float")]
    pub abr: Option<f64>,
    /// The exact size in bytes.
    #[serde(default, deserialize_with = "unsigned")]
    pub filesize: Option<u64>,
    /// An estimate of the size in bytes.
    #[serde(default, deserialize_with = "unsigned")]
    pub filesize_approx: Option<u64>,
    /// `SDR`, `HDR10`, `HLG`, ...
    #[serde(default, deserialize_with = "string")]
    pub dynamic_range: Option<String>,
    /// Protected by DRM (`true`; yt-dlp's `"maybe"` counts as not).
    #[serde(default, deserialize_with = "flag")]
    pub has_drm: bool,
    /// A note from the site (`1080p`, `medium`, `Default`, ...).
    #[serde(default, deserialize_with = "string")]
    pub format_note: Option<String>,
    /// The audio language, where known.
    #[serde(default, deserialize_with = "string")]
    pub language: Option<String>,
}

/// A stream's codec as yt-dlp reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    /// The format has no such stream (yt-dlp's `"none"`).
    None,
    /// yt-dlp does not know (field missing or `null`). yt-dlp's format
    /// selection treats this as present, and so does Boltget.
    #[default]
    Unknown,
    /// A named codec, such as `avc1.640028` or `opus`.
    Named(String),
}

impl Codec {
    /// Whether the format may carry this stream (named or unknown).
    pub fn present(&self) -> bool {
        !matches!(self, Codec::None)
    }

    /// The codec's name, if known.
    pub fn name(&self) -> Option<&str> {
        match self {
            Codec::Named(n) => Some(n),
            _ => None,
        }
    }
}

impl Format {
    /// Whether it may carry video.
    pub fn has_video(&self) -> bool {
        self.vcodec.present()
    }

    /// Whether it may carry audio.
    pub fn has_audio(&self) -> bool {
        self.acodec.present()
    }

    /// Audio only (yt-dlp's `ba`): audio, and certainly no video.
    pub fn audio_only(&self) -> bool {
        self.has_audio() && self.vcodec == Codec::None
    }

    /// The size: exact if known, else the estimate (`true` when estimated).
    pub fn size(&self) -> Option<(u64, bool)> {
        self.filesize
            .map(|s| (s, false))
            .or(self.filesize_approx.map(|s| (s, true)))
    }
}

/// One thumbnail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Thumbnail {
    /// Its address.
    #[serde(default, deserialize_with = "string_or_empty")]
    pub url: String,
    /// Width in pixels.
    #[serde(default, deserialize_with = "unsigned32")]
    pub width: Option<u32>,
    /// Height in pixels.
    #[serde(default, deserialize_with = "unsigned32")]
    pub height: Option<u32>,
}

/// One subtitle file a language offers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubtitleTrack {
    /// `vtt`, `srt`, `ttml`, `json3`, ...
    #[serde(default, deserialize_with = "string")]
    pub ext: Option<String>,
    /// The language's name, where the site gives one.
    #[serde(default, deserialize_with = "string")]
    pub name: Option<String>,
}

/// A playlist entry. With `--flat-playlist` only these few fields are known.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Entry {
    /// The site's id for it.
    #[serde(default, deserialize_with = "string")]
    pub id: Option<String>,
    /// Its title.
    #[serde(default, deserialize_with = "string")]
    pub title: Option<String>,
    /// Its page.
    #[serde(default, deserialize_with = "string")]
    pub url: Option<String>,
    /// Length in seconds.
    #[serde(default, deserialize_with = "float")]
    pub duration: Option<f64>,
    /// Who published it (some sites name each entry's channel).
    #[serde(default, deserialize_with = "string")]
    pub uploader: Option<String>,
    /// The channel, where the site has one.
    #[serde(default, deserialize_with = "string")]
    pub channel: Option<String>,
    /// The thumbnails a flat entry carries (some sites give them), worst
    /// first.
    #[serde(default, deserialize_with = "list")]
    pub thumbnails: Vec<Thumbnail>,
    /// One thumbnail, where the site names a single one.
    #[serde(default, deserialize_with = "string")]
    pub thumbnail: Option<String>,
    /// Its place in the playlist (from 1), when yt-dlp says.
    #[serde(default, deserialize_with = "unsigned")]
    pub playlist_index: Option<u64>,
    /// `public`, `unlisted`, `private`, `needs_auth`, ... where the site
    /// says.
    #[serde(default, deserialize_with = "string")]
    pub availability: Option<String>,
}

/// The titles sites give entries that cannot be watched (placeholders for
/// private and deleted videos).
const PLACEHOLDER_TITLES: &[&str] = &["[Private video]", "[Deleted video]", "[Unavailable video]"];

impl Entry {
    /// Whether the entry can be downloaded on its own: it has an `http(s)`
    /// page and is not a placeholder for a private or deleted video.
    pub fn available(&self) -> bool {
        let page = self.url.as_deref().is_some_and(|u| {
            url::Url::parse(u).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.has_host())
        });
        let placeholder = self
            .title
            .as_deref()
            .is_some_and(|t| PLACEHOLDER_TITLES.contains(&t.trim()));
        page && !placeholder && self.availability.as_deref() != Some("private")
    }

    /// The best thumbnail: the largest with known dimensions, else the one
    /// named, else the last listed.
    pub fn best_thumbnail(&self) -> Option<&str> {
        best_thumbnail(&self.thumbnails, self.thumbnail.as_deref())
    }

    /// Who published it: the uploader, else the channel.
    pub fn publisher(&self) -> Option<&str> {
        self.uploader.as_deref().or(self.channel.as_deref())
    }
}

fn best_thumbnail<'a>(thumbnails: &'a [Thumbnail], named: Option<&'a str>) -> Option<&'a str> {
    let sized = thumbnails
        .iter()
        .filter(|t| !t.url.is_empty())
        .filter_map(|t| Some((u64::from(t.width?) * u64::from(t.height?), t)))
        .max_by_key(|(area, _)| *area)
        .map(|(_, t)| t.url.as_str());
    sized.or(named).or_else(|| {
        thumbnails
            .iter()
            .rev()
            .find(|t| !t.url.is_empty())
            .map(|t| t.url.as_str())
    })
}

/// A language offered as subtitles or automatic captions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubtitleLanguage {
    /// The language code as the site names it (`en`, `pt-BR`).
    pub code: String,
    /// The language's name, if the site gives one.
    pub name: Option<String>,
    /// Automatic captions rather than subtitles.
    pub automatic: bool,
}

impl MediaInfo {
    /// Parses `yt-dlp -J` output.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        // Serde would also read a struct from an array; yt-dlp writes an object.
        if bytes.trim_ascii_start().first() != Some(&b'{') {
            return Err("not a JSON object".into());
        }
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    /// Whether this is a playlist (or another list of entries) rather than one
    /// video.
    pub fn is_playlist(&self) -> bool {
        matches!(self.kind.as_deref(), Some("playlist" | "multi_video"))
    }

    /// How many entries a playlist has: the site's count, else the entries
    /// listed.
    pub fn entry_count(&self) -> usize {
        self.playlist_count
            .map_or(self.entries.len(), |n| n as usize)
            .max(self.entries.len())
    }

    /// Who published it: the uploader, else the channel.
    pub fn publisher(&self) -> Option<&str> {
        self.uploader.as_deref().or(self.channel.as_deref())
    }

    /// Whether the media is protected by DRM and nothing else is offered:
    /// yt-dlp reported it, or every format it listed is protected.
    pub fn drm_only(&self) -> bool {
        self.has_drm || (!self.formats.is_empty() && self.formats.iter().all(|f| f.has_drm))
    }

    /// The highest video height among the formats.
    pub fn max_height(&self) -> Option<u32> {
        self.formats
            .iter()
            .filter(|f| f.has_video() && !f.has_drm)
            .filter_map(|f| f.height)
            .max()
    }

    /// The best thumbnail: the largest one with known dimensions, else the one
    /// yt-dlp chose, else the last listed.
    pub fn best_thumbnail(&self) -> Option<&str> {
        best_thumbnail(&self.thumbnails, self.thumbnail.as_deref())
    }

    /// Every subtitle language, then every automatic-caption language, each
    /// in code order. A language with no tracks is left out.
    pub fn subtitle_languages(&self) -> Vec<SubtitleLanguage> {
        let list = |map: &BTreeMap<String, Vec<SubtitleTrack>>, automatic: bool| {
            map.iter()
                .filter(|(code, tracks)| !tracks.is_empty() && code.as_str() != "live_chat")
                .map(|(code, tracks)| SubtitleLanguage {
                    code: code.clone(),
                    name: tracks.iter().find_map(|t| t.name.clone()),
                    automatic,
                })
                .collect::<Vec<_>>()
        };
        let mut out = list(&self.subtitles, false);
        out.extend(list(&self.automatic_captions, true));
        out
    }
}

// Lenient field readers ------------------------------------------------------

fn string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn string_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    string(d).map(Option::unwrap_or_default)
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        // Some extractors put numbers in strings.
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f64| f.is_finite())
}

fn float<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(number(&Value::deserialize(d)?))
}

fn unsigned<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match &v {
        Value::Number(n) if n.as_u64().is_some() => n.as_u64(),
        _ => number(&v)
            .filter(|f| *f >= 0.0 && *f < u64::MAX as f64)
            .map(|f| f.round() as u64),
    })
}

fn unsigned32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    Ok(unsigned(d)?.and_then(|v| u32::try_from(v).ok()))
}

fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(matches!(Value::deserialize(d)?, Value::Bool(true)))
}

fn codec<'de, D: Deserializer<'de>>(d: D) -> Result<Codec, D::Error> {
    Ok(match string(d)? {
        None => Codec::Unknown,
        Some(s) if s.eq_ignore_ascii_case("none") => Codec::None,
        Some(s) if s.trim().is_empty() => Codec::Unknown,
        Some(s) => Codec::Named(s),
    })
}

/// A list whose elements that do not fit `T` are skipped.
fn list<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => Vec::new(),
    })
}

fn tracks<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Vec<SubtitleTrack>>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Object(map) => map
            .into_iter()
            .map(|(lang, v)| {
                let tracks = match v {
                    Value::Array(items) => items
                        .into_iter()
                        .filter_map(|v| serde_json::from_value(v).ok())
                        .collect(),
                    _ => Vec::new(),
                };
                (lang, tracks)
            })
            .collect(),
        _ => BTreeMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_values_are_read_leniently() {
        let info = MediaInfo::from_json(
            br#"{
                "title": 42,
                "duration": "61.5",
                "uploader": null,
                "channel": "Chan",
                "formats": [
                    {"format_id": "a", "vcodec": "none", "acodec": "opus", "filesize": 1000.4},
                    {"format_id": "b", "height": 720.0, "width": "1280", "tbr": null, "has_drm": "maybe"},
                    "not a format",
                    {"format_id": "c", "height": -5, "filesize_approx": 1e30, "vcodec": ""}
                ],
                "subtitles": {"en": [{"ext": "vtt", "name": "English"}], "xx": "bad", "live_chat": [{"ext": "json"}]},
                "thumbnails": null,
                "_has_drm": "yes",
                "unknown_field": {"anything": [1, 2]}
            }"#,
        )
        .unwrap();
        assert_eq!(info.title.as_deref(), Some("42"));
        assert_eq!(info.duration, Some(61.5));
        assert_eq!(info.publisher(), Some("Chan"));
        assert_eq!(info.formats.len(), 3);
        let [a, b, c] = &info.formats[..] else {
            panic!("three formats")
        };
        assert!(a.audio_only());
        assert_eq!(a.size(), Some((1000, false)));
        assert_eq!((b.width, b.height), (Some(1280), Some(720)));
        assert!(b.has_video() && b.has_audio() && !b.has_drm);
        assert_eq!(b.vcodec, Codec::Unknown);
        assert_eq!(c.height, None);
        assert_eq!(c.filesize_approx, None);
        assert_eq!(c.vcodec, Codec::Unknown);
        assert!(!info.has_drm, "only true counts");
        assert_eq!(info.thumbnails.len(), 0);
        let langs = info.subtitle_languages();
        assert_eq!(langs.len(), 1, "{langs:?}");
        assert_eq!(langs[0].name.as_deref(), Some("English"));
        assert!(MediaInfo::from_json(b"[]").is_err());
        assert!(MediaInfo::from_json(b"{}").is_ok());
    }
}
