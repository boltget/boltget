//! The request context captured when a download is created.

use http::{HeaderMap, HeaderName, HeaderValue, header};
use url::Url;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::cookies::{Cookie, CookieJar};
use crate::error::{Error, Result};
use crate::proxy::{Proxy, ProxyChoice};

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

/// Headers that are secrets and therefore never written to the database. A
/// resumed download needs them supplied again (`docs/ARCHITECTURE.md`, "Security").
const SECRET_HEADERS: &[HeaderName] = &[header::AUTHORIZATION];

/// The persisted form of a [`RequestContext`] (without the URL, stored separately).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SavedContext {
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    suggested_name: Option<String>,
    /// `"direct"`, or the proxy URL without credentials (they are secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy: Option<String>,
    /// The `cookies.txt` the cookies came from, read again on resume (the
    /// cookies themselves are secrets and are not saved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cookie_file: Option<PathBuf>,
}

/// Everything needed to (re)issue requests for a download.
#[derive(Debug, Clone)]
pub struct RequestContext {
    url: Url,
    headers: HeaderMap,
    suggested_name: Option<String>,
    proxy: ProxyChoice,
    /// Shared by all workers of the download (`Set-Cookie` updates it).
    cookies: Option<Arc<Mutex<CookieJar>>>,
    cookie_file: Option<PathBuf>,
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
            proxy: ProxyChoice::Default,
            cookies: None,
            cookie_file: None,
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

    /// Sets the proxy for this download (default: the engine's rules).
    pub fn set_proxy(&mut self, proxy: ProxyChoice) {
        self.proxy = proxy;
    }

    /// Builder-style [`RequestContext::set_proxy`].
    #[must_use]
    pub fn with_proxy(mut self, proxy: ProxyChoice) -> Self {
        self.proxy = proxy;
        self
    }

    /// The proxy choice.
    pub fn proxy(&self) -> &ProxyChoice {
        &self.proxy
    }

    /// Uses these cookies (replacing any). They are not saved: a resumed
    /// download needs them again, unless they came from a file.
    #[must_use]
    pub fn with_cookies(mut self, jar: CookieJar) -> Self {
        self.cookies = Some(Arc::new(Mutex::new(jar)));
        self.cookie_file = None;
        self
    }

    /// Loads cookies from a Netscape `cookies.txt` file. The path is saved with
    /// the download and the file is read again when it resumes.
    pub fn with_cookie_file(mut self, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let path = std::path::absolute(&path).unwrap_or(path);
        let jar = CookieJar::load(&path)?;
        self.cookies = Some(Arc::new(Mutex::new(jar)));
        self.cookie_file = Some(path);
        Ok(self)
    }

    /// Adds a cookie to the jar (creating it if needed).
    pub fn add_cookie(&mut self, cookie: Cookie) {
        let jar = self
            .cookies
            .get_or_insert_with(|| Arc::new(Mutex::new(CookieJar::new())));
        jar.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(cookie);
    }

    /// Adds a cookie for exactly this URL's host (path `/`, no subdomains), like
    /// `curl --cookie name=value`. Not saved with the download.
    pub fn cookie(&mut self, name: &str, value: &str) {
        if let Some(host) = self.url.host_str() {
            let host = host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned();
            self.add_cookie(Cookie::for_host(&host, name, value));
        }
    }

    /// The cookie jar, if the download has cookies.
    pub(crate) fn cookies(&self) -> Option<&Mutex<CookieJar>> {
        self.cookies.as_deref()
    }

    /// The `cookies.txt` the cookies were loaded from.
    pub fn cookie_file(&self) -> Option<&Path> {
        self.cookie_file.as_deref()
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

    /// JSON for the database: allowlisted headers except secrets, and the
    /// suggested name.
    pub(crate) fn to_saved(&self) -> String {
        let saved = SavedContext {
            headers: self
                .headers
                .iter()
                .filter(|(n, _)| !SECRET_HEADERS.contains(n))
                .filter_map(|(n, v)| Some((n.as_str().to_owned(), v.to_str().ok()?.to_owned())))
                .collect(),
            suggested_name: self.suggested_name.clone(),
            cookie_file: self.cookie_file.clone(),
            proxy: match &self.proxy {
                ProxyChoice::Default => None,
                ProxyChoice::Direct => Some("direct".into()),
                ProxyChoice::Via(p) => Some(p.without_credentials()),
            },
        };
        serde_json::to_string(&saved).unwrap_or_else(|_| "{}".into())
    }

    /// Rebuilds a context from the database. Unknown or invalid headers are dropped.
    pub(crate) fn from_saved(url: &str, saved: &str) -> Result<Self> {
        let mut ctx = Self::new(url)?;
        let saved: SavedContext = serde_json::from_str(saved).unwrap_or_default();
        for (n, v) in saved.headers {
            if let (Ok(n), Ok(v)) = (HeaderName::try_from(n), HeaderValue::try_from(v)) {
                ctx.header(n, v);
            }
        }
        ctx.suggested_name = saved.suggested_name;
        if let Some(path) = saved.cookie_file {
            // The file may be gone or changed; a download without its cookies
            // still runs (and asks for a new link if the server wants them).
            match ctx.clone().with_cookie_file(&path) {
                Ok(c) => ctx = c,
                Err(e) => tracing::warn!(error = %e, "could not read the saved cookie file"),
            }
        }
        ctx.proxy = match saved.proxy.as_deref() {
            None => ProxyChoice::Default,
            Some("direct") => ProxyChoice::Direct,
            Some(p) => Proxy::parse(p).map_or(ProxyChoice::Default, ProxyChoice::Via),
        };
        Ok(ctx)
    }
}

/// An `Authorization: Basic` value for `user` and `password` (marked sensitive).
pub fn basic_auth(user: &str, password: &str) -> HeaderValue {
    let mut v = HeaderValue::from_str(&format!(
        "Basic {}",
        crate::proxy::base64(format!("{user}:{password}").as_bytes())
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("Basic"));
    v.set_sensitive(true);
    v
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
    fn saved_form_drops_secrets() {
        let ctx = RequestContext::new("https://example.com/a?x=1")
            .unwrap()
            .with_header(header::REFERER, HeaderValue::from_static("https://r/"))
            .with_header(header::AUTHORIZATION, HeaderValue::from_static("Bearer t"))
            .with_suggested_name("a.bin");
        let saved = ctx.to_saved();
        assert!(!saved.contains("Bearer"), "{saved}");
        let back = RequestContext::from_saved(ctx.url().as_str(), &saved).unwrap();
        assert_eq!(back.url(), ctx.url());
        assert_eq!(back.headers().get(header::REFERER).unwrap(), "https://r/");
        assert!(back.headers().get(header::AUTHORIZATION).is_none());
        assert_eq!(back.suggested_name(), Some("a.bin"));
        // Garbage degrades to an empty context rather than failing the resume.
        let bare = RequestContext::from_saved("https://example.com/a", "not json").unwrap();
        assert!(bare.headers().is_empty());
    }

    #[test]
    fn redaction() {
        let u = Url::parse("https://user:pw@example.com/f.iso?sig=secret#frag").unwrap();
        assert_eq!(redact_url(&u), "https://example.com/f.iso?…");
    }
}
