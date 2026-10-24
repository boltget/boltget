//! Engine configuration. Every number here is a *design target* from
//! `docs/ENGINE.md`, to be revisited with benchmark data.

use std::time::Duration;

/// Engine-wide settings.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EngineConfig {
    /// Connections a download starts with (design target 8, like IDM's default).
    pub initial_connections: u32,
    /// Hard cap on connections per download (design target 16, range 1–32).
    pub max_connections: u32,
    /// Connections shared by all downloads to one host (scheme + host + port).
    pub max_per_host: u32,
    /// Connections across all downloads.
    pub max_total: u32,
    /// Smallest segment worth its own connection (design target 1 MiB).
    pub min_split_size: u64,
    /// Segment boundary alignment (64 KiB).
    pub segment_align: u64,
    /// Delay between starting additional workers (design target 50 ms).
    pub worker_stagger: Duration,
    /// Extra reaction time added to in-flight estimates when splitting (100 ms).
    pub reaction_slack: Duration,
    /// Network reads are coalesced per worker into chunks of this size (1 MiB).
    pub coalesce_size: usize,
    /// Capacity of the writer channel, in chunks (16).
    pub writer_queue: usize,
    /// Flush (fsync) at least this often while data is arriving (2 s).
    pub flush_interval: Duration,
    /// Flush after this many bytes written since the last flush (64 MiB).
    pub flush_bytes: u64,
    /// TCP connect timeout (15 s).
    pub connect_timeout: Duration,
    /// TLS handshake timeout (15 s).
    pub tls_timeout: Duration,
    /// Time allowed from request to response headers (30 s).
    pub first_byte_timeout: Duration,
    /// No bytes on a connection for this long counts as a stall (30 s).
    pub stall_timeout: Duration,
    /// Base delay of exponential backoff with full jitter (1 s).
    pub backoff_base: Duration,
    /// Cap of the computed backoff delay (60 s).
    pub backoff_max: Duration,
    /// `Retry-After` longer than this fails the download instead of waiting (15 min).
    pub retry_after_max: Duration,
    /// Consecutive failures without progress before a segment gives up (10).
    pub max_attempts: u32,
    /// Maximum redirects followed (10).
    pub max_redirects: usize,
    /// Allow following a redirect from `https` to `http` (off by default).
    pub allow_insecure_redirects: bool,
    /// Enable endgame duplicate requests. Experimental, off by default.
    pub endgame: bool,
    /// Endgame triggers only if the slowest segment needs longer than this (2 s).
    pub endgame_threshold: Duration,
    /// A worker below this fraction of the median worker throughput ... (10%)
    pub slow_fraction: f64,
    /// ... for this long is reconnected (20 s). `Duration::ZERO` disables it.
    pub slow_window: Duration,
    /// Free space required beyond the file size (16 MiB).
    pub disk_margin: u64,
    /// Write the Windows Mark of the Web (`Zone.Identifier`) on completed files.
    pub mark_of_the_web: bool,
    /// `User-Agent` used when the request context does not provide one.
    pub user_agent: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            initial_connections: 8,
            max_connections: 16,
            max_per_host: 16,
            max_total: 64,
            min_split_size: 1024 * 1024,
            segment_align: 64 * 1024,
            worker_stagger: Duration::from_millis(50),
            reaction_slack: Duration::from_millis(100),
            coalesce_size: 1024 * 1024,
            writer_queue: 16,
            flush_interval: Duration::from_secs(2),
            flush_bytes: 64 * 1024 * 1024,
            connect_timeout: Duration::from_secs(15),
            tls_timeout: Duration::from_secs(15),
            first_byte_timeout: Duration::from_secs(30),
            stall_timeout: Duration::from_secs(30),
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            retry_after_max: Duration::from_secs(15 * 60),
            max_attempts: 10,
            max_redirects: 10,
            allow_insecure_redirects: false,
            endgame: false,
            endgame_threshold: Duration::from_secs(2),
            slow_fraction: 0.10,
            slow_window: Duration::from_secs(20),
            disk_margin: 16 * 1024 * 1024,
            mark_of_the_web: true,
            user_agent: concat!("BoltGet/", env!("CARGO_PKG_VERSION")).to_owned(),
        }
    }
}

impl EngineConfig {
    /// Clamps settings to their documented ranges.
    pub(crate) fn normalized(mut self) -> Self {
        self.max_connections = self.max_connections.clamp(1, 32);
        self.initial_connections = self.initial_connections.clamp(1, self.max_connections);
        self.max_per_host = self.max_per_host.clamp(1, 32);
        self.max_total = self.max_total.clamp(1, 256);
        self.coalesce_size = self.coalesce_size.max(4096);
        self.writer_queue = self.writer_queue.max(1);
        self.max_attempts = self.max_attempts.max(1);
        self.segment_align = self.segment_align.max(1);
        self.min_split_size = self.min_split_size.max(1);
        self
    }
}
