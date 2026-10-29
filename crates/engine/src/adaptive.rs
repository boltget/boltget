//! The adaptive connection-count controller (`docs/ENGINE.md`, "Adaptive
//! connection count"). Pure logic driven by the supervisor's ticks, so it can
//! be tested against simulated networks without timers.
//!
//! Hill climbing on measured aggregate throughput:
//!
//! - After every change it waits `settle` (new connections ramp up through TCP
//!   slow start), then measures throughput over `window`. Measuring starts
//!   after half of `settle`, but only an early pass (below) and a steady first
//!   baseline may use that first half.
//! - The first baseline is measured from `settle / 2` on, but accepted only once
//!   throughput has stopped rising: the second half of its window (at least
//!   half of `window`) is not faster than the first by more than `gain` (the
//!   connections are past TCP slow start). At the latest it is the full window
//!   one window later.
//! - An increase is kept only if throughput rose by more than `gain`; then it
//!   increases again: doubling while each doubling pays off at least half of
//!   what it ideally could, then one connection at a time. A doubling adds at
//!   most 4 connections until a step pays off at least three quarters of its
//!   ideal (the server limits each connection); from then on it doubles fully.
//!   The count confirmed then is reported as the host's start for later
//!   downloads ([`Controller::per_connection_count`]).
//! - A step that pays off half its ideal after a quarter window is kept at
//!   once; any other verdict waits for the full window.
//! - Nothing is probed when the rest of the download would end before a step
//!   could be judged (`settle` + `window` at the current rate).
//! - An increase that did not pay off is reverted, and the count holds for
//!   `cooldown` before probing again (from a fresh baseline).
//! - `429`/`503` or refused connections halve the count and set a ceiling it
//!   never probes above again.
//! - Windows in which fewer connections ran than the target (the tail of the
//!   download, or workers still starting) are discarded, and nothing is probed
//!   while there is no work left to split.

use std::collections::VecDeque;
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
    /// The bandwidth limit in force (bytes per second), if any: at the limit,
    /// more connections cannot help.
    pub(crate) cap: Option<u64>,
    /// Bytes still to download, if the size is known.
    pub(crate) remaining: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Last {
    /// No change yet, or the count was confirmed.
    Steady,
    /// Increased from `from` by `step`; waiting for the verdict.
    Increased { from: u32, step: u32 },
}

/// One tick of the current measurement.
#[derive(Debug, Clone, Copy)]
struct Sample {
    at: Duration,
    bytes: u64,
    /// Whether at least `target` workers ran.
    full: bool,
}

/// Timer jitter allowed when comparing tick times: the supervisor ticks every
/// 500 ms, and a tick that fires a millisecond early must not stretch a 1 s
/// window (or a 500 ms settle) by a whole tick.
const SLACK: Duration = Duration::from_millis(50);

/// Whether `elapsed` covers `span`, allowing for timer jitter.
fn covers(elapsed: Duration, span: Duration) -> bool {
    elapsed + SLACK >= span
}

