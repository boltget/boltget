//! Clipboard monitoring (docs/APP-DESIGN.md #9, "Settings as built"): off by
//! default; when it is on, a downloadable link another program copies is
//! offered, never added. So is a copied link to a video page of a
//! well-known site (`video`), which opens the media sheet when taken; a
//! setting of its own turns that part off. The page shows a quiet chip;
//! while the window is not in front, a notification offers it too (if
//! notifications are on).
//!
//! The clipboard is read only while the setting is on: Windows tells the
//! listener when it changes (`platform::watch_clipboard`), so there is no
//! polling, and turning the setting off stops the listener. Text Boltget
//! copied itself and text marked private (password managers) is never read.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use boltget_engine::RequestContext;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::bridge::Bridge;
use crate::platform::{self, ClipboardWatch};
use crate::settings::{self, Category};

mod video;

use video::{VideoPage, video_page};

/// More links than this at once is a page of text, not a copied link.
const MAX_LINKS: usize = 50;
/// Links offered recently, so copying one again does not offer it again.
const REMEMBERED: usize = 32;

/// Something worth offering was copied (sent to the page without showing
/// the window).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum ClipboardOffer {
    /// Downloadable links, one per line: taking them opens the add sheet.
    Files { links: String },
    /// A link to a video page of a well-known site: taking it opens the
    /// media sheet for it.
    Page {
        url: String,
        /// The site's name (Vimeo, say).
        site: String,
        /// A music site: the offer says audio rather than video.
        audio: bool,
    },
}

/// The video page in copied `text`, if that is what was copied: one http(s)
/// link, and nothing else, to a page of a well-known video or music site.
pub(crate) fn page_offer(text: &str) -> Option<(String, VideoPage)> {
    let mut tokens = text.split_whitespace();
    let link = tokens.next()?;
    if tokens.next().is_some() {
        return None;
    }
    video_page(link).map(|page| (link.to_owned(), page))
}

/// The links in copied `text` worth offering: the text must be nothing but
/// http(s) links (copied prose with a link in it is not an offer), and each
/// offered link must look like a file: its name, or a `filename` in its
/// query, has an extension of one of the categories.
pub(crate) fn offer(text: &str, categories: &[Category]) -> Vec<String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > MAX_LINKS {
        return Vec::new();
    }
    let mut links: Vec<String> = Vec::new();
    for token in tokens {
        let lower = token.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://"))
            || RequestContext::new(token).is_err()
        {
            return Vec::new();
        }
        if looks_like_a_file(token, categories) && !links.iter().any(|l| l == token) {
            links.push(token.to_owned());
        }
    }
    links
}

/// Whether a link names a file of a known type.
fn looks_like_a_file(link: &str, categories: &[Category]) -> bool {
    let known = |name: &str| settings::category_of(categories, name).is_some();
    let rest = link.split_once("://").map_or(link, |(_, r)| r);
    let rest = rest.split('#').next().unwrap_or(rest);
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let name = path.split_once('/').map_or("", |(_, p)| p);
    let last = name.rsplit('/').next().unwrap_or("");
    if known(&percent_decode(last)) {
        return true;
    }
    // Signed links often carry the name in the query instead
    // (`?filename=a.zip`, `response-content-disposition=attachment;
    // filename="a.zip"`).
    query.split('&').any(|pair| {
        let value = percent_decode(pair.split_once('=').map_or("", |(_, v)| v));
        let key = pair.split('=').next().unwrap_or("").to_ascii_lowercase();
        let named = if key == "filename" || key == "file" {
            Some(value.as_str())
        } else {
            value
                .to_ascii_lowercase()
                .find("filename")
                .and_then(|at| value[at..].split_once('='))
                .map(|(_, v)| v)
        };
        named.is_some_and(|n| {
            let n = n.trim_start_matches(['"', '\'', ' ']);
            let n = n.split(['"', '\'', ';', '&']).next().unwrap_or(n);
            known(n.trim())
        })
    })
}

/// `%XX` decoding for names in links; anything malformed stays as it is.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(&h), Some(&l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(h), Some(l)) = (hex(h), hex(l))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Links offered lately: a link is offered once, however often it is
/// copied (APP-DESIGN #9, "repeats collapse").
#[derive(Debug, Default)]
pub(crate) struct Recent(VecDeque<String>);

