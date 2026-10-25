//! The adaptive connection-count controller (`docs/ENGINE.md`, "Adaptive
//! connection count"). Pure logic driven by the supervisor's ticks, so it can
//! be tested against simulated networks without timers.
//!
//! Hill climbing on measured aggregate throughput:
//!
//! - After every change it waits `settle` (new connections ramp up through TCP
//!   slow start), then measures throughput over `window`.
//! - An increase is kept only if throughput rose by more than `gain`; then it
//!   increases again: doubling while each doubling pays off at least half of
//!   what it ideally could, then one connection at a time.
//! - An increase that did not pay off is reverted, and the count holds for
//!   `cooldown` before probing again (from a fresh baseline).
//! - `429`/`503` or refused connections halve the count and set a ceiling it
//!   never probes above again.
//! - Windows in which fewer connections ran than the target (the tail of the
//!   download, or workers still starting) are discarded, and nothing is probed
//!   while there is no work left to split.

use std::time::Duration;

/// Controller tunables (design targets, see `EngineConfig`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct AdaptConfig {
    pub(crate) window: Duration,
    pub(crate) settle: Duration,
    pub(crate) gain: f64,
    pub(crate) cooldown: Duration,
    pub(crate) max: u32,
}

/// What the supervisor observed at a tick.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Observation {
    /// Time since the download started.
    pub(crate) now: Duration,
    /// Payload bytes received so far (monotonic).
    pub(crate) bytes: u64,
    /// Workers currently running.
    pub(crate) active: u32,
    /// Whether a new worker would find work (a segment worth splitting).
    pub(crate) can_grow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Last {
    /// No change yet, or the count was confirmed.
    Steady,
    /// Increased from `from` by `step`; waiting for the verdict.
    Increased { from: u32, step: u32 },
}

/// The controller of one download.
#[derive(Debug, Clone)]
pub(crate) struct Controller {
    cfg: AdaptConfig,
    target: u32,
    ceiling: u32,
    /// Throughput measured at the current confirmed count.
    baseline: Option<f64>,
    last: Last,
    /// Doubling until an increase pays off less than half its ideal.
    slow_start: bool,
    /// The current measurement: (start time, bytes at start).
    window_start: Option<(Duration, u64)>,
    /// No measurement starts before this.
    quiet_until: Duration,
    /// Whether the current window saw fewer workers than the target.
    tainted: bool,
    /// When the server last pushed back.
    last_throttle: Option<Duration>,
}

impl Controller {
    /// A controller starting at `initial` connections, never above `ceiling`.
    pub(crate) fn new(cfg: AdaptConfig, initial: u32, ceiling: u32) -> Self {
        let ceiling = ceiling.clamp(1, cfg.max.max(1));
        Self {
            cfg,
            target: initial.clamp(1, ceiling),
            ceiling,
            baseline: None,
            last: Last::Steady,
            slow_start: true,
            window_start: None,
            quiet_until: cfg.settle,
            tainted: false,
            last_throttle: None,
        }
    }

    /// The connection count the download should run with.
    #[cfg(test)]
    pub(crate) fn target(&self) -> u32 {
        self.target
    }

    /// The highest count this download may use.
    pub(crate) fn ceiling(&self) -> u32 {
        self.ceiling
    }

    fn restart_measurement(&mut self, now: Duration, wait: Duration) {
        self.window_start = None;
        self.quiet_until = now + wait;
        self.tainted = false;
    }

    /// Feeds one observation; returns the new target if it changed.
    pub(crate) fn tick(&mut self, o: Observation) -> Option<u32> {
        if o.now < self.quiet_until {
            return None;
        }
        let Some((t0, b0)) = self.window_start else {
            self.window_start = Some((o.now, o.bytes));
            self.tainted = false;
            return None;
        };
        if o.active < self.target {
            self.tainted = true;
        }
        let elapsed = o.now.saturating_sub(t0);
        if elapsed < self.cfg.window {
            return None;
        }
        let rate = o.bytes.saturating_sub(b0) as f64 / elapsed.as_secs_f64().max(1e-3);
        let tainted = self.tainted;
        // The next window starts now (later decisions may push it back).
        self.window_start = Some((o.now, o.bytes));
        self.tainted = false;
        if tainted {
            // Not all connections were running: the number says nothing about
            // the target. An increase whose workers found nothing to do is
            // simply undone.
            if let Last::Increased { from, .. } = self.last
                && !o.can_grow
            {
                self.last = Last::Steady;
                self.target = from;
                return Some(from);
            }
            return None;
        }
        let before = self.target;
        match self.last {
            Last::Increased { from, step } => {
                let base = self.baseline.unwrap_or(0.0);
                if rate > base * (1.0 + self.cfg.gain) {
                    // It paid off. Keep doubling while it pays off at least half
                    // of the ideal (+step/from).
                    let ideal = f64::from(step) / f64::from(from.max(1));
                    if base > 0.0 && rate / base - 1.0 < ideal / 2.0 {
                        self.slow_start = false;
                    }
                    self.baseline = Some(rate);
                    self.last = Last::Steady;
                    self.grow(o);
                } else {
                    // Revert and hold.
                    self.target = from;
                    self.last = Last::Steady;
                    self.slow_start = false;
                    self.baseline = None;
                    self.restart_measurement(o.now, self.cfg.cooldown);
                }
            }
            Last::Steady => {
                self.baseline = Some(rate);
                self.grow(o);
            }
        }
        (self.target != before).then_some(self.target)
    }

