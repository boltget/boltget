//! Parsers and formatters for the HTTP headers the engine depends on:
//! `Content-Range`, `Range`, `Retry-After`, and validators (`ETag`, `Last-Modified`).

use std::time::{Duration, SystemTime};

use http::HeaderMap;
use http::header::{self, HeaderValue};

/// A parsed `Content-Range: bytes first-last/total` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentRange {
    /// `bytes first-last/total` (`total` is `None` for `*`). `last` is inclusive.
    Bytes {
        first: u64,
        last: u64,
        total: Option<u64>,
    },
    /// `bytes */total`, sent with 416.
    Unsatisfied { total: u64 },
}

impl ContentRange {
    /// Parses a `Content-Range` value. Returns `None` for anything malformed.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let rest = value.trim().strip_prefix("bytes")?;
        // RFC 9110 uses a single space; be lenient about extra whitespace or '='.
        let rest = rest.trim_start_matches([' ', '=']).trim();
        let (range, total) = rest.split_once('/')?;
        let total = total.trim();
        let total = if total == "*" {
            None
        } else {
            Some(total.parse::<u64>().ok()?)
        };
        let range = range.trim();
        if range == "*" {
            return total.map(|total| ContentRange::Unsatisfied { total });
        }
        let (first, last) = range.split_once('-')?;
        let first = first.trim().parse::<u64>().ok()?;
        let last = last.trim().parse::<u64>().ok()?;
        if last < first {
            return None;
        }
        if let Some(t) = total
            && last >= t
        {
            return None;
        }
        Some(ContentRange::Bytes { first, last, total })
    }

    pub(crate) fn from_headers(headers: &HeaderMap) -> Option<Self> {
        headers
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(Self::parse)
    }
}

/// Formats a half-open range `[start, end)` as a `Range` header value.
/// `end == None` produces an open-ended range (`bytes=start-`).
pub(crate) fn range_header(start: u64, end: Option<u64>) -> HeaderValue {
    let s = match end {
        Some(end) => {
            debug_assert!(end > start, "empty range requested");
            format!("bytes={start}-{}", end - 1)
        }
        None => format!("bytes={start}-"),
    };
    HeaderValue::from_str(&s).expect("range header is ASCII by construction")
}

/// Parses `Retry-After` (delay-seconds or HTTP-date) relative to `now`.
pub(crate) fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = httpdate::parse_http_date(value).ok()?;
    Some(when.duration_since(now).unwrap_or(Duration::ZERO))
}

pub(crate) fn retry_after_from(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_retry_after(v, SystemTime::now()))
}

/// The value of `Content-Length`, if present and valid.
pub(crate) fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}

/// Whether `Content-Encoding` says the body is encoded (anything but `identity`).
pub(crate) fn is_encoded(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .any(|v| !v.is_empty() && !v.eq_ignore_ascii_case("identity"))
}

/// Validators recorded from the probe response, used to detect a changed resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Validators {
    /// `ETag`, verbatim (including any `W/` prefix).
    pub etag: Option<String>,
    /// `Last-Modified`, verbatim.
    pub last_modified: Option<String>,
    /// The response `Date`, verbatim.
    pub date: Option<String>,
}

