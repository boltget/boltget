//! Explicit per-worker HTTP connections on hyper's low-level client API.
//!
//! A pooled client would multiplex HTTP/2 requests onto one TCP connection and
//! defeat segmentation (`docs/ENGINE.md`, "HTTP/2 considerations"), so every
//! worker owns exactly one [`Conn`]: its own TCP + TLS session, carrying one
//! request at a time.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http::header::{self, HeaderMap, HeaderValue};
use http::{Method, Request, StatusCode, Uri};
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::client::conn::{http1, http2};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::task::{AbortHandle, JoinSet};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use url::Url;

use crate::config::EngineConfig;
use crate::error::{Error, Result};
use crate::headers::range_header;
use crate::proxy::{self, Proxy, ProxyChoice, ProxyKind};
use crate::request::{RequestContext, redact_url};

/// Builds the shared rustls configuration: `ring` provider, OS trust store via
/// `rustls-platform-verifier`, ALPN `h2` + `http/1.1`, session resumption on.
pub(crate) fn tls_config() -> Result<Arc<ClientConfig>> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .with_platform_verifier()
        .map_err(|e| Error::Tls(e.to_string()))?
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    // Resumption (tickets + session ids) is on by default in rustls; it lets the
    // second..n-th worker to the same host skip the full handshake.
    Ok(Arc::new(cfg))
}

/// scheme + host + port; the unit for connection reuse and per-host caps.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Origin {
    pub(crate) https: bool,
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl Origin {
    pub(crate) fn of(url: &Url) -> Result<Self> {
        let host = url
            .host_str()
            .ok_or_else(|| Error::InvalidUrl("missing host".into()))?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| Error::InvalidUrl("missing port".into()))?;
        Ok(Self {
            https: url.scheme() == "https",
            host,
            port,
        })
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.https { "https" } else { "http" };
        write!(f, "{scheme}://{}:{}", self.host, self.port)
    }
}

trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}

enum Sender {
    H1(http1::SendRequest<Empty<Bytes>>),
    H2(http2::SendRequest<Empty<Bytes>>),
}

/// A connection to a forward HTTP proxy.
#[derive(Clone)]
struct Forward {
    /// `Proxy-Authorization`, if the proxy has credentials.
    auth: Option<HeaderValue>,
}

/// One HTTP connection owned by one worker.
pub(crate) struct Conn {
    origin: Origin,
    sender: Sender,
    task: AbortHandle,
    /// Requests go to an HTTP proxy in absolute form (plain `http` targets).
    forward: Option<Forward>,
}

impl fmt::Debug for Conn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conn")
            .field("origin", &self.origin)
            .field("h2", &self.is_h2())
            .finish_non_exhaustive()
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        // Closing the socket is the only way to stop an HTTP/1.1 response early.
        self.task.abort();
    }
}

impl Conn {
    pub(crate) fn is_h2(&self) -> bool {
        matches!(self.sender, Sender::H2(_))
    }

    fn is_usable(&self) -> bool {
        match &self.sender {
            Sender::H1(s) => !s.is_closed(),
            Sender::H2(s) => !s.is_closed(),
        }
    }

    async fn send(&mut self, req: Request<Empty<Bytes>>) -> Result<http::Response<Incoming>> {
        let res = match &mut self.sender {
            Sender::H1(s) => {
                s.ready()
                    .await
                    .map_err(|e| Error::network("connection closed", e))?;
                s.send_request(req).await
            }
            Sender::H2(s) => {
                s.ready()
                    .await
                    .map_err(|e| Error::network("connection closed", e))?;
                s.send_request(req).await
            }
        };
        res.map_err(|e| Error::network("sending request", e))
    }
}

type DnsMap = HashMap<(String, u16), Arc<[SocketAddr]>>;

/// Per-download DNS cache: resolve each host once and share the addresses.
#[derive(Debug, Default, Clone)]
pub(crate) struct Dns {
    cache: Arc<Mutex<DnsMap>>,
}