impl Recent {
    /// The links not offered lately, remembering them.
    pub(crate) fn fresh(&mut self, links: Vec<String>) -> Vec<String> {
        let fresh: Vec<String> = links
            .into_iter()
            .filter(|l| !self.0.iter().any(|r| r == l))
            .collect();
        for link in &fresh {
            if self.0.len() == REMEMBERED {
                self.0.pop_front();
            }
            self.0.push_back(link.clone());
        }
        fresh
    }
}

/// The listener while monitoring is on, and what it offered lately.
#[derive(Debug, Default)]
pub(crate) struct Monitor {
    watch: Mutex<Option<ClipboardWatch>>,
    recent: Mutex<Recent>,
}

/// Follows the setting for as long as the app runs: listens while it is on,
/// and not at all while it is off.
pub(crate) fn start(app: &AppHandle) {
    let mut settings = app.state::<Bridge>().watch_settings();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let on = settings.borrow_and_update().clipboard;
            set_listening(&app, on);
            if settings.changed().await.is_err() {
                return;
            }
        }
    });
}

fn set_listening(app: &AppHandle, on: bool) {
    let monitor = app.state::<Monitor>();
    let mut watch = monitor.watch.lock().unwrap_or_else(PoisonError::into_inner);
    match (on, watch.is_some()) {
        (true, false) => {
            let handle = app.clone();
            *watch = platform::watch_clipboard(move |text| copied(&handle, &text));
            if watch.is_some() {
                tracing::info!("clipboard monitoring is on");
            }
        }
        (false, true) => {
            // Dropping the watch stops the listener.
            *watch = None;
            tracing::info!("clipboard monitoring is off");
        }
        _ => {}
    }
}

/// What copied `text` offers, if anything: a video page (when that part of
/// monitoring is on) wins over the same link as a file, so a media page
/// whose address ends in `.webm` opens the media sheet; anything else is
/// offered as files, as before.
pub(crate) fn offer_for(
    text: &str,
    categories: &[Category],
    video_pages: bool,
) -> Option<ClipboardOffer> {
    if video_pages && let Some((url, page)) = page_offer(text) {
        return Some(ClipboardOffer::Page {
            url,
            site: page.site,
            audio: page.audio,
        });
    }
    let links = offer(text, categories);
    (!links.is_empty()).then(|| ClipboardOffer::Files {
        links: links.join("\n"),
    })
}

