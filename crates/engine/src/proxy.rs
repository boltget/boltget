//! Proxies (`docs/ENGINE.md`, "Proxies"): HTTP proxies (absolute-form requests
//! for `http` targets, `CONNECT` tunnels for `https`), HTTPS proxies (TLS to the
//! proxy), and SOCKS5 with remote DNS and username/password authentication.
//!
//! Proxy credentials are secrets: [`Proxy`]'s `Debug` and
//! [`Proxy::redacted`] never show them, they never appear in errors or logs,
//! and the database keeps only [`Proxy::without_credentials`].

use std::fmt;

use http::HeaderValue;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use url::Url;

use crate::error::{Error, Result};

/// How to talk to a proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    /// Plain HTTP proxy.
    Http,
    /// HTTP proxy reached over TLS.
    Https,
    /// SOCKS5; target names are resolved by the proxy (like `socks5h`).
    Socks5,
}

#[derive(Clone, PartialEq, Eq)]
struct Credentials {
    user: String,
    password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A proxy server, possibly with credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    kind: ProxyKind,
    host: String,
    port: u16,
    auth: Option<Credentials>,
}

impl Proxy {
    /// Parses `http://[user:pass@]host[:port]`, `https://...`, `socks5://...` or
    /// `socks5h://...` (both SOCKS forms resolve names at the proxy). Without a
    /// scheme, `http` is assumed. Default ports follow curl: 1080, or 443 for
    /// `https`. Credentials may be percent-encoded.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let with_scheme = if s.contains("://") {
            s.to_owned()
        } else {
            format!("http://{s}")
        };
        // Never echo the input: it may contain a password.
        let url = Url::parse(&with_scheme).map_err(|e| format!("invalid proxy URL: {e}"))?;
        let kind = match url.scheme() {
            "http" => ProxyKind::Http,
            "https" => ProxyKind::Https,
            "socks5" | "socks5h" => ProxyKind::Socks5,
            other => {
                return Err(format!(
                    "unsupported proxy scheme {other:?} (use http, https or socks5)"
                ));
            }
        };
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or("the proxy URL has no host")?
            .to_owned();
        let port = url.port().unwrap_or(match kind {
            ProxyKind::Https => 443,
            _ => 1080,
        });
        let decode = |v: &str| {
            percent_encoding::percent_decode_str(v)
                .decode_utf8_lossy()
                .into_owned()
        };
        let auth = (!url.username().is_empty()).then(|| Credentials {
            user: decode(url.username()),
            password: decode(url.password().unwrap_or("")),
        });
        Ok(Self {
            kind,
            host,
            port,
            auth,
        })
    }

    /// The protocol.
    pub fn kind(&self) -> ProxyKind {
        self.kind
    }

    /// Whether the proxy URL carried credentials.
    pub fn has_credentials(&self) -> bool {
        self.auth.is_some()
    }

    fn scheme(&self) -> &'static str {
        match self.kind {
            ProxyKind::Http => "http",
            ProxyKind::Https => "https",
            ProxyKind::Socks5 => "socks5",
        }
    }

    /// `scheme://host:port`, without credentials (what the database keeps).
    pub fn without_credentials(&self) -> String {
        format!("{}://{}:{}", self.scheme(), self.host, self.port)
    }

    /// For logs and messages: credentials shown as `***`.
    pub fn redacted(&self) -> String {
        match self.auth {
            Some(_) => format!("{}://***@{}:{}", self.scheme(), self.host, self.port),
            None => self.without_credentials(),
        }
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// `Proxy-Authorization: Basic ...` for HTTP proxies with credentials.
    pub(crate) fn basic_auth(&self) -> Option<HeaderValue> {
        let a = self.auth.as_ref()?;
        let mut v = HeaderValue::from_str(&format!(
            "Basic {}",
            base64(format!("{}:{}", a.user, a.password).as_bytes())
        ))
        .ok()?;
        v.set_sensitive(true);
        Some(v)
    }
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Hosts that bypass the proxy (`NO_PROXY`): `*` for all, exact hosts or IP
/// addresses, and domains (`example.com` and `.example.com` both match the
/// domain and its subdomains). A `:port` suffix on an entry is ignored. CIDR
/// ranges are not supported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoProxy {
    all: bool,
    entries: Vec<String>,
}

