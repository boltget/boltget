//! A page's thumbnail for the picker, fetched with the engine and handed to
//! the window as a `data:` URL (the app's content security policy loads no
//! image from the network, and the window never fetches anything itself).

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use boltget_engine::{AddRequest, ConflictPolicy, Destination, Engine, RequestContext};

use crate::cookies::JOBS_DIR;
use crate::{Error, Result, Sidecars};

/// The largest thumbnail fetched.
pub const MAX_THUMBNAIL: u64 = 2 * 1024 * 1024;

/// An image the picker can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailImage {
    /// `image/jpeg`, `image/png` or `image/webp`, from the bytes themselves.
    pub mime: &'static str,
    /// The image.
    pub bytes: Vec<u8>,
}

impl ThumbnailImage {
    /// Recognises a JPEG, PNG or WebP image by its first bytes; anything
    /// else (SVG, HTML, ...) is refused.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let mime = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "image/jpeg"
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            "image/webp"
        } else {
            return Err(Error::Thumbnail("not a JPEG, PNG or WebP image".into()));
        };
        Ok(Self { mime, bytes })
    }

    /// The image as a `data:` URL.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime, base64(&self.bytes))
    }
}

impl Sidecars {
    /// Fetches the thumbnail at `url` (`http` or `https`, at most
    /// [`MAX_THUMBNAIL`] bytes) with the engine, through the proxy the
    /// manager's downloads use.
    pub async fn thumbnail(&self, url: &str) -> Result<ThumbnailImage> {
        let ctx = RequestContext::new(url)?;
        let mut cfg = self.engine_config().clone();
        cfg.allowed_hosts = None;
        cfg.mark_of_the_web = false;
        let engine = Engine::new(cfg)?;
        let result = fetch(&engine, ctx, self).await;
        engine.shutdown().await;
        ThumbnailImage::from_bytes(result?)
    }
}

async fn fetch(engine: &Engine, ctx: RequestContext, sidecars: &Sidecars) -> Result<Vec<u8>> {
    let probe = engine.probe(&ctx).await?;
    match probe.size {
        Some(size) if size <= MAX_THUMBNAIL => {}
        Some(_) => return Err(Error::Thumbnail("larger than 2 MiB".into())),
        None => return Err(Error::Thumbnail("its size is unknown".into())),
    }
    let dir = sidecars.dir().join(JOBS_DIR);
    fs::create_dir_all(&dir).map_err(|e| Error::io(format!("creating {}", dir.display()), e))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = dir.join(format!("thumb-{}-{nanos}.img", std::process::id()));
    let req = AddRequest::new(ctx, Destination::File(path.clone()))
        .conflict(ConflictPolicy::Overwrite)
        .server_digest(false);
    let done = engine.add(req).wait().await;
    let bytes = done
        .map_err(Error::from)
        .and_then(|_| fs::read(&path).map_err(|e| Error::io("reading the thumbnail", e)));
    let _ = fs::remove_file(&path);
    let bytes = bytes?;
    if bytes.len() as u64 > MAX_THUMBNAIL {
        return Err(Error::Thumbnail("larger than 2 MiB".into()));
    }
    Ok(bytes)
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sextet = |shift: u32| char::from(TABLE[((n >> shift) & 63) as usize]);
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_examples() {
        for (input, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), encoded);
        }
    }

    #[test]
    fn only_raster_images_are_shown() {
        let jpeg = ThumbnailImage::from_bytes(vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2]).unwrap();
        assert_eq!(jpeg.mime, "image/jpeg");
        assert!(jpeg.data_url().starts_with("data:image/jpeg;base64,/9j/"));
        let png = ThumbnailImage::from_bytes(b"\x89PNG\r\n\x1a\nrest".to_vec()).unwrap();
        assert_eq!(png.mime, "image/png");
        let webp = ThumbnailImage::from_bytes(b"RIFF\0\0\0\0WEBPVP8 ".to_vec()).unwrap();
        assert_eq!(webp.mime, "image/webp");
        for bad in [&b"<svg xmlns"[..], b"<html>", b"GIF89a", b"RIFF"] {
            assert!(ThumbnailImage::from_bytes(bad.to_vec()).is_err());
        }
    }
}