    fn grow(&mut self, o: Observation) {
        if !o.can_grow || self.target >= self.ceiling {
            return;
        }
        let step = if self.slow_start { self.target } else { 1 };
        let next = (self.target + step).min(self.ceiling);
        self.last = Last::Increased {
            from: self.target,
            step: next - self.target,
        };
        self.target = next;
        self.restart_measurement(o.now, self.cfg.settle);
    }

    /// The server pushed back (`429`/`503`, refused or reset connections) while
    /// `active` connections ran: halve, and never probe above the result.
    pub(crate) fn throttled(&mut self, now: Duration, active: u32) -> u32 {
        // One burst of `429`s (every worker hits it) halves once.
        let recent = self
            .last_throttle
            .is_some_and(|t| now.saturating_sub(t) < self.cfg.cooldown);
        let n = if recent {
            self.target.min(active).max(1)
        } else {
            (active.max(1) / 2).max(1)
        };
        self.last_throttle = Some(now);
        self.target = n;
        self.ceiling = n;
        self.last = Last::Steady;
        self.slow_start = false;
        self.baseline = None;
        self.restart_measurement(now, self.cfg.cooldown);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AdaptConfig {
        AdaptConfig {
            window: Duration::from_secs(1),
            settle: Duration::from_millis(500),
            gain: 0.10,
            cooldown: Duration::from_secs(10),
            max: 16,
        }
    }

    /// Runs `secs` of simulated download where `k` connections deliver
    /// `rate(k)` bytes per second (±`noise`), ticking every 250 ms. Returns the
    /// target over time.
    fn simulate(
        mut c: Controller,
        secs: u32,
        noise: f64,
        rate: impl Fn(u32) -> f64,
    ) -> (Controller, Vec<u32>) {
        let mut rng = fastrand::Rng::with_seed(42);
        let mut bytes = 0f64;
        let mut seen = Vec::new();
        for i in 1..=secs * 4 {
            let k = c.target();
            let jitter = 1.0 + noise * (rng.f64() * 2.0 - 1.0);
            bytes += rate(k) * jitter * 0.25;
            c.tick(Observation {
                now: Duration::from_millis(u64::from(i) * 250),
                bytes: bytes as u64,
                active: k,
                can_grow: true,
            });
            seen.push(c.target());
        }
        (c, seen)
    }

    const MB: f64 = 1e6;

    #[test]
    fn climbs_when_each_connection_is_throttled() {
        // 1 MB/s per connection, 100 MB/s line: more is always better.
        let (c, _) = simulate(Controller::new(cfg(), 2, 16), 60, 0.05, |k| {
            f64::from(k) * MB
        });
        assert_eq!(c.target(), 16);
    }

    #[test]
    fn stops_at_the_knee() {
        // 3 MB/s per connection, 12 MB/s line: the knee is at 4.
        let (c, seen) = simulate(Controller::new(cfg(), 2, 16), 90, 0.05, |k| {
            (f64::from(k) * 3.0 * MB).min(12.0 * MB)
        });
        assert!((4..=5).contains(&c.target()), "{seen:?}");
        // It never ran away to the cap.
        assert!(seen.iter().all(|&k| k <= 8), "{seen:?}");
    }

    #[test]
    fn stays_put_when_one_connection_fills_the_line() {
        // One connection already gets 100 MB/s; more only add overhead.
        let (c, seen) = simulate(Controller::new(cfg(), 2, 16), 60, 0.10, |k| {
            100.0 * MB * (1.0 - 0.02 * f64::from(k))
        });
        assert_eq!(c.target(), 2, "{seen:?}");
        assert!(seen.iter().all(|&k| k <= 4), "{seen:?}");
    }

    #[test]
    fn latency_bound_host_gains_up_to_a_point() {
        // Per-connection rate falls as the line fills (like the smoke test's
        // RTT-bound host: 19 MB/s with 1, ~28 with 8).
        let rate = |k: u32| 30.0 * MB * (1.0 - (-f64::from(k) / 2.0).exp());
        let (c, seen) = simulate(Controller::new(cfg(), 2, 16), 60, 0.05, rate);
        assert!((4..=8).contains(&c.target()), "{seen:?}");
    }

    #[test]
    fn throttling_halves_and_caps() {
        let mut c = Controller::new(cfg(), 8, 16);
        assert_eq!(c.throttled(Duration::from_secs(3), 8), 4);
        assert_eq!(c.target(), 4);
        assert_eq!(c.ceiling(), 4);
        let (c, seen) = simulate(c, 60, 0.0, |k| f64::from(k) * MB);
        assert!(seen.iter().all(|&k| k <= 4), "{seen:?}");
        assert_eq!(c.target(), 4);
        let mut one = Controller::new(cfg(), 1, 16);
        assert_eq!(one.throttled(Duration::ZERO, 1), 1);
    }

    #[test]
    fn nothing_happens_without_work_or_with_missing_workers() {
        let mut c = Controller::new(cfg(), 2, 16);
        for i in 1..200u64 {
            let r = c.tick(Observation {
                now: Duration::from_millis(i * 250),
                bytes: i * 1_000_000,
                active: 2,
                can_grow: false,
            });
            assert_eq!(r, None);
        }
        // An increase whose new workers never ran is undone.
        let mut c = Controller::new(cfg(), 2, 16);
        let mut t = 0;
        let mut changed = Vec::new();
        for i in 1..40u64 {
            t += 250;
            let active = if c.target() > 2 { 2 } else { c.target() };
            if let Some(n) = c.tick(Observation {
                now: Duration::from_millis(t),
                bytes: i * 1_000_000,
                active,
                can_grow: c.target() <= 2,
            }) {
                changed.push(n);
            }
        }
        assert_eq!(changed.first(), Some(&4));
        assert_eq!(changed.get(1), Some(&2), "{changed:?}");
    }
}
