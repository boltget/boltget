//! Framing: a 4-byte little-endian length, then that many bytes of UTF-8
//! JSON. The same framing as browser native messaging (there in native byte
//! order, which is little-endian on every platform Boltget ships for), so
//! one server can serve the CLI and the native host.

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::Error;

/// The largest message either side accepts: 1 MiB, as for native messaging
/// (docs/ARCHITECTURE.md, "Security").
pub const MAX_FRAME: u32 = 1 << 20;

/// Reads one frame. `Ok(None)` when the stream ends cleanly before a frame
/// starts; a stream that ends inside a frame is an error.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>, Error> {
    let mut header = [0u8; 4];
    if r.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    r.read_exact(&mut header[1..]).await?;
    let len = u32::from_le_bytes(header);
    if len > MAX_FRAME {
        return Err(Error::TooLarge(len));
    }
    let mut buf = vec![0; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

/// Writes one frame and flushes it.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, payload: &[u8]) -> Result<(), Error> {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    if len > MAX_FRAME {
        return Err(Error::TooLarge(len));
    }
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one message; `Ok(None)` at a clean end of stream.
pub async fn read_message<T: DeserializeOwned, R: AsyncRead + Unpin>(
    r: &mut R,
) -> Result<Option<T>, Error> {
    match read_frame(r).await? {
        Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        None => Ok(None),
    }
}

/// Writes one message.
pub async fn write_message<T: Serialize, W: AsyncWrite + Unpin>(
    w: &mut W,
    msg: &T,
) -> Result<(), Error> {
    write_frame(w, &serde_json::to_vec(msg)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip() {
        let (mut a, mut b) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            write_frame(&mut a, b"{\"type\":\"list\"}").await.unwrap();
            write_frame(&mut a, b"").await.unwrap();
        });
        assert_eq!(
            read_frame(&mut b).await.unwrap().as_deref(),
            Some(&b"{\"type\":\"list\"}"[..])
        );
        assert_eq!(read_frame(&mut b).await.unwrap().as_deref(), Some(&b""[..]));
        writer.await.unwrap();
        assert!(read_frame(&mut b).await.unwrap().is_none(), "clean end");
    }

    #[tokio::test]
    async fn length_is_little_endian() {
        let mut out = Vec::new();
        write_frame(&mut out, b"abc").await.unwrap();
        assert_eq!(out, [3, 0, 0, 0, b'a', b'b', b'c']);
    }

    #[tokio::test]
    async fn rejects_oversized_frames_before_reading_them() {
        let mut input: &[u8] = &(MAX_FRAME + 1).to_le_bytes();
        assert!(matches!(
            read_frame(&mut input).await,
            Err(Error::TooLarge(n)) if n == MAX_FRAME + 1
        ));
        let big = vec![b' '; MAX_FRAME as usize + 1];
        assert!(matches!(
            write_frame(&mut Vec::new(), &big).await,
            Err(Error::TooLarge(_))
        ));
    }

    #[tokio::test]
    async fn a_stream_cut_inside_a_frame_is_an_error() {
        let mut input: &[u8] = &[10, 0, 0, 0, b'{'];
        assert!(matches!(read_frame(&mut input).await, Err(Error::Io(_))));
        let mut input: &[u8] = &[10, 0];
        assert!(matches!(read_frame(&mut input).await, Err(Error::Io(_))));
    }
}
