//! A deliberately misbehaving local HTTP/1.1 server for BoltGet's tests.
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
//!
//! Other routes:
//!
//! - `/redirect?hops=N&code=C&to=<pct-encoded URL>`: a chain of N redirects ending at `to`.
//! - `/switch?after=K&to=<url>&then=<url>`: redirects to `to` for the first K requests,
//!   then to `then` (a redirect target that moved).
//! - `/signed?valid=M&...`: redirects to a fresh `/s/<token>?...` URL that serves
//!   `/data` for M requests, then answers 403 (an expiring signed URL).
//! - `/auth?...`: like `/data`, but 401 unless `Authorization` is present.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http::{HeaderValue, Request, Response, StatusCode, header};
use http_body::{Body, Frame};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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
    /// The status the server answered with.
    pub status: u16,
    /// HTTP method.
    pub method: String,
    /// When the request arrived, relative to server start.
    pub at: Duration,
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
    tokens: Mutex<HashMap<String, usize>>,
    next_token: AtomicUsize,
}

/// A running test server. Stops when dropped.
#[derive(Debug)]
pub struct TestServer {
    addr: SocketAddr,
    state: Arc<State>,
    stop: CancellationToken,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl TestServer {
    /// Starts a server on an ephemeral localhost port.
    pub async fn start() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(State {
            started: std::time::Instant::now(),
            counters: Mutex::default(),
            log: Mutex::default(),
            active: AtomicUsize::default(),
            max_active: AtomicUsize::default(),
            connections: AtomicUsize::default(),
            body_bytes: AtomicU64::default(),
            tokens: Mutex::default(),
            next_token: AtomicUsize::default(),
        });
        let stop = CancellationToken::new();
        let (s, st) = (state.clone(), stop.clone());
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    () = st.cancelled() => break,
                    a = listener.accept() => a,
                };
                let Ok((tcp, _)) = accepted else { continue };
                let s = s.clone();
                let st = st.clone();
                tokio::spawn(async move {
                    s.connections.fetch_add(1, Ordering::SeqCst);
                    let now = s.active.fetch_add(1, Ordering::SeqCst) + 1;
                    s.max_active.fetch_max(now, Ordering::SeqCst);
                    let svc_state = s.clone();
                    let svc = service_fn(move |req| {
                        let s = svc_state.clone();
                        async move { Ok::<_, Infallible>(handle(&s, addr, &req)) }
                    });
                    let conn = http1::Builder::new()
                        .keep_alive(true)
                        .serve_connection(TokioIo::new(tcp), svc);
                    tokio::select! {
                        () = st.cancelled() => {}
                        _ = conn => {}
                    }
                    s.active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        TestServer { addr, state, stop }
    }

    /// `http://127.0.0.1:<port><path_and_query>`.
    pub fn url(&self, path_and_query: &str) -> String {
        format!("http://{}{}", self.addr, path_and_query)
    }

    /// The listening address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
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

fn handle(state: &Arc<State>, addr: SocketAddr, req: &Request<Incoming>) -> Response<BoxBody> {
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
        status: 0,
        method: req.method().to_string(),
        at: state.started.elapsed(),
    };
    let q = query_map(&query);
    let resp = if path == "/redirect" {
        redirect(&q, addr)
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
            data(state, &q, &query, req)
        } else {
            status(StatusCode::FORBIDDEN)
        }
    } else if path == "/auth" {
        if record.authorization {
            data(state, &q, &query, req)
        } else {
            status(StatusCode::UNAUTHORIZED)
        }
    } else if path == "/data" {
        data(state, &q, &query, req)
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

fn redirect(q: &HashMap<String, String>, addr: SocketAddr) -> Response<BoxBody> {
    let hops = num(q, "hops").unwrap_or(1);
    let code = num(q, "code").unwrap_or(302) as u16;
    let to = q.get("to").cloned().unwrap_or_default();
    let location = if hops <= 1 {
        percent_decode_str(&to).decode_utf8_lossy().into_owned()
    } else {
        format!(
            "http://{addr}/redirect?hops={}&code={code}&to={to}",
            hops - 1
        )
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

    let plan = Plan {
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
    };
    let (tx, rx) = mpsc::channel(4);
    let st = state.clone();
    tokio::spawn(produce(plan, tx, st));
    b.status(status)
        .body(http_body_util::BodyExt::boxed(ChannelBody { rx }))
        .expect("valid response")
}

#[derive(Debug, Clone, Copy)]
struct Plan {
    seed: u64,
    start: u64,
    len: u64,
    throttle: Option<u64>,
    stall_after: Option<u64>,
    reset_after: Option<u64>,
    short: bool,
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
        let mut n = CHUNK.min(limit - sent);
        if let Some(s) = p.stall_after.filter(|s| *s > sent) {
            n = n.min(s - sent);
        }
        if let Some(r) = p.reset_after.filter(|r| *r > sent) {
            n = n.min(r - sent);
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
}
