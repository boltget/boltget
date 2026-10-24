//! The per-download writer: a dedicated OS thread doing positional writes.
//!
//! Workers send [`WriteChunk`]s over a bounded channel; when it is full they wait,
//! stop reading their sockets, and TCP flow control slows the server down. The
//! writer flushes (`sync_data`) every `flush_interval` or `flush_bytes`, and only
//! *after* a successful flush reports the new durable offsets, which is the
//! ordering the checkpoint invariant depends on (`docs/ENGINE.md`, "Flush policy").

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use crate::error::{Error, Result};
use crate::platform;

/// Bytes to write at an absolute offset, on behalf of a segment.
#[derive(Debug)]
pub(crate) struct WriteChunk {
    pub(crate) segment: u32,
    pub(crate) offset: u64,
    pub(crate) bytes: Bytes,
}

/// After a flush: for each segment written since the last report, the absolute
/// offset up to which its bytes are durable.
pub(crate) type DurableReport = Vec<(u32, u64)>;

/// Handle to a running writer thread.
#[derive(Debug)]
pub(crate) struct Writer {
    pub(crate) tx: mpsc::Sender<WriteChunk>,
    pub(crate) durable: mpsc::UnboundedReceiver<DurableReport>,
    pub(crate) done: oneshot::Receiver<Result<File>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FlushPolicy {
    pub(crate) interval: Duration,
    pub(crate) bytes: u64,
}

/// Starts the writer thread for `file`. The thread ends when every sender is
/// dropped (after draining and a final flush) or on the first I/O error.
pub(crate) fn spawn(
    file: File,
    path: Arc<PathBuf>,
    capacity: usize,
    policy: FlushPolicy,
) -> Result<Writer> {
    let (tx, rx) = mpsc::channel(capacity);
    let (dtx, drx) = mpsc::unbounded_channel();
    let (done_tx, done_rx) = oneshot::channel();
    thread::Builder::new()
        .name("boltget-writer".into())
        .spawn(move || {
            let res = run(file, &path, rx, &dtx, policy);
            let _ = done_tx.send(res);
        })
        .map_err(|e| Error::Internal(format!("spawning writer thread: {e}")))?;
    Ok(Writer {
        tx,
        durable: drx,
        done: done_rx,
    })
}

fn run(
    file: File,
    path: &PathBuf,
    mut rx: mpsc::Receiver<WriteChunk>,
    durable: &mpsc::UnboundedSender<DurableReport>,
    policy: FlushPolicy,
) -> Result<File> {
    // Next expected offset per segment. Each segment is written by one worker,
    // strictly in order, so anything else is an engine bug.
    let mut next: HashMap<u32, u64> = HashMap::new();
    let mut dirty: HashMap<u32, u64> = HashMap::new();
    let mut since_flush = 0u64;
    let mut last_flush = Instant::now();

    let flush = |dirty: &mut HashMap<u32, u64>| -> Result<()> {
        if dirty.is_empty() {
            return Ok(());
        }
        file.sync_data().map_err(|e| Error::disk(path, e))?;
        let report: DurableReport = dirty.drain().collect();
        let _ = durable.send(report);
        Ok(())
    };

    while let Some(chunk) = rx.blocking_recv() {
        let len = chunk.bytes.len() as u64;
        if let Some(&expect) = next.get(&chunk.segment)
            && expect != chunk.offset
        {
            return Err(Error::Internal(format!(
                "segment {} write at {} but expected {}",
                chunk.segment, chunk.offset, expect
            )));
        }
        platform::write_all_at(&file, &chunk.bytes, chunk.offset)
            .map_err(|e| Error::disk(path, e))?;
        let end = chunk.offset + len;
        next.insert(chunk.segment, end);
        dirty.insert(chunk.segment, end);
        since_flush += len;
        if since_flush >= policy.bytes || last_flush.elapsed() >= policy.interval {
            flush(&mut dirty)?;
            since_flush = 0;
            last_flush = Instant::now();
        }
    }
    // All senders gone: final flush.
    flush(&mut dirty)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_flushes_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.bin");
        let (f, _) = platform::create_preallocated(&p, Some(8)).unwrap();
        let policy = FlushPolicy {
            interval: Duration::from_secs(3600),
            bytes: u64::MAX,
        };
        let mut w = spawn(f, Arc::new(p.clone()), 2, policy).unwrap();
        for (seg, off, data) in [(1u32, 4u64, &b"wxyz"[..]), (0, 0, b"ab"), (0, 2, b"cd")] {
            w.tx.send(WriteChunk {
                segment: seg,
                offset: off,
                bytes: Bytes::from_static(data),
            })
            .await
            .unwrap();
        }
        drop(w.tx);
        w.done.await.unwrap().unwrap();
        let mut report = w.durable.recv().await.unwrap();
        report.sort_unstable();
        assert_eq!(report, vec![(0, 4), (1, 8)]);
        assert_eq!(std::fs::read(&p).unwrap(), b"abcdwxyz");
    }

    #[tokio::test]
    async fn out_of_order_write_is_an_internal_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.bin");
        let (f, _) = platform::create_preallocated(&p, Some(8)).unwrap();
        let policy = FlushPolicy {
            interval: Duration::ZERO,
            bytes: 0,
        };
        let w = spawn(f, Arc::new(p), 4, policy).unwrap();
        for off in [0u64, 4] {
            let _ =
                w.tx.send(WriteChunk {
                    segment: 0,
                    offset: off,
                    bytes: Bytes::from_static(b"ab"),
                })
                .await;
        }
        drop(w.tx);
        assert!(matches!(w.done.await.unwrap(), Err(Error::Internal(_))));
    }
}