impl Dns {
    async fn resolve(&self, origin: &Origin) -> Result<Arc<[SocketAddr]>> {
        let key = (origin.host.clone(), origin.port);
        if let Some(addrs) = self.cache.lock().expect("dns lock").get(&key) {
            return Ok(addrs.clone());
        }
        let host = origin.host.trim_start_matches('[').trim_end_matches(']');
        let addrs: Vec<SocketAddr> = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, origin.port)]
        } else {
            tokio::net::lookup_host((host, origin.port))
                .await
                .map_err(|e| Error::network("resolving host", e))?
                .collect()
        };
        if addrs.is_empty() {
            return Err(Error::network(
                "resolving host",
                format!("no addresses for {host}"),
            ));
        }
        let addrs: Arc<[SocketAddr]> = interleave_families(addrs).into();
        self.cache
            .lock()
            .expect("dns lock")
            .insert(key, addrs.clone());
        Ok(addrs)
    }
}

/// Orders addresses IPv6, IPv4, IPv6, ... as RFC 8305 recommends.
fn interleave_families(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let (v6, v4): (Vec<_>, Vec<_>) = addrs.into_iter().partition(SocketAddr::is_ipv6);
    let mut out = Vec::with_capacity(v6.len() + v4.len());
    let (mut a, mut b) = (v6.into_iter(), v4.into_iter());
    loop {
        match (a.next(), b.next()) {
            (None, None) => break,
            (x, y) => out.extend(x.into_iter().chain(y)),
        }
    }
    out
}

/// "Happy Eyeballs"-style connect: start the next address attempt every 250 ms
/// until one succeeds.
async fn connect_tcp(addrs: &[SocketAddr]) -> Result<TcpStream> {
    const ATTEMPT_DELAY: Duration = Duration::from_millis(250);
    let mut set = JoinSet::new();
    let mut pending = addrs.iter().copied();
    let mut last_err: Option<std::io::Error> = None;
    if let Some(a) = pending.next() {
        set.spawn(TcpStream::connect(a));
    }
    loop {
        tokio::select! {
            res = set.join_next(), if !set.is_empty() => match res {
                Some(Ok(Ok(stream))) => {
                    set.abort_all();
                    return Ok(stream);
                }
                Some(Ok(Err(e))) => {
                    last_err = Some(e);
                    if let Some(a) = pending.next() {
                        set.spawn(TcpStream::connect(a));
                    }
                }
                Some(Err(e)) => return Err(Error::Internal(format!("connect task: {e}"))),
                None => {}
            },
            () = tokio::time::sleep(ATTEMPT_DELAY), if pending.len() > 0 => {
                if let Some(a) = pending.next() {
                    set.spawn(TcpStream::connect(a));
                }
            }
        }
        if set.is_empty() && pending.len() == 0 {
            let err = last_err.unwrap_or_else(|| std::io::Error::other("no address"));
            return Err(Error::network("connecting", err));
        }
    }
}

/// Opens connections for one download.
#[derive(Clone)]
pub(crate) struct Connector {
    tls: TlsConnector,
    /// For TLS to an HTTPS proxy, which speaks HTTP/1.1 (`CONNECT`) only.
    tls_proxy: TlsConnector,
    dns: Dns,
    cfg: Arc<EngineConfig>,
    proxy: ProxyChoice,
}

impl fmt::Debug for Connector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connector").finish_non_exhaustive()
    }
}

