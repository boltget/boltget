//! Signature verification against real data: yt-dlp 2026.10.26's signed
//! `SHA2-256SUMS`, checked with the pinned release key, and test signatures.
#![allow(clippy::unwrap_used)] // Test helpers outside #[test] functions.

mod support;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use boltget_media::pgp::verify_detached;
use boltget_media::sources::{YT_DLP_KEY_FINGERPRINT, yt_dlp_key};
use support::{TestKey, fixture};

fn real() -> (Vec<u8>, Vec<u8>) {
    let dir = fixture("yt-dlp-2026.10.26");
    (
        std::fs::read(dir.join("SHA2-256SUMS")).unwrap(),
        std::fs::read(dir.join("SHA2-256SUMS.sig")).unwrap(),
    )
}

#[test]
fn yt_dlps_real_signature_verifies_with_the_pinned_key() {
    let (sums, sig) = real();
    assert_eq!(yt_dlp_key().fingerprint_hex(), YT_DLP_KEY_FINGERPRINT);
    verify_detached(&yt_dlp_key(), &sums, &sig, SystemTime::now()).unwrap();
}

#[test]
fn any_change_breaks_it() {
    let (sums, sig) = real();
    let key = yt_dlp_key();
    let now = SystemTime::now();
    // One byte of the document.
    let mut doc = sums.clone();
    doc[10] ^= 1;
    assert!(verify_detached(&key, &doc, &sig, now).is_err());
    // A line added (the attack that matters: a hash for another file).
    let mut doc = sums.clone();
    doc.extend_from_slice(b"00  evil.exe\n");
    assert!(verify_detached(&key, &doc, &sig, now).is_err());
    // Every byte of the signature after the packet header, except the
    // unhashed subpackets (bytes 40 to 49 here: the issuer key ID, which the
    // signature does not cover; a changed ID is refused as another key's,
    // a changed type is ignored as an unknown hint).
    for i in (3..40).chain(50..sig.len()) {
        let mut s = sig.clone();
        s[i] ^= 0x40;
        assert!(verify_detached(&key, &sums, &s, now).is_err(), "byte {i}");
    }
    // Truncated and extended.
    assert!(verify_detached(&key, &sums, &sig[..sig.len() - 1], now).is_err());
    let mut s = sig.clone();
    s.push(0);
    assert!(verify_detached(&key, &sums, &s, now).is_err());
}

#[test]
fn another_key_does_not_verify_it() {
    let (sums, sig) = real();
    let other = TestKey::load().pinned();
    assert!(verify_detached(&other, &sums, &sig, SystemTime::now()).is_err());
}

#[test]
fn test_signatures_verify_and_respect_the_key() {
    let key = TestKey::load();
    let data = b"0123  file\n";
    let sig = key.sign(data);
    verify_detached(&key.pinned(), data, &sig, SystemTime::now()).unwrap();
    assert!(verify_detached(&yt_dlp_key(), data, &sig, SystemTime::now()).is_err());
    // Signed long ago and still valid: these signatures do not expire.
    verify_detached(
        &key.pinned(),
        data,
        &sig,
        UNIX_EPOCH + Duration::from_secs(4_000_000_000),
    )
    .unwrap();
}
