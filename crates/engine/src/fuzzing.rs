//! Entry points for the fuzz targets in `fuzz/` (`docs/ENGINE.md`, "Testing
//! strategy"). Not part of the engine's API: compiled only for tests and with the
//! `fuzzing` feature, which only the fuzz crate enables.
//!
//! Each function takes arbitrary bytes, feeds them to one family of parsers for
//! untrusted input (response headers, `cookies.txt`, user-typed values) and
//! asserts the properties their callers rely on. A panic is a finding. The unit
//! tests below run every target over the checked-in seed corpus
//! (`fuzz/corpus/<target>/`) and over random and mutated inputs, so the targets
//! are exercised by a plain `cargo test` on every platform; the libFuzzer runs
//! happen in the nightly CI workflow.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use url::Url;

use crate::cookies::CookieJar;
use crate::filename::{self, NameSources};
use crate::headers::{self, ContentRange, Validators};
use crate::integrity::{self, Checksum};
use crate::probe;
use crate::proxy::{NoProxy, Proxy};
use crate::ratelimit;

/// The names of the targets, as used by `fuzz/` and its corpus directories.
pub const TARGETS: &[&str] = &[
    "content_range",
    "content_disposition",
    "retry_after",
    "digest_headers",
    "cookies",
    "probe_response",
    "user_input",
];

/// Runs the target called `name` on `data`. Returns `false` for an unknown name.
pub fn run(name: &str, data: &[u8]) -> bool {
    let f: fn(&[u8]) = match name {
        "content_range" => content_range,
        "content_disposition" => content_disposition,
        "retry_after" => retry_after,
        "digest_headers" => digest_headers,
        "cookies" => cookies,
        "probe_response" => probe_response,
        "user_input" => user_input,
        _ => return false,
    };
    f(data);
    true
}

/// A fixed "now" (2023-11-14), so a finding reproduces on any day.
fn fixed_now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// Splits `data` into header lines (`name: value`); lines that are not a valid
/// header are skipped, as hyper would reject them before the engine sees them.
fn header_map(data: &[u8]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for line in data.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        let value = value.trim_ascii();
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name), HeaderValue::from_bytes(value)) {
            map.append(n, v);
        }
    }
    map
}

/// `Content-Range` values.
pub fn content_range(data: &[u8]) {
    let s = String::from_utf8_lossy(data);
    let Some(parsed) = ContentRange::parse(&s) else {
        return;
    };
    let canonical = match parsed {
        ContentRange::Bytes { first, last, total } => {
            assert!(first <= last, "{s:?}: first > last");
            if let Some(t) = total {
                assert!(last < t, "{s:?}: last >= total");
            }
            match total {
                Some(t) => format!("bytes {first}-{last}/{t}"),
                None => format!("bytes {first}-{last}/*"),
            }
        }
        ContentRange::Unsatisfied { total } => format!("bytes */{total}"),
    };
    assert_eq!(
        ContentRange::parse(&canonical),
        Some(parsed),
        "{s:?} does not round-trip"
    );
}

/// The properties every name the engine creates a file under must have.
fn assert_safe_name(name: &str, from: &[u8]) {
    let ctx = || String::from_utf8_lossy(from).into_owned();
    assert!(!name.is_empty(), "{:?}: empty name", ctx());
    assert!(
        !name.chars().any(
            |c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
                || (c as u32) < 0x20
                || c == '\u{7f}'
        ),
        "{:?}: unsafe character in {name:?}",
        ctx()
    );
    assert!(
        !name.ends_with(['.', ' ']) && !name.starts_with(' '),
        "{:?}: leading or trailing dot or space in {name:?}",
        ctx()
    );
    assert!(
        name.encode_utf16().count() <= 255,
        "{:?}: name longer than 255 UTF-16 units",
        ctx()
    );
    assert_eq!(
        filename::sanitize(name),
        name,
        "{:?}: sanitize is not idempotent",
        ctx()
    );
}

/// `Content-Disposition` values, then filename resolution and sanitization.
pub fn content_disposition(data: &[u8]) {
    if let Some(name) = filename::parse_content_disposition(data) {
        assert!(
            !name.is_empty() && name.trim() == name,
            "{name:?} not trimmed"
        );
    }
    let text = String::from_utf8_lossy(data);
    let sanitized = filename::sanitize(&text);
    if !sanitized.is_empty() {
        assert_safe_name(&sanitized, data);
    }
    let url = Url::parse("https://example.com/files/")
        .ok()
        .and_then(|base| base.join(&text).ok());
    let resolved = filename::resolve(NameSources {
        user: None,
        content_disposition: Some(data),
        suggested: None,
        final_url: url.as_ref(),
        content_type: Some(text.as_ref()),
    });
    assert_safe_name(&resolved, data);
}

/// `Retry-After` values (delay-seconds or HTTP-date).
pub fn retry_after(data: &[u8]) {
    let s = String::from_utf8_lossy(data);
    let now = fixed_now();
    let parsed = headers::parse_retry_after(&s, now);
    if let Ok(secs) = s.trim().parse::<u64>() {
        assert_eq!(parsed, Some(Duration::from_secs(secs)));
    }
    let mut map = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_bytes(data) {
        map.insert(http::header::RETRY_AFTER, v);
        let _ = headers::retry_after_from(&map);
    }
}

