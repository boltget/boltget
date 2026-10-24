//! The request context captured when a download is created.

use http::{HeaderMap, HeaderName, HeaderValue, header};
use url::Url;

use crate::error::{Error, Result};

/// Request headers copied from the caller (browser or user). Everything else is
/// dropped: hop-by-hop headers, `Range`, `If-*`, `Accept-Encoding`, `Host`,
/// `Content-Length` are controlled by the engine.
const ALLOWED_HEADERS: &[HeaderName] = &[
    header::USER_AGENT,
    header::REFERER,
    header::ACCEPT,
    header::ACCEPT_LANGUAGE,
    header::AUTHORIZATION,
    header::ORIGIN,
];

/// Everything needed to (re)issue requests for a download.
#[derive(Debug, Clone)]
pub struct RequestContext {
    url: Url,
    headers: HeaderMap,
    suggested_name: Option<String>,
}

impl RequestContext {
    /// Creates a context for a `GET` of `url`. Only `http` and `https` are supported.
    pub fn new(url: &str) -> Result<Self> {
        let url = Url::parse(url).map_err(|e| Error::InvalidUrl(format!("{e}")))?;
        Self::from_url(url)
    }

    /// Creates a context from a parsed URL.
    pub fn from_url(url: Url) -> Result<Self> {
        match url.scheme() {
            "http" | "https" => {}
            other => return Err(Error::InvalidUrl(format!("unsupported scheme {other:?}"))),
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err(Error::InvalidUrl("missing host".into()));
        }
        Ok(Self {
            url,
            headers: HeaderMap::new(),
            suggested_name: None,
        })
    }

    /// Adds a header if it is on the allowlist; returns whether it was kept.
    pub fn header(&mut self, name: HeaderName, value: HeaderValue) -> bool {
        if ALLOWED_HEADERS.contains(&name) {
            self.headers.insert(name, value);
            true
        } else {
            false
        }
    }

    /// Builder-style [`RequestContext::header`].
    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.header(name, value);
        self
    }

    /// Sets the file name suggested by the browser (used after `Content-Disposition`).
    #[must_use]
    pub fn with_suggested_name(mut self, name: impl Into<String>) -> Self {
        self.suggested_name = Some(name.into());
        self
    }

    /// The original URL.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The allowlisted headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The browser-suggested file name.
    pub fn suggested_name(&self) -> Option<&str> {
        self.suggested_name.as_deref()
    }
}

/// A URL with the query string, fragment and credentials removed, for logs and
/// error messages (signed URLs carry credentials in the query).
pub fn redact_url(url: &Url) -> String {
    let mut u = url.clone();
    let had_query = u.query().is_some();
    u.set_query(None);
    u.set_fragment(None);
    let _ = u.set_username("");
    let _ = u.set_password(None);
    let mut s = u.to_string();
    if had_query {
        s.push_str("?…");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        let mut ctx = RequestContext::new("https://example.com/a").unwrap();
        assert!(ctx.header(header::REFERER, HeaderValue::from_static("https://r/")));
        assert!(!ctx.header(header::RANGE, HeaderValue::from_static("bytes=0-")));
        assert!(!ctx.header(header::CONNECTION, HeaderValue::from_static("close")));
        assert!(!ctx.header(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip")));
        assert_eq!(ctx.headers().len(), 1);
    }

    #[test]
    fn schemes() {
        assert!(RequestContext::new("ftp://example.com/a").is_err());
        assert!(RequestContext::new("not a url").is_err());
        assert!(RequestContext::new("http://example.com").is_ok());
    }

    #[test]
    fn redaction() {
        let u = Url::parse("https://user:pw@example.com/f.iso?sig=secret#frag").unwrap();
        assert_eq!(redact_url(&u), "https://example.com/f.iso?…");
    }
}