/// TLS handshake to `host` over `io`. Returns the stream and whether ALPN chose `h2`.
async fn tls_handshake(
    connector: &TlsConnector,
    host: &str,
    io: Box<dyn Io>,
    limit: Duration,
) -> Result<(Box<dyn Io>, bool)> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let name = ServerName::try_from(host.to_owned())
        .map_err(|e| Error::InvalidUrl(format!("bad TLS server name: {e}")))?;
    let tls = timeout(limit, connector.connect(name, io))
        .await
        .map_err(|_| Error::Timeout("TLS handshake"))?
        .map_err(|e| {
            // Certificate problems surface as InvalidData; they are permanent.
            if e.kind() == std::io::ErrorKind::InvalidData {
                Error::Tls(e.to_string())
            } else {
                Error::network("TLS handshake", e)
            }
        })?;
    let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
    Ok((Box::new(tls), h2))
}

impl Connector {
    /// A connector for one download; `proxy` is the download's proxy setting.
    pub(crate) fn new(tls: Arc<ClientConfig>, cfg: Arc<EngineConfig>, proxy: ProxyChoice) -> Self {
        let mut proxy_tls = (*tls).clone();
        proxy_tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        Self {
            tls: TlsConnector::from(tls),
            tls_proxy: TlsConnector::from(Arc::new(proxy_tls)),
            dns: Dns::default(),
            cfg,
            proxy,
        }
    }

    pub(crate) fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    /// The proxy for requests to `origin`, if any.
    fn proxy_for(&self, origin: &Origin) -> Option<&Proxy> {
        match &self.proxy {
            ProxyChoice::Direct => None,
            ProxyChoice::Via(p) => Some(p),
            ProxyChoice::Default => self.cfg.proxy.for_target(origin.https, &origin.host),
        }
    }

    async fn tcp(&self, origin: &Origin) -> Result<TcpStream> {
        let addrs = self.dns.resolve(origin).await?;
        let tcp = timeout(self.cfg.connect_timeout, connect_tcp(&addrs))
            .await
            .map_err(|_| Error::Timeout("connect"))??;
        let _ = tcp.set_nodelay(true);
        Ok(tcp)
    }

    /// Connects to `origin`, through the proxy if there is one. Returns the
    /// stream, whether it speaks HTTP/2, and the forward-proxy state.
    async fn open_stream(&self, origin: &Origin) -> Result<(Box<dyn Io>, bool, Option<Forward>)> {
        let Some(p) = self.proxy_for(origin).cloned() else {
            let tcp: Box<dyn Io> = Box::new(self.tcp(origin).await?);
            if origin.https {
                let (io, h2) =
                    tls_handshake(&self.tls, &origin.host, tcp, self.cfg.tls_timeout).await?;
                return Ok((io, h2, None));
            }
            return Ok((tcp, false, None));
        };
        let at = Origin {
            https: false,
            host: p.host().to_owned(),
            port: p.port(),
        };
        let mut io: Box<dyn Io> = Box::new(self.tcp(&at).await?);
        if p.kind() == ProxyKind::Https {
            io = tls_handshake(&self.tls_proxy, p.host(), io, self.cfg.tls_timeout)
                .await?
                .0;
        }
        let limit = self.cfg.connect_timeout;
        match p.kind() {
            ProxyKind::Socks5 => {
                timeout(
                    limit,
                    proxy::socks5_connect(&mut io, &p, &origin.host, origin.port),
                )
                .await
                .map_err(|_| Error::Timeout("SOCKS5 handshake"))??;
            }
            ProxyKind::Http | ProxyKind::Https if origin.https => {
                timeout(
                    limit,
                    proxy::http_connect(&mut io, &p, &origin.host, origin.port),
                )
                .await
                .map_err(|_| Error::Timeout("proxy CONNECT"))??;
            }
            // Plain `http` through an HTTP proxy: absolute-form requests.
            ProxyKind::Http | ProxyKind::Https => {
                return Ok((
                    io,
                    false,
                    Some(Forward {
                        auth: p.basic_auth(),
                    }),
                ));
            }
        }
        if origin.https {
            let (io, h2) = tls_handshake(&self.tls, &origin.host, io, self.cfg.tls_timeout).await?;
            Ok((io, h2, None))
        } else {
            Ok((io, false, None))
        }
    }