/// Another program copied `text` (on the listener's thread).
fn copied(app: &AppHandle, text: &str) {
    let bridge = app.state::<Bridge>();
    let current = bridge.settings();
    if !current.clipboard || bridge.engine().is_err() {
        return;
    }
    let Some(offer) = offer_for(text, &current.categories, current.clipboard_video) else {
        return;
    };
    let monitor = app.state::<Monitor>();
    let offer = {
        let mut recent = monitor
            .recent
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // A link is offered once, whichever way it was offered.
        match offer {
            ClipboardOffer::Files { links } => {
                let links = recent.fresh(links.lines().map(str::to_owned).collect());
                if links.is_empty() {
                    return;
                }
                ClipboardOffer::Files {
                    links: links.join("\n"),
                }
            }
            ClipboardOffer::Page { url, site, audio } => {
                if recent.fresh(vec![url.clone()]).is_empty() {
                    return;
                }
                ClipboardOffer::Page { url, site, audio }
            }
        }
    };
    if let Err(e) = offer.emit(app) {
        tracing::warn!(error = %e, "could not offer a copied link");
    }
    crate::notify::offer(app, &offer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    fn offered(text: &str) -> Vec<String> {
        offer(text, &AppSettings::default().categories)
    }

    #[test]
    fn file_links_are_offered() {
        assert_eq!(
            offered("https://example.com/files/ubuntu-24.04.iso"),
            ["https://example.com/files/ubuntu-24.04.iso"]
        );
        assert_eq!(
            offered("  http://x.test/a%20b.MKV?token=1#top \n"),
            ["http://x.test/a%20b.MKV?token=1#top"]
        );
        // Several links, one per line; only the files, once each.
        assert_eq!(
            offered(
                "https://x.test/a.zip\nhttps://x.test/page\nhttps://x.test/a.zip https://x.test/b.pdf"
            ),
            ["https://x.test/a.zip", "https://x.test/b.pdf"]
        );
    }

    #[test]
    fn names_in_the_query_count() {
        assert_eq!(
            offered("https://cdn.test/get?id=7&filename=setup.exe").len(),
            1
        );
        assert_eq!(
            offered(
                "https://bucket.test/obj?response-content-disposition=attachment%3B%20filename%3D%22report.pdf%22&sig=abc"
            )
            .len(),
            1
        );
        assert_eq!(offered("https://cdn.test/get?id=7&name=setup").len(), 0);
    }

    #[test]
    fn pages_prose_and_other_schemes_are_not_offered() {
        for text in [
            "",
            "   ",
            "https://example.com/",
            "https://example.com/blog/post",
            "https://example.com/index.html",
            "https://example.com/search?q=a.zip",
            "Get it at https://example.com/a.zip today",
            "ftp://example.com/a.zip",
            "file:///C:/Users/me/a.zip",
            "C:\\Users\\me\\a.zip",
            "a.zip",
            "https://example.com/.zip",
        ] {
            assert!(offered(text).is_empty(), "{text:?}");
        }
        let many = (0..51)
            .map(|i| format!("https://x.test/{i}.zip"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(offered(&many).len(), 0);
    }

    #[test]
    fn categories_decide_what_a_file_is() {
        let mut s = AppSettings::default();
        assert_eq!(offer("https://x.test/a.xyz", &s.categories).len(), 0);
        s.categories[0].extensions.push("xyz".into());
        assert_eq!(offer("https://x.test/a.xyz", &s.categories).len(), 1);
    }

    #[test]
    fn repeats_collapse() {
        let mut recent = Recent::default();
        let a = || vec!["https://x.test/a.zip".to_owned()];
        assert_eq!(recent.fresh(a()).len(), 1);
        assert_eq!(recent.fresh(a()).len(), 0);
        // Old links are forgotten eventually.
        for i in 0..REMEMBERED {
            recent.fresh(vec![format!("https://x.test/{i}.zip")]);
        }
        assert_eq!(recent.fresh(a()).len(), 1);
    }

    #[test]
    fn a_copied_video_page_is_offered_as_one() {
        let categories = AppSettings::default().categories;
        let offer = |text: &str, on: bool| offer_for(text, &categories, on);
        assert_eq!(
            offer(" https://www.youtube.com/watch?v=dQw4w9WgXcQ\r\n", true),
            Some(ClipboardOffer::Page {
                url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
                site: "YouTube".into(),
                audio: false,
            })
        );
        assert!(matches!(
            offer("https://soundcloud.com/an-artist/a-track", true),
            Some(ClipboardOffer::Page { audio: true, .. })
        ));
        // A media page that looks like a file is a page when pages are on,
        // and a file as before when they are off.
        let commons = "https://commons.wikimedia.org/wiki/File:Big_Buck_Bunny_4K.webm";
        assert!(matches!(
            offer(commons, true),
            Some(ClipboardOffer::Page { .. })
        ));
        assert!(matches!(
            offer(commons, false),
            Some(ClipboardOffer::Files { .. })
        ));
        // Off: a video page is a web page, which is not offered.
        assert_eq!(offer("https://youtu.be/dQw4w9WgXcQ", false), None);
        // Only a link on its own: two links, or prose, are not a page.
        assert_eq!(offer("https://youtu.be/a https://youtu.be/b", true), None);
        assert_eq!(offer("watch https://youtu.be/a", true), None);
        assert_eq!(
            offer("https://youtu.be/a\nhttps://x.test/b.zip", true),
            Some(ClipboardOffer::Files {
                links: "https://x.test/b.zip".into()
            })
        );
        // Files are still files; home pages are nothing.
        assert_eq!(
            offer("https://x.test/a.zip", true),
            Some(ClipboardOffer::Files {
                links: "https://x.test/a.zip".into()
            })
        );
        assert_eq!(offer("https://www.youtube.com/", true), None);
    }

    #[test]
    fn percent_decoding_is_forgiving() {
        assert_eq!(percent_decode("a%20b%2Ezip"), "a b.zip");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("a+b"), "a b");
    }
}
