//! Verification of OpenPGP detached signatures against one pinned RSA key.
//!
//! yt-dlp signs its checksum files (`SHA2-256SUMS.sig`) with a GnuPG key. This
//! is the smallest strict subset of RFC 4880 that verifies them: one binary
//! version 4 signature packet over a binary document (signature type `0x00`),
//! made with RSA (PKCS #1 v1.5) and SHA-256, SHA-384 or SHA-512, by the pinned
//! key. Anything else (other packet types, trailing data, unknown critical
//! subpackets, an expired signature, a signature by another key) is refused.
//! The RSA arithmetic is `ring`'s; this module only parses and assembles the
//! signed bytes. See `docs/VIDEO.md`, "Verification".

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use ring::signature::{self, RsaPublicKeyComponents};

/// A pinned OpenPGP version 4 RSA public key.
#[derive(Clone, PartialEq, Eq)]
pub struct PinnedKey {
    /// The key's creation time (seconds since the Unix epoch), part of its
    /// fingerprint.
    pub created: u32,
    /// The RSA modulus, big-endian without leading zeros.
    pub n: Vec<u8>,
    /// The RSA public exponent, big-endian without leading zeros.
    pub e: Vec<u8>,
}

impl fmt::Debug for PinnedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinnedKey")
            .field("fingerprint", &self.fingerprint_hex())
            .finish_non_exhaustive()
    }
}

impl PinnedKey {
    /// The version 4 fingerprint: SHA-1 over the public-key packet body.
    pub fn fingerprint(&self) -> [u8; 20] {
        let mut body = vec![4u8];
        body.extend_from_slice(&self.created.to_be_bytes());
        body.push(1); // RSA (encrypt or sign)
        push_mpi(&mut body, &self.n);
        push_mpi(&mut body, &self.e);
        let mut framed = vec![0x99];
        framed.extend_from_slice(&(body.len() as u16).to_be_bytes());
        framed.extend_from_slice(&body);
        let d = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &framed);
        let mut out = [0u8; 20];
        out.copy_from_slice(d.as_ref());
        out
    }

    /// The fingerprint as upper-case hex, as GnuPG prints it.
    pub fn fingerprint_hex(&self) -> String {
        self.fingerprint().iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02X}");
            s
        })
    }
}

fn push_mpi(out: &mut Vec<u8>, v: &[u8]) {
    let v = strip_zeros(v);
    let bits = match v.first() {
        Some(b) => (v.len() as u32 - 1) * 8 + (8 - b.leading_zeros()),
        None => 0,
    };
    out.extend_from_slice(&(bits as u16).to_be_bytes());
    out.extend_from_slice(v);
}

fn strip_zeros(v: &[u8]) -> &[u8] {
    let start = v.iter().position(|b| *b != 0).unwrap_or(v.len());
    &v[start..]
}

/// Verifies `sig` (a binary detached signature) over `data` with `key`.
/// `now` bounds a signature expiration time, if the signature has one.
pub fn verify_detached(
    key: &PinnedKey,
    data: &[u8],
    sig: &[u8],
    now: SystemTime,
) -> Result<(), String> {
    let body = single_packet(sig, 2)?;
    let parsed = parse_signature(body)?;
    let fpr = key.fingerprint();
    if let Some(issuer) = parsed.issuer_fpr
        && issuer != fpr
    {
        return Err("made by another key (issuer fingerprint)".into());
    }
    if let Some(id) = parsed.issuer_id
        && id[..] != fpr[12..]
    {
        return Err("made by another key (issuer key ID)".into());
    }
    if let (Some(created), Some(expires)) = (parsed.created, parsed.expires)
        && expires != 0
    {
        let now = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        if now > u64::from(created) + u64::from(expires) {
            return Err("the signature has expired".into());
        }
    }
    let (params, digest) = match parsed.hash_algo {
        8 => (
            &signature::RSA_PKCS1_2048_8192_SHA256,
            &ring::digest::SHA256,
        ),
        9 => (
            &signature::RSA_PKCS1_2048_8192_SHA384,
            &ring::digest::SHA384,
        ),
        10 => (
            &signature::RSA_PKCS1_2048_8192_SHA512,
            &ring::digest::SHA512,
        ),
        other => return Err(format!("unsupported hash algorithm {other}")),
    };
    let n = strip_zeros(&key.n);
    let mpi = strip_zeros(parsed.mpi);
    if mpi.len() > n.len() {
        return Err("signature longer than the key".into());
    }
    let mut padded = vec![0u8; n.len() - mpi.len()];
    padded.extend_from_slice(mpi);
    // What a version 4 signature covers: the document, the hashed part of the
    // signature packet, then the trailer 0x04 0xFF and that part's length.
    let mut message = Vec::with_capacity(data.len() + parsed.hashed.len() + 6);
    message.extend_from_slice(data);
    message.extend_from_slice(parsed.hashed);
    message.extend_from_slice(&[4, 0xFF]);
    message.extend_from_slice(&(parsed.hashed.len() as u32).to_be_bytes());
    // The quick check: the digest's first two bytes, stored in the packet.
    if ring::digest::digest(digest, &message).as_ref()[..2] != parsed.left16 {
        return Err("bad signature (digest prefix)".into());
    }
    let components = RsaPublicKeyComponents {
        n,
        e: strip_zeros(&key.e),
    };
    components
        .verify(params, &message, &padded)
        .map_err(|_| "bad signature".to_owned())
}

