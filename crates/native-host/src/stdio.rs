//! Native messaging framing, as browsers define it: each message is a
//! 32-bit length in native byte order, then that many bytes of UTF-8 JSON.
//! Chromium sends a host at most 64 MiB at once and takes at most 1 MB
//! back; Firefox's limits are looser, so Chromium's apply.

use std::io::{self, Read, Write};

use boltget_ipc::browser::{MAX_FRAME, MAX_MESSAGE, MAX_REPLY};

/// One frame from the browser.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// A message of at most [`MAX_MESSAGE`] bytes.
    Message(Vec<u8>),
    /// A larger message (up to [`MAX_FRAME`]), read and thrown away so the
    /// stream stays in step; it gets an error reply.
    Skipped(u32),
}

/// Reads one frame. `Ok(None)` when the stream ends cleanly before a frame
/// starts (the browser closed the port); an error when it ends inside one
/// or announces more than [`MAX_FRAME`] bytes (after which the stream
/// cannot be trusted).
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut header = [0u8; 4];
    let mut got = 0;
    while got < header.len() {
        match r.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_ne_bytes(header);
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a frame of {len} bytes exceeds the 64 MiB limit"),
        ));
    }
    if len as usize > MAX_MESSAGE {
        let skipped = io::copy(&mut r.by_ref().take(u64::from(len)), &mut io::sink())?;
        if skipped < u64::from(len) {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        return Ok(Some(Frame::Skipped(len)));
    }
    let mut buf = vec![0; len as usize];
    r.read_exact(&mut buf)?;
    Ok(Some(Frame::Message(buf)))
}

/// Writes one frame and flushes it. A payload over [`MAX_REPLY`] is refused:
/// the browser would close the port.
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_REPLY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a reply over 1 MiB",
        ));
    }
    let len = u32::try_from(payload.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_ne_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_frame(&mut out, payload).unwrap();
        out
    }

    #[test]
    fn round_trip_in_native_order() {
        let bytes = framed(b"{}");
        assert_eq!(&bytes[..4], &2u32.to_ne_bytes());
        let mut input = [bytes.clone(), framed(b"")].concat();
        let mut r = &input[..];
        assert_eq!(
            read_frame(&mut r).unwrap(),
            Some(Frame::Message(b"{}".to_vec()))
        );
        assert_eq!(
            read_frame(&mut r).unwrap(),
            Some(Frame::Message(Vec::new()))
        );
        assert_eq!(read_frame(&mut r).unwrap(), None);
        input.truncate(3);
        assert!(
            read_frame(&mut &input[..]).is_err(),
            "cut inside the length"
        );
        let cut = &bytes[..5];
        assert!(read_frame(&mut &cut[..]).is_err(), "cut inside the message");
    }

    #[test]
    fn oversized_frames_are_skipped_up_to_64_mib() {
        let len = MAX_MESSAGE as u32 + 10;
        let mut input = len.to_ne_bytes().to_vec();
        input.resize(4 + len as usize, b' ');
        input.extend(framed(b"next"));
        let mut r = &input[..];
        assert_eq!(read_frame(&mut r).unwrap(), Some(Frame::Skipped(len)));
        assert_eq!(
            read_frame(&mut r).unwrap(),
            Some(Frame::Message(b"next".to_vec()))
        );

        let mut short = len.to_ne_bytes().to_vec();
        short.resize(100, b' ');
        assert!(
            read_frame(&mut &short[..]).is_err(),
            "ends inside the skipped frame"
        );

        let huge = (MAX_FRAME + 1).to_ne_bytes();
        assert!(read_frame(&mut &huge[..]).is_err());
    }

    #[test]
    fn replies_over_the_browser_limit_are_refused() {
        let big = vec![b' '; MAX_REPLY + 1];
        assert!(write_frame(&mut Vec::new(), &big).is_err());
        let fits = vec![b' '; MAX_REPLY];
        write_frame(&mut Vec::new(), &fits).unwrap();
    }
}
