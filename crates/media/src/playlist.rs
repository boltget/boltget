//! Playlists (`docs/VIDEO.md`, "Playlists: as built"): a playlist's entries
//! as Boltget lists them, the folder its entries go to, the file name of
//! each, and the item ranges of `--playlist-items`.
//!
//! A playlist is listed with `--flat-playlist`, so its entries are not
//! probed one by one: each selected entry becomes a download of its own
//! page with the same format expression, and yt-dlp's own `/` fallbacks
//! choose for each entry what it has.

use std::fmt;

use serde::Serialize;

use crate::info::{Entry, MediaInfo};

/// Entries listed per probe: a playlist with more offers "load more", which
/// probes the next page with `--playlist-items` (see [`ItemRange`]).
pub const PAGE_SIZE: u32 = 200;

/// The longest folder name made from a playlist's title (characters). Long
/// enough for any real title, short enough that folder, title and file name
/// stay well within Windows' path limit.
pub const MAX_FOLDER_CHARS: usize = 80;

/// A playlist's entry as the picker lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaylistEntry {
    /// Its place in the playlist, from 1.
    pub index: u32,
    /// The site's id for it.
    pub id: Option<String>,
    /// Its title.
    pub title: Option<String>,
    /// Its page: what its download fetches.
    pub url: Option<String>,
    /// Length in seconds.
    pub duration: Option<f64>,
    /// Who published it, when the site names each entry's channel.
    pub uploader: Option<String>,
    /// A thumbnail, when the flat entry carries one.
    pub thumbnail: Option<String>,
    /// It can be downloaded: it has a page and is not private or deleted.
    pub available: bool,
}

impl PlaylistEntry {
    fn new(entry: &Entry, index: u32) -> Self {
        Self {
            index,
            id: entry.id.clone(),
            title: entry.title.clone(),
            url: entry.url.clone(),
            duration: entry.duration,
            uploader: entry.publisher().map(str::to_owned),
            thumbnail: entry.best_thumbnail().map(str::to_owned),
            available: entry.available(),
        }
    }
}

/// A playlist's entries, numbered: yt-dlp's `playlist_index` where it gives
/// one, else their place in the list counted from `first` (the start of
/// the probed range).
pub fn entries(info: &MediaInfo, first: u32) -> Vec<PlaylistEntry> {
    info.entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let place = first.saturating_add(u32::try_from(i).unwrap_or(u32::MAX));
            let index = e
                .playlist_index
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n > 0)
                .unwrap_or(place);
            PlaylistEntry::new(e, index)
        })
        .collect()
}

/// The folder a playlist's entries go to, inside the Video folder: its
/// title made safe as a folder name (Windows' rules on every platform), cut
/// at [`MAX_FOLDER_CHARS`]; `fallback` (the playlist's id, else
/// "Playlist") when nothing usable is left.
pub fn folder_name(title: Option<&str>, fallback: Option<&str>) -> String {
    let name = |s: &str| {
        let cut: String = s.trim().chars().take(MAX_FOLDER_CHARS).collect();
        // Cutting can leave a trailing dot or space; the sanitiser drops them.
        boltget_engine::safe_file_name(&cut)
    };
    [title, fallback]
        .into_iter()
        .flatten()
        .map(name)
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "Playlist".to_owned())
}

/// The output template of a playlist's entry: its place, zero-padded to
/// the width of the largest place, then the default name:
/// `07 - %(title)s [%(id)s].%(ext)s`. The place is written by Boltget, not
/// taken from yt-dlp's `%(playlist_index)s`, since each entry is downloaded
/// from its own page, where yt-dlp knows no playlist.
pub fn entry_template(index: u32, last: u32) -> String {
    let width = digits(last.max(index)).max(2);
    format!(
        "{index:0width$} - {}",
        boltget_engine::DEFAULT_OUTPUT_TEMPLATE
    )
}

fn digits(n: u32) -> usize {
    n.checked_ilog10().map_or(1, |d| d as usize + 1)
}

/// Whether a page's address names a list it belongs to (`list=` in its
/// query, as a video's page opened from a playlist has on some sites): the
/// picker then offers "this video" or "the whole
/// playlist". A heuristic over the address only; the probe with
/// [`PlaylistMode::Entries`](crate::PlaylistMode::Entries) decides what the
/// list holds.
pub fn names_a_list(page: &str) -> bool {
    url::Url::parse(page).is_ok_and(|u| {
        u.query_pairs()
            .any(|(k, v)| k == "list" && !v.trim().is_empty())
    })
}

/// A range of a playlist's places for `--playlist-items` (`-I`): `start`
/// to `end`, both from 1 and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemRange {
    /// The first place.
    pub start: u32,
    /// The last place.
    pub end: u32,
}

impl ItemRange {
    /// The page of [`PAGE_SIZE`] entries that starts at `start` (from 1).
    pub fn page(start: u32) -> Self {
        let start = start.max(1);
        Self {
            start,
            end: start.saturating_add(PAGE_SIZE - 1),
        }
    }
}

impl fmt::Display for ItemRange {
    /// yt-dlp's `START:END` (both inclusive in `--playlist-items`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.start, self.end)
    }
}

