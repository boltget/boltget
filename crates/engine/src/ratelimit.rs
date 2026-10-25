//! Token-bucket bandwidth limiting (`docs/ENGINE.md`, "Bandwidth limiting").
//!
//! A worker that has read `n` payload bytes acquires `n` tokens from its
//! download's bucket, then from the global bucket, before it reads again. While
//! it waits it does not poll the socket, so the kernel receive buffer fills, the
//! TCP window closes and the server slows down.
//!
//! - Burst: 250 ms worth of the rate, at least one read unit (16 KiB).
//! - A read larger than the burst is allowed once enough tokens for a full burst
//!   are there; the bucket then goes into debt, which later reads repay. Over
//!   time the rate is exact; only the size of single reads makes it bursty.
//! - Fairness: waiters queue on a FIFO async mutex, so one worker (or one
//!   download, for the global bucket) cannot starve the others.
//! - A rate change wakes the current waiter, so it takes effect within one
//!   refill step (at most 100 ms) without reconnecting.

use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

/// Smallest burst, in bytes: one read unit.
const MIN_BURST: f64 = 16.0 * 1024.0;
/// Burst as a fraction of a second of the rate.
const BURST_SECS: f64 = 0.25;
/// Longest single sleep, so a rate change or a cancellation is seen promptly.
const MAX_SLEEP: Duration = Duration::from_millis(100);
/// Shortest sleep: the timer resolution. A zero sleep would spin.
const MIN_SLEEP: Duration = Duration::from_millis(1);

#[derive(Debug)]
struct Bucket {
    /// Bytes per second; `None` is unlimited.
    rate: Option<u64>,
    /// Available tokens (bytes). Negative while a large read is being repaid.
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn burst(rate: u64) -> f64 {
        (rate as f64 * BURST_SECS).max(MIN_BURST)
    }

    fn refill(&mut self, now: Instant) {
        if let Some(rate) = self.rate {
            let dt = now.saturating_duration_since(self.last).as_secs_f64();
            self.tokens = (self.tokens + dt * rate as f64).min(Self::burst(rate));
        }
        self.last = now;
    }
}

/// One token bucket. Cheap to share behind an `Arc`.
#[derive(Debug)]
pub(crate) struct RateLimiter {
    bucket: Mutex<Bucket>,
    /// Waiters take turns in FIFO order (tokio's mutex is fair).
    queue: tokio::sync::Mutex<()>,
    changed: Notify,
}

