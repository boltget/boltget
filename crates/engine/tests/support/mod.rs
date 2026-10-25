//! Helpers shared by the phase-3 integration tests (limits, integrity, link
//! refresh, proxies, cookies, adaptive connections).
#![allow(dead_code, unreachable_pub)]

use std::path::Path;
use std::time::Duration;

use boltget_engine::{
    AddRequest, Completed, Destination, Engine, EngineConfig, Error, RequestContext,
};
use boltget_testserver::content;
use sha2::{Digest, Sha256};

pub const KIB: u64 = 1024;
pub const MIB: u64 = 1024 * 1024;

/// Small, fast tunables so tests exercise splitting and retries quickly.
pub fn cfg() -> EngineConfig {
    let mut c = EngineConfig::default();
    c.min_split_size = 64 * KIB;
    c.segment_align = 4 * KIB;
    c.coalesce_size = 32 * 1024;
    c.worker_stagger = Duration::from_millis(5);
    c.flush_interval = Duration::from_millis(50);
    c.backoff_base = Duration::from_millis(10);
    c.backoff_max = Duration::from_millis(100);
    c.stall_timeout = Duration::from_millis(800);
    c.first_byte_timeout = Duration::from_secs(10);
    c.connect_timeout = Duration::from_secs(5);
    c.disk_margin = 0;
    c
}

pub fn sha(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

pub fn request(url: &str, dir: &Path) -> AddRequest {
    AddRequest::new(
        RequestContext::new(url).expect("valid URL"),
        Destination::Directory(dir.to_path_buf()),
    )
}

pub async fn get_with(engine: &Engine, req: AddRequest) -> Result<Completed, Error> {
    tokio::time::timeout(Duration::from_secs(60), engine.add(req).wait())
        .await
        .expect("download timed out")
}

pub fn assert_content(done: &Completed, seed: u64, size: u64) {
    assert_eq!(done.bytes, size);
    let got = std::fs::read(&done.path).expect("read downloaded file");
    assert_eq!(got.len() as u64, size);
    assert_eq!(sha(&got), sha(&content(seed, size)), "content mismatch");
    let partial = format!("{}{}", done.path.display(), boltget_engine::PARTIAL_SUFFIX);
    assert!(!Path::new(&partial).exists(), "partial file left behind");
}
