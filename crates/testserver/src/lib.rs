//! A deliberately misbehaving local HTTP server for Boltget's tests.
//!
//! It speaks plain HTTP/1.1 ([`TestServer::start`]) or HTTPS with HTTP/2 and
//! HTTP/1.1 chosen through ALPN ([`TestServer::start_tls`]). The TLS server
//! uses a certificate authority generated at start-up; clients trust it through
//! [`TestServer::ca_cert_der`] / [`TestServer::ca_cert_pem`]. Every behaviour
//! below works over both protocols. Over HTTP/2 each request is a stream, so
//! `reset_after` and `short_count` reset that stream (`RST_STREAM`) while
//! `drop_after` and `goaway_after` take down the whole connection.
//!
//! Content is generated deterministically from a seed, so tests can compute the
//! expected bytes with [`content`] without storing files. Behaviour is chosen per
//! request with query parameters on `/data`:
//!
//! | Parameter | Effect |
//! |---|---|
//! | `size=N` | Resource size in bytes (default 1 MiB) |
//! | `seed=S` | Content seed (default 1) |
//! | `ranges=none` | Ignore `Range`; always `200` without `Accept-Ranges` |
//! | `ranges=ignore-zero` | `200` (with `Accept-Ranges: bytes`) for ranges starting at 0, `206` otherwise |
//! | `accept_ranges=0` | Omit `Accept-Ranges` on `200` responses |
//! | `etag=strong\|weak\|none` | `ETag` style (default strong) |
//! | `lastmod=1` | Send `Last-Modified` one day before `Date` |
//! | `change_after=K` | From request K on (0-based), serve different content and `ETag` |
//! | `ifrange=ignore` | Do not evaluate `If-Range` (serve 206 even if it mismatches) |
//! | `throttle=BPS` | Limit each response to BPS bytes per second |
//! | `line_bps=BPS` | All responses with the same `key` share one line of BPS bytes per second (a saturated last mile) |
//! | `crawl_after=B&crawl_bps=BPS&crawl_count=N` | The first N requests (all if absent) slow down to BPS after B body bytes |
//! | `slow_index=K&slow_bps=BPS` | Request K (0-based) is limited to BPS instead (one bad connection) |
//! | `stall_after=B&stall_count=N` | The first N requests stop sending after B body bytes |
//! | `reset_after=B&reset_count=N` | The first N requests abort the connection after B body bytes |
//! | `short_count=N` | The first N requests declare the full `Content-Length` but send half and close |
//! | `fail_status=C&fail_count=N&retry_after=S` | N requests (all if absent) get status C (optional `Retry-After`) |
//! | `fail_from=K` | Start failing at request K (default 0) |
//! | `cd=V` | `Content-Disposition` value (percent-decoded to raw bytes) |
//! | `ctype=V` | `Content-Type` value |
//! | `encoding=gzip` | Send `Content-Encoding: gzip` (body is not really compressed) |
//! | `chunked=1` | No `Content-Length` on `200` (chunked) |
//! | `star_total=1` | `Content-Range: bytes a-b/*` |
//! | `key=K` | Request counter key (defaults to the whole query string) |
//! | `digest=repr|content|legacy|md5|goog|amz` | Advertise the content's digest: `Repr-Digest` / `Content-Digest` (SHA-256), `Digest` (SHA-512), `Content-MD5`, `x-goog-hash` (MD5), `x-amz-checksum-sha256`. Body digests (`content`, `md5`) describe the bytes of this response |
//! | `digest_bad=1` | ... but advertise a wrong digest |
//! | `etag_salt=X` | Same content, different `ETag` (another storage backend) |
//! | `html_from=K` | From request K on, answer `200` with an HTML page (a login page) |
//! | `drop_after=B&drop_count=N` | The first N requests close their whole connection after B body bytes (over HTTP/2: every stream on it) |
//! | `goaway_after=B&goaway_count=N` | ... the same, but HTTP/2 first sends `GOAWAY` (a server shutting down) |
//!
//! Lab shaping (any route that serves data; see `docs/ENGINE.md`, "Benchmarks plan"):
//!
//! | Parameter | Effect |
//! |---|---|
//! | `latency_ms=D` | Wait D ms before each response head (one round trip), 2 x D for the first request on a connection (connection set-up + request). Combine with `throttle` for a window-limited long path |
//! | `conn_limit=N` | At most N connections per `key` may fetch at a time; a request on any other connection gets `503` (with `Retry-After` if `retry_after` is set, `Connection: close` over HTTP/1.1) until one of the N closes |
//!
//! Packet loss is not modelled: on loopback it cannot be done faithfully at
//! the application layer (TCP hides it), so it is left to a second-machine
//! `tc netem` lab.
//!
//! Other routes:
//!
//! - `/tls/<path>?<query>`: on a plain server with an extra TLS listener
//!   ([`TestServer::listen_tls`], `boltget-testserver --tls-port`), a `307` to the
//!   same path and query on the TLS listener, so plain `{lab}` benchmark URLs
//!   can reach HTTP/2. `404` without one.
//! - `/redirect?hops=N&code=C&to=<pct-encoded URL>`: a chain of N redirects ending at `to`.
//! - `/switch?after=K&to=<url>&then=<url>`: redirects to `to` for the first K requests,
//!   then to `then` (a redirect target that moved).
//! - `/signed?valid=M&...`: redirects to a fresh `/s/<token>?...` URL that serves
//!   `/data` for M requests, then answers 403 (an expiring signed URL).
//! - `/auth?...`: like `/data`, but 401 unless `Authorization` is present.
//! - `/cookie?need=<name=value>&...`: like `/data`, but 403 unless that cookie is sent.
//! - `/setcookie?set=<Set-Cookie value>&to=<url>`: a redirect that sets a cookie.
//! - `/_stats`: JSON counters (`connections`, `max_concurrent`, `requests`,
//!   `body_bytes`, `max_streams`: the most requests in flight on one
//!   connection, `refused`: requests turned away by `conn_limit`) since start
//!   or the last `/_stats?reset=1`, which reads and then resets them. A
//!   connection counts from its first other request, so asking does not change
//!   them. The benchmark harness uses it to see how many connections a client
//!   really opened.
//!
//! The `boltget-testserver` binary serves the same routes on a fixed port, for
//! benchmarks: `boltget-testserver --port 8090 [--tls-port 8443 --ca-out lab-ca.pem]`.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http::{HeaderValue, Request, Response, StatusCode, Version, header};
use http_body::{Body, Frame, SizeHint};
use hyper::body::Incoming;
use hyper::server::conn::{http1, http2};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::graceful::GracefulConnection;
use percent_encoding::percent_decode_str;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

