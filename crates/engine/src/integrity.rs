//! Checksum verification (`docs/ENGINE.md`, "Integrity").
//!
//! - [`Checksum`]: an expected digest, given by the user or discovered in the
//!   server's response headers (`Repr-Digest`, `Content-Digest`, `Digest`,
//!   `Content-MD5`, `x-goog-hash`, `x-amz-checksum-sha256`).
//! - The hasher thread computes the digest **in file order while the download
//!   runs**: the writer forwards every chunk it wrote; a chunk at the hash cursor
//!   is hashed straight from memory, and ranges that were written ahead of the
//!   cursor (other segments) are read back from the file (normally the page
//!   cache) once the cursor reaches them. Every byte is hashed exactly once and
//!   the file is not read again after the download, except for the parts a
//!   resumed download already had on disk.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

use bytes::Bytes;
use http::HeaderMap;
use sha2::Digest as _;
use tokio::sync::oneshot;

use crate::error::{Error, Result};
use crate::platform;

/// A hash algorithm for [`Checksum`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgo {
    /// MD5 (weak; only detects accidents).
    Md5,
    /// SHA-1 (weak; only detects accidents).
    Sha1,
    /// SHA-256.
    Sha256,
    /// SHA-512.
    Sha512,
}

impl HashAlgo {
    /// Name used in the database, the CLI and JSON (`md5`, `sha1`, `sha256`, `sha512`).
    pub fn as_str(self) -> &'static str {
        match self {
            HashAlgo::Md5 => "md5",
            HashAlgo::Sha1 => "sha1",
            HashAlgo::Sha256 => "sha256",
            HashAlgo::Sha512 => "sha512",
        }
    }

    /// Parses a name: `sha256`, `sha-256`, `SHA256`, ... (`sha` alone is SHA-1,
    /// as in RFC 3230).
    pub fn parse(s: &str) -> Option<Self> {
        let n: String = s
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_lowercase();
        Some(match n.as_str() {
            "md5" => HashAlgo::Md5,
            "sha" | "sha1" => HashAlgo::Sha1,
            "sha256" => HashAlgo::Sha256,
            "sha512" => HashAlgo::Sha512,
            _ => return None,
        })
    }

    /// Digest length in bytes.
    pub fn digest_len(self) -> usize {
        match self {
            HashAlgo::Md5 => 16,
            HashAlgo::Sha1 => 20,
            HashAlgo::Sha256 => 32,
            HashAlgo::Sha512 => 64,
        }
    }

    fn from_hex_len(n: usize) -> Option<Self> {
        [
            HashAlgo::Md5,
            HashAlgo::Sha1,
            HashAlgo::Sha256,
            HashAlgo::Sha512,
        ]
        .into_iter()
        .find(|a| a.digest_len() * 2 == n)
    }
}

impl fmt::Display for HashAlgo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An expected digest of the whole file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Checksum {
    /// Algorithm.
    pub algo: HashAlgo,
    /// Lowercase hex digest.
    pub hex: String,
}

impl Checksum {
    /// Builds a checksum from an algorithm and a hex digest (any case).
    pub fn new(algo: HashAlgo, hex: &str) -> Result<Self, String> {
        let hex = hex.trim().to_ascii_lowercase();
        if hex.len() != algo.digest_len() * 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "a {algo} checksum is {} hex digits, got {:?}",
                algo.digest_len() * 2,
                hex
            ));
        }
        Ok(Self { algo, hex })
    }

    /// Parses `algo:hex` (`sha256:ab12...`, also `sha-256=...`) or bare hex,
    /// whose algorithm is inferred from its length (32 MD5, 40 SHA-1, 64 SHA-256,
    /// 128 SHA-512).
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if let Some((algo, hex)) = s.split_once([':', '=']) {
            let a = HashAlgo::parse(algo).ok_or_else(|| {
                format!("unknown checksum algorithm {algo:?} (use md5, sha1, sha256 or sha512)")
            })?;
            return Self::new(a, hex);
        }
        let a = HashAlgo::from_hex_len(s.len()).ok_or_else(|| {
            format!(
                "cannot tell the algorithm of a {}-digit checksum; write it as sha256:<hex>",
                s.len()
            )
        })?;
        Self::new(a, s)
    }

    fn from_bytes(algo: HashAlgo, bytes: &[u8]) -> Option<Self> {
        (bytes.len() == algo.digest_len()).then(|| Self {
            algo,
            hex: to_hex(bytes),
        })
    }
}