impl RateLimiter {
    /// A bucket with `rate` bytes per second (`None` or 0: unlimited), starting full.
    pub(crate) fn new(rate: Option<u64>) -> Self {
        let rate = rate.filter(|r| *r > 0);
        Self {
            bucket: Mutex::new(Bucket {
                rate,
                tokens: rate.map_or(0.0, Bucket::burst),
                last: Instant::now(),
            }),
            queue: tokio::sync::Mutex::new(()),
            changed: Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Bucket> {
        self.bucket
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The current rate in bytes per second, `None` if unlimited.
    pub(crate) fn rate(&self) -> Option<u64> {
        self.lock().rate
    }

    /// Changes the rate (`None` or 0: unlimited). Waiters see it at once.
    pub(crate) fn set_rate(&self, rate: Option<u64>) {
        let rate = rate.filter(|r| *r > 0);
        {
            let mut b = self.lock();
            let now = Instant::now();
            b.refill(now);
            match (b.rate, rate) {
                // Turning a limit on starts with a full burst, not with a debt.
                (None, Some(r)) => b.tokens = Bucket::burst(r),
                (_, Some(r)) => b.tokens = b.tokens.min(Bucket::burst(r)),
                (_, None) => b.tokens = 0.0,
            }
            b.rate = rate;
        }
        self.changed.notify_waiters();
    }

    /// Waits until `n` bytes may be consumed, then takes them.
    pub(crate) async fn acquire(&self, n: u64) {
        if n == 0 || self.lock().rate.is_none() {
            return;
        }
        let _turn = self.queue.lock().await;
        loop {
            let wait = {
                let mut b = self.lock();
                b.refill(Instant::now());
                let Some(rate) = b.rate else { return };
                let need = (n as f64).min(Bucket::burst(rate));
                // Half a byte of slack absorbs floating-point drift in the refill.
                if b.tokens + 0.5 >= need {
                    b.tokens -= n as f64;
                    return;
                }
                Duration::from_secs_f64((need - b.tokens) / rate as f64)
            };
            // Register for change notifications before sleeping, so a
            // `set_rate` between the check above and the sleep is not missed.
            let changed = self.changed.notified();
            tokio::select! {
                () = tokio::time::sleep(wait.clamp(MIN_SLEEP, MAX_SLEEP)) => {}
                () = changed => {}
            }
        }
    }
}

/// The buckets a worker acquires from, innermost first (download, then global).
#[derive(Debug, Clone)]
pub(crate) struct Limiters {
    pub(crate) download: std::sync::Arc<RateLimiter>,
    pub(crate) global: std::sync::Arc<RateLimiter>,
}

impl Limiters {
    pub(crate) async fn acquire(&self, n: u64) {
        self.download.acquire(n).await;
        self.global.acquire(n).await;
    }
}

/// Parses a rate such as `500K`, `5M`, `1.5MiB/s` or `1048576` into bytes per
/// second. Suffixes are binary (`K` = 1024, as in curl's `--limit-rate`); `B`,
/// `iB` and `/s` are optional. `0`, `off`, `none` and `unlimited` mean no limit
/// (`Ok(None)`).
pub fn parse_rate(s: &str) -> Result<Option<u64>, String> {
    let t = s.trim();
    if matches!(
        t.to_ascii_lowercase().as_str(),
        "0" | "off" | "none" | "unlimited"
    ) {
        return Ok(None);
    }
    let lower = t.to_ascii_lowercase();
    let body = lower.strip_suffix("/s").unwrap_or(&lower);
    let body = body
        .strip_suffix("ib")
        .or_else(|| body.strip_suffix('b'))
        .unwrap_or(body);
    let (num, mult) = match body.chars().last() {
        Some('k') => (&body[..body.len() - 1], 1024f64),
        Some('m') => (&body[..body.len() - 1], 1024f64 * 1024.0),
        Some('g') => (&body[..body.len() - 1], 1024f64 * 1024.0 * 1024.0),
        _ => (body, 1f64),
    };
    let v: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("{s:?} is not a rate (examples: 500K, 5M, 1048576)"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("{s:?} is not a rate"));
    }
    let bps = (v * mult).round();
    if bps > u64::MAX as f64 / 2.0 {
        return Err(format!("{s:?} is too large"));
    }
    Ok(Some(bps as u64).filter(|b| *b > 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn rate_parsing() {
        assert_eq!(parse_rate("1048576"), Ok(Some(1_048_576)));
        assert_eq!(parse_rate("500K"), Ok(Some(512_000)));
        assert_eq!(parse_rate("5M"), Ok(Some(5 * 1024 * 1024)));
        assert_eq!(parse_rate("5m"), Ok(Some(5 * 1024 * 1024)));
        assert_eq!(parse_rate("1.5MiB/s"), Ok(Some(1_572_864)));
        assert_eq!(parse_rate("2MB"), Ok(Some(2 * 1024 * 1024)));
        assert_eq!(parse_rate("1G"), Ok(Some(1024 * 1024 * 1024)));
        assert_eq!(parse_rate("0"), Ok(None));
        assert_eq!(parse_rate("off"), Ok(None));
        assert!(parse_rate("fast").is_err());
        assert!(parse_rate("-5M").is_err());
        assert!(parse_rate("").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn unlimited_never_waits() {
        let l = RateLimiter::new(None);
        let t0 = Instant::now();
        for _ in 0..1000 {
            l.acquire(1 << 20).await;
        }
        assert_eq!(t0.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn sustained_rate_is_exact() {
        // 1 MiB/s; 64 reads of 64 KiB = 4 MiB. The first burst (256 KiB) is free.
        let rate = 1024 * 1024;
        let l = RateLimiter::new(Some(rate));
        let t0 = Instant::now();
        for _ in 0..64 {
            l.acquire(64 * 1024).await;
        }
        let secs = t0.elapsed().as_secs_f64();
        let expected = (4.0 * 1024.0 * 1024.0 - Bucket::burst(rate)) / rate as f64;
        assert!((secs - expected).abs() < 0.05, "{secs} vs {expected}");
    }

    #[tokio::test(start_paused = true)]
    async fn reads_larger_than_the_burst_are_repaid() {
        // 64 KiB/s has a 16 KiB burst; 1 MiB reads must still average 64 KiB/s.
        let rate = 64 * 1024;
        let l = RateLimiter::new(Some(rate));
        let t0 = Instant::now();
        for _ in 0..4 {
            l.acquire(1024 * 1024).await;
        }
        // Three full reads must be repaid before the fourth can start.
        let secs = t0.elapsed().as_secs_f64();
        assert!((secs - 48.0).abs() < 0.5, "{secs}");
    }

    #[tokio::test(start_paused = true)]
    async fn waiters_share_the_rate_fairly() {
        let rate = 1024 * 1024;
        let l = Arc::new(RateLimiter::new(Some(rate)));
        // Without waiting, the first task would take the whole initial burst.
        l.acquire(256 * 1024).await;
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let l = l.clone();
            tasks.push(tokio::spawn(async move {
                let t0 = Instant::now();
                for _ in 0..32 {
                    l.acquire(16 * 1024).await;
                }
                t0.elapsed().as_secs_f64()
            }));
        }
        let mut times = Vec::new();
        for t in tasks {
            times.push(t.await.unwrap());
        }
        // 2 MiB in total at 1 MiB/s: everyone finishes near the end, nobody early.
        for t in &times {
            assert!(*t > 1.8 && *t < 2.1, "{times:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn rate_changes_apply_to_waiters() {
        let l = Arc::new(RateLimiter::new(Some(1024)));
        l.acquire(16 * 1024).await; // drain the burst
        let waiter = {
            let l = l.clone();
            tokio::spawn(async move {
                let t0 = Instant::now();
                l.acquire(16 * 1024).await; // 16 s at 1 KiB/s
                t0.elapsed()
            })
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        l.set_rate(None);
        let waited = waiter.await.unwrap();
        assert!(waited < Duration::from_millis(1200), "{waited:?}");
        assert_eq!(l.rate(), None);
    }
}