mod proxy;
mod tls;
pub use proxy::{ProxyMode, ProxyRecord, TestProxy};
pub use tls::{Alpn, TestCa};

/// The byte at `offset` of the resource with `seed`.
pub fn byte_at(seed: u64, offset: u64) -> u8 {
    let word = splitmix(seed ^ (offset / 8).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    word.to_le_bytes()[(offset % 8) as usize]
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fills `buf` with the resource bytes starting at `offset`.
pub fn fill(seed: u64, offset: u64, buf: &mut [u8]) {
    let mut o = offset;
    let mut i = 0;
    while i < buf.len() {
        let word = splitmix(seed ^ (o / 8).wrapping_mul(0x9E37_79B9_7F4A_7C15)).to_le_bytes();
        let start = (o % 8) as usize;
        let n = (8 - start).min(buf.len() - i);
        buf[i..i + n].copy_from_slice(&word[start..start + n]);
        i += n;
        o += n as u64;
    }
}

/// The whole resource with `seed` and `size`.
pub fn content(seed: u64, size: u64) -> Vec<u8> {
    let mut v = vec![0; size as usize];
    fill(seed, 0, &mut v);
    v
}

/// What the server saw for one request.
#[derive(Debug, Clone)]
pub struct RequestRecord {
    /// Path and query.
    pub path: String,
    /// `Range` header, if any.
    pub range: Option<String>,
    /// `If-Range` header, if any.
    pub if_range: Option<String>,
    /// Whether `Authorization` was present.
    pub authorization: bool,
    /// `Authorization` header, if any.
    pub authorization_value: Option<String>,
    /// `Cookie` header, if any.
    pub cookie: Option<String>,
    /// The status the server answered with.
    pub status: u16,
    /// HTTP method.
    pub method: String,
    /// When the request arrived, relative to server start.
    pub at: Duration,
    /// The HTTP version it arrived over (`HTTP_11` or `HTTP_2`).
    pub version: Version,
    /// The connection it arrived on (numbered from 0 in accept order).
    pub connection: u64,
}

/// One accepted connection.
#[derive(Debug)]
struct ConnCtx {
    id: u64,
    /// Requests seen on it, `/_stats` excluded.
    requests: AtomicUsize,
    /// Responses still being sent on it (HTTP/2 streams, or 0/1 for HTTP/1.1).
    in_flight: AtomicUsize,
    /// Counted in `connections` / `active` (from its first non-`/_stats` request).
    counted: AtomicBool,
    /// Cancelled to take the whole connection down (`drop_after`, `goaway_after`).
    kill: CancellationToken,
    /// ... with an HTTP/2 `GOAWAY` first.
    goaway: AtomicBool,
}

/// Counts a response in flight on its connection until its body is dropped.
#[derive(Debug)]
struct InFlight(Arc<ConnCtx>);

impl InFlight {
    fn start(state: &State, conn: &Arc<ConnCtx>) -> Self {
        let n = conn.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        state.max_streams.fetch_max(n, Ordering::SeqCst);
        InFlight(conn.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A response body that keeps its [`InFlight`] guard alive.
struct Tracked {
    inner: BoxBody,
    _guard: InFlight,
}

impl Body for Tracked {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, std::io::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[derive(Debug)]
struct State {
    started: std::time::Instant,
    counters: Mutex<HashMap<String, usize>>,
    log: Mutex<Vec<RequestRecord>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    connections: AtomicUsize,
    body_bytes: AtomicU64,
    /// Requests since start or the last stats reset (`/_stats` excluded).
    requests: AtomicUsize,
    tokens: Mutex<HashMap<String, usize>>,
    /// Shared "lines" for `line_bps`: when the line is free again, per key.
    lines: Mutex<HashMap<String, Arc<Mutex<tokio::time::Instant>>>>,
    next_token: AtomicUsize,
    /// Connection ids, in accept order.
    next_conn: AtomicU64,
    /// Most responses in flight on one connection at a time.
    max_streams: AtomicUsize,
    /// Requests turned away by `conn_limit`.
    refused: AtomicUsize,
    /// `conn_limit`: the connections that hold a slot, per key.
    holders: Mutex<HashMap<String, HashSet<u64>>>,
    /// Base URL of the TLS listener, for `/tls/` redirects.
    tls_base: Mutex<Option<String>>,
}

/// A running test server. Stops when dropped.
#[derive(Debug)]
pub struct TestServer {
    addr: SocketAddr,
    tls_main: bool,
    state: Arc<State>,
    stop: CancellationToken,
    ca: Option<Arc<TestCa>>,
    tls_addr: Option<SocketAddr>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// What every connection accepted on one listener shares.
#[derive(Clone)]
struct Listener {
    state: Arc<State>,
    stop: CancellationToken,
    /// `http://addr` or `https://addr`, for absolute redirects.
    base: Arc<str>,
    tls: Option<TlsAcceptor>,
}

impl TestServer {
    /// Starts a plain HTTP/1.1 server on an ephemeral localhost port.
    pub async fn start() -> TestServer {
        TestServer::start_on(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("bind test server")
    }

    /// Starts an HTTPS server on an ephemeral localhost port, offering `alpn`.
    /// Clients must trust [`TestServer::ca_cert_der`].
    pub async fn start_tls(alpn: Alpn) -> TestServer {
        TestServer::start_tls_on(SocketAddr::from(([127, 0, 0, 1], 0)), alpn)
            .await
            .expect("bind TLS test server")
    }

    /// Starts a plain HTTP/1.1 server on `addr` (port 0 picks a free port).
    pub async fn start_on(addr: SocketAddr) -> std::io::Result<TestServer> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State::new());
        let stop = CancellationToken::new();
        spawn_listener(
            listener,
            Listener {
                state: state.clone(),
                stop: stop.clone(),
                base: format!("http://{addr}").into(),
                tls: None,
            },
        );
        Ok(TestServer {
            addr,
            tls_main: false,
            state,
            stop,
            ca: None,
            tls_addr: None,
        })
    }

    /// Starts an HTTPS server on `addr` (port 0 picks a free port), offering
    /// `alpn`, with a newly generated certificate authority.
    pub async fn start_tls_on(addr: SocketAddr, alpn: Alpn) -> std::io::Result<TestServer> {
        let ca = Arc::new(TestCa::generate().map_err(std::io::Error::other)?);
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State::new());
        let stop = CancellationToken::new();
        let acceptor = TlsAcceptor::from(ca.server_config(alpn).map_err(std::io::Error::other)?);
        spawn_listener(
            listener,
            Listener {
                state: state.clone(),
                stop: stop.clone(),
                base: format!("https://{addr}").into(),
                tls: Some(acceptor),
            },
        );
        Ok(TestServer {
            addr,
            tls_main: true,
            state,
            stop,
            ca: Some(ca),
            tls_addr: Some(addr),
        })
    }

    /// Adds an HTTPS listener on `addr` to a plain server, offering `alpn`. It
    /// shares routes, counters and `/_stats` with the plain listener, and
    /// `/tls/...` on the plain listener redirects to it. Returns its address.
    pub async fn listen_tls(
        &mut self,
        addr: SocketAddr,
        alpn: Alpn,
    ) -> std::io::Result<SocketAddr> {
        let ca = match &self.ca {
            Some(ca) => ca.clone(),
            None => Arc::new(TestCa::generate().map_err(std::io::Error::other)?),
        };
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let acceptor = TlsAcceptor::from(ca.server_config(alpn).map_err(std::io::Error::other)?);
        let base = format!("https://{addr}");
        *self.state.tls_base.lock().expect("tls lock") = Some(base.clone());
        spawn_listener(
            listener,
            Listener {
                state: self.state.clone(),
                stop: self.stop.clone(),
                base: base.into(),
                tls: Some(acceptor),
            },
        );
        self.ca = Some(ca);
        self.tls_addr = Some(addr);
        Ok(addr)
    }

    /// `http://127.0.0.1:<port><path_and_query>` (`https` for a TLS server).
    pub fn url(&self, path_and_query: &str) -> String {
        let scheme = if self.tls_main { "https" } else { "http" };
        format!("{scheme}://{}{}", self.addr, path_and_query)
    }

    /// `https://127.0.0.1:<port><path_and_query>` on the TLS listener, if any.
    pub fn tls_url(&self, path_and_query: &str) -> Option<String> {
        self.tls_addr
            .map(|a| format!("https://{a}{path_and_query}"))
    }

    /// The listening address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The test CA certificate (DER) that issued the TLS listener's
    /// certificate, if there is a TLS listener.
    pub fn ca_cert_der(&self) -> Option<&[u8]> {
        self.ca.as_deref().map(TestCa::der)
    }

    /// The same certificate as PEM.
    pub fn ca_cert_pem(&self) -> Option<&str> {
        self.ca.as_deref().map(TestCa::pem)
    }

    /// The most responses that were in flight on one connection at a time
    /// (HTTP/2 streams multiplexed on it; 1 for HTTP/1.1 without pipelining).
    pub fn max_streams_per_connection(&self) -> usize {
        self.state.max_streams.load(Ordering::SeqCst)
    }

    /// Requests turned away because of `conn_limit`.
    pub fn refused_requests(&self) -> usize {
        self.state.refused.load(Ordering::SeqCst)
    }

    /// Every request received so far.
    pub fn requests(&self) -> Vec<RequestRecord> {
        self.state.log.lock().expect("log lock").clone()
    }

    /// The highest number of simultaneously open connections.
    pub fn max_concurrent_connections(&self) -> usize {
        self.state.max_active.load(Ordering::SeqCst)
    }

    /// Connections accepted so far.
    pub fn total_connections(&self) -> usize {
        self.state.connections.load(Ordering::SeqCst)
    }

    /// Body bytes sent so far (all responses).
    pub fn body_bytes_sent(&self) -> u64 {
        self.state.body_bytes.load(Ordering::SeqCst)
    }

    /// Resets the counters behind `/_stats` (as `/_stats?reset=1` does).
    pub fn reset_stats(&self) {
        self.state.reset_stats();
    }
}

impl State {
    fn new() -> State {
        State {
            started: std::time::Instant::now(),
            counters: Mutex::default(),
            log: Mutex::default(),
            active: AtomicUsize::default(),
            max_active: AtomicUsize::default(),
            connections: AtomicUsize::default(),
            body_bytes: AtomicU64::default(),
            requests: AtomicUsize::default(),
            tokens: Mutex::default(),
            lines: Mutex::default(),
            next_token: AtomicUsize::default(),
            next_conn: AtomicU64::default(),
            max_streams: AtomicUsize::default(),
            refused: AtomicUsize::default(),
            holders: Mutex::default(),
            tls_base: Mutex::default(),
        }
    }

    fn reset_stats(&self) {
        self.connections.store(0, Ordering::SeqCst);
        // Connections still open count towards the next maximum.
        self.max_active
            .store(self.active.load(Ordering::SeqCst), Ordering::SeqCst);
        self.body_bytes.store(0, Ordering::SeqCst);
        self.requests.store(0, Ordering::SeqCst);
        self.max_streams.store(0, Ordering::SeqCst);
        self.refused.store(0, Ordering::SeqCst);
    }

    fn stats_json(&self) -> String {
        format!(
            "{{\"connections\":{},\"max_concurrent\":{},\"requests\":{},\"body_bytes\":{},\"max_streams\":{},\"refused\":{}}}\n",
            self.connections.load(Ordering::SeqCst),
            self.max_active.load(Ordering::SeqCst),
            self.requests.load(Ordering::SeqCst),
            self.body_bytes.load(Ordering::SeqCst),
            self.max_streams.load(Ordering::SeqCst),
            self.refused.load(Ordering::SeqCst),
        )
    }

    /// `conn_limit`: may connection `conn` fetch `key`? A connection keeps its
    /// slot until it closes.
    fn admit(&self, key: &str, conn: u64, limit: usize) -> bool {
        let mut holders = self.holders.lock().expect("holders lock");
        let set = holders.entry(key.to_owned()).or_default();
        set.contains(&conn) || (set.len() < limit && set.insert(conn))
    }

    /// Frees the `conn_limit` slots of a closed connection.
    fn release(&self, conn: u64) {
        let mut holders = self.holders.lock().expect("holders lock");
        for set in holders.values_mut() {
            set.remove(&conn);
        }
        holders.retain(|_, s| !s.is_empty());
    }
}

fn spawn_listener(listener: TcpListener, l: Listener) {
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                () = l.stop.cancelled() => break,
                a = listener.accept() => a,
            };
            let Ok((tcp, _)) = accepted else { continue };
            let _ = tcp.set_nodelay(true);
            tokio::spawn(l.clone().serve(tcp));
        }
    });
}

impl Listener {
    async fn serve(self, tcp: TcpStream) {
        let conn = Arc::new(ConnCtx {
            id: self.state.next_conn.fetch_add(1, Ordering::SeqCst),
            requests: AtomicUsize::default(),
            in_flight: AtomicUsize::default(),
            counted: AtomicBool::new(false),
            kill: CancellationToken::new(),
            goaway: AtomicBool::new(false),
        });
        let svc = {
            let (state, conn, base) = (self.state.clone(), conn.clone(), self.base.clone());
            service_fn(move |req: Request<Incoming>| {
                respond(state.clone(), conn.clone(), base.clone(), req)
            })
        };
        match &self.tls {
            None => {
                let c = http1::Builder::new()
                    .keep_alive(true)
                    .serve_connection(TokioIo::new(tcp), svc);
                drive(c, &self.stop, &conn).await;
            }
            Some(acceptor) => {
                let handshake = tokio::select! {
                    () = self.stop.cancelled() => return,
                    r = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)) => r,
                };
                let Ok(Ok(tls)) = handshake else { return };
                let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
                let io = TokioIo::new(tls);
                if h2 {
                    let c = http2::Builder::new(TokioExecutor::new()).serve_connection(io, svc);
                    drive(c, &self.stop, &conn).await;
                } else {
                    let c = http1::Builder::new()
                        .keep_alive(true)
                        .serve_connection(io, svc);
                    drive(c, &self.stop, &conn).await;
                }
            }
        }
        if conn.counted.load(Ordering::SeqCst) {
            self.state.active.fetch_sub(1, Ordering::SeqCst);
        }
        self.state.release(conn.id);
    }
}

