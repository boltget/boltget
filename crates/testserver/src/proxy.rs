//! A test proxy: an HTTP proxy (absolute-form requests and `CONNECT`) or a
//! SOCKS5 proxy, optionally requiring credentials. It records what clients
//! asked for, so tests can check which requests went through it and how.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// The protocol a [`TestProxy`] speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMode {
    /// HTTP proxy: absolute-form requests for `http`, `CONNECT` tunnels.
    Http,
    /// SOCKS5 (RFC 1928) with optional username/password (RFC 1929).
    Socks5,
}

/// One connection a client opened through the proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRecord {
    /// `"absolute"` (a forwarded `GET http://...`), `"connect"` or `"socks5"`.
    pub kind: &'static str,
    /// `host:port` (SOCKS5: as the client sent it; names are resolved here).
    pub target: String,
    /// Whether the client passed the proxy's credential check.
    pub authorized: bool,
}

#[derive(Debug)]
struct ProxyState {
    mode: ProxyMode,
    auth: Option<(String, String)>,
    log: Mutex<Vec<ProxyRecord>>,
}

/// A running test proxy on localhost. Stops when dropped.
#[derive(Debug)]
pub struct TestProxy {
    addr: SocketAddr,
    state: Arc<ProxyState>,
    stop: CancellationToken,
}

impl Drop for TestProxy {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl TestProxy {
    /// Starts a proxy; with `auth`, clients must send that username and password.
    pub async fn start(mode: ProxyMode, auth: Option<(&str, &str)>) -> TestProxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(ProxyState {
            mode,
            auth: auth.map(|(u, p)| (u.to_owned(), p.to_owned())),
            log: Mutex::default(),
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
                    tokio::select! {
                        () = st.cancelled() => {}
                        () = serve(s, tcp) => {}
                    }
                });
            }
        });
        TestProxy { addr, state, stop }
    }

    /// The proxy URL with the given credentials embedded (`None`: without).
    pub fn url_with(&self, auth: Option<(&str, &str)>) -> String {
        let scheme = match self.state.mode {
            ProxyMode::Http => "http",
            ProxyMode::Socks5 => "socks5",
        };
        match auth {
            Some((u, p)) => format!("{scheme}://{u}:{p}@{}", self.addr),
            None => format!("{scheme}://{}", self.addr),
        }
    }

    /// The proxy URL with the credentials it requires, if any.
    pub fn url(&self) -> String {
        let auth = self.state.auth.clone();
        self.url_with(auth.as_ref().map(|(u, p)| (u.as_str(), p.as_str())))
    }

    /// Connections opened through the proxy so far.
    pub fn records(&self) -> Vec<ProxyRecord> {
        self.state.log.lock().expect("proxy log").clone()
    }
}

async fn read_head<S: AsyncRead + Unpin>(s: &mut S) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 64 * 1024 || s.read(&mut b).await.ok()? == 0 {
            return None;
        }
        head.push(b[0]);
    }
    Some(head)
}

async fn serve(state: Arc<ProxyState>, mut tcp: TcpStream) {
    match state.mode {
        ProxyMode::Http => serve_http(&state, &mut tcp).await,
        ProxyMode::Socks5 => serve_socks(&state, &mut tcp).await,
    }
}

fn record(state: &ProxyState, kind: &'static str, target: &str, authorized: bool) {
    state.log.lock().expect("proxy log").push(ProxyRecord {
        kind,
        target: target.to_owned(),
        authorized,
    });
}