    pub(crate) async fn connect(&self, origin: &Origin) -> Result<Conn> {
        let (io, h2, forward) = self.open_stream(origin).await?;
        let io = TokioIo::new(io);
        let (sender, task) = if h2 {
            let (sender, conn) = http2::Builder::new(TokioExecutor::new())
                .initial_stream_window_size(8 * 1024 * 1024)
                .initial_connection_window_size(8 * 1024 * 1024)
                .handshake(io)
                .await
                .map_err(|e| Error::network("HTTP/2 handshake", e))?;
            let task = tokio::spawn(async move {
                let _ = conn.await;
            });
            (Sender::H2(sender), task.abort_handle())
        } else {
            let (sender, conn) = http1::Builder::new()
                .handshake(io)
                .await
                .map_err(|e| Error::network("HTTP/1.1 handshake", e))?;
            let task = tokio::spawn(async move {
                let _ = conn.await;
            });
            (Sender::H1(sender), task.abort_handle())
        };
        Ok(Conn {
            origin: origin.clone(),
            sender,
            task,
            forward,
        })
    }

    /// Sends `GET url` with the given range and validator, following redirects.
    ///
    /// `conn` is reused if it is connected to the right origin. `original` is the
    /// origin the context headers were meant for: `Authorization` is dropped on
    /// any request to another origin.
    pub(crate) async fn get(
        &self,
        conn: Option<Conn>,
        url: &Url,
        original: &Origin,
        ctx: &RequestContext,
        range: Option<(u64, Option<u64>)>,
        if_range: Option<&str>,
    ) -> Result<Response> {
        let mut conn = conn;
        let mut url = url.clone();
        let mut redirects = Vec::new();
        loop {
            let origin = Origin::of(&url)?;
            let mut headers = self.request_headers(ctx.headers(), &origin, original);
            // Only the cookies that match this hop's URL (RFC 6265).
            if let Some(jar) = ctx.cookies()
                && let Some(v) = jar
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .header_for(&url, std::time::SystemTime::now())
            {
                headers.insert(header::COOKIE, v);
            }
            if let Some((start, end)) = range {
                headers.insert(header::RANGE, range_header(start, end));
            }
            if let Some(v) = if_range
                && let Ok(v) = HeaderValue::from_str(v)
            {
                headers.insert(header::IF_RANGE, v);
            }
            let reuse = conn.take().filter(|c| c.origin == origin && c.is_usable());
            let reused = reuse.is_some();
            let mut c = match reuse {
                Some(c) => c,
                None => self.connect(&origin).await?,
            };
            let req = build_request(&url, &origin, headers.clone(), &c)?;
            let sent = timeout(self.cfg.first_byte_timeout, c.send(req)).await;
            let resp = match sent {
                Err(_) => return Err(Error::Timeout("waiting for response headers")),
                Ok(Ok(r)) => r,
                Ok(Err(e)) if reused => {
                    // A kept-alive connection the server already closed: retry once
                    // on a fresh connection without counting it as a failure.
                    tracing::debug!(error = %e, "reused connection failed; reconnecting");
                    let mut fresh = self.connect(&origin).await?;
                    let req = build_request(&url, &origin, headers, &fresh)?;
                    let r = timeout(self.cfg.first_byte_timeout, fresh.send(req))
                        .await
                        .map_err(|_| Error::Timeout("waiting for response headers"))??;
                    c = fresh;
                    r
                }
                Ok(Err(e)) => return Err(e),
            };
            let status = resp.status();
            if let Some(jar) = ctx.cookies() {
                let mut jar = jar
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for v in resp.headers().get_all(header::SET_COOKIE) {
                    if let Ok(v) = v.to_str() {
                        jar.set_cookie(&url, v, std::time::SystemTime::now());
                    }
                }
            }
            if is_redirect(status) {
                let Some(location) = resp
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                else {
                    return Err(Error::Protocol(format!("{status} without Location")));
                };
                let next = url
                    .join(location)
                    .map_err(|e| Error::Protocol(format!("bad Location: {e}")))?;
                if redirects.len() >= self.cfg.max_redirects {
                    return Err(Error::TooManyRedirects(self.cfg.max_redirects));
                }
                if url.scheme() == "https"
                    && next.scheme() == "http"
                    && !self.cfg.allow_insecure_redirects
                {
                    return Err(Error::InsecureRedirect(redact_url(&next)));
                }
                if !matches!(next.scheme(), "http" | "https") {
                    return Err(Error::InvalidUrl(format!(
                        "redirect to unsupported scheme {:?}",
                        next.scheme()
                    )));
                }
                tracing::debug!(%status, to = %redact_url(&next), "following redirect");
                // Drain a small redirect body so the connection can be reused.
                let mut body = resp.into_body();
                let drained = timeout(Duration::from_secs(2), async {
                    let mut n = 0usize;
                    while let Some(f) = body.frame().await {
                        let Ok(f) = f else { return false };
                        n += f.data_ref().map_or(0, Bytes::len);
                        if n > 64 * 1024 {
                            return false;
                        }
                    }
                    true
                })
                .await
                .unwrap_or(false);
                if drained {
                    conn = Some(c);
                }
                redirects.push(url);
                url = next;
                continue;
            }
            let (parts, body) = resp.into_parts();
            return Ok(Response {
                status,
                headers: parts.headers,
                body,
                conn: Some(c),
                url,
                redirects,
                eof: false,
            });
        }
    }