/// Runs a connection until it ends, the server stops, or a request kills it.
async fn drive<C: GracefulConnection>(c: C, stop: &CancellationToken, conn: &ConnCtx) {
    let mut c = std::pin::pin!(c);
    tokio::select! {
        () = stop.cancelled() => {}
        _ = c.as_mut() => {}
        () = conn.kill.cancelled() => {
            if conn.goaway.load(Ordering::SeqCst) {
                // HTTP/2: GOAWAY, then (the stream that asked for it never
                // finishes) close the socket shortly after.
                c.as_mut().graceful_shutdown();
                let _ = tokio::time::timeout(Duration::from_millis(100), c.as_mut()).await;
            }
        }
    }
}

/// One request: connection bookkeeping and lab latency around [`handle`].
async fn respond(
    state: Arc<State>,
    conn: Arc<ConnCtx>,
    base: Arc<str>,
    req: Request<Incoming>,
) -> Result<Response<BoxBody>, Infallible> {
    if req.uri().path() == "/_stats" {
        return Ok(handle(&state, &conn, &base, &req));
    }
    // A connection counts from its first request that is not `/_stats`, so
    // asking for the counters does not change them.
    if !conn.counted.swap(true, Ordering::SeqCst) {
        state.connections.fetch_add(1, Ordering::SeqCst);
        let now = state.active.fetch_add(1, Ordering::SeqCst) + 1;
        state.max_active.fetch_max(now, Ordering::SeqCst);
    }
    let guard = InFlight::start(&state, &conn);
    let first = conn.requests.fetch_add(1, Ordering::SeqCst) == 0;
    let latency = query_map(req.uri().query().unwrap_or(""))
        .get("latency_ms")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms > 0);
    if let Some(ms) = latency {
        // One round trip per request; a new connection pays one more for its set-up.
        let rtts = if first { 2 } else { 1 };
        tokio::time::sleep(Duration::from_millis(ms * rtts)).await;
    }
    let resp = handle(&state, &conn, &base, &req);
    Ok(resp.map(|b| {
        http_body_util::BodyExt::boxed(Tracked {
            inner: b,
            _guard: guard,
        })
    }))
}

type BoxBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;

/// A body fed from a channel, so a producer task can throttle, stall or fail it.
struct ChannelBody {
    rx: mpsc::Receiver<Result<Bytes, std::io::Error>>,
}

impl Body for ChannelBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, std::io::Error>>> {
        self.rx.poll_recv(cx).map(|o| o.map(|r| r.map(Frame::data)))
    }
}

fn empty() -> BoxBody {
    use http_body_util::BodyExt;
    http_body_util::Empty::new()
        .map_err(|never: Infallible| match never {})
        .boxed()
}

fn text(s: &'static str) -> BoxBody {
    use http_body_util::BodyExt;
    http_body_util::Full::new(Bytes::from_static(s.as_bytes()))
        .map_err(|never: Infallible| match never {})
        .boxed()
}

fn full(s: String) -> BoxBody {
    use http_body_util::BodyExt;
    http_body_util::Full::new(Bytes::from(s))
        .map_err(|never: Infallible| match never {})
        .boxed()
}

fn query_map(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.to_owned(), v.to_owned())
        })
        .collect()
}

fn num(q: &HashMap<String, String>, k: &str) -> Option<u64> {
    q.get(k).and_then(|v| v.parse().ok())
}

fn handle(
    state: &Arc<State>,
    conn: &Arc<ConnCtx>,
    base: &str,
    req: &Request<Incoming>,
) -> Response<BoxBody> {
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or("").to_owned();
    let get = |h: header::HeaderName| {
        req.headers()
            .get(h)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let mut record = RequestRecord {
        path: req.uri().to_string(),
        range: get(header::RANGE),
        if_range: get(header::IF_RANGE),
        authorization: req.headers().contains_key(header::AUTHORIZATION),
        authorization_value: get(header::AUTHORIZATION),
        cookie: get(header::COOKIE),
        status: 0,
        method: req.method().to_string(),
        at: state.started.elapsed(),
        version: req.version(),
        connection: conn.id,
    };
    let q = query_map(&query);
    if path == "/_stats" {
        let body = state.stats_json();
        if q.contains_key("reset") {
            state.reset_stats();
        }
        return Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::CONNECTION, "close")
            .body(full(body))
            .expect("valid response");
    }
    state.requests.fetch_add(1, Ordering::SeqCst);
    let limit = num(&q, "conn_limit").map(|l| {
        let key = q.get("key").cloned().unwrap_or_else(|| query.clone());
        state.admit(&key, conn.id, l as usize)
    });
    let resp = if limit == Some(false) {
        state.refused.fetch_add(1, Ordering::SeqCst);
        let mut b = Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(header::CONTENT_TYPE, "text/plain");
        if req.version() < Version::HTTP_2 {
            b = b.header(header::CONNECTION, "close");
        }
        if let Some(ra) = q.get("retry_after") {
            b = b.header(header::RETRY_AFTER, ra.as_str());
        }
        b.body(text("too many connections\n"))
            .expect("valid response")
    } else if let Some(rest) = path.strip_prefix("/tls/") {
        let tls = state.tls_base.lock().expect("tls lock").clone();
        match tls {
            Some(tls) => {
                let mut location = format!("{tls}/{rest}");
                if !query.is_empty() {
                    location.push('?');
                    location.push_str(&query);
                }
                Response::builder()
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .header(header::LOCATION, location)
                    .body(empty())
                    .expect("valid response")
            }
            None => status(StatusCode::NOT_FOUND),
        }
    } else if path == "/redirect" {
        redirect(&q, base)
    } else if path == "/switch" {
        let n = {
            let mut c = state.counters.lock().expect("counter lock");
            let e = c.entry(format!("switch:{query}")).or_insert(0);
            *e += 1;
            *e - 1
        };
        let after = num(&q, "after").unwrap_or(1) as usize;
        let to = q
            .get(if n < after { "to" } else { "then" })
            .cloned()
            .unwrap_or_default();
        Response::builder()
            .status(StatusCode::FOUND)
            .header(
                header::LOCATION,
                percent_decode_str(&to).decode_utf8_lossy().into_owned(),
            )
            .body(empty())
            .expect("valid response")
    } else if path == "/signed" {
        let token = state.next_token.fetch_add(1, Ordering::SeqCst).to_string();
        let valid = num(&q, "valid").unwrap_or(1) as usize;
        state
            .tokens
            .lock()
            .expect("token lock")
            .insert(token.clone(), valid);
        Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, format!("/s/{token}?{query}"))
            .body(empty())
            .expect("valid response")
    } else if let Some(token) = path.strip_prefix("/s/") {
        let ok = {
            let mut tokens = state.tokens.lock().expect("token lock");
            match tokens.get_mut(token) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    true
                }
                _ => false,
            }
        };
        if ok {
            data(state, conn, &q, &query, req)
        } else {
            status(StatusCode::FORBIDDEN)
        }
    } else if path == "/setcookie" {
        let to = q.get("to").cloned().unwrap_or_default();
        let set = q.get("set").cloned().unwrap_or_default();
        Response::builder()
            .status(StatusCode::FOUND)
            .header(
                header::LOCATION,
                percent_decode_str(&to).decode_utf8_lossy().into_owned(),
            )
            .header(
                header::SET_COOKIE,
                percent_decode_str(&set).decode_utf8_lossy().into_owned(),
            )
            .body(empty())
            .expect("valid response")
    } else if path == "/cookie" {
        let need = q
            .get("need")
            .map(|n| percent_decode_str(n).decode_utf8_lossy().into_owned())
            .unwrap_or_default();
        let has = record
            .cookie
            .as_deref()
            .is_some_and(|c| c.split("; ").any(|kv| kv == need));
        if has {
            data(state, conn, &q, &query, req)
        } else {
            status(StatusCode::FORBIDDEN)
        }
    } else if path == "/auth" {
        if record.authorization {
            data(state, conn, &q, &query, req)
        } else {
            status(StatusCode::UNAUTHORIZED)
        }
    } else if path == "/data" {
        data(state, conn, &q, &query, req)
    } else {
        status(StatusCode::NOT_FOUND)
    };
    record.status = resp.status().as_u16();
    state.log.lock().expect("log lock").push(record);
    resp
}