impl fmt::Display for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algo, self.hex)
    }
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Decodes standard or URL-safe base64, with or without padding.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim().bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Where an expected checksum came from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumSource {
    /// Given by the user.
    User,
    /// Sent by the server in this header.
    Server(String),
}

impl fmt::Display for ChecksumSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChecksumSource::User => f.write_str("given by the user"),
            ChecksumSource::Server(h) => write!(f, "from the server's {h} header"),
        }
    }
}

impl ChecksumSource {
    pub(crate) fn as_db(&self) -> String {
        match self {
            ChecksumSource::User => "user".into(),
            ChecksumSource::Server(h) => h.clone(),
        }
    }

    pub(crate) fn from_db(s: &str) -> Self {
        if s == "user" {
            ChecksumSource::User
        } else {
            ChecksumSource::Server(s.to_owned())
        }
    }
}

/// Digests of the whole file advertised in response headers, strongest first.
///
/// `whole_body` says whether this response's body is the entire file (a `200`,
/// or a `206` for `0-(n-1)/n`): only then do `Content-Digest` and `Content-MD5`,
/// which describe the body, also describe the file. `Repr-Digest`, `Digest`
/// (RFC 3230 instance digests), `x-goog-hash` and `x-amz-checksum-sha256` always
/// describe the whole representation. S3 composite checksums (`...-N`) and
/// `ETag`s are never used.
pub(crate) fn server_digests(headers: &HeaderMap, whole_body: bool) -> Vec<(Checksum, String)> {
    let mut found: Vec<(Checksum, String)> = Vec::new();
    let all = |name: &str| -> Vec<String> {
        headers
            .get_all(name)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    };
    // RFC 9530 dictionaries: `sha-256=:<base64>:`.
    let mut structured = |name: &str| {
        for item in all(name) {
            let Some((k, v)) = item.split_once('=') else {
                continue;
            };
            let v = v.trim();
            let Some(b64) = v.strip_prefix(':').and_then(|v| v.strip_suffix(':')) else {
                continue;
            };
            if let (Some(a), Some(bytes)) = (HashAlgo::parse(k.trim()), decode_base64(b64))
                && let Some(c) = Checksum::from_bytes(a, &bytes)
            {
                found.push((c, name.to_owned()));
            }
        }
    };
    structured("repr-digest");
    if whole_body {
        structured("content-digest");
    }
    // RFC 3230: `SHA-256=<base64>`, `MD5=<base64>`.
    for item in all("digest") {
        if let Some((k, v)) = item.split_once('=')
            && let (Some(a), Some(bytes)) = (HashAlgo::parse(k.trim()), decode_base64(v))
            && let Some(c) = Checksum::from_bytes(a, &bytes)
        {
            found.push((c, "digest".into()));
        }
    }
    if whole_body
        && let Some(v) = headers.get("content-md5").and_then(|v| v.to_str().ok())
        && let Some(c) = decode_base64(v).and_then(|b| Checksum::from_bytes(HashAlgo::Md5, &b))
    {
        found.push((c, "content-md5".into()));
    }
    // Google Cloud Storage: `x-goog-hash: crc32c=..., md5=...` (whole object).
    for item in all("x-goog-hash") {
        if let Some(v) = item.strip_prefix("md5=")
            && let Some(c) = decode_base64(v).and_then(|b| Checksum::from_bytes(HashAlgo::Md5, &b))
        {
            found.push((c, "x-goog-hash".into()));
        }
    }
    // Amazon S3: only full-object checksums (composite ones end in `-<parts>`).
    let full_object = headers
        .get("x-amz-checksum-type")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|t| t.eq_ignore_ascii_case("FULL_OBJECT"));
    if full_object
        && let Some(v) = headers
            .get("x-amz-checksum-sha256")
            .and_then(|v| v.to_str().ok())
        && !v.contains('-')
        && let Some(c) = decode_base64(v).and_then(|b| Checksum::from_bytes(HashAlgo::Sha256, &b))
    {
        found.push((c, "x-amz-checksum-sha256".into()));
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.0.algo));
    found
}

