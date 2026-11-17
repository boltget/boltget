//! Which copied links are video pages (docs/VIDEO.md, "Copied video links:
//! as built"): a short, curated list of well-known video and audio sites
//! and the shapes of their page links, kept as data in `video_sites.json`
//! (the browser-only dev UI reads the same file). It is deliberately not
//! yt-dlp's extractor list: a match is only offered, and the media sheet's
//! probe decides what the page really holds.

use std::sync::LazyLock;

use serde::Deserialize;
use url::Url;

use super::percent_decode;

/// The list, as shipped.
const SITES_JSON: &str = include_str!("video_sites.json");

/// A copied link that looks like a page with a video (or, on a music site,
/// audio).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VideoPage {
    /// The site's name, as people know it (Vimeo, say).
    pub(crate) site: String,
    /// A music site: the offer says audio.
    pub(crate) audio: bool,
}

#[derive(Debug, Deserialize)]
struct SiteList {
    sites: Vec<Entry>,
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the tests only"))]
    #[serde(rename = "notPages", default)]
    not_pages: Vec<String>,
}

/// One kind of page link of a site.
#[derive(Debug, Deserialize)]
struct Entry {
    site: String,
    #[serde(default)]
    audio: bool,
    hosts: Vec<String>,
    paths: Vec<String>,
    #[serde(default)]
    except: Vec<String>,
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the tests only"))]
    #[serde(default)]
    examples: Vec<String>,
}

static SITES: LazyLock<SiteList> = LazyLock::new(|| {
    serde_json::from_str(SITES_JSON).unwrap_or_else(|error| {
        // The tests read the same file; this cannot happen in a build that
        // passed them.
        tracing::warn!(%error, "the list of video sites is unreadable");
        SiteList {
            sites: Vec::new(),
            not_pages: Vec::new(),
        }
    })
});

/// The video page `link` is on a well-known site, if it is one: an http(s)
/// link whose host is one of a site's and whose path (and query) has one of
/// its page shapes. A site's home page, search or channel pages are not.
pub(crate) fn video_page(link: &str) -> Option<VideoPage> {
    let url = Url::parse(link).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    let segments: Vec<String> = url
        .path_segments()
        .map(|s| {
            s.filter(|s| !s.is_empty())
                .map(|s| percent_decode(s).to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let query: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let path_matches = |pattern: &str| matches_path(pattern, &segments, &query);
    SITES
        .sites
        .iter()
        .find(|site| {
            site.hosts.iter().any(|h| matches_host(h, &host))
                && site.paths.iter().any(|p| path_matches(p))
                && !site.except.iter().any(|p| path_matches(p))
        })
        .map(|site| VideoPage {
            site: site.site.clone(),
            audio: site.audio,
        })
}

/// `example.com` is that host; `*.example.com` is it or any subdomain.
fn matches_host(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => {
            host == domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|sub| sub.ends_with('.'))
        }
        None => host == pattern,
    }
}

/// `/a/b?key`: the path starts with segments matching `a` and `b`, and the
/// query has a non-empty `key`. `segments` are decoded and lowercase.
fn matches_path(pattern: &str, segments: &[String], query: &[(String, String)]) -> bool {
    let (path, key) = pattern.split_once('?').unwrap_or((pattern, ""));
    let wanted: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    !wanted.is_empty()
        && wanted.len() <= segments.len()
        && wanted
            .iter()
            .zip(segments)
            .all(|(w, s)| matches_segment(&w.to_ascii_lowercase(), s))
        && (key.is_empty() || query.iter().any(|(k, v)| k == key && !v.is_empty()))
}

/// `*` is any segment, `{n}` digits, `x*y` starts with x and ends with y
/// with something between; anything else is itself.
fn matches_segment(pattern: &str, segment: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern == "{n}" {
        return segment.bytes().all(|b| b.is_ascii_digit());
    }
    match pattern.split_once('*') {
        Some((head, tail)) => {
            segment.len() > head.len() + tail.len()
                && segment.starts_with(head)
                && segment.ends_with(tail)
        }
        None => pattern == segment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_reads_and_every_pattern_names_a_path() {
        let list: SiteList = serde_json::from_str(SITES_JSON).expect("video_sites.json reads");
        assert!(list.sites.len() >= 20);
        for site in &list.sites {
            assert!(
                !site.hosts.is_empty() && !site.paths.is_empty(),
                "{}",
                site.site
            );
            assert!(!site.examples.is_empty(), "{} has no examples", site.site);
            for host in &site.hosts {
                assert_eq!(host, &host.to_ascii_lowercase(), "{host}");
                assert!(!host.contains('/') && !host.trim_start_matches("*.").contains('*'));
            }
            // A pattern without a segment would offer a site's home page.
            for path in site.paths.iter().chain(&site.except) {
                assert!(path.starts_with('/'), "{path}");
                let (p, _) = path.split_once('?').unwrap_or((path, ""));
                assert!(p.split('/').any(|s| !s.is_empty()), "{path}");
            }
        }
    }

    #[test]
    fn every_example_is_a_page_of_its_site() {
        for site in &SITES.sites {
            for example in &site.examples {
                let page = video_page(example);
                assert_eq!(
                    page.as_ref().map(|p| p.site.as_str()),
                    Some(site.site.as_str()),
                    "{example}"
                );
                assert_eq!(page.map(|p| p.audio), Some(site.audio), "{example}");
            }
        }
    }

    #[test]
    fn lookalikes_and_other_pages_are_not() {
        assert!(SITES.not_pages.len() >= 20);
        for link in &SITES.not_pages {
            assert_eq!(video_page(link), None, "{link}");
        }
        for link in ["", "not a link", "youtube.com/watch?v=x", "https://"] {
            assert_eq!(video_page(link), None, "{link}");
        }
    }

    #[test]
    fn hosts_match_whole_labels() {
        assert!(matches_host("*.youtube.com", "youtube.com"));
        assert!(matches_host("*.youtube.com", "m.youtube.com"));
        assert!(!matches_host("*.youtube.com", "notyoutube.com"));
        assert!(!matches_host("*.youtube.com", "youtube.com.evil.example"));
        assert!(matches_host("youtu.be", "youtu.be"));
        assert!(!matches_host("youtu.be", "www.youtu.be"));
    }

    #[test]
    fn segments_match_by_shape() {
        assert!(matches_segment("{n}", "123"));
        assert!(!matches_segment("{n}", "12a"));
        assert!(matches_segment("v*", "v4abc-title.html"));
        assert!(!matches_segment("v*", "v"));
        assert!(matches_segment("file:*.webm", "file:a.webm"));
        assert!(!matches_segment("file:*.webm", "file:.webm"));
        assert!(!matches_segment("file:*.webm", "file:a.webm.jpg"));
    }
}