    fn request_headers(&self, ctx: &HeaderMap, target: &Origin, original: &Origin) -> HeaderMap {
        let mut h = ctx.clone();
        if target != original {
            h.remove(header::AUTHORIZATION);
        }
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        if !h.contains_key(header::USER_AGENT)
            && let Ok(v) = HeaderValue::from_str(&self.cfg.user_agent)
        {
            h.insert(header::USER_AGENT, v);
        }
        if !h.contains_key(header::ACCEPT) {
            h.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        }
        h
    }
}

fn is_redirect(s: StatusCode) -> bool {
    matches!(s.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn build_request(
    url: &Url,
    origin: &Origin,
    headers: HeaderMap,
    conn: &Conn,
) -> Result<Request<Empty<Bytes>>> {
    build_request_with(url, origin, headers, conn.is_h2(), conn.forward.as_ref())
}

fn build_request_with(
    url: &Url,
    origin: &Origin,
    mut headers: HeaderMap,
    h2: bool,
    forward: Option<&Forward>,
) -> Result<Request<Empty<Bytes>>> {
    if let Some(auth) = forward.and_then(|f| f.auth.as_ref()) {
        headers.insert(header::PROXY_AUTHORIZATION, auth.clone());
    }
    // HTTP/2 and requests to a forward proxy use the absolute form.
    let uri: Uri = if h2 || forward.is_some() {
        url.as_str().parse()
    } else {
        let mut pq = url.path().to_owned();
        if let Some(q) = url.query() {
            pq.push('?');
            pq.push_str(q);
        }
        if pq.is_empty() {
            pq.push('/');
        }
        pq.parse()
    }
    .map_err(|e| Error::InvalidUrl(format!("{e}")))?;
    if !h2 {
        let default_port = if origin.https { 443 } else { 80 };
        let host = if origin.port == default_port {
            origin.host.clone()
        } else {
            format!("{}:{}", origin.host, origin.port)
        };
        let v = HeaderValue::from_str(&host).map_err(|e| Error::InvalidUrl(format!("{e}")))?;
        headers.insert(header::HOST, v);
    }
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Empty::new())
        .map_err(|e| Error::Internal(format!("building request: {e}")))?;
    *req.headers_mut() = headers;
    Ok(req)
}

/// A response whose body is still on the wire, together with its connection.
pub(crate) struct Response {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    body: Incoming,
    conn: Option<Conn>,
    /// The URL that produced this response (after redirects).
    pub(crate) url: Url,
    /// URLs that redirected, in order.
    pub(crate) redirects: Vec<Url>,
    eof: bool,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("url", &redact_url(&self.url))
            .finish_non_exhaustive()
    }
}

