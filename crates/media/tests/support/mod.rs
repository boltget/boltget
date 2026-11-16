//! Test support: a tiny HTTP server for fake releases, an OpenPGP signer
//! written independently of `boltget_media::pgp`, and zip building.
#![allow(dead_code, unreachable_pub, clippy::unwrap_used)]

use std::collections::HashMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use boltget_media::Program;
use boltget_media::pgp::PinnedKey;
use boltget_media::sources::{AssetSpec, Layout, PinnedBuild, Sources};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA512, RsaKeyPair, RsaPublicKeyComponents};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// What the server answers for a path.
#[derive(Debug, Clone)]
pub enum Route {
    Body(Vec<u8>),
    Redirect(String),
}

/// A minimal HTTP/1.1 server: one request per connection, exact paths.
#[derive(Debug, Clone)]
pub struct Server {
    addr: SocketAddr,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Server {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Server {
            addr,
            routes: Arc::default(),
            hits: Arc::default(),
        };
        let s = server.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let s = s.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 65536 {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&buf);
                    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    s.hits.lock().unwrap().push(path.clone());
                    let route = s.routes.lock().unwrap().get(&path).cloned();
                    let (status, extra, body) = match route {
                        Some(Route::Body(b)) => ("200 OK", String::new(), b),
                        Some(Route::Redirect(to)) => {
                            ("302 Found", format!("Location: {to}\r\n"), Vec::new())
                        }
                        None => ("404 Not Found", String::new(), b"not found".to_vec()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n{extra}Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        server
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn set(&self, path: &str, route: Route) {
        self.routes.lock().unwrap().insert(path.to_owned(), route);
    }

    pub fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}

/// The RSA test key in `tests/fixtures` (test use only).
pub struct TestKey {
    pair: RsaKeyPair,
    created: u32,
}

impl TestKey {
    pub fn load() -> Self {
        let der = std::fs::read(fixture("test-only-rsa2048.der")).unwrap();
        TestKey {
            pair: RsaKeyPair::from_der(&der).unwrap(),
            created: 1_700_000_000,
        }
    }

    pub fn pinned(&self) -> PinnedKey {
        let public: RsaPublicKeyComponents<Vec<u8>> = self.pair.public().into();
        PinnedKey {
            created: self.created,
            n: public.n,
            e: public.e,
        }
    }

    /// A binary OpenPGP v4 detached signature (SHA-512) over `data`.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let fpr = self.pinned().fingerprint();
        let mut sub = vec![5, 2];
        sub.extend_from_slice(&(self.created + 10).to_be_bytes());
        sub.extend_from_slice(&[22, 33, 4]);
        sub.extend_from_slice(&fpr);
        let mut hashed = vec![4, 0x00, 1, 10];
        hashed.extend_from_slice(&(sub.len() as u16).to_be_bytes());
        hashed.extend_from_slice(&sub);
        let mut message = data.to_vec();
        message.extend_from_slice(&hashed);
        message.extend_from_slice(&[4, 0xFF]);
        message.extend_from_slice(&(hashed.len() as u32).to_be_bytes());
        let mut sig = vec![0u8; self.pair.public().modulus_len()];
        self.pair
            .sign(&RSA_PKCS1_SHA512, &SystemRandom::new(), &message, &mut sig)
            .unwrap();
        let mut body = hashed;
        body.extend_from_slice(&[0, 10, 9, 16]);
        body.extend_from_slice(&fpr[12..]);
        let digest = ring::digest::digest(&ring::digest::SHA512, &message);
        body.extend_from_slice(&digest.as_ref()[..2]);
        let start = sig.iter().position(|b| *b != 0).unwrap();
        let sig = &sig[start..];
        let bits = (sig.len() as u16 - 1) * 8 + (8 - sig[0].leading_zeros() as u16);
        body.extend_from_slice(&bits.to_be_bytes());
        body.extend_from_slice(sig);
        let mut packet = vec![0xC2];
        let len = body.len();
        assert!((192..8384).contains(&len));
        packet.push((((len - 192) >> 8) + 192) as u8);
        packet.push(((len - 192) & 0xFF) as u8);
        packet.extend_from_slice(&body);
        packet
    }
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The fake sidecar's bytes.
pub fn fake_exe() -> Vec<u8> {
    std::fs::read(env!("CARGO_BIN_EXE_boltget-fake-sidecar")).unwrap()
}

pub fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// A zip (stored, no compression) of `(name, bytes)` entries; names ending in
/// `/` are directories.
pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);
    for (name, bytes) in entries {
        if name.ends_with('/') {
            w.add_directory(*name, opts).unwrap();
        } else {
            w.start_file(*name, opts).unwrap();
            w.write_all(bytes).unwrap();
        }
    }
    w.finish().unwrap().into_inner()
}

/// The asset name the fake yt-dlp release uses.
pub const YT_ASSET: &str = "yt-dlp_test.zip";

/// Sources pointing at `server`, with the test key and a fake FFmpeg pin.
pub fn sources(server: &Server, key: &TestKey, ffmpeg: Option<PinnedBuild>) -> Sources {
    Sources {
        yt_dlp_stable: server.url("/yt/stable/latest"),
        yt_dlp_nightly: server.url("/yt/nightly/latest"),
        yt_dlp_asset: Some(AssetSpec {
            name: YT_ASSET.into(),
            layout: Layout::Zip {
                strip: 0,
                keep: None,
            },
            programs: vec![(Program::YtDlp, exe("yt-dlp"))],
        }),
        yt_dlp_key: key.pinned(),
        ffmpeg,
        allowed_hosts: vec!["127.0.0.1".into()],
        require_https: false,
    }
}

/// Options for a fake yt-dlp release.
#[derive(Default)]
pub struct Fake {
    /// Extra files in the zip (`fake-fail`, `fake-version.txt`, ...).
    pub extra: Vec<(&'static str, Vec<u8>)>,
    /// List this hash instead of the real one.
    pub wrong_hash: bool,
    /// Change the checksum file after signing it.
    pub tamper_sums: bool,
    /// Point the asset at this URL instead of the server.
    pub asset_url: Option<String>,
}

/// Publishes a fake yt-dlp release `version` as the latest on `channel`.
pub fn publish_yt_dlp(server: &Server, key: &TestKey, channel: &str, version: &str, fake: &Fake) {
    let exe_bytes = fake_exe();
    let yt = exe("yt-dlp");
    let mut entries: Vec<(&str, &[u8])> = vec![(yt.as_str(), &exe_bytes), ("_internal/", b"")];
    entries.push(("_internal/base_library.zip", b"python"));
    for (n, b) in &fake.extra {
        entries.push((n, b));
    }
    let zip = zip(&entries);
    let hash = if fake.wrong_hash {
        sha256_hex(b"something else")
    } else {
        sha256_hex(&zip)
    };
    let mut sums = format!("{}  yt-dlp\n{hash}  {YT_ASSET}\n", sha256_hex(b"other"));
    let sig = key.sign(sums.as_bytes());
    if fake.tamper_sums {
        sums = format!("{sums}{}  extra\n", sha256_hex(b"x"));
    }
    let base = format!("/yt/{channel}/download/{version}");
    server.set(&format!("{base}/{YT_ASSET}"), Route::Body(zip.clone()));
    server.set(
        &format!("{base}/SHA2-256SUMS"),
        Route::Body(sums.into_bytes()),
    );
    server.set(&format!("{base}/SHA2-256SUMS.sig"), Route::Body(sig));
    let asset_url = fake
        .asset_url
        .clone()
        .unwrap_or_else(|| server.url(&format!("{base}/{YT_ASSET}")));
    let json = serde_json::json!({
        "tag_name": version,
        "assets": [
            {"name": "SHA2-256SUMS", "size": 100, "browser_download_url": server.url(&format!("{base}/SHA2-256SUMS"))},
            {"name": "SHA2-256SUMS.sig", "size": 100, "browser_download_url": server.url(&format!("{base}/SHA2-256SUMS.sig"))},
            {"name": YT_ASSET, "size": zip.len(), "browser_download_url": asset_url},
        ]
    });
    server.set(
        &format!("/yt/{channel}/latest"),
        Route::Body(serde_json::to_vec(&json).unwrap()),
    );
}

/// A fake FFmpeg build served at `/ff/<version>.zip`, and its pin.
pub fn publish_ffmpeg(server: &Server, version: &str, extra: &[(&str, &[u8])]) -> PinnedBuild {
    let exe_bytes = fake_exe();
    let ff = format!("ffmpeg-{version}/bin/{}", exe("ffmpeg"));
    let fp = format!("ffmpeg-{version}/bin/{}", exe("ffprobe"));
    let top = format!("ffmpeg-{version}/");
    let bin = format!("ffmpeg-{version}/bin/");
    let doc = format!("ffmpeg-{version}/doc/readme.txt");
    let mut entries: Vec<(&str, &[u8])> = vec![
        (&top, b""),
        (&bin, b""),
        (&ff, &exe_bytes),
        (&fp, &exe_bytes),
        (&doc, b"docs"),
    ];
    let extra: Vec<(String, &[u8])> = extra
        .iter()
        .map(|(n, b)| (format!("ffmpeg-{version}/bin/{n}"), *b))
        .collect();
    for (n, b) in &extra {
        entries.push((n.as_str(), b));
    }
    let zip = zip(&entries);
    let path = format!("/ff/{version}.zip");
    server.set(&path, Route::Body(zip.clone()));
    PinnedBuild {
        version: version.into(),
        url: server.url(&path),
        size: zip.len() as u64,
        sha256: sha256_hex(&zip),
        layout: Layout::Zip {
            strip: 1,
            keep: Some("bin/".into()),
        },
        programs: vec![
            (Program::Ffmpeg, format!("bin/{}", exe("ffmpeg"))),
            (Program::Ffprobe, format!("bin/{}", exe("ffprobe"))),
        ],
    }
}