async fn serve_http(state: &ProxyState, tcp: &mut TcpStream) {
    let Some(head) = read_head(tcp).await else {
        return;
    };
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut words = text.split_whitespace();
    let (Some(method), Some(target)) = (words.next(), words.next()) else {
        return;
    };
    let given = text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("proxy-authorization")
            .then(|| v.trim().to_owned())
    });
    let authorized = match &state.auth {
        None => true,
        Some((u, p)) => {
            let want = format!("Basic {}", crate::base64(format!("{u}:{p}").as_bytes()));
            given.as_deref() == Some(want.as_str())
        }
    };
    let (kind, authority) = if method == "CONNECT" {
        ("connect", target.to_owned())
    } else {
        let rest = target.strip_prefix("http://").unwrap_or(target);
        ("absolute", rest.split('/').next().unwrap_or("").to_owned())
    };
    record(state, kind, &authority, authorized);
    if !authorized {
        let _ = tcp
            .write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\n\
                  Proxy-Authenticate: Basic realm=\"test\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        return;
    }
    let Ok(mut up) = TcpStream::connect(authority.as_str()).await else {
        let _ = tcp
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
            .await;
        return;
    };
    if kind == "connect" {
        if tcp
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .is_err()
        {
            return;
        }
    } else if up.write_all(&head).await.is_err() {
        // The test server understands absolute-form requests, so this request
        // (and any later one on the same connection) is passed through as is.
        return;
    }
    let _ = tokio::io::copy_bidirectional(tcp, &mut up).await;
}

async fn read_vec(tcp: &mut TcpStream, n: usize) -> Option<Vec<u8>> {
    let mut v = vec![0u8; n];
    tcp.read_exact(&mut v).await.ok()?;
    Some(v)
}

async fn serve_socks(state: &ProxyState, tcp: &mut TcpStream) {
    let Some(hello) = read_vec(tcp, 2).await else {
        return;
    };
    if hello[0] != 5 {
        return;
    }
    let Some(methods) = read_vec(tcp, usize::from(hello[1])).await else {
        return;
    };
    if let Some((u, p)) = &state.auth {
        if !methods.contains(&2) {
            let _ = tcp.write_all(&[5, 0xff]).await;
            record(state, "socks5", "", false);
            return;
        }
        if tcp.write_all(&[5, 2]).await.is_err() {
            return;
        }
        let Some(v) = read_vec(tcp, 2).await else {
            return;
        };
        let Some(user) = read_vec(tcp, usize::from(v[1])).await else {
            return;
        };
        let Some(plen) = read_vec(tcp, 1).await else {
            return;
        };
        let Some(pass) = read_vec(tcp, usize::from(plen[0])).await else {
            return;
        };
        let ok = user == u.as_bytes() && pass == p.as_bytes();
        let _ = tcp.write_all(&[1, u8::from(!ok)]).await;
        if !ok {
            record(state, "socks5", "", false);
            return;
        }
    } else if tcp.write_all(&[5, 0]).await.is_err() {
        return;
    }
    let Some(req) = read_vec(tcp, 4).await else {
        return;
    };
    if req[1] != 1 {
        return;
    }
    let host = match req[3] {
        1 => match read_vec(tcp, 4).await {
            Some(a) => std::net::Ipv4Addr::new(a[0], a[1], a[2], a[3]).to_string(),
            None => return,
        },
        4 => match read_vec(tcp, 16).await {
            Some(a) => {
                let mut o = [0u8; 16];
                o.copy_from_slice(&a);
                format!("[{}]", std::net::Ipv6Addr::from(o))
            }
            None => return,
        },
        3 => {
            let Some(n) = read_vec(tcp, 1).await else {
                return;
            };
            match read_vec(tcp, usize::from(n[0])).await {
                Some(name) => String::from_utf8_lossy(&name).into_owned(),
                None => return,
            }
        }
        _ => return,
    };
    let Some(port) = read_vec(tcp, 2).await else {
        return;
    };
    let target = format!("{host}:{}", u16::from_be_bytes([port[0], port[1]]));
    record(state, "socks5", &target, true);
    // Resolve here, as a SOCKS5 proxy with remote DNS does. Prefer IPv4: the
    // test server listens on 127.0.0.1 only.
    let addrs: Vec<SocketAddr> = match tokio::net::lookup_host(target.as_str()).await {
        Ok(a) => a.collect(),
        Err(_) => Vec::new(),
    };
    let addr = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied();
    let up = match addr {
        Some(a) => TcpStream::connect(a).await.ok(),
        None => None,
    };
    let Some(mut up) = up else {
        let _ = tcp.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        return;
    };
    if tcp
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .is_err()
    {
        return;
    }
    let _ = tokio::io::copy_bidirectional(tcp, &mut up).await;
}