impl Validators {
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let get = |name: header::HeaderName| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        Self {
            etag: get(header::ETAG),
            last_modified: get(header::LAST_MODIFIED),
            date: get(header::DATE),
        }
    }

    /// The strong `ETag`, if the recorded `ETag` is strong.
    pub fn strong_etag(&self) -> Option<&str> {
        self.etag.as_deref().filter(|e| !e.starts_with("W/"))
    }

    /// The value to send in `If-Range`, following RFC 9110: a strong `ETag`, else a
    /// `Last-Modified` at least one second before `Date`, else nothing.
    pub fn if_range(&self) -> Option<&str> {
        if let Some(etag) = self.strong_etag() {
            return Some(etag);
        }
        let lm = self.last_modified.as_deref()?;
        let date = self.date.as_deref()?;
        let lm_t = httpdate::parse_http_date(lm).ok()?;
        let date_t = httpdate::parse_http_date(date).ok()?;
        let gap = date_t.duration_since(lm_t).ok()?;
        (gap >= Duration::from_secs(1)).then_some(lm)
    }

    /// Checks a later response's validators against the recorded ones. Only
    /// validators present on both sides are compared. Returns a reason on mismatch.
    pub(crate) fn mismatch(&self, other: &Validators) -> Option<String> {
        if let (Some(a), Some(b)) = (&self.etag, &other.etag) {
            // Compare opaque tags; weak and strong forms of the same tag match weakly.
            if a.trim_start_matches("W/") != b.trim_start_matches("W/") {
                return Some(format!("ETag changed from {a} to {b}"));
            }
        }
        if let (Some(a), Some(b)) = (&self.last_modified, &other.last_modified)
            && a != b
        {
            return Some(format!("Last-Modified changed from {a} to {b}"));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            ContentRange::parse("bytes 0-99/100"),
            Some(ContentRange::Bytes {
                first: 0,
                last: 99,
                total: Some(100)
            })
        );
        assert_eq!(
            ContentRange::parse("bytes 5-9/*"),
            Some(ContentRange::Bytes {
                first: 5,
                last: 9,
                total: None
            })
        );
        assert_eq!(
            ContentRange::parse("bytes */0"),
            Some(ContentRange::Unsatisfied { total: 0 })
        );
        assert_eq!(
            ContentRange::parse("bytes=0-1/2"),
            Some(ContentRange::Bytes {
                first: 0,
                last: 1,
                total: Some(2)
            })
        );
        assert_eq!(ContentRange::parse("bytes 10-5/100"), None);
        assert_eq!(ContentRange::parse("bytes 0-100/100"), None);
        assert_eq!(ContentRange::parse("items 0-1/2"), None);
        assert_eq!(ContentRange::parse("bytes */*"), None);
        assert_eq!(ContentRange::parse("bytes x-1/2"), None);
    }

    #[test]
    fn range_header_format() {
        assert_eq!(range_header(0, None), "bytes=0-");
        assert_eq!(range_header(10, Some(20)), "bytes=10-19");
    }

    #[test]
    fn retry_after_parsing() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        assert_eq!(
            parse_retry_after("120", now),
            Some(Duration::from_secs(120))
        );
        let later = httpdate::fmt_http_date(now + Duration::from_secs(30));
        assert_eq!(
            parse_retry_after(&later, now),
            Some(Duration::from_secs(30))
        );
        let earlier = httpdate::fmt_http_date(now - Duration::from_secs(30));
        assert_eq!(parse_retry_after(&earlier, now), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("soon", now), None);
    }

    #[test]
    fn if_range_selection() {
        let strong = Validators {
            etag: Some("\"abc\"".into()),
            last_modified: None,
            date: None,
        };
        assert_eq!(strong.if_range(), Some("\"abc\""));

        let weak_with_dates = Validators {
            etag: Some("W/\"abc\"".into()),
            last_modified: Some("Sun, 06 Nov 1994 08:49:37 GMT".into()),
            date: Some("Sun, 06 Nov 1994 08:49:38 GMT".into()),
        };
        assert_eq!(
            weak_with_dates.if_range(),
            Some("Sun, 06 Nov 1994 08:49:37 GMT")
        );

        let same_second = Validators {
            etag: None,
            last_modified: Some("Sun, 06 Nov 1994 08:49:37 GMT".into()),
            date: Some("Sun, 06 Nov 1994 08:49:37 GMT".into()),
        };
        assert_eq!(same_second.if_range(), None);
        assert_eq!(Validators::default().if_range(), None);
    }

    #[test]
    fn validator_mismatch() {
        let a = Validators {
            etag: Some("\"1\"".into()),
            last_modified: Some("x".into()),
            date: None,
        };
        let mut b = a.clone();
        assert!(a.mismatch(&b).is_none());
        b.etag = Some("W/\"1\"".into());
        assert!(a.mismatch(&b).is_none());
        b.etag = Some("\"2\"".into());
        assert!(a.mismatch(&b).is_some());
        assert!(a.mismatch(&Validators::default()).is_none());
    }

    #[test]
    fn encoding_detection() {
        let mut h = HeaderMap::new();
        assert!(!is_encoded(&h));
        h.insert(
            header::CONTENT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        assert!(!is_encoded(&h));
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert!(is_encoded(&h));
    }
}