fn status(code: StatusCode) -> Response<BoxBody> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(text("error\n"))
        .expect("valid response")
}

fn redirect(q: &HashMap<String, String>, base: &str) -> Response<BoxBody> {
    let hops = num(q, "hops").unwrap_or(1);
    let code = num(q, "code").unwrap_or(302) as u16;
    let to = q.get("to").cloned().unwrap_or_default();
    let location = if hops <= 1 {
        percent_decode_str(&to).decode_utf8_lossy().into_owned()
    } else {
        format!("{base}/redirect?hops={}&code={code}&to={to}", hops - 1)
    };
    Response::builder()
        .status(StatusCode::from_u16(code).unwrap_or(StatusCode::FOUND))
        .header(header::LOCATION, location)
        .body(empty())
        .expect("valid response")
}

/// Parses a single `bytes=` range against `size`. `Err(())` = unsatisfiable.
#[allow(clippy::result_unit_err)]
fn parse_range(v: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = v.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None; // multi-range: ignore
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let r = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        if n == 0 || size == 0 {
            Err(())
        } else {
            Ok((size.saturating_sub(n), size - 1))
        }
    } else {
        let start: u64 = a.parse().ok()?;
        let end: u64 = if b.is_empty() {
            size.saturating_sub(1)
        } else {
            b.parse::<u64>().ok()?.min(size.saturating_sub(1))
        };
        if start >= size || end < start {
            Err(())
        } else {
            Ok((start, end))
        }
    };
    Some(r)
}

