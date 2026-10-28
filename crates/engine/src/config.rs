//! Engine configuration. Every number here is a *design target* from
//! `docs/ENGINE.md`, to be revisited with benchmark data.

use std::time::Duration;

/// Engine-wide settings.
#[derive(Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent on/off tunables, each documented with its design target"
)]
#[non_exhaustive]
pub struct EngineConfig {
    /// Connections a download starts with (4, measured in v0.1; the adaptive
    /// controller then adds more while they help).
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
    /// ... or for at most this long (250 ms), so slow connections still reach the
    /// disk (and the checkpoint) promptly instead of holding up to a chunk in memory.
    pub coalesce_delay: Duration,
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
    /// Global bandwidth limit in bytes per second, shared by all downloads
    /// (`None`: unlimited). [`Engine::open`](crate::Engine::open) replaces it
    /// with the stored [`Settings::speed_limit`] if one is set.
    pub speed_limit: Option<u64>,
    /// Proxies for downloads that do not choose their own
    /// ([`ProxyChoice::Default`](crate::ProxyChoice::Default)). Direct by default;
    /// the CLI fills it from the environment ([`ProxyRules::from_env`](crate::ProxyRules::from_env)).
    pub proxy: crate::proxy::ProxyRules,
    /// Adapt the connection count of downloads that do not fix one
    /// ([`AddRequest::connections`](crate::AddRequest::connections)): start at
    /// `initial_connections` and add connections while they raise throughput.
    pub adaptive: bool,
    /// Throughput is measured over this window after each change (1 s). A step
    /// that clearly pays off is kept after half of it.
    pub adapt_window: Duration,
    /// ... starting this long after the change, once new connections ramped up (500 ms).
    pub adapt_settle: Duration,
    /// An increase is kept only if throughput rose by more than this fraction (10%).
    pub adapt_gain: f64,
    /// After an increase that did not pay off, hold this long before probing again (10 s).
    pub adapt_cooldown: Duration,
    /// How long a host's learned connection ceiling (after `429`/`503`) is kept (24 h).
    pub host_ceiling_ttl: Duration,
    /// Near the end, a connection that would need more than `tail_eta` for its
    /// remainder has it split down to halves of this size (64 KiB; 0 disables).
    pub tail_split_size: u64,
    /// See `tail_split_size` (1 s).
    pub tail_eta: Duration,
    /// A slow connection that is the last one still transferring is reconnected
    /// after this long instead of `slow_window` (1 s; a starved connection while
    /// others are healthy gets 3 x this).
    pub tail_slow_window: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            initial_connections: 4,
            max_connections: 16,
            max_per_host: 16,
            max_total: 64,
            min_split_size: 1024 * 1024,
            segment_align: 64 * 1024,
            worker_stagger: Duration::from_millis(50),
            reaction_slack: Duration::from_millis(100),
            coalesce_size: 1024 * 1024,
            coalesce_delay: Duration::from_millis(250),
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
            speed_limit: None,
            proxy: crate::proxy::ProxyRules::direct(),
            adaptive: true,
            adapt_window: Duration::from_secs(1),
            adapt_settle: Duration::from_millis(500),
            adapt_gain: 0.10,
            adapt_cooldown: Duration::from_secs(10),
            host_ceiling_ttl: Duration::from_secs(24 * 3600),
            tail_split_size: 64 * 1024,
            tail_eta: Duration::from_secs(1),
            tail_slow_window: Duration::from_secs(1),
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
        self.speed_limit = self.speed_limit.filter(|r| *r > 0);
        self
    }
}

/// Persistent user settings, stored in the database's `setting` table one row
/// per key (`docs/ARCHITECTURE.md`, "Schema"). Keys this version does not know
/// (written by a newer one) are left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct Settings {
    /// Global bandwidth limit in bytes per second (key `engine.speed_limit_bps`).
    pub speed_limit: Option<u64>,
}
