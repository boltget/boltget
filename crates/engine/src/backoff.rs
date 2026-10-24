//! Exponential backoff with full jitter: `random(0, min(max, base * 2^attempt))`.

use std::time::Duration;

/// Upper bound of the backoff window for `attempt` (0-based).
pub(crate) fn ceiling(base: Duration, max: Duration, attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
    base.checked_mul(factor).unwrap_or(max).min(max)
}

/// A random delay in `[0, ceiling]`.
pub(crate) fn delay(base: Duration, max: Duration, attempt: u32) -> Duration {
    let ceil = ceiling(base, max, attempt);
    let nanos = u64::try_from(ceil.as_nanos()).unwrap_or(u64::MAX);
    Duration::from_nanos(fastrand::u64(0..=nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceiling_grows_and_caps() {
        let b = Duration::from_secs(1);
        let m = Duration::from_secs(60);
        assert_eq!(ceiling(b, m, 0), Duration::from_secs(1));
        assert_eq!(ceiling(b, m, 1), Duration::from_secs(2));
        assert_eq!(ceiling(b, m, 5), Duration::from_secs(32));
        assert_eq!(ceiling(b, m, 6), m);
        assert_eq!(ceiling(b, m, 100), m);
    }

    #[test]
    fn delay_within_window() {
        let b = Duration::from_millis(100);
        let m = Duration::from_secs(5);
        for attempt in 0..20 {
            for _ in 0..50 {
                assert!(delay(b, m, attempt) <= ceiling(b, m, attempt));
            }
        }
    }
}