#[allow(clippy::many_single_char_names)] // q, n, b, s, e are conventional here.
fn data(
    state: &Arc<State>,
    conn: &Arc<ConnCtx>,
    q: &HashMap<String, String>,
    query: &str,
    req: &Request<Incoming>,
) -> Response<BoxBody> {
    let key = q.get("key").cloned().unwrap_or_else(|| query.to_owned());
    let n = {
        let mut c = state.counters.lock().expect("counter lock");
        let e = c.entry(key).or_insert(0);
        let n = *e;
        *e += 1;
        n as u64
    };
    let size = num(q, "size").unwrap_or(1024 * 1024);
    let changed = num(q, "change_after").is_some_and(|k| n >= k);
    let seed = num(q, "seed").unwrap_or(1) + u64::from(changed);
    let version = if changed { "v2" } else { "v1" };

    if let Some(code) = num(q, "fail_status") {
        let from = num(q, "fail_from").unwrap_or(0);
        let failing = n >= from && num(q, "fail_count").is_none_or(|c| n < from + c);
        if failing {
            let mut b = Response::builder().status(
                StatusCode::from_u16(code as u16).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            );
            if let Some(ra) = q.get("retry_after") {
                b = b.header(header::RETRY_AFTER, ra.as_str());
            }
            return b.body(text("failing\n")).expect("valid response");
        }
    }

    // Another storage backend can tag the same bytes differently.
    let salt = q
        .get("etag_salt")
        .map_or(String::new(), |s| format!("-{s}"));
    let etag = match q.get("etag").map_or("strong", String::as_str) {
        "none" => None,
        "weak" => Some(format!("W/\"{seed}-{size}-{version}{salt}\"")),
        _ => Some(format!("\"{seed}-{size}-{version}{salt}\"")),
    };
    if num(q, "html_from").is_some_and(|k| n >= k) {
        // A login or error page served with a success status.
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(text(
                "<html><body>Please log in</body></html>
",
            ))
            .expect("valid response");
    }
    let now = SystemTime::now();
    let last_modified = (num(q, "lastmod") == Some(1)).then(|| {
        // Fixed per version, so it is a usable validator; a changed resource has a new date.
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        httpdate::fmt_http_date(base + Duration::from_secs(if changed { 3600 } else { 0 }))
    });
    let ranges = q.get("ranges").map_or("yes", String::as_str);
    let range_hdr = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let if_range = req
        .headers()
        .get(header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let mut range = match (ranges, range_hdr.as_deref()) {
        ("none", _) | (_, None) => None,
        (_, Some(v)) => parse_range(v, size),
    };
    if ranges == "ignore-zero" && matches!(range, Some(Ok((0, _)))) {
        range = None;
    }
    // If-Range: serve the full resource when the validator does not match.
    if let (Some(Ok(_)), Some(ir)) = (&range, &if_range) {
        let matches =
            etag.as_deref() == Some(ir.as_str()) || last_modified.as_deref() == Some(ir.as_str());
        if !matches && q.get("ifrange").map(String::as_str) != Some("ignore") {
            range = None;
        }
    }

    let mut b = Response::builder()
        .header(header::DATE, httpdate::fmt_http_date(now))
        .header(
            header::CONTENT_TYPE,
            q.get("ctype")
                .map_or("application/octet-stream", String::as_str),
        );
    if let Some(e) = &etag {
        b = b.header(header::ETAG, e.as_str());
    }
    if let Some(lm) = &last_modified {
        b = b.header(header::LAST_MODIFIED, lm.as_str());
    }
    if let Some(cd) = q.get("cd") {
        let raw: Vec<u8> = percent_decode_str(cd).collect();
        if let Ok(v) = HeaderValue::from_bytes(&raw) {
            b = b.header(header::CONTENT_DISPOSITION, v);
        }
    }
    if q.get("encoding").map(String::as_str) == Some("gzip") {
        b = b.header(header::CONTENT_ENCODING, "gzip");
    }
    let digest = q.get("digest").cloned();
    let digest_bad = num(q, "digest_bad") == Some(1);

    let (status, start, len) = match range {
        Some(Err(())) => {
            return b
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .body(empty())
                .expect("valid response");
        }
        Some(Ok((s, e))) => {
            let total = if num(q, "star_total") == Some(1) {
                "*".to_owned()
            } else {
                size.to_string()
            };
            b = b.header(header::CONTENT_RANGE, format!("bytes {s}-{e}/{total}"));
            (StatusCode::PARTIAL_CONTENT, s, e - s + 1)
        }
        None => {
            if ranges != "none" && num(q, "accept_ranges") != Some(0) {
                b = b.header(header::ACCEPT_RANGES, "bytes");
            }
            (StatusCode::OK, 0, size)
        }
    };
    if let Some(kind) = &digest {
        let (name, value) = digest_header(kind, seed, size, start, len, digest_bad);
        if let Some(name) = name {
            b = b.header(name, value);
        }
    }
    let chunked = status == StatusCode::OK && num(q, "chunked") == Some(1);
    if !chunked {
        b = b.header(header::CONTENT_LENGTH, len);
    }

    let line = num(q, "line_bps").filter(|b| *b > 0).map(|bps| {
        let k = q.get("key").cloned().unwrap_or_else(|| query.to_owned());
        let slot = state
            .lines
            .lock()
            .expect("line lock")
            .entry(k)
            .or_insert_with(|| Arc::new(Mutex::new(tokio::time::Instant::now())))
            .clone();
        (slot, bps)
    });
    let plan = Plan {
        line,
        crawl: num(q, "crawl_after")
            .zip(num(q, "crawl_bps"))
            .filter(|_| num(q, "crawl_count").is_none_or(|c| n < c)),
        seed,
        start,
        len,
        throttle: if num(q, "slow_index") == Some(n) {
            num(q, "slow_bps")
        } else {
            num(q, "throttle")
        },
        stall_after: num(q, "stall_after").filter(|_| num(q, "stall_count").is_none_or(|c| n < c)),
        reset_after: num(q, "reset_after").filter(|_| num(q, "reset_count").is_none_or(|c| n < c)),
        short: num(q, "short_count").is_some_and(|c| n < c),
        kill: num(q, "goaway_after")
            .filter(|_| num(q, "goaway_count").is_none_or(|c| n < c))
            .map(|b| (b, true))
            .or_else(|| {
                num(q, "drop_after")
                    .filter(|_| num(q, "drop_count").is_none_or(|c| n < c))
                    .map(|b| (b, false))
            }),
        conn: conn.clone(),
    };
    let (tx, rx) = mpsc::channel(4);
    let st = state.clone();
    tokio::spawn(produce(plan, tx, st));
    b.status(status)
        .body(http_body_util::BodyExt::boxed(ChannelBody { rx }))
        .expect("valid response")
}

#[derive(Debug, Clone)]
struct Plan {
    /// After this many bytes, slow down to this rate (a connection that starts
    /// to crawl).
    crawl: Option<(u64, u64)>,
    /// A line shared by all responses with the same key: (free-at, bytes per second).
    line: Option<(Arc<Mutex<tokio::time::Instant>>, u64)>,
    seed: u64,
    start: u64,
    len: u64,
    throttle: Option<u64>,
    stall_after: Option<u64>,
    reset_after: Option<u64>,
    short: bool,
    /// After this many bytes, take the whole connection down (with `GOAWAY` first if true).
    kill: Option<(u64, bool)>,
    conn: Arc<ConnCtx>,
}

async fn produce(p: Plan, tx: mpsc::Sender<Result<Bytes, std::io::Error>>, state: Arc<State>) {
    const CHUNK: u64 = 16 * 1024;
    let limit = if p.short { p.len / 2 } else { p.len };
    let mut sent = 0u64;
    let started = tokio::time::Instant::now();
    while sent < limit {
        if p.stall_after.is_some_and(|s| sent >= s) {
            // Stop sending, keep the connection open until the client gives up.
            tx.closed().await;
            return;
        }
        if p.reset_after.is_some_and(|r| sent >= r) {
            let _ = tx
                .send(Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "simulated reset",
                )))
                .await;
            return;
        }
        if let Some((_, goaway)) = p.kill.filter(|(k, _)| sent >= *k) {
            p.conn.goaway.store(goaway, Ordering::SeqCst);
            p.conn.kill.cancel();
            tx.closed().await;
            return;
        }
        let mut n = CHUNK.min(limit - sent);
        if let Some(s) = p.stall_after.filter(|s| *s > sent) {
            n = n.min(s - sent);
        }
        if let Some(r) = p.reset_after.filter(|r| *r > sent) {
            n = n.min(r - sent);
        }
        if let Some((k, _)) = p.kill.filter(|(k, _)| *k > sent) {
            n = n.min(k - sent);
        }
        let mut buf = vec![0u8; n as usize];
        fill(p.seed, p.start + sent, &mut buf);
        if let Some(bps) = p.throttle.filter(|b| *b > 0) {
            let due = Duration::from_secs_f64((sent + n) as f64 / bps as f64);
            tokio::select! {
                () = tokio::time::sleep_until(started + due) => {}
                () = tx.closed() => return,
            }
        }
        if let Some((_, bps)) = p.crawl.filter(|(a, b)| sent >= *a && *b > 0) {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs_f64(n as f64 / bps as f64)) => {}
                () = tx.closed() => return,
            }
        }
        if let Some((slot, bps)) = &p.line {
            // Book this chunk's time on the shared line (FIFO), then wait for it.
            let done_at = {
                let mut free = slot.lock().expect("line lock");
                // Up to 50 ms of credit, so a sender that wakes late (coarse
                // timers on Windows) does not leave the line idle.
                let now = tokio::time::Instant::now();
                let begin = (*free).max(now.checked_sub(Duration::from_millis(50)).unwrap_or(now));
                *free = begin + Duration::from_secs_f64(n as f64 / *bps as f64);
                *free
            };
            tokio::select! {
                () = tokio::time::sleep_until(done_at) => {}
                () = tx.closed() => return,
            }
        }
        if tx.send(Ok(Bytes::from(buf))).await.is_err() {
            return;
        }
        state.body_bytes.fetch_add(n, Ordering::SeqCst);
        sent += n;
    }
    if p.short {
        // Close early: the client sees fewer bytes than Content-Length.
        let _ = tx
            .send(Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "simulated short body",
            )))
            .await;
    }
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The digest header for `digest=<kind>`: whole-representation digests cover
/// `[0, size)`, body digests cover this response's `[start, start + len)`.
fn digest_header(
    kind: &str,
    seed: u64,
    size: u64,
    start: u64,
    len: u64,
    bad: bool,
) -> (Option<&'static str>, String) {
    use md5::Digest as _;
    let slice = |a: u64, n: u64| {
        let mut v = vec![0u8; n as usize];
        fill(seed, a, &mut v);
        if bad && !v.is_empty() {
            v[0] ^= 0xff;
        }
        v
    };
    let whole = || slice(0, size);
    let body = || slice(start, len);
    match kind {
        "repr" => (
            Some("repr-digest"),
            format!("sha-256=:{}:", base64(&sha2::Sha256::digest(whole()))),
        ),
        "content" => (
            Some("content-digest"),
            format!("sha-256=:{}:", base64(&sha2::Sha256::digest(body()))),
        ),
        "legacy" => (
            Some("digest"),
            format!("SHA-512={}", base64(&sha2::Sha512::digest(whole()))),
        ),
        "md5" => (Some("content-md5"), base64(&md5::Md5::digest(body()))),
        "goog" => (
            Some("x-goog-hash"),
            format!("crc32c=AAAAAA==,md5={}", base64(&md5::Md5::digest(whole()))),
        ),
        "amz" => (
            Some("x-amz-checksum-sha256"),
            base64(&sha2::Sha256::digest(whole())),
        ),
        _ => (None, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_matches_byte_at() {
        let mut buf = vec![0u8; 1000];
        fill(7, 13, &mut buf);
        for (i, b) in buf.iter().enumerate() {
            assert_eq!(*b, byte_at(7, 13 + i as u64));
        }
        assert_ne!(content(1, 64), content(2, 64));
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 10), Some(Ok((0, 9))));
        assert_eq!(parse_range("bytes=2-4", 10), Some(Ok((2, 4))));
        assert_eq!(parse_range("bytes=5-100", 10), Some(Ok((5, 9))));
        assert_eq!(parse_range("bytes=-3", 10), Some(Ok((7, 9))));
        assert_eq!(parse_range("bytes=10-", 10), Some(Err(())));
        assert_eq!(parse_range("bytes=0-", 0), Some(Err(())));
        assert_eq!(parse_range("items=0-1", 10), None);
    }

    /// Sends one HTTP/1.1 GET on `tcp` and returns the status code and the
    /// time to the first response byte. Reads the whole (small) response.
    async fn get(tcp: &mut TcpStream, pq: &str) -> (u16, Duration) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let t = std::time::Instant::now();
        tcp.write_all(format!("GET {pq} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buf = Vec::new();
        let mut first = None;
        loop {
            let mut chunk = [0u8; 4096];
            let n = tcp.read(&mut chunk).await.unwrap();
            first.get_or_insert_with(|| t.elapsed());
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf);
            if let Some(end) = text.find("\r\n\r\n") {
                let len: usize = text[..end]
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .map_or(0, |v| v.trim().parse().unwrap());
                if buf.len() >= end + 4 + len || n == 0 {
                    let status = text[9..12].parse().unwrap();
                    return (status, first.unwrap());
                }
            }
            assert!(n > 0, "connection closed: {text}");
        }
    }

    #[tokio::test]
    async fn latency_delays_each_response_head() {
        let srv = TestServer::start().await;
        let mut tcp = TcpStream::connect(srv.addr()).await.unwrap();
        let (status, first) = get(&mut tcp, "/data?size=10&latency_ms=150").await;
        assert_eq!(status, 200);
        // A new connection pays two round trips, a reused one one.
        assert!(first >= Duration::from_millis(290), "{first:?}");
        let (_, second) = get(&mut tcp, "/data?size=10&latency_ms=150").await;
        assert!(second >= Duration::from_millis(140), "{second:?}");
    }

    #[tokio::test]
    async fn connection_limit_refuses_extra_connections_until_one_closes() {
        let srv = TestServer::start().await;
        let pq = "/data?size=10&conn_limit=2&key=k";
        let mut a = TcpStream::connect(srv.addr()).await.unwrap();
        let mut b = TcpStream::connect(srv.addr()).await.unwrap();
        let mut c = TcpStream::connect(srv.addr()).await.unwrap();
        assert_eq!(get(&mut a, pq).await.0, 200);
        assert_eq!(get(&mut b, pq).await.0, 200);
        assert_eq!(get(&mut c, pq).await.0, 503);
        // A holder keeps its slot for further requests.
        assert_eq!(get(&mut a, pq).await.0, 200);
        // Another key has its own slots.
        assert_eq!(
            get(&mut a, "/data?size=10&conn_limit=1&key=other").await.0,
            200
        );
        assert_eq!(srv.refused_requests(), 1);
        drop(b);
        let mut d = TcpStream::connect(srv.addr()).await.unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if get(&mut d, pq).await.0 == 200 {
                break;
            }
            // `503` closes the connection; the server notices `b` closing soon.
            assert!(std::time::Instant::now() < deadline, "slot never freed");
            tokio::time::sleep(Duration::from_millis(20)).await;
            d = TcpStream::connect(srv.addr()).await.unwrap();
        }
    }
}
