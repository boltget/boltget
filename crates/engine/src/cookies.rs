//! A per-download cookie jar (`docs/ENGINE.md`, "Authentication and cookies").
//!
//! Cookies are records with RFC 6265 domain, path, `Secure` and expiry rules,
//! not a header string: each request gets only the cookies that match its URL,
//! so a redirect to another site never receives them. `Set-Cookie` from any
//! response (including redirect hops) updates the jar. Cookies load from
//! Netscape `cookies.txt` files, the format browser export extensions, curl and
//! wget write.
//!
//! Cookies are secrets: they are never logged, and the database never holds
//! them (a download remembers only the path of its cookie file).

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::HeaderValue;
use url::Url;

use crate::error::{Error, Result};

/// One cookie.
#[derive(Clone, PartialEq, Eq)]
pub struct Cookie {
    name: String,
    value: String,
    /// Lowercase, without a leading dot.
    domain: String,
    /// Sent only to `domain` itself, not to its subdomains.
    host_only: bool,
    path: String,
    secure: bool,
    /// `None`: a session cookie.
    expires: Option<SystemTime>,
}

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The value is a secret.
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Cookie {
    /// A cookie for exactly `host` (no subdomains), path `/`, no expiry: what
    /// `boltget get --cookie name=value` creates.
    pub fn for_host(host: &str, name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: host.trim_end_matches('.').to_ascii_lowercase(),
            host_only: true,
            path: "/".into(),
            secure: false,
            expires: None,
        }
    }

    /// The cookie's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn expired(&self, now: SystemTime) -> bool {
        self.expires.is_some_and(|t| t <= now)
    }

    fn same_slot(&self, other: &Cookie) -> bool {
        self.name == other.name && self.domain == other.domain && self.path == other.path
    }
}

/// RFC 6265 5.1.3: does `host` domain-match `domain`?
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
            && host.parse::<std::net::IpAddr>().is_err())
}

/// RFC 6265 5.1.4: does `request_path` path-match `cookie_path`?
fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

/// RFC 6265 5.1.4: the default path of a cookie set by `url`.
fn default_path(url: &Url) -> String {
    let p = url.path();
    match p.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => p[..i].to_owned(),
    }
}

fn host_of(url: &Url) -> Option<String> {
    Some(
        url.host_str()?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches('.')
            .to_ascii_lowercase(),
    )
}