/// The body of the only packet in `input`, which must have tag `tag`.
fn single_packet(input: &[u8], tag: u8) -> Result<&[u8], String> {
    let (&first, rest) = input.split_first().ok_or("empty signature")?;
    if first & 0x80 == 0 {
        return Err("not a binary OpenPGP packet (armored signatures are not accepted)".into());
    }
    let (found, len, rest) = if first & 0x40 == 0 {
        // Old format: tag in bits 5-2, length type in bits 1-0.
        let found = (first >> 2) & 0x0F;
        let (len, rest) = match first & 3 {
            0 => take_be(rest, 1)?,
            1 => take_be(rest, 2)?,
            2 => take_be(rest, 4)?,
            _ => return Err("indeterminate packet length".into()),
        };
        (found, len, rest)
    } else {
        let found = first & 0x3F;
        let (&o1, rest) = rest.split_first().ok_or("truncated packet length")?;
        match o1 {
            0..=191 => (found, usize::from(o1), rest),
            192..=223 => {
                let (&o2, rest) = rest.split_first().ok_or("truncated packet length")?;
                (
                    found,
                    ((usize::from(o1) - 192) << 8) + usize::from(o2) + 192,
                    rest,
                )
            }
            255 => {
                let (len, rest) = take_be(rest, 4)?;
                (found, len, rest)
            }
            _ => return Err("partial body lengths are not accepted".into()),
        }
    };
    if found != tag {
        return Err(format!("expected packet type {tag}, found {found}"));
    }
    if rest.len() != len {
        return Err("packet length does not match the data (one packet expected)".into());
    }
    Ok(rest)
}

fn take_be(input: &[u8], n: usize) -> Result<(usize, &[u8]), String> {
    if input.len() < n {
        return Err("truncated".into());
    }
    let v = input[..n]
        .iter()
        .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
    Ok((v, &input[n..]))
}

#[derive(Debug)]
struct Signature<'a> {
    /// Version through the end of the hashed subpackets.
    hashed: &'a [u8],
    hash_algo: u8,
    created: Option<u32>,
    expires: Option<u32>,
    issuer_fpr: Option<[u8; 20]>,
    issuer_id: Option<[u8; 8]>,
    left16: [u8; 2],
    mpi: &'a [u8],
}

fn parse_signature(body: &[u8]) -> Result<Signature<'_>, String> {
    if body.len() < 6 {
        return Err("truncated signature".into());
    }
    if body[0] != 4 {
        return Err(format!("signature version {} (4 expected)", body[0]));
    }
    if body[1] != 0x00 {
        return Err(format!(
            "signature type {:#04x} (0x00, a binary document, expected)",
            body[1]
        ));
    }
    if body[2] != 1 {
        return Err(format!(
            "public-key algorithm {} (1, RSA, expected)",
            body[2]
        ));
    }
    let hash_algo = body[3];
    let (hlen, rest) = take_be(&body[4..], 2)?;
    if rest.len() < hlen {
        return Err("truncated hashed subpackets".into());
    }
    let (hashed_sub, rest) = rest.split_at(hlen);
    let hashed = &body[..6 + hlen];
    let (ulen, rest) = take_be(rest, 2)?;
    if rest.len() < ulen {
        return Err("truncated unhashed subpackets".into());
    }
    let (unhashed_sub, rest) = rest.split_at(ulen);
    // Two bytes of the digest (a quick check), then one MPI.
    if rest.len() < 4 {
        return Err("truncated signature value".into());
    }
    let left16 = [rest[0], rest[1]];
    let (bits, rest) = take_be(&rest[2..], 2)?;
    let bytes = bits.div_ceil(8);
    if rest.len() != bytes || bytes == 0 {
        return Err("signature value length does not match".into());
    }
    let mut sig = Signature {
        hashed,
        hash_algo,
        created: None,
        expires: None,
        issuer_fpr: None,
        issuer_id: None,
        left16,
        mpi: rest,
    };
    subpackets(hashed_sub, true, &mut sig)?;
    // Unhashed subpackets are not covered by the signature: only the issuer
    // hints are read from them, and only to refuse a signature by another key.
    subpackets(unhashed_sub, false, &mut sig)?;
    if sig.created.is_none() {
        return Err("no signature creation time".into());
    }
    Ok(sig)
}