/// Digest headers (`Repr-Digest`, `Content-Digest`, `Digest`, `Content-MD5`,
/// `x-goog-hash`, `x-amz-checksum-*`), given as header lines.
pub fn digest_headers(data: &[u8]) {
    let map = header_map(data);
    let whole = integrity::server_digests(&map, true);
    let partial = integrity::server_digests(&map, false);
    assert!(partial.len() <= whole.len());
    for (c, _) in whole.iter().chain(&partial) {
        assert_eq!(c.hex.len(), c.algo.digest_len() * 2);
        assert!(
            c.hex
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        );
    }
    assert!(
        whole.windows(2).all(|w| w[0].0.algo >= w[1].0.algo),
        "not sorted strongest first"
    );
}

/// `cookies.txt` files, and the same lines as `Set-Cookie` headers.
pub fn cookies(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let mut jar = CookieJar::from_netscape(&text).unwrap_or_default();
    let now = fixed_now();
    let urls = [
        "https://www.example.com/dir/file.iso",
        "http://example.com/",
        "https://127.0.0.1:8443/a/b",
    ]
    .map(|u| Url::parse(u).expect("valid test URL"));
    for line in text.lines() {
        for url in &urls {
            jar.set_cookie(url, line, now);
        }
    }
    for url in &urls {
        if let Some(v) = jar.header_for(url, now) {
            assert!(v.is_sensitive());
        }
    }
}

/// Probe responses: a status code (first two bytes) and header lines (the rest).
pub fn probe_response(data: &[u8]) {
    let Some((status, rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let code = 100 + u16::from_be_bytes(*status) % 500;
    let Ok(status) = StatusCode::from_u16(code) else {
        return;
    };
    let map = header_map(rest);
    let url = Url::parse("https://example.com/file.bin").expect("valid test URL");
    if let Ok(kind) = probe::interpret(status, &map, &url) {
        let _ = kind.total();
    }
    let v = Validators::from_headers(&map);
    assert!(
        v.mismatch(&v).is_none(),
        "validators differ from themselves"
    );
    let _ = v.if_range();
    let _ = headers::content_length(&map);
    let _ = headers::is_html(&map);
    let _ = headers::retry_after_from(&map);
}

/// Values a user types: checksums, rates, proxy URLs, `no_proxy` lists.
pub fn user_input(data: &[u8]) {
    let s = String::from_utf8_lossy(data);
    if let Ok(c) = Checksum::parse(&s) {
        assert_eq!(c.hex.len(), c.algo.digest_len() * 2);
        assert_eq!(Checksum::parse(&c.to_string()).as_ref(), Ok(&c));
    }
    if let Ok(Some(bps)) = ratelimit::parse_rate(&s) {
        assert!(bps > 0);
    }
    if let Ok(p) = Proxy::parse(&s)
        && p.has_credentials()
    {
        assert!(!p.without_credentials().contains('@'));
        assert!(p.redacted().contains("***@"));
    }
    let np = NoProxy::parse(&s);
    for host in ["example.com", "[::1]", "a.b.example.com.", ""] {
        let _ = np.matches(host);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::path::PathBuf;
    use std::sync::LazyLock;

    use proptest::prelude::*;
    use proptest::sample::Index;

    use super::*;

    fn corpus_dir(target: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/corpus")
            .join(target)
    }

    fn seeds(target: &str) -> Vec<Vec<u8>> {
        let dir = corpus_dir(target);
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        files.iter().map(|p| std::fs::read(p).unwrap()).collect()
    }

    /// Every target's seeds, in `TARGETS` order.
    static SEEDS: LazyLock<Vec<Vec<Vec<u8>>>> =
        LazyLock::new(|| TARGETS.iter().map(|t| seeds(t)).collect());

    #[test]
    fn every_target_has_seeds_and_a_fuzz_binary() {
        let fuzz = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz");
        let manifest = std::fs::read_to_string(fuzz.join("Cargo.toml")).unwrap();
        for t in TARGETS {
            assert!(!seeds(t).is_empty(), "no seeds for {t}");
            assert!(
                fuzz.join("fuzz_targets").join(format!("{t}.rs")).exists(),
                "no fuzz_targets/{t}.rs"
            );
            assert!(
                manifest.contains(&format!("name = \"{t}\"")),
                "{t} missing from fuzz/Cargo.toml"
            );
            assert!(run(t, b""));
        }
        assert!(!run("no_such_target", b""));
    }

    #[test]
    fn seed_corpus_passes() {
        for (t, all) in TARGETS.iter().zip(SEEDS.iter()) {
            for seed in all {
                run(t, seed);
            }
        }
    }

    fn cases() -> u32 {
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(256)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

        #[test]
        fn random_bytes(target in any::<Index>(), data in proptest::collection::vec(any::<u8>(), 0..512)) {
            run(target.get(TARGETS), &data);
        }

        #[test]
        fn mutated_seeds(
            target in any::<Index>(),
            seed in any::<Index>(),
            edits in proptest::collection::vec((any::<Index>(), any::<u8>(), 0u8..3), 1..8),
        ) {
            let t = *target.get(TARGETS);
            let all = &SEEDS[target.index(TARGETS.len())];
            let mut data = seed.get(all).clone();
            for (at, byte, op) in edits {
                let len = data.len();
                match op {
                    0 if len > 0 => data[at.index(len)] = byte,
                    1 => data.insert(at.index(len + 1), byte),
                    _ if len > 0 => { data.remove(at.index(len)); }
                    _ => data.push(byte),
                }
            }
            run(t, &data);
        }
    }
}