/// The cookies of one download.
#[derive(Debug, Clone, Default)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    /// An empty jar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cookies (including expired ones not yet dropped).
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// Whether the jar is empty.
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// Adds a cookie, replacing one with the same name, domain and path.
    pub fn insert(&mut self, cookie: Cookie) {
        self.cookies.retain(|c| !c.same_slot(&cookie));
        self.cookies.push(cookie);
    }

    /// Parses a Netscape `cookies.txt`: one cookie per line, seven tab-separated
    /// fields (domain, include subdomains, path, secure, expiry in Unix seconds
    /// with 0 for a session cookie, name, value). `#` lines are comments, except
    /// the `#HttpOnly_` prefix curl writes. Expired cookies are dropped.
    pub fn from_netscape(text: &str) -> Result<Self, String> {
        let now = SystemTime::now();
        let mut jar = Self::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches(['\r', '\n']);
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let mut f: Vec<&str> = line.split('\t').collect();
            if f.len() < 7 {
                // Some tools write spaces; the value is the rest of the line.
                f = line
                    .splitn(7, char::is_whitespace)
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            if f.len() < 6 {
                return Err(format!(
                    "line {} is not a cookies.txt line (7 tab-separated fields expected)",
                    i + 1
                ));
            }
            let value = f.get(6).copied().unwrap_or("");
            let flag = |s: &str| s.eq_ignore_ascii_case("true");
            let expiry: i64 = f[4]
                .trim()
                .parse()
                .map_err(|_| format!("line {}: the expiry is not a number", i + 1))?;
            // An expiry beyond what `SystemTime` can hold means "never" within
            // this download, like a session cookie.
            let expires = if expiry > 0 {
                UNIX_EPOCH.checked_add(Duration::from_secs(expiry as u64))
            } else {
                None
            };
            let domain = f[0].trim();
            let cookie = Cookie {
                name: f[5].to_owned(),
                value: value.to_owned(),
                domain: domain.trim_start_matches('.').to_ascii_lowercase(),
                // curl and browsers mark subdomain cookies both with TRUE and
                // with a leading dot.
                host_only: !flag(f[1]) && !domain.starts_with('.'),
                path: if f[2].starts_with('/') {
                    f[2].to_owned()
                } else {
                    "/".into()
                },
                secure: flag(f[3]),
                expires,
            };
            if !cookie.expired(now) && !cookie.domain.is_empty() {
                jar.insert(cookie);
            }
        }
        Ok(jar)
    }

    /// Loads a `cookies.txt` file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::disk(path, e))?;
        Self::from_netscape(&text).map_err(|why| Error::Disk {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, why),
        })
    }

    /// The `Cookie` header for a request to `url` (RFC 6265 5.4): matching,
    /// unexpired cookies, longer paths first. Marked sensitive.
    pub(crate) fn header_for(&self, url: &Url, now: SystemTime) -> Option<HeaderValue> {
        let host = host_of(url)?;
        let https = url.scheme() == "https";
        let mut matching: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| !c.expired(now))
            .filter(|c| {
                if c.host_only {
                    host == c.domain
                } else {
                    domain_matches(&host, &c.domain)
                }
            })
            .filter(|c| path_matches(url.path(), &c.path))
            .filter(|c| https || !c.secure)
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        let s = matching
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");
        let mut v = HeaderValue::from_str(&s).ok()?;
        v.set_sensitive(true);
        Some(v)
    }

    /// Applies one `Set-Cookie` header received from `url` (RFC 6265 5.2, 5.3).
    /// Invalid cookies, and cookies for a domain the response's host does not
    /// belong to, are ignored.
    pub(crate) fn set_cookie(&mut self, url: &Url, header: &str, now: SystemTime) {
        let Some(host) = host_of(url) else { return };
        let mut parts = header.split(';');
        let Some((name, value)) = parts.next().and_then(|nv| nv.split_once('=')) else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut c = Cookie {
            name: name.to_owned(),
            value: value.trim().trim_matches('"').to_owned(),
            domain: host.clone(),
            host_only: true,
            path: default_path(url),
            secure: false,
            expires: None,
        };
        let mut max_age: Option<i64> = None;
        for attr in parts {
            let (k, v) = attr.split_once('=').unwrap_or((attr, ""));
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "expires" => {
                    if let Ok(t) = httpdate::parse_http_date(v) {
                        c.expires = Some(t);
                    }
                }
                "max-age" => max_age = v.parse().ok(),
                "domain" if !v.is_empty() => {
                    let d = v
                        .trim_start_matches('.')
                        .trim_end_matches('.')
                        .to_ascii_lowercase();
                    // A host may set cookies for itself and its parent domains,
                    // but not for a bare top-level label.
                    if !domain_matches(&host, &d) || (!d.contains('.') && d != host) {
                        return;
                    }
                    c.domain = d;
                    c.host_only = false;
                }
                "path" if v.starts_with('/') => v.clone_into(&mut c.path),
                "secure" => c.secure = true,
                _ => {}
            }
        }
        if let Some(secs) = max_age {
            // Max-Age wins over Expires; zero or less deletes the cookie. One
            // too large for `SystemTime` never expires (RFC 6265 5.2.2: the
            // latest representable time).
            c.expires = if secs <= 0 {
                Some(UNIX_EPOCH)
            } else {
                now.checked_add(Duration::from_secs(secs as u64))
            };
        }
        if c.secure && url.scheme() != "https" {
            return;
        }
        if c.expired(now) {
            self.cookies.retain(|o| !o.same_slot(&c));
        } else {
            self.insert(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn header(jar: &CookieJar, url: &str) -> Option<String> {
        jar.header_for(&u(url), SystemTime::now())
            .map(|v| v.to_str().unwrap().to_owned())
    }

    #[test]
    fn netscape_files() {
        let text = "# Netscape HTTP Cookie File\n\
            .example.com\tTRUE\t/\tFALSE\t0\tsession\tabc\n\
            files.example.com\tFALSE\t/dl\tTRUE\t4102444800\ttoken\tx=y\n\
            #HttpOnly_example.org\tFALSE\t/\tFALSE\t0\thttponly\t1\n\
            old.example.com\tFALSE\t/\tFALSE\t1\texpired\tgone\n\
            \n";
        let jar = CookieJar::from_netscape(text).unwrap();
        assert_eq!(jar.len(), 3);
        assert_eq!(
            header(&jar, "http://example.com/a").as_deref(),
            Some("session=abc")
        );
        assert_eq!(
            header(&jar, "http://cdn.example.com/a").as_deref(),
            Some("session=abc")
        );
        // Secure cookie only over https, and only under its path.
        assert_eq!(
            header(&jar, "http://files.example.com/dl/f").as_deref(),
            Some("session=abc")
        );
        assert_eq!(
            header(&jar, "https://files.example.com/dl/f").as_deref(),
            Some("token=x=y; session=abc")
        );
        assert_eq!(
            header(&jar, "https://example.org/").as_deref(),
            Some("httponly=1")
        );
        assert_eq!(header(&jar, "https://sub.example.org/"), None);
        assert_eq!(header(&jar, "https://example.net/"), None);
        assert!(CookieJar::from_netscape("not a cookie line").is_err());
        let spaced = CookieJar::from_netscape("example.com FALSE / FALSE 0 a b c").unwrap();
        assert_eq!(
            header(&spaced, "http://example.com/").as_deref(),
            Some("a=b c")
        );
    }

    #[test]
    fn expiry_beyond_system_time_never_expires() {
        // Found by the `cookies` fuzz target: both used to panic on overflow.
        let jar = CookieJar::from_netscape(
            ".example.com	TRUE	/	FALSE	9223372036854775807	forever	1
",
        )
        .unwrap();
        assert_eq!(
            header(&jar, "https://example.com/").as_deref(),
            Some("forever=1")
        );
        let mut jar = CookieJar::new();
        let url = u("https://example.com/");
        jar.set_cookie(&url, "a=b; Max-Age=9223372036854775807", SystemTime::now());
        assert_eq!(header(&jar, "https://example.com/").as_deref(), Some("a=b"));
    }

    #[test]
    fn matching_rules() {
        assert!(domain_matches("a.example.com", "example.com"));
        assert!(!domain_matches("badexample.com", "example.com"));
        assert!(!domain_matches("1.2.3.4", "2.3.4"));
        assert!(path_matches("/dl/file", "/dl"));
        assert!(path_matches("/dl/file", "/dl/"));
        assert!(!path_matches("/dlx", "/dl"));
        assert_eq!(default_path(&u("http://h/a/b/c")), "/a/b");
        assert_eq!(default_path(&u("http://h/a")), "/");
    }

    #[test]
    fn set_cookie_updates_the_jar() {
        let now = SystemTime::now();
        let mut jar = CookieJar::new();
        let from = u("https://login.example.com/auth/start");
        jar.set_cookie(
            &from,
            "sid=1; Path=/; Domain=example.com; Secure; HttpOnly",
            now,
        );
        jar.set_cookie(&from, "local=2", now);
        jar.set_cookie(&from, "evil=1; Domain=other.com", now);
        jar.set_cookie(&from, "tld=1; Domain=com", now);
        jar.set_cookie(&u("http://example.com/"), "insecure=1; Secure", now);
        assert_eq!(jar.len(), 2);
        assert_eq!(
            header(&jar, "https://files.example.com/x").as_deref(),
            Some("sid=1")
        );
        assert_eq!(
            header(&jar, "https://login.example.com/auth/next").as_deref(),
            Some("local=2; sid=1")
        );
        // Replace, then delete with Max-Age=0.
        jar.set_cookie(&from, "sid=3; Path=/; Domain=example.com; Secure", now);
        assert_eq!(
            header(&jar, "https://example.com/").as_deref(),
            Some("sid=3")
        );
        jar.set_cookie(&from, "sid=; Path=/; Domain=example.com; Max-Age=0", now);
        assert_eq!(header(&jar, "https://example.com/"), None);
        // Expires in the past is ignored; in the future is kept.
        jar.set_cookie(
            &from,
            "e=1; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
            now,
        );
        assert_eq!(header(&jar, "https://login.example.com/"), None);
        assert!(header(&jar, "https://login.example.com/auth/x").is_some());
        let v = jar
            .header_for(&u("https://login.example.com/auth/x"), now)
            .unwrap();
        assert!(v.is_sensitive());
        assert!(!format!("{jar:?}").contains("local=2"));
    }
}