/// Parses a selection of places as the CLI takes it (`1-5,8`, `3`, `10-`):
/// places from 1, single or as ranges (`a-b`, both inclusive; `a-` to the
/// end), separated by commas. Returns whether a place is selected, given the
/// last place. Refuses anything else, so the text never reaches yt-dlp.
pub fn parse_items(text: &str) -> Result<Items, String> {
    let mut parts = Vec::new();
    for raw in text.split(',') {
        let part = raw.trim();
        let number = |s: &str| -> Result<u32, String> {
            match s.trim().parse::<u32>() {
                Ok(n) if n > 0 => Ok(n),
                _ => Err(format!(
                    "{part:?} is not a place in the playlist (1, 2, ...)"
                )),
            }
        };
        let item = match part.split_once('-') {
            None => {
                let n = number(part)?;
                (n, Some(n))
            }
            Some((a, b)) if b.trim().is_empty() => (number(a)?, None),
            Some((a, b)) => {
                let (a, b) = (number(a)?, number(b)?);
                if b < a {
                    return Err(format!("{part:?} ends before it starts"));
                }
                (a, Some(b))
            }
        };
        parts.push(item);
    }
    Ok(Items(parts))
}

/// Places of a playlist chosen with [`parse_items`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Items(Vec<(u32, Option<u32>)>);

impl Items {
    /// Whether place `index` (from 1) is chosen.
    pub fn contains(&self, index: u32) -> bool {
        self.0
            .iter()
            .any(|&(a, b)| index >= a && b.is_none_or(|b| index <= b))
    }

    /// The highest place named, or `None` when a range runs to the end.
    pub fn last(&self) -> Option<u32> {
        self.0
            .iter()
            .map(|&(_, b)| b)
            .try_fold(0, |max, b| b.map(|b| max.max(b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(
            folder_name(Some("Weekend projects"), None),
            "Weekend projects"
        );
        assert_eq!(
            folder_name(Some(r#"A/B: "best" of <2026>?"#), None),
            "B_ _best_ of _2026__"
        );
        assert_eq!(folder_name(Some("CON"), None), "_CON");
        assert_eq!(folder_name(Some(" ... "), Some("PL123")), "PL123");
        assert_eq!(folder_name(None, None), "Playlist");
        assert_eq!(folder_name(Some("..\\..\\x"), None), "x");
        let long = "word ".repeat(40);
        let name = folder_name(Some(&long), None);
        assert!(name.chars().count() <= MAX_FOLDER_CHARS, "{name}");
        assert!(!name.ends_with(' '));
        assert!(!folder_name(Some("tab\there"), None).contains('\t'));
    }

    #[test]
    fn entry_templates_number_the_files() {
        assert_eq!(entry_template(7, 12), "07 - %(title)s [%(id)s].%(ext)s");
        assert_eq!(entry_template(7, 150), "007 - %(title)s [%(id)s].%(ext)s");
        assert_eq!(entry_template(3, 5), "03 - %(title)s [%(id)s].%(ext)s");
        assert_eq!(entry_template(1200, 5), "1200 - %(title)s [%(id)s].%(ext)s");
        // A file name for yt-dlp's -o: the allowlist takes it.
        crate::args::check_yt_dlp(&["-o".into(), entry_template(7, 12)]).unwrap();
    }

    #[test]
    fn lists_in_addresses() {
        assert!(names_a_list("https://www.youtube.com/watch?v=a&list=PL1"));
        assert!(names_a_list("https://v.test/watch?list=PL1&index=3"));
        assert!(!names_a_list("https://www.youtube.com/watch?v=a"));
        assert!(!names_a_list("https://v.test/watch?v=a&list="));
        assert!(!names_a_list("https://v.test/list/watch?playlist=a"));
        assert!(!names_a_list("not a url"));
    }

    #[test]
    fn item_ranges() {
        assert_eq!(ItemRange::page(1).to_string(), "1:200");
        assert_eq!(ItemRange::page(201).to_string(), "201:400");
        assert_eq!(ItemRange::page(0).to_string(), "1:200");
        let items = parse_items("1-5, 8,10-").unwrap();
        assert!(items.contains(1) && items.contains(5) && items.contains(8));
        assert!(!items.contains(6) && !items.contains(9));
        assert!(items.contains(10) && items.contains(9999));
        assert_eq!(items.last(), None);
        assert_eq!(parse_items("3,1-2").unwrap().last(), Some(3));
        for bad in ["", "0", "a", "5-2", "1,,2", "-3", "1-x", "1:5"] {
            assert!(parse_items(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn entries_are_numbered_and_checked() {
        let info = MediaInfo::from_json(
            br#"{"_type": "playlist", "entries": [
                {"id": "a", "url": "https://v.test/w?v=a", "title": "A", "duration": 61,
                 "thumbnails": [{"url": "https://i.test/a-s.jpg", "width": 120, "height": 90},
                                {"url": "https://i.test/a-l.jpg", "width": 480, "height": 360}]},
                {"id": "b", "url": "https://v.test/w?v=b", "title": "[Private video]"},
                {"id": "c", "url": "c", "title": "Only an id"},
                {"id": "d", "url": "https://v.test/w?v=d", "title": "D", "playlist_index": 9,
                 "channel": "Chan"},
                {"id": "e", "url": "https://v.test/w?v=e", "title": "E", "availability": "private"}
            ]}"#,
        )
        .unwrap();
        let list = entries(&info, 201);
        let index: Vec<u32> = list.iter().map(|e| e.index).collect();
        assert_eq!(index, [201, 202, 203, 9, 205]);
        let available: Vec<bool> = list.iter().map(|e| e.available).collect();
        assert_eq!(available, [true, false, false, true, false]);
        assert_eq!(list[0].thumbnail.as_deref(), Some("https://i.test/a-l.jpg"));
        assert_eq!(list[3].uploader.as_deref(), Some("Chan"));
    }
}