impl Response {
    /// Reads the next body chunk. `Ok(None)` is a clean end of body; an
    /// incomplete body (connection closed before `Content-Length`) is an error.
    pub(crate) async fn chunk(&mut self, stall: Duration) -> Result<Option<Bytes>> {
        loop {
            if self.eof {
                return Ok(None);
            }
            let frame = timeout(stall, self.body.frame())
                .await
                .map_err(|_| Error::Timeout("stalled connection"))?;
            match frame {
                None => {
                    self.eof = true;
                    return Ok(None);
                }
                Some(Err(e)) => return Err(Error::network("reading body", e)),
                Some(Ok(f)) => {
                    if let Ok(data) = f.into_data()
                        && !data.is_empty()
                    {
                        return Ok(Some(data));
                    }
                }
            }
        }
    }

    /// Gives the connection back for reuse if the response was fully consumed
    /// (HTTP/1.1) or the protocol allows abandoning the stream (HTTP/2).
    pub(crate) fn into_conn(mut self) -> Option<Conn> {
        let conn = self.conn.take()?;
        (self.eof || conn.is_h2()).then_some(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_interleaving() {
        let v4: SocketAddr = "1.1.1.1:80".parse().unwrap();
        let v4b: SocketAddr = "1.1.1.2:80".parse().unwrap();
        let v6: SocketAddr = "[::1]:80".parse().unwrap();
        assert_eq!(interleave_families(vec![v4, v4b, v6]), vec![v6, v4, v4b]);
    }

    #[test]
    fn origin_form_and_host() {
        let url = Url::parse("http://example.com:8080/a/b?x=1").unwrap();
        let o = Origin::of(&url).unwrap();
        let req = build_request_with(&url, &o, HeaderMap::new(), false, None).unwrap();
        assert_eq!(req.uri(), "/a/b?x=1");
        assert_eq!(req.headers()[header::HOST], "example.com:8080");
        let fwd = Forward {
            auth: Some(HeaderValue::from_static("Basic eA==")),
        };
        let req = build_request_with(&url, &o, HeaderMap::new(), false, Some(&fwd)).unwrap();
        assert_eq!(req.uri().to_string(), "http://example.com:8080/a/b?x=1");
        assert_eq!(req.headers()[header::HOST], "example.com:8080");
        assert_eq!(req.headers()[header::PROXY_AUTHORIZATION], "Basic eA==");
        let req = build_request_with(&url, &o, HeaderMap::new(), true, None).unwrap();
        assert_eq!(req.uri().to_string(), "http://example.com:8080/a/b?x=1");
        assert!(req.headers().get(header::HOST).is_none());
    }

    #[test]
    fn authorization_dropped_cross_origin() {
        let cfg = Arc::new(EngineConfig::default());
        let c = Connector::new(tls_config().unwrap(), cfg, ProxyChoice::Default);
        let mut ctx = HeaderMap::new();
        ctx.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer x"));
        let a = Origin::of(&Url::parse("https://a.example/").unwrap()).unwrap();
        let b = Origin::of(&Url::parse("https://b.example/").unwrap()).unwrap();
        assert!(
            c.request_headers(&ctx, &a, &a)
                .contains_key(header::AUTHORIZATION)
        );
        assert!(
            !c.request_headers(&ctx, &b, &a)
                .contains_key(header::AUTHORIZATION)
        );
        assert_eq!(
            c.request_headers(&ctx, &b, &a)[header::ACCEPT_ENCODING],
            "identity"
        );
    }
}
