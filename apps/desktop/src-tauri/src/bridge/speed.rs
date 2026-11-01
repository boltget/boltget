//! Download speed as a moving average (docs/APP-DESIGN.md #27: speed is a
//! 3 s moving average, so the number the row shows does not jitter).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The averaging window.
pub(crate) const WINDOW: Duration = Duration::from_secs(3);

/// Byte counts sampled over time; the speed is the slope across the window.
#[derive(Debug, Clone)]
pub(crate) struct SpeedMeter {
    window: Duration,
    samples: VecDeque<(Instant, u64)>,
}

impl Default for SpeedMeter {
    fn default() -> Self {
        Self::new(WINDOW)
    }
}

impl SpeedMeter {
    pub(crate) fn new(window: Duration) -> Self {
        Self {
            window,
            samples: VecDeque::new(),
        }
    }

    /// Records the byte count at `now` and returns the speed in bytes per
    /// second. A count that goes down (a download that started over) starts
    /// a new measurement.
    pub(crate) fn record(&mut self, now: Instant, bytes: u64) -> u64 {
        if self.samples.back().is_some_and(|&(_, last)| bytes < last) {
            self.samples.clear();
        }
        self.samples.push_back((now, bytes));
        // Keep one sample at or beyond the window's start, so the slope
        // always spans the whole window once there is enough history.
        while self.samples.len() > 2
            && self
                .samples
                .get(1)
                .is_some_and(|&(t, _)| now.saturating_duration_since(t) >= self.window)
        {
            self.samples.pop_front();
        }
        self.rate()
    }

    /// Bytes per second across the samples held.
    pub(crate) fn rate(&self) -> u64 {
        let (Some(&(t0, b0)), Some(&(t1, b1))) = (self.samples.front(), self.samples.back()) else {
            return 0;
        };
        let secs = t1.saturating_duration_since(t0).as_secs_f64();
        if secs <= 0.0 {
            return 0;
        }
        ((b1 - b0) as f64 / secs).round() as u64
    }

    /// Forgets the history (the download stopped).
    pub(crate) fn reset(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn steady_rate() {
        let t = Instant::now();
        let mut m = SpeedMeter::default();
        assert_eq!(m.record(t, 0), 0, "one sample has no slope");
        assert_eq!(m.record(t + ms(250), 250_000), 1_000_000);
        for i in 2..40 {
            assert_eq!(m.record(t + ms(250 * i), 250_000 * i), 1_000_000);
        }
        assert!(m.samples.len() <= 14, "bounded by the window");
    }

    #[test]
    fn averages_over_the_window_and_decays_when_stalled() {
        let t = Instant::now();
        let mut m = SpeedMeter::new(Duration::from_secs(1));
        m.record(t, 0);
        m.record(t + ms(500), 1_000_000);
        assert_eq!(m.record(t + ms(1000), 2_000_000), 2_000_000);
        // Stalled for half a second: the average halves.
        assert_eq!(m.record(t + ms(1500), 2_000_000), 1_000_000);
        // Stalled for a full window: zero.
        assert_eq!(m.record(t + ms(2000), 2_000_000), 0);
    }

    #[test]
    fn restart_from_zero_starts_a_new_measurement() {
        let t = Instant::now();
        let mut m = SpeedMeter::default();
        m.record(t, 5_000_000);
        m.record(t + ms(250), 6_000_000);
        assert_eq!(m.record(t + ms(500), 0), 0);
        assert_eq!(m.record(t + ms(750), 100_000), 400_000);
        m.reset();
        assert_eq!(m.rate(), 0);
    }
}