impl NoProxy {
    /// Parses a comma- or space-separated list.
    pub fn parse(s: &str) -> Self {
        let mut np = NoProxy::default();
        for raw in s.split([',', ' ']).map(str::trim).filter(|e| !e.is_empty()) {
            if raw == "*" {
                np.all = true;
                continue;
            }
            let lower = raw.trim_start_matches('.').to_ascii_lowercase();
            // Strip a port, but not the colons of a bare IPv6 address.
            let e = if let Some(stripped) = lower.strip_prefix('[') {
                stripped.split(']').next().unwrap_or("").to_owned()
            } else if lower.matches(':').count() == 1
                && let Some((h, _)) = lower.split_once(':')
            {
                h.to_owned()
            } else {
                lower
            };
            if !e.is_empty() {
                np.entries.push(e);
            }
        }
        np
    }

    /// Whether `host` (as in a URL, IPv6 in brackets) bypasses the proxy.
    pub fn matches(&self, host: &str) -> bool {
        if self.all {
            return true;
        }
        let h = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches('.')
            .to_ascii_lowercase();
        self.entries.iter().any(|e| {
            h == *e
                || (h.len() > e.len()
                    && h.ends_with(e.as_str())
                    && h.as_bytes()[h.len() - e.len() - 1] == b'.')
        })
    }
}

/// Which proxy each URL uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyRules {
    /// Proxy for `http` URLs.
    pub http: Option<Proxy>,
    /// Proxy for `https` URLs.
    pub https: Option<Proxy>,
    /// Hosts that go direct.
    pub no_proxy: NoProxy,
}

impl ProxyRules {
    /// No proxy at all.
    pub fn direct() -> Self {
        Self::default()
    }

    /// The same proxy for every URL, with no exceptions.
    pub fn all(proxy: Proxy) -> Self {
        Self {
            http: Some(proxy.clone()),
            https: Some(proxy),
            no_proxy: NoProxy::default(),
        }
    }

    /// Rules from the environment, as curl reads them: `http_proxy` (lowercase
    /// first, then `HTTP_PROXY`), `HTTPS_PROXY`/`https_proxy`, `ALL_PROXY`/
    /// `all_proxy` for both, and `NO_PROXY`/`no_proxy`. Invalid values are
    /// skipped with a warning that does not repeat the value.
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub(crate) fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let first = |names: &[&str]| -> Option<Proxy> {
            names.iter().find_map(|n| {
                let v = get(n).filter(|v| !v.trim().is_empty())?;
                match Proxy::parse(&v) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        tracing::warn!(variable = n, error = %e, "ignoring proxy variable");
                        None
                    }
                }
            })
        };
        let all = first(&["all_proxy", "ALL_PROXY"]);
        Self {
            http: first(&["http_proxy", "HTTP_PROXY"]).or_else(|| all.clone()),
            https: first(&["https_proxy", "HTTPS_PROXY"]).or(all),
            no_proxy: get("no_proxy")
                .or_else(|| get("NO_PROXY"))
                .map(|v| NoProxy::parse(&v))
                .unwrap_or_default(),
        }
    }

    /// The proxy for a request to `https`/`host`, if any.
    pub(crate) fn for_target(&self, https: bool, host: &str) -> Option<&Proxy> {
        let p = if https { &self.https } else { &self.http };
        p.as_ref().filter(|_| !self.no_proxy.matches(host))
    }
}

/// A download's proxy setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyChoice {
    /// Use the engine's rules ([`EngineConfig::proxy`](crate::EngineConfig::proxy)).
    Default,
    /// No proxy for this download.
    Direct,
    /// This proxy for every request of this download.
    Via(Proxy),
}

/// Reads an HTTP response head (up to the empty line) byte by byte, so nothing
/// that follows it is consumed.
async fn read_head<S: AsyncRead + Unpin>(s: &mut S) -> Result<String> {
    let mut head = Vec::with_capacity(256);
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return Err(Error::Protocol(
                "the proxy sent an oversized response".into(),
            ));
        }
        let n = s
            .read(&mut b)
            .await
            .map_err(|e| Error::network("reading the proxy response", e))?;
        if n == 0 {
            return Err(Error::network(
                "reading the proxy response",
                "the proxy closed the connection",
            ));
        }
        head.push(b[0]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// Opens a `CONNECT` tunnel to `host:port` through an HTTP proxy.
pub(crate) async fn http_connect<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    proxy: &Proxy,
    host: &str,
    port: u16,
) -> Result<()> {
    let authority = format!("{host}:{port}");
    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(v) = proxy.basic_auth()
        && let Ok(v) = v.to_str()
    {
        req.push_str("Proxy-Authorization: ");
        req.push_str(v);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())
        .await
        .map_err(|e| Error::network("sending CONNECT to the proxy", e))?;
    let head = read_head(s).await?;
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| Error::Protocol("the proxy sent an invalid CONNECT response".into()))?;
    if (200..300).contains(&status) {
        return Ok(());
    }
    Err(Error::Status {
        status,
        url: format!("CONNECT {authority} via {}", proxy.redacted()),
        retry_after: None,
        previously_worked: false,
    })
}