/// Bytes per second between two samples.
fn rate_between(a: Sample, b: Sample) -> f64 {
    b.bytes.saturating_sub(a.bytes) as f64 / b.at.saturating_sub(a.at).as_secs_f64().max(1e-3)
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
    /// A step paid off at least three quarters of its ideal (the server limits
    /// each connection): double fully instead of by at most 4.
    proportional: bool,
    /// The highest count confirmed while the server limited each connection.
    per_connection: Option<u32>,
    /// The current measurement, oldest first (at most about one window long).
    samples: VecDeque<Sample>,
    /// No measurement starts before this.
    quiet_until: Duration,
    /// A verdict that is not an early pass only uses samples from this time on
    /// (the last change plus `settle`).
    settled_at: Duration,
    /// Until this time, the first baseline waits for steady throughput.
    steady_by: Option<Duration>,
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
            proportional: false,
            per_connection: None,
            samples: VecDeque::new(),
            // Measuring starts after half the settle time: the first baseline
            // waits for steady throughput anyway.
            quiet_until: cfg.settle / 2,
            settled_at: cfg.settle,
            // Every connection starts in TCP slow start, and a baseline measured
            // then makes any increase look good. So the first window is trusted
            // only once throughput is steady, and at the latest it is the
            // second full window.
            steady_by: Some(cfg.settle + cfg.window * 2),
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

    /// The highest count confirmed after a step paid off in proportion (the
    /// server limits each connection), if any: a good start for the next
    /// download from the same host.
    pub(crate) fn per_connection_count(&self) -> Option<u32> {
        self.per_connection
    }

    fn restart_measurement(&mut self, now: Duration, wait: Duration) {
        self.samples.clear();
        self.quiet_until = now + wait;
        self.settled_at = now + wait;
    }

    /// The next window starts at `now` (later decisions may push it back).
    fn next_window(&mut self, now: Sample) {
        self.samples.clear();
        self.samples.push_back(now);
    }

    /// Whether throughput has stopped rising: the second half of the current
    /// window is not faster than the first by more than `gain`. (Only rising
    /// matters: TCP slow start makes the baseline too low. Half-second rates
    /// vary by ±10% even on a steady line, so a two-sided test rarely passes.)
    fn steady(&self) -> bool {
        let (Some(&first), Some(&last)) = (self.samples.front(), self.samples.back()) else {
            return false;
        };
        let mid = first.at + last.at.saturating_sub(first.at) / 2;
        let inner = self.samples.len().saturating_sub(2);
        let Some(&m) = self
            .samples
            .iter()
            .skip(1)
            .take(inner)
            .min_by_key(|s| s.at.abs_diff(mid))
        else {
            return false;
        };
        let (a, b) = (rate_between(first, m), rate_between(m, last));
        a > 0.0 && b <= a * (1.0 + self.cfg.gain)
    }

    /// Feeds one observation; returns the new target if it changed.
    pub(crate) fn tick(&mut self, o: Observation) -> Option<u32> {
        if o.now + SLACK < self.quiet_until {
            return None;
        }
        let now = Sample {
            at: o.now,
            bytes: o.bytes,
            full: o.active >= self.target,
        };
        self.samples.push_back(now);
        // The window slides: it is the newest span that covers `window`.
        while self.samples.len() > 2
            && covers(o.now.saturating_sub(self.samples[1].at), self.cfg.window)
        {
            self.samples.pop_front();
        }
        let first = *self.samples.front()?;
        if self.samples.len() < 2 {
            return None;
        }
        let elapsed = o.now.saturating_sub(first.at);
        let full_window = covers(elapsed, self.cfg.window);
        let rate = rate_between(first, now);
        if self.samples.iter().skip(1).any(|s| !s.full) {
            // Not all connections were running: the number says nothing about
            // the target. An increase whose workers found nothing to do is
            // simply undone.
            if full_window
                && let Last::Increased { from, .. } = self.last
                && !o.can_grow
            {
                self.last = Last::Steady;
                self.target = from;
                self.next_window(now);
                return Some(from);
            }
            if full_window {
                self.next_window(now);
            }
            return None;
        }
        let before = self.target;
        match self.last {
            Last::Increased { from, step } => {
                let base = self.baseline.unwrap_or(0.0);
                let ideal = f64::from(step) / f64::from(from.max(1));
                let gained = if base > 0.0 { rate / base - 1.0 } else { 0.0 };
                let passed = rate > base * (1.0 + self.cfg.gain);
                // A quarter window that already shows a clear gain is enough,
                // even before the new connections have fully settled (TCP slow
                // start can only make the gain look smaller); any other verdict
                // waits for the full window after `settle`.
                let early = passed && gained >= ideal / 2.0 && covers(elapsed, self.cfg.window / 4);
                let settled = first.at + SLACK >= self.settled_at;
                if !(early || full_window && settled) {
                    return None;
                }
                if passed {
                    // It paid off. Keep doubling while it pays off at least half
                    // of the ideal (+step/from).
                    if base > 0.0 && gained < ideal / 2.0 {
                        self.slow_start = false;
                    }
                    if base > 0.0 && gained >= ideal * 0.75 {
                        self.proportional = true;
                    }
                    if self.proportional {
                        self.per_connection = self.per_connection.max(Some(self.target));
                    }
                    self.baseline = Some(rate);
                    self.last = Last::Steady;
                    self.next_window(now);
                    self.grow(o, rate);
                } else {
                    // Revert and hold.
                    self.target = from;
                    self.last = Last::Steady;
                    self.slow_start = false;
                    self.proportional = false;
                    self.baseline = None;
                    self.restart_measurement(o.now, self.cfg.cooldown);
                }
            }
            Last::Steady => {
                if let Some(by) = self.steady_by {
                    // The first baseline: half a window or more once throughput
                    // is steady, at the latest the full window ending at `by`.
                    let early = covers(elapsed, self.cfg.window / 2) && self.steady();
                    if !(early || full_window && o.now + SLACK >= by) {
                        return None;
                    }
                    self.steady_by = None;
                } else if !full_window {
                    return None;
                }
                self.baseline = Some(rate);
                self.next_window(now);
                self.grow(o, rate);
            }
        }
        (self.target != before).then_some(self.target)
    }

    fn grow(&mut self, o: Observation, rate: f64) {
        if !o.can_grow || self.target >= self.ceiling {
            return;
        }
        if o.cap.is_some_and(|c| rate >= 0.75 * c as f64) {
            return;
        }
        // A step that would end with the download cannot be judged, and its
        // new connections would only add handshakes to the tail.
        let verdict = (self.cfg.settle + self.cfg.window).as_secs_f64();
        if o.remaining.is_some_and(|r| r as f64 <= rate * verdict) {
            return;
        }
        // Doubling, but by at most 4 at a time until the server has shown that
        // it limits each connection: some servers punish bursts of new
        // connections from one client. (Not straight to the ceiling: a host
        // whose line fills at a few more connections would get a burst of
        // probes it cannot use.)
        let step = match (self.slow_start, self.proportional) {
            (true, true) => self.target,
            (true, false) => self.target.min(4),
            (false, _) => 1,
        };
        let next = (self.target + step).min(self.ceiling);
        self.last = Last::Increased {
            from: self.target,
            step: next - self.target,
        };
        self.target = next;
        // Measuring starts after half the settle time, for an early pass; the
        // full verdict still starts at `settle`.
        self.restart_measurement(o.now, self.cfg.settle / 2);
        self.settled_at = o.now + self.cfg.settle;
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
        self.proportional = false;
        self.baseline = None;
        self.steady_by = None;
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
                cap: None,
                remaining: None,
            });
            seen.push(c.target());
        }
        (c, seen)
    }

    const MB: f64 = 1e6;

    /// Like `simulate`, at the supervisor's real tick (`tick`, which may run
    /// slightly early) and with a rate that may depend on time. Returns the
    /// time of every change and the count it changed to.
    fn simulate_ticks(
        mut c: Controller,
        tick: Duration,
        until: Duration,
        noise: f64,
        rate: impl Fn(u32, Duration) -> f64,
    ) -> (Controller, Vec<(Duration, u32)>) {
        let mut rng = fastrand::Rng::with_seed(7);
        let mut bytes = 0f64;
        let mut changes = Vec::new();
        let mut now = Duration::ZERO;
        while now < until {
            now += tick;
            let k = c.target();
            let jitter = 1.0 + noise * (rng.f64() * 2.0 - 1.0);
            bytes += rate(k, now) * jitter * tick.as_secs_f64();
            if let Some(n) = c.tick(Observation {
                now,
                bytes: bytes as u64,
                active: k,
                can_grow: true,
                cap: None,
                remaining: None,
            }) {
                changes.push((now, n));
            }
        }
        (c, changes)
    }

    #[test]
    fn ramps_fast_when_each_connection_is_throttled() {
        // 2 MB/s per connection (the lab-throttled benchmark target), ticks of
        // 499 ms: a 1 s window must not take three ticks. Steady from the
        // start, so the first baseline is taken after 1.5 s; the step to 8
        // doubles the rate, which lifts the 4-connection cap on the next one.
        let (c, changes) = simulate_ticks(
            Controller::new(cfg(), 4, 16),
            Duration::from_millis(499),
            Duration::from_secs(10),
            0.05,
            |k, _| f64::from(k) * 2.0 * MB,
        );
        assert_eq!(c.target(), 16, "{changes:?}");
        let counts: Vec<u32> = changes.iter().map(|&(_, n)| n).collect();
        assert_eq!(counts, [8, 16], "{changes:?}");
        // 4 -> 8 after one second of steady baseline, 8 -> 16 one second later.
        assert!(changes[0].0 <= Duration::from_millis(1600), "{changes:?}");
        assert!(changes[1].0 <= Duration::from_millis(2600), "{changes:?}");
    }

    #[test]
    fn waits_for_tcp_slow_start_before_the_first_baseline() {
        // One connection fills the line, but every connection ramps up over the
        // first 2 s. A baseline taken during the ramp would make the probe to 8
        // look like a gain; the controller waits until the rate stops rising.
        let ramp = |t: Duration| (t.as_secs_f64() / 2.0).min(1.0);
        let (c, changes) = simulate_ticks(
            Controller::new(cfg(), 4, 16),
            Duration::from_millis(500),
            Duration::from_secs(30),
            0.02,
            |_, t| 100.0 * MB * ramp(t),
        );
        assert_eq!(c.target(), 4, "{changes:?}");
        // Probes beyond 4 are reverted, and nothing is probed during the ramp.
        assert!(changes.iter().all(|&(_, n)| n <= 8), "{changes:?}");
        assert!(
            changes
                .first()
                .is_none_or(|&(t, _)| t >= Duration::from_secs(2)),
            "{changes:?}"
        );
    }

    #[test]
    fn ramps_within_two_seconds_at_the_supervisor_tick() {
        // The supervisor's 250 ms tick, firing a millisecond early, on a server
        // that limits each connection to 2 MB/s (lab-throttled-100mib): the
        // first baseline is taken over [0.25 s, 0.75 s] once steady, and each
        // doubling is kept after a quarter window past half the settle time.
        let (c, changes) = simulate_ticks(
            Controller::new(cfg(), 4, 16),
            Duration::from_millis(249),
            Duration::from_secs(10),
            0.05,
            |k, _| f64::from(k) * 2.0 * MB,
        );
        let counts: Vec<u32> = changes.iter().map(|&(_, n)| n).collect();
        assert_eq!(counts, [8, 16], "{changes:?}");
        assert!(changes[0].0 <= Duration::from_millis(800), "{changes:?}");
        assert!(changes[1].0 <= Duration::from_millis(1300), "{changes:?}");
        // Both steps paid off in proportion: the host limits each connection,
        // and the next download from it may start at 16.
        assert_eq!(c.per_connection_count(), Some(16));
    }

    #[test]
    fn a_full_line_is_not_mistaken_for_a_per_connection_limit() {
        // One connection fills the line after a 1 s TCP ramp; quarter-second
        // ticks must not take a baseline during the ramp, and nothing is
        // learned for the host.
        let ramp = |t: Duration| t.as_secs_f64().min(1.0);
        let (c, changes) = simulate_ticks(
            Controller::new(cfg(), 4, 16),
            Duration::from_millis(249),
            Duration::from_secs(30),
            0.05,
            |_, t| 100.0 * MB * ramp(t),
        );
        assert_eq!(c.target(), 4, "{changes:?}");
        assert!(changes.iter().all(|&(_, n)| n <= 8), "{changes:?}");
        assert!(
            changes
                .first()
                .is_none_or(|&(t, _)| t >= Duration::from_secs(1)),
            "{changes:?}"
        );
        assert_eq!(c.per_connection_count(), None);
    }

    #[test]
    fn a_step_that_pays_off_too_little_teaches_nothing() {
        // Beyond 4 connections each adds only 40% of a connection's rate: the
        // step to 8 is kept (it beats the 10% gain), but it is not what a
        // per-connection limit looks like.
        let rate = |k: u32| {
            let k = f64::from(k);
            if k <= 4.0 { k } else { 4.0 + 0.4 * (k - 4.0) }
        };
        let (c, changes) = simulate_ticks(
            Controller::new(cfg(), 4, 16),
            Duration::from_millis(249),
            Duration::from_secs(10),
            0.0,
            |k, _| rate(k) * MB,
        );
        assert!(c.target() >= 8, "{changes:?}");
        assert_eq!(c.per_connection_count(), None, "{changes:?}");
    }

    #[test]
    fn does_not_probe_when_the_download_ends_before_a_verdict() {
        // 8 MB/s, 4 MB left at the first baseline: the download ends in about
        // half a second, before any step could be judged (settle + window).
        let mut c = Controller::new(cfg(), 4, 16);
        let total = 10_000_000u64;
        let mut bytes = 0u64;
        for i in 1..=20u64 {
            bytes = (bytes + 2_000_000).min(total);
            let r = c.tick(Observation {
                now: Duration::from_millis(i * 250),
                bytes,
                active: 4,
                can_grow: true,
                cap: None,
                remaining: Some(total - bytes),
            });
            assert_eq!(r, None, "probed at tick {i}");
        }
        // With plenty left, the same rate is probed.
        let mut c = Controller::new(cfg(), 4, 16);
        let probed = (1..=8u64).any(|i| {
            c.tick(Observation {
                now: Duration::from_millis(i * 250),
                bytes: i * 2_000_000,
                active: 4,
                can_grow: true,
                cap: None,
                remaining: Some(100_000_000),
            })
            .is_some()
        });
        assert!(probed);
    }

    #[test]
    fn adds_at_most_four_until_a_step_pays_off_in_proportion() {
        // Beyond 4 connections each new one adds only 60% of a connection's
        // rate: 4 -> 8 pays off 60% of its ideal (enough to keep doubling, not
        // enough to lift the cap), so the next step is 8 -> 12, not 8 -> 16.
        let rate = |k: u32| {
            let k = f64::from(k);
            if k <= 4.0 { k } else { 4.0 + 0.6 * (k - 4.0) }
        };
        let (_, changes) = simulate_ticks(
            Controller::new(cfg(), 4, 16),
            Duration::from_millis(500),
            Duration::from_secs(6),
            0.0,
            |k, _| rate(k) * MB,
        );
        let counts: Vec<u32> = changes.iter().map(|&(_, n)| n).collect();
        assert_eq!(counts.get(..2), Some(&[8, 12][..]), "{changes:?}");
    }

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
    fn does_not_grow_past_a_bandwidth_limit() {
        // 2 MB/s per connection, but the user limited the download to 5 MB/s.
        let mut c = Controller::new(cfg(), 4, 16);
        let mut bytes = 0u64;
        for i in 1..=200u64 {
            let k = u64::from(c.target());
            bytes += (k * 2_000_000).min(5_000_000) / 4;
            c.tick(Observation {
                now: Duration::from_millis(i * 250),
                bytes,
                active: c.target(),
                can_grow: true,
                cap: Some(5_000_000),
                remaining: None,
            });
        }
        assert_eq!(c.target(), 4);
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
                cap: None,
                remaining: None,
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
                cap: None,
                remaining: None,
            }) {
                changed.push(n);
            }
        }
        assert_eq!(changed.first(), Some(&4));
        assert_eq!(changed.get(1), Some(&2), "{changed:?}");
    }
}