fn subpackets<'a>(mut data: &'a [u8], hashed: bool, sig: &mut Signature<'a>) -> Result<(), String> {
    while !data.is_empty() {
        let (&o1, rest) = data.split_first().ok_or("truncated subpacket")?;
        let (len, rest) = match o1 {
            0..=191 => (usize::from(o1), rest),
            192..=254 => {
                let (&o2, rest) = rest.split_first().ok_or("truncated subpacket length")?;
                (((usize::from(o1) - 192) << 8) + usize::from(o2) + 192, rest)
            }
            255 => take_be(rest, 4)?,
        };
        if len == 0 || rest.len() < len {
            return Err("bad subpacket length".into());
        }
        let (sub, next) = rest.split_at(len);
        data = next;
        let critical = sub[0] & 0x80 != 0;
        let kind = sub[0] & 0x7F;
        let value = &sub[1..];
        match kind {
            2 if hashed => sig.created = Some(be_u32(value)?),
            3 if hashed => sig.expires = Some(be_u32(value)?),
            16 => {
                let id: [u8; 8] = value.try_into().map_err(|_| "bad issuer key ID")?;
                sig.issuer_id = Some(id);
            }
            33 => {
                if value.len() != 21 || value[0] != 4 {
                    return Err("bad issuer fingerprint".into());
                }
                let mut fpr = [0u8; 20];
                fpr.copy_from_slice(&value[1..]);
                sig.issuer_fpr = Some(fpr);
            }
            _ if critical => return Err(format!("unknown critical subpacket {kind}")),
            _ => {}
        }
    }
    Ok(())
}

fn be_u32(v: &[u8]) -> Result<u32, String> {
    let b: [u8; 4] = v.try_into().map_err(|_| "bad time subpacket")?;
    Ok(u32::from_be_bytes(b))
}

/// The fuzz target `pgp_signature` (`fuzz/fuzz_targets/pgp_signature.rs`):
/// `data` is taken as a detached signature over yt-dlp 2026.10.26's real
/// `SHA2-256SUMS` and checked against yt-dlp's pinned key. The parser must
/// never panic, and nothing may verify over a changed document: that would be
/// a forgery. Not an API; the seed corpus (`fuzz/corpus/pgp_signature/`) also
/// runs in this crate's tests.
#[cfg(any(test, feature = "fuzzing"))]
#[doc(hidden)]
pub fn fuzz(data: &[u8]) {
    const DOCUMENT: &[u8] = include_bytes!("../tests/fixtures/yt-dlp-2026.10.26/SHA2-256SUMS");
    // A fixed time (2026-11-01), so a finding reproduces on any day.
    let now = UNIX_EPOCH + std::time::Duration::from_secs(1_793_491_200);
    let key = crate::sources::yt_dlp_key();
    let _ = verify_detached(&key, DOCUMENT, data, now);
    let mut changed = DOCUMENT.to_vec();
    changed.push(b'\n');
    assert!(
        verify_detached(&key, &changed, data, now).is_err(),
        "a signature verified over a changed document"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fuzz_seed_corpus_passes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/corpus/pgp_signature");
        let mut seeds = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            seeds.push(std::fs::read(entry.unwrap().path()).unwrap());
        }
        assert!(!seeds.is_empty(), "no seeds in {}", dir.display());
        let manifest = std::fs::read_to_string(dir.join("../../Cargo.toml")).unwrap();
        assert!(manifest.contains("name = \"pgp_signature\""));
        // The real signature is among the seeds and verifies.
        let real = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/yt-dlp-2026.10.26/SHA2-256SUMS.sig"),
        )
        .unwrap();
        assert!(seeds.contains(&real));
        for seed in &seeds {
            fuzz(seed);
        }
        // A small deterministic mutation run, so a plain `cargo test`
        // exercises the target; libFuzzer runs it properly every night.
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..3000 {
            let mut data = seeds[(next() as usize) % seeds.len()].clone();
            for _ in 0..=(next() % 4) {
                if data.is_empty() {
                    data.push(0xC2);
                }
                let i = (next() as usize) % data.len();
                match next() % 3 {
                    0 => data[i] = next() as u8,
                    1 => {
                        data.remove(i);
                    }
                    _ => data.insert(i, next() as u8),
                }
            }
            fuzz(&data);
        }
    }

    #[test]
    fn mpi_bit_counts() {
        let mut out = Vec::new();
        push_mpi(&mut out, &[0x00, 0x01, 0x00, 0x01]);
        assert_eq!(out, [0x00, 0x11, 0x01, 0x00, 0x01]);
    }

    #[test]
    fn refuses_garbage_and_armor() {
        let key = crate::sources::yt_dlp_key();
        let now = SystemTime::now();
        assert!(verify_detached(&key, b"x", b"", now).is_err());
        assert!(verify_detached(&key, b"x", b"-----BEGIN PGP SIGNATURE-----", now).is_err());
        assert!(verify_detached(&key, b"x", &[0x89, 0x00, 0x01, 0x04], now).is_err());
        // A packet of another type (public key, tag 6).
        assert!(verify_detached(&key, b"x", &[0x99, 0x00, 0x01, 0x04], now).is_err());
    }
}