/// SOCKS5 handshake (RFC 1928, RFC 1929): `CONNECT` to `host:port`, sending the
/// name so the proxy resolves it.
pub(crate) async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    proxy: &Proxy,
    host: &str,
    port: u16,
) -> Result<()> {
    let io = |e: std::io::Error| Error::network("SOCKS5 handshake", e);
    let methods: &[u8] = if proxy.auth.is_some() {
        &[0x00, 0x02]
    } else {
        &[0x00]
    };
    let mut hello = vec![0x05, methods.len() as u8];
    hello.extend_from_slice(methods);
    s.write_all(&hello).await.map_err(io)?;
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply).await.map_err(io)?;
    if reply[0] != 0x05 {
        return Err(Error::Protocol("the proxy does not speak SOCKS5".into()));
    }
    match reply[1] {
        0x00 => {}
        0x02 => {
            let a = proxy.auth.as_ref().ok_or_else(|| {
                Error::ProxyAuth("the SOCKS5 proxy asks for a username and password".into())
            })?;
            let (u, p) = (a.user.as_bytes(), a.password.as_bytes());
            if u.len() > 255 || p.len() > 255 {
                return Err(Error::ProxyAuth(
                    "SOCKS5 usernames and passwords are limited to 255 bytes".into(),
                ));
            }
            let mut msg = vec![0x01, u.len() as u8];
            msg.extend_from_slice(u);
            msg.push(p.len() as u8);
            msg.extend_from_slice(p);
            s.write_all(&msg).await.map_err(io)?;
            let mut st = [0u8; 2];
            s.read_exact(&mut st).await.map_err(io)?;
            if st[1] != 0x00 {
                return Err(Error::ProxyAuth(
                    "the SOCKS5 proxy rejected the username or password".into(),
                ));
            }
        }
        _ => {
            return Err(Error::ProxyAuth(
                "the SOCKS5 proxy accepts none of the offered authentication methods".into(),
            ));
        }
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let mut req = vec![0x05, 0x01, 0x00];
    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            req.push(0x01);
            req.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            req.push(0x04);
            req.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            if bare.len() > 255 {
                return Err(Error::InvalidUrl("host name too long for SOCKS5".into()));
            }
            req.push(0x03);
            req.push(bare.len() as u8);
            req.extend_from_slice(bare.as_bytes());
        }
    }
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.map_err(io)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.map_err(io)?;
    if head[1] != 0x00 {
        let why = match head[1] {
            0x02 => "the connection is not allowed by the proxy's rules",
            0x03 => "network unreachable",
            0x04 => "host unreachable",
            0x05 => "connection refused",
            0x06 => "TTL expired",
            0x07 => "command not supported",
            0x08 => "address type not supported",
            _ => "general failure",
        };
        return Err(Error::network(
            "connecting through the SOCKS5 proxy",
            format!("the proxy could not reach {host}:{port}: {why}"),
        ));
    }
    // Skip the bound address.
    let skip = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n).await.map_err(io)?;
            usize::from(n[0])
        }
        _ => return Err(Error::Protocol("invalid SOCKS5 reply".into())),
    };
    let mut rest = vec![0u8; skip + 2];
    s.read_exact(&mut rest).await.map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn proxy_urls() {
        let p = Proxy::parse("http://user:p%40ss@proxy.example:3128").unwrap();
        assert_eq!(p.kind(), ProxyKind::Http);
        assert_eq!((p.host(), p.port()), ("proxy.example", 3128));
        assert!(p.has_credentials());
        assert_eq!(p.without_credentials(), "http://proxy.example:3128");
        assert_eq!(p.redacted(), "http://***@proxy.example:3128");
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("p@ss") && !dbg.contains("user"), "{dbg}");
        // user:p@ss
        assert_eq!(
            p.basic_auth().unwrap().to_str().unwrap(),
            "Basic dXNlcjpwQHNz"
        );
        assert!(p.basic_auth().unwrap().is_sensitive());
        assert_eq!(Proxy::parse("proxy:8080").unwrap().kind(), ProxyKind::Http);
        assert_eq!(Proxy::parse("socks5h://s").unwrap().port(), 1080);
        assert_eq!(Proxy::parse("https://s").unwrap().port(), 443);
        assert_eq!(
            Proxy::parse("socks5://s").unwrap().kind(),
            ProxyKind::Socks5
        );
        let e = Proxy::parse("ftp://user:secret@x").unwrap_err();
        assert!(!e.contains("secret"), "{e}");
        assert!(Proxy::parse("http://").is_err());
    }

    #[test]
    fn no_proxy_matching() {
        let np = NoProxy::parse("localhost, .internal.example,10.0.0.1:8080 [::1] corp.test");
        assert!(np.matches("localhost"));
        assert!(np.matches("a.internal.example"));
        assert!(np.matches("internal.example"));
        assert!(!np.matches("notinternal.example"));
        assert!(np.matches("10.0.0.1"));
        assert!(np.matches("[::1]"));
        assert!(np.matches("x.corp.test"));
        assert!(!np.matches("example.com"));
        assert!(NoProxy::parse("*").matches("anything"));
        assert!(!NoProxy::parse("").matches("x"));
    }

    #[test]
    fn rules_from_environment() {
        let env: HashMap<&str, &str> = [
            ("http_proxy", "http://lower:1"),
            ("HTTP_PROXY", "http://upper:2"),
            ("ALL_PROXY", "socks5://all:3"),
            ("NO_PROXY", "skip.test"),
        ]
        .into_iter()
        .collect();
        let r = ProxyRules::from_lookup(|k| env.get(k).map(|v| (*v).to_owned()));
        assert_eq!(r.for_target(false, "a.test").unwrap().host(), "lower");
        assert_eq!(r.for_target(true, "a.test").unwrap().host(), "all");
        assert!(r.for_target(true, "skip.test").is_none());
        let none = ProxyRules::from_lookup(|_| None);
        assert!(none.for_target(true, "x").is_none());
        let bad = ProxyRules::from_lookup(|k| (k == "https_proxy").then(|| "gopher://x".into()));
        assert!(bad.https.is_none());
    }

    #[tokio::test]
    async fn connect_tunnel_handshake() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let proxy = Proxy::parse("http://u:pw@proxy:3128").unwrap();
        let srv = tokio::spawn(async move {
            let head = read_head(&mut server).await.unwrap();
            server
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\nTUNNEL")
                .await
                .unwrap();
            head
        });
        http_connect(&mut client, &proxy, "files.example", 443)
            .await
            .unwrap();
        let head = srv.await.unwrap();
        assert!(
            head.starts_with("CONNECT files.example:443 HTTP/1.1\r\n"),
            "{head}"
        );
        assert!(
            head.contains("Proxy-Authorization: Basic dTpwdw==\r\n"),
            "{head}"
        );
        // Bytes after the response head belong to the tunnel.
        let mut rest = [0u8; 6];
        client.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"TUNNEL");

        let (mut client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = read_head(&mut server).await;
            let _ = server
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await;
        });
        let e = http_connect(
            &mut client,
            &Proxy::parse("http://proxy").unwrap(),
            "h",
            443,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Status { status: 407, .. }), "{e}");
    }

    #[tokio::test]
    async fn socks5_handshake_with_password_and_remote_dns() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let srv = tokio::spawn(async move {
            let mut b = [0u8; 4];
            server.read_exact(&mut b).await.unwrap();
            assert_eq!(b, [5, 2, 0, 2]);
            server.write_all(&[5, 2]).await.unwrap();
            let mut auth = [0u8; 1 + 1 + 1 + 1 + 2];
            server.read_exact(&mut auth).await.unwrap();
            assert_eq!(&auth, &[1, 1, b'u', 2, b'p', b'w']);
            server.write_all(&[1, 0]).await.unwrap();
            let mut req = vec![0u8; 5 + 13 + 2];
            server.read_exact(&mut req).await.unwrap();
            assert_eq!(&req[..5], &[5, 1, 0, 3, 13]);
            assert_eq!(&req[5..18], b"files.example");
            assert_eq!(&req[18..], &443u16.to_be_bytes());
            server
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0x1f, 0x90])
                .await
                .unwrap();
        });
        let proxy = Proxy::parse("socks5://u:pw@proxy").unwrap();
        socks5_connect(&mut client, &proxy, "files.example", 443)
            .await
            .unwrap();
        srv.await.unwrap();

        let (mut client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut b = [0u8; 3];
            let _ = server.read_exact(&mut b).await;
            let _ = server.write_all(&[5, 0]).await;
            let mut req = [0u8; 10];
            let _ = server.read_exact(&mut req).await;
            assert_eq!(req[3], 1, "IP literals are sent as addresses");
            let _ = server.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        });
        let e = socks5_connect(
            &mut client,
            &Proxy::parse("socks5://proxy").unwrap(),
            "10.1.2.3",
            80,
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("connection refused"), "{e}");
        assert!(e.is_retryable());
    }
}