/// A running hash of one of the supported algorithms.
enum Hasher {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl Hasher {
    fn new(algo: HashAlgo) -> Self {
        match algo {
            HashAlgo::Md5 => Hasher::Md5(md5::Md5::new()),
            HashAlgo::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            HashAlgo::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            HashAlgo::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Hasher::Md5(h) => to_hex(&h.finalize()),
            Hasher::Sha1(h) => to_hex(&h.finalize()),
            Hasher::Sha256(h) => to_hex(&h.finalize()),
            Hasher::Sha512(h) => to_hex(&h.finalize()),
        }
    }
}

/// The result of the in-order hash.
#[derive(Debug)]
pub(crate) struct Hashed {
    /// Bytes hashed (the contiguous prefix of the file).
    pub(crate) bytes: u64,
    /// Lowercase hex digest of those bytes.
    pub(crate) hex: String,
}

/// The writer's end of the hasher: every written chunk, in each segment's order.
pub(crate) type HashFeed = SyncSender<(u64, Bytes)>;

/// Starts the hasher thread. `on_disk` lists ranges `[start, end)` that are
/// already in the file (a resumed download's checkpoint); they are read back
/// when the hash cursor reaches them. The thread ends, and `done` resolves,
/// once every [`HashFeed`] is dropped.
pub(crate) fn spawn_hasher(
    algo: HashAlgo,
    file: File,
    path: PathBuf,
    on_disk: &[(u64, u64)],
) -> Result<(HashFeed, oneshot::Receiver<Result<Hashed>>)> {
    // About 64 MiB of chunks in flight at most; a slower hasher slows the writer.
    let (tx, rx) = sync_channel(64);
    let (done_tx, done_rx) = oneshot::channel();
    let mut ahead = BTreeMap::new();
    for &(s, e) in on_disk {
        if e > s {
            insert_range(&mut ahead, s, e);
        }
    }
    thread::Builder::new()
        .name("boltget-hash".into())
        .spawn(move || {
            let r = run_hasher(algo, &file, &path, &rx, ahead, &done_tx);
            let _ = done_tx.send(r);
        })
        .map_err(|e| Error::Internal(format!("spawning hasher thread: {e}")))?;
    Ok((tx, done_rx))
}

/// Adds `[s, e)` to a set of disjoint ranges, merging neighbours.
fn insert_range(set: &mut BTreeMap<u64, u64>, mut s: u64, mut e: u64) {
    // Merge with a range that starts at or before `s` and reaches it.
    if let Some((&ps, &pe)) = set.range(..=s).next_back()
        && pe >= s
    {
        s = ps;
        e = e.max(pe);
        set.remove(&ps);
    }
    // Merge with ranges starting inside `[s, e]`.
    let inside: Vec<(u64, u64)> = set.range(s..=e).map(|(a, b)| (*a, *b)).collect();
    for (a, b) in inside {
        e = e.max(b);
        set.remove(&a);
    }
    set.insert(s, e);
}

fn run_hasher(
    algo: HashAlgo,
    file: &File,
    path: &PathBuf,
    rx: &Receiver<(u64, Bytes)>,
    mut ahead: BTreeMap<u64, u64>,
    result: &oneshot::Sender<Result<Hashed>>,
) -> Result<Hashed> {
    const READ: usize = 1024 * 1024;
    let mut hasher = Hasher::new(algo);
    let mut cursor = 0u64;
    let mut buf = vec![0u8; READ];
    // Hashes ranges written ahead of the cursor once it reaches them.
    let mut catch_up =
        |hasher: &mut Hasher, cursor: &mut u64, ahead: &mut BTreeMap<u64, u64>| -> Result<()> {
            while let Some((&s, &e)) = ahead.first_key_value() {
                if s > *cursor {
                    break;
                }
                ahead.remove(&s);
                while *cursor < e {
                    if result.is_closed() {
                        // Nobody waits for the result (the download stopped): do
                        // not read the rest of the file for nothing.
                        return Err(Error::Cancelled);
                    }
                    let n = ((e - *cursor) as usize).min(READ);
                    platform::read_exact_at(file, &mut buf[..n], *cursor)
                        .map_err(|err| Error::disk(path, err))?;
                    hasher.update(&buf[..n]);
                    *cursor += n as u64;
                }
            }
            Ok(())
        };
    catch_up(&mut hasher, &mut cursor, &mut ahead)?;
    while let Ok((offset, bytes)) = rx.recv() {
        let len = bytes.len() as u64;
        if offset < cursor {
            // Only a download that cannot resume rewrites bytes: it restarted
            // from zero, so the hash starts over too.
            hasher = Hasher::new(algo);
            cursor = 0;
            ahead.clear();
        }
        if offset == cursor {
            hasher.update(&bytes);
            cursor += len;
            // Anything written ahead that this chunk overlapped is hashed.
            while let Some((&s, &e)) = ahead.first_key_value()
                && s < cursor
            {
                ahead.remove(&s);
                if e > cursor {
                    ahead.insert(cursor, e);
                }
            }
            catch_up(&mut hasher, &mut cursor, &mut ahead)?;
        } else {
            insert_range(&mut ahead, offset, offset + len);
        }
    }
    catch_up(&mut hasher, &mut cursor, &mut ahead)?;
    Ok(Hashed {
        bytes: cursor,
        hex: hasher.finish(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn sha256_hex(data: &[u8]) -> String {
        to_hex(&sha2::Sha256::digest(data))
    }

    #[test]
    fn checksum_parsing() {
        let h = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
        let c = Checksum::parse(h).unwrap();
        assert_eq!(c.algo, HashAlgo::Sha256);
        assert_eq!(c.hex, h.to_ascii_lowercase());
        assert_eq!(Checksum::parse(&format!("sha256:{h}")).unwrap(), c);
        assert_eq!(Checksum::parse(&format!("SHA-256={h}")).unwrap(), c);
        assert_eq!(
            Checksum::parse("d41d8cd98f00b204e9800998ecf8427e")
                .unwrap()
                .algo,
            HashAlgo::Md5
        );
        assert_eq!(
            Checksum::parse("da39a3ee5e6b4b0d3255bfef95601890afd80709")
                .unwrap()
                .algo,
            HashAlgo::Sha1
        );
        assert!(Checksum::parse("abc").is_err());
        assert!(Checksum::parse("sha256:abc").is_err());
        assert!(Checksum::parse("crc32:12345678").is_err());
        assert!(Checksum::parse(&format!("md5:{h}")).is_err());
        assert!(Checksum::parse(&"zz".repeat(32)).is_err());
        assert_eq!(c.to_string(), format!("sha256:{}", c.hex));
    }

    #[test]
    fn base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode_base64("-_8").unwrap(), vec![0xfb, 0xff]);
        assert!(decode_base64("a b").is_none());
    }

    #[test]
    fn server_digest_headers() {
        // sha256("hello") and md5("hello").
        let sha = "LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
        let md5 = "XUFAKrxLKna5cZ2REBfFkg==";
        let mut h = HeaderMap::new();
        h.insert(
            "repr-digest",
            HeaderValue::from_str(&format!("sha-256=:{sha}:")).unwrap(),
        );
        h.insert("content-md5", HeaderValue::from_static(md5));
        let all = server_digests(&h, true);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0.algo, HashAlgo::Sha256);
        assert_eq!(all[0].0.hex, sha256_hex(b"hello"));
        assert_eq!(all[0].1, "repr-digest");
        assert_eq!(all[1].0.hex, "5d41402abc4b2a76b9719d911017c592");
        // Body digests do not describe the file when the body is a part of it.
        let partial = server_digests(&h, false);
        assert_eq!(partial.len(), 1);

        let mut h = HeaderMap::new();
        h.insert(
            "content-digest",
            HeaderValue::from_str(&format!("sha-256=:{sha}:")).unwrap(),
        );
        assert_eq!(server_digests(&h, false).len(), 0);
        assert_eq!(server_digests(&h, true).len(), 1);

        let mut h = HeaderMap::new();
        h.insert(
            "digest",
            HeaderValue::from_str(&format!("SHA-256={sha}, MD5={md5}")).unwrap(),
        );
        assert_eq!(server_digests(&h, false).len(), 2);

        let mut h = HeaderMap::new();
        h.insert(
            "x-goog-hash",
            HeaderValue::from_str(&format!("crc32c=n03x6A==, md5={md5}")).unwrap(),
        );
        assert_eq!(server_digests(&h, false)[0].0.algo, HashAlgo::Md5);

        let mut h = HeaderMap::new();
        h.insert("x-amz-checksum-sha256", HeaderValue::from_static(sha));
        assert_eq!(server_digests(&h, false).len(), 1);
        h.insert("x-amz-checksum-type", HeaderValue::from_static("COMPOSITE"));
        assert_eq!(server_digests(&h, false).len(), 0);
        let mut h = HeaderMap::new();
        h.insert("x-amz-checksum-sha256", HeaderValue::from_static("abc=-3"));
        assert_eq!(server_digests(&h, false).len(), 0);
        // Garbage and wrong lengths are ignored.
        let mut h = HeaderMap::new();
        h.insert(
            "repr-digest",
            HeaderValue::from_static("sha-256=:AAAA:, foo=:bar:"),
        );
        assert_eq!(server_digests(&h, true).len(), 0);
    }

    #[test]
    fn range_merging() {
        let mut s = BTreeMap::new();
        insert_range(&mut s, 10, 20);
        insert_range(&mut s, 30, 40);
        insert_range(&mut s, 20, 30);
        assert_eq!(s.into_iter().collect::<Vec<_>>(), vec![(10, 40)]);
        let mut s = BTreeMap::new();
        insert_range(&mut s, 30, 40);
        insert_range(&mut s, 5, 8);
        insert_range(&mut s, 7, 35);
        assert_eq!(s.into_iter().collect::<Vec<_>>(), vec![(5, 40)]);
    }

    fn file_with(data: &[u8]) -> (tempfile::TempDir, PathBuf, File) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, data).unwrap();
        let f = File::open(&p).unwrap();
        (dir, p, f)
    }

    #[tokio::test]
    async fn hashes_out_of_order_chunks_in_file_order() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let (_d, p, f) = file_with(&data);
        let (tx, done) = spawn_hasher(HashAlgo::Sha256, f, p, &[]).unwrap();
        let chunk = |a: usize, b: usize| (a as u64, Bytes::copy_from_slice(&data[a..b]));
        // Two "segments" interleaved: [0, 100k) and [100k, 300k).
        tx.send(chunk(100_000, 150_000)).unwrap();
        tx.send(chunk(0, 60_000)).unwrap();
        tx.send(chunk(150_000, 300_000)).unwrap();
        tx.send(chunk(60_000, 100_000)).unwrap();
        drop(tx);
        let h = done.await.unwrap().unwrap();
        assert_eq!(h.bytes, 300_000);
        assert_eq!(h.hex, sha256_hex(&data));
    }

    #[tokio::test]
    async fn resumed_ranges_are_read_back_and_restarts_reset() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let (_d, p, f) = file_with(&data);
        // The checkpoint had [0, 50k) and [100k, 120k) on disk.
        let (tx, done) =
            spawn_hasher(HashAlgo::Md5, f, p, &[(0, 50_000), (100_000, 120_000)]).unwrap();
        let chunk = |a: usize, b: usize| (a as u64, Bytes::copy_from_slice(&data[a..b]));
        tx.send(chunk(120_000, 200_000)).unwrap();
        tx.send(chunk(50_000, 100_000)).unwrap();
        drop(tx);
        let h = done.await.unwrap().unwrap();
        assert_eq!(h.bytes, 200_000);
        assert_eq!(h.hex, to_hex(&md5::Md5::digest(&data)));

        // A restart from zero discards the old hash state.
        let (_d, p, f) = file_with(&data);
        let (tx, done) = spawn_hasher(HashAlgo::Sha1, f, p, &[]).unwrap();
        tx.send(chunk(0, 10_000)).unwrap();
        tx.send(chunk(0, 200_000)).unwrap();
        drop(tx);
        let h = done.await.unwrap().unwrap();
        assert_eq!(h.hex, to_hex(&sha1::Sha1::digest(&data)));
    }
}
