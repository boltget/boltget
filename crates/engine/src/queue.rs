//! Queues and the scheduler that runs them (`docs/ENGINE.md`, "Queues").
//!
//! Every download belongs to a queue. A queue has an order, a limit of
//! downloads it runs at once, a started or stopped state, an optional daily
//! start and stop time, and an optional action for when it finishes. The
//! scheduler, which a long-running owner turns on with
//! [`Engine::start_queues`](crate::Engine::start_queues), starts waiting
//! downloads in order as slots free.
//!
//! This module holds the public types and the planning itself, as plain
//! functions over plain values so the rules are tested without I/O; the
//! engine (`engine.rs`) reads the database, calls [`plan`] and acts on it.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::events::{DownloadId, State};

/// Identifies a queue. The default queue is [`DEFAULT_QUEUE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct QueueId(pub u64);

impl fmt::Display for QueueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The default queue: every download belongs to it unless placed elsewhere.
/// It cannot be deleted.
pub const DEFAULT_QUEUE: QueueId = QueueId(1);

/// The default queue's name where no front end names it (the CLI).
pub const DEFAULT_QUEUE_NAME: &str = "Main";

/// How many downloads a new queue runs at once.
pub const DEFAULT_MAX_CONCURRENT: u32 = 3;

/// The highest limit a queue may have.
pub const MAX_CONCURRENT: u32 = 32;

/// The longest queue name, in characters.
pub const MAX_NAME_CHARS: usize = 40;

/// What to do when a queue finishes: its last running download ended on its
/// own (completed or failed) and nothing waits in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishAction {
    /// Nothing (the default).
    #[default]
    Nothing,
    /// Put the computer to sleep.
    Sleep,
    /// Shut the computer down.
    ShutDown,
}

impl FinishAction {
    /// The name used in the database and in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            FinishAction::Nothing => "nothing",
            FinishAction::Sleep => "sleep",
            FinishAction::ShutDown => "shut_down",
        }
    }

    /// Parses [`FinishAction::as_str`] output.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "nothing" => FinishAction::Nothing,
            "sleep" => FinishAction::Sleep,
            "shut_down" => FinishAction::ShutDown,
            _ => return None,
        })
    }
}

/// A queue's daily times, in minutes after local midnight (0 to 1439): at
/// `start` the queue starts, at `stop` it stops. Either may be absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct DailyWindow {
    /// When the queue starts each day.
    pub start: Option<u16>,
    /// When the queue stops each day.
    pub stop: Option<u16>,
}

impl DailyWindow {
    /// Whether neither time is set.
    pub fn is_empty(self) -> bool {
        self.start.is_none() && self.stop.is_none()
    }

    fn validate(self) -> Result<()> {
        let ok = |m: Option<u16>| m.is_none_or(|m| m < MINUTES_PER_DAY);
        if !ok(self.start) || !ok(self.stop) || (self.start.is_some() && self.start == self.stop) {
            return Err(Error::QueueRefused(QueueProblem::Window));
        }
        Ok(())
    }
}

/// A queue as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Queue {
    /// Its id.
    pub id: QueueId,
    /// Its name; `None` for the default queue until the user renames it
    /// (front ends show their own word for it, [`DEFAULT_QUEUE_NAME`] in
    /// English).
    pub name: Option<String>,
    /// Whether this is the default queue.
    pub is_default: bool,
    /// Downloads it runs at once (1 to [`MAX_CONCURRENT`]).
    pub max_concurrent: u32,
    /// Whether it starts waiting downloads as slots free.
    pub started: bool,
    /// Its daily start and stop times.
    pub window: DailyWindow,
    /// What to do when it finishes.
    pub on_finish: FinishAction,
}

impl Queue {
    /// The name to show where no front end names the default queue.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(DEFAULT_QUEUE_NAME)
    }
}

/// Changes to a queue's settings for
/// [`Engine::update_queue`](crate::Engine::update_queue); `None` keeps a value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueChange {
    /// A new name (trimmed; not empty, unique ignoring case).
    pub name: Option<String>,
    /// A new limit (1 to [`MAX_CONCURRENT`]).
    pub max_concurrent: Option<u32>,
    /// New daily times. Changing them never acts on a time already passed
    /// today: the next one counts.
    pub window: Option<DailyWindow>,
    /// A new finish action.
    pub on_finish: Option<FinishAction>,
}

/// Why a queue change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueueProblem {
    /// The name is empty.
    NameEmpty,
    /// Another queue has this name.
    NameTaken,
    /// The name is longer than [`MAX_NAME_CHARS`].
    NameTooLong,
    /// The limit is outside 1 to [`MAX_CONCURRENT`].
    Limit,
    /// A daily time is out of range, or start and stop are the same.
    Window,
    /// The default queue cannot be deleted.
    DefaultQueue,
}

impl QueueProblem {
    /// The message-catalog key.
    pub fn message_key(self) -> &'static str {
        match self {
            QueueProblem::NameEmpty => "error.queue.name_empty",
            QueueProblem::NameTaken => "error.queue.name_taken",
            QueueProblem::NameTooLong => "error.queue.name_too_long",
            QueueProblem::Limit => "error.queue.limit",
            QueueProblem::Window => "error.queue.window",
            QueueProblem::DefaultQueue => "error.queue.default",
        }
    }
}

impl fmt::Display for QueueProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            QueueProblem::NameEmpty => "a queue needs a name",
            QueueProblem::NameTaken => "another queue has that name",
            QueueProblem::NameTooLong => "the queue name is too long",
            QueueProblem::Limit => "a queue runs 1 to 32 downloads at once",
            QueueProblem::Window => "the daily start and stop times are not valid",
            QueueProblem::DefaultQueue => "the main queue cannot be deleted",
        })
    }
}

/// A clean queue name, or why it is not one.
pub(crate) fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::QueueRefused(QueueProblem::NameEmpty));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(Error::QueueRefused(QueueProblem::NameTooLong));
    }
    Ok(name.to_owned())
}

pub(crate) fn check_limit(n: u32) -> Result<u32> {
    if (1..=MAX_CONCURRENT).contains(&n) {
        Ok(n)
    } else {
        Err(Error::QueueRefused(QueueProblem::Limit))
    }
}

pub(crate) fn check_window(w: DailyWindow) -> Result<DailyWindow> {
    w.validate().map(|()| w)
}

/// Where a download moves within its queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    /// One place earlier.
    Up,
    /// One place later.
    Down,
    /// First.
    Top,
    /// Last.
    Bottom,
}

/// Where the scheduler gets the time: the system clock, or a clock a test
/// moves by hand.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Now.
    fn now(&self) -> SystemTime;
    /// The local time's offset from UTC at `at`, in seconds east.
    fn utc_offset(&self, at: SystemTime) -> i32;
}

/// The system's clock and time zone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn utc_offset(&self, at: SystemTime) -> i32 {
        use chrono::{Local, Offset, TimeZone};
        let ms = to_ms(at);
        Local
            .timestamp_millis_opt(ms)
            .single()
            .map_or(0, |t| t.offset().fix().local_minus_utc())
    }
}

/// Unix milliseconds of `t` (0 before 1970).
pub(crate) fn to_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

const MINUTES_PER_DAY: u16 = 24 * 60;
const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

/// The times (Unix ms) at which `minute` falls on the local days around
/// `now`: yesterday, today and tomorrow.
fn around(now: i64, offset_s: i32, minute: u16) -> [i64; 3] {
    let offset = i64::from(offset_s) * 1000;
    let midnight = (now + offset).div_euclid(DAY_MS) * DAY_MS - offset;
    let at = midnight + i64::from(minute) * MINUTE_MS;
    [at - DAY_MS, at, at + DAY_MS]
}

/// The most recent daily start or stop at or before `now`: its time and
/// whether it is a start.
pub(crate) fn last_boundary(now: i64, offset_s: i32, w: DailyWindow) -> Option<(i64, bool)> {
    let mut best: Option<(i64, bool)> = None;
    for (minute, is_start) in [(w.start, true), (w.stop, false)] {
        let Some(minute) = minute else { continue };
        for at in around(now, offset_s, minute) {
            if at <= now && best.is_none_or(|(b, _)| at > b) {
                best = Some((at, is_start));
            }
        }
    }
    best
}

/// The next daily start or stop after `now`.
pub(crate) fn next_boundary(now: i64, offset_s: i32, w: DailyWindow) -> Option<i64> {
    [w.start, w.stop]
        .into_iter()
        .flatten()
        .flat_map(|m| around(now, offset_s, m))
        .filter(|&at| at > now)
        .min()
}

/// A queue as the scheduler sees it.
#[derive(Debug, Clone)]
pub(crate) struct PlanQueue {
    pub(crate) queue: Queue,
    /// The last daily start or stop acted on.
    pub(crate) window_applied: Option<i64>,
}

/// A download that is not completed, as the scheduler sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanRow {
    pub(crate) id: DownloadId,
    pub(crate) state: State,
    pub(crate) queue: QueueId,
    pub(crate) position: i64,
    pub(crate) start_at: Option<i64>,
}

/// A daily start or stop to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowStep {
    pub(crate) queue: QueueId,
    /// The queue's new state; `None` records the time only.
    pub(crate) started: Option<bool>,
    pub(crate) applied: i64,
}

/// What one scheduler pass does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Downloads to start, in this order.
    pub(crate) start: Vec<DownloadId>,
    /// Daily starts and stops that came due.
    pub(crate) windows: Vec<WindowStep>,
    /// When to look again (Unix ms), if anything is timed.
    pub(crate) wake_at: Option<i64>,
}

fn earliest(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// One scheduler pass over the queues and the downloads that are not
/// completed (`rows`, in queue order), given the downloads running now.
///
/// - A daily start or stop that came due since the last one acted on
///   changes the queue's state (only the latest counts: a stop that came
///   after a start while the engine was closed wins).
/// - A waiting download whose start time has come starts, whatever its
///   queue does: the user picked the time.
/// - A started queue then fills its free slots with its waiting downloads
///   in order (those with a start time wait for it).
pub(crate) fn plan(
    queues: &[PlanQueue],
    rows: &[PlanRow],
    running: &HashSet<DownloadId>,
    now: i64,
    offset_s: i32,
) -> Plan {
    let mut out = Plan::default();
    let mut started: HashMap<QueueId, bool> = HashMap::new();
    for pq in queues {
        let q = &pq.queue;
        let mut on = q.started;
        if !q.window.is_empty() {
            match (last_boundary(now, offset_s, q.window), pq.window_applied) {
                (Some((at, is_start)), Some(applied)) if at > applied => {
                    on = is_start;
                    out.windows.push(WindowStep {
                        queue: q.id,
                        started: Some(is_start),
                        applied: at,
                    });
                }
                (_, None) => out.windows.push(WindowStep {
                    queue: q.id,
                    started: None,
                    applied: now,
                }),
                _ => {}
            }
            out.wake_at = earliest(out.wake_at, next_boundary(now, offset_s, q.window));
        }
        started.insert(q.id, on);
    }

    let waiting = |r: &&PlanRow| r.state == State::Queued && !running.contains(&r.id);
    for r in rows.iter().filter(waiting) {
        if let Some(at) = r.start_at {
            if at <= now {
                out.start.push(r.id);
            } else {
                out.wake_at = earliest(out.wake_at, Some(at));
            }
        }
    }

    let mut busy: HashMap<QueueId, u32> = HashMap::new();
    for r in rows {
        if running.contains(&r.id) || out.start.contains(&r.id) {
            *busy.entry(r.queue).or_default() += 1;
        }
    }
    for pq in queues {
        let q = &pq.queue;
        if !started.get(&q.id).copied().unwrap_or(false) {
            continue;
        }
        let mut free = q
            .max_concurrent
            .saturating_sub(busy.get(&q.id).copied().unwrap_or(0));
        for r in rows
            .iter()
            .filter(waiting)
            .filter(|r| r.queue == q.id && r.start_at.is_none())
        {
            if free == 0 {
                break;
            }
            out.start.push(r.id);
            free -= 1;
        }
    }
    out
}

/// Whether `queue` has finished: nothing in it runs or waits (a download
/// with a start time still waits), and nothing is about to start.
pub(crate) fn is_finished(
    queue: QueueId,
    rows: &[PlanRow],
    running: &HashSet<DownloadId>,
    starting: &[DownloadId],
) -> bool {
    !rows.iter().any(|r| {
        r.queue == queue
            && (r.state == State::Queued || running.contains(&r.id) || starting.contains(&r.id))
    })
}

/// Whether a download that stopped in `state` ended on its own, which is
/// what finishes a queue (a pause or a cancel by the user does not).
pub(crate) fn ended_on_its_own(state: State) -> bool {
    matches!(
        state,
        State::Completed | State::Failed | State::Corrupt | State::NeedsNewLink
    )
}

/// The order of a queue after moving `ids` before `before` (`None`: to the
/// end). `order` is the queue's current order; ids not in it are added.
pub(crate) fn place(
    order: &[DownloadId],
    ids: &[DownloadId],
    before: Option<DownloadId>,
) -> Vec<DownloadId> {
    let moving: HashSet<DownloadId> = ids.iter().copied().collect();
    let mut rest: Vec<DownloadId> = order
        .iter()
        .copied()
        .filter(|id| !moving.contains(id))
        .collect();
    let at = before
        .filter(|b| !moving.contains(b))
        .and_then(|b| rest.iter().position(|&id| id == b))
        .unwrap_or(rest.len());
    let mut unique = Vec::with_capacity(ids.len());
    for &id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    rest.splice(at..at, unique);
    rest
}

/// The order of a queue after shifting `id`, among the downloads that take
/// part in the order (`order`, which holds it).
pub(crate) fn shift(order: &[DownloadId], id: DownloadId, by: Shift) -> Vec<DownloadId> {
    let mut out = order.to_vec();
    let Some(i) = out.iter().position(|&x| x == id) else {
        return out;
    };
    match by {
        Shift::Up if i > 0 => out.swap(i, i - 1),
        Shift::Down if i + 1 < out.len() => out.swap(i, i + 1),
        Shift::Top => {
            let x = out.remove(i);
            out.insert(0, x);
        }
        Shift::Bottom => {
            let x = out.remove(i);
            out.push(x);
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    fn queue(id: u64, max: u32, started: bool) -> PlanQueue {
        PlanQueue {
            queue: Queue {
                id: QueueId(id),
                name: None,
                is_default: id == 1,
                max_concurrent: max,
                started,
                window: DailyWindow::default(),
                on_finish: FinishAction::Nothing,
            },
            window_applied: None,
        }
    }

    fn row(id: u64, queue: u64, state: State) -> PlanRow {
        PlanRow {
            id: DownloadId(id),
            state,
            queue: QueueId(queue),
            position: i64::try_from(id).unwrap(),
            start_at: None,
        }
    }

    fn ids(v: &[u64]) -> Vec<DownloadId> {
        v.iter().copied().map(DownloadId).collect()
    }

    #[test]
    fn a_started_queue_fills_its_free_slots_in_order() {
        let queues = [queue(1, 2, true)];
        let mut rows: Vec<PlanRow> = (1..=6).map(|i| row(i, 1, State::Queued)).collect();
        // Order is by position, not id.
        rows[4].position = 0;
        rows.sort_by_key(|r| r.position);
        let p = plan(&queues, &rows, &HashSet::new(), 0, 0);
        assert_eq!(p.start, ids(&[5, 1]));
        // One runs: one slot left.
        let running: HashSet<_> = ids(&[5]).into_iter().collect();
        rows[0].state = State::Downloading;
        let p = plan(&queues, &rows, &running, 0, 0);
        assert_eq!(p.start, ids(&[1]));
        assert_eq!(p.wake_at, None);
    }

    #[test]
    fn stopped_queues_and_paused_downloads_wait() {
        let queues = [queue(1, 3, false), queue(2, 1, true)];
        let rows = [
            row(1, 1, State::Queued),
            row(2, 2, State::Paused),
            row(3, 2, State::Failed),
            row(4, 2, State::Queued),
            row(5, 2, State::Queued),
        ];
        let p = plan(&queues, &rows, &HashSet::new(), 0, 0);
        assert_eq!(p.start, ids(&[4]));
    }

    #[test]
    fn a_download_started_outside_the_order_takes_a_slot() {
        // Download 3 was started by hand beyond the limit: nothing more starts.
        let queues = [queue(1, 2, true)];
        let rows = [
            row(1, 1, State::Downloading),
            row(2, 1, State::Downloading),
            row(3, 1, State::Downloading),
            row(4, 1, State::Queued),
        ];
        let running: HashSet<_> = ids(&[1, 2, 3]).into_iter().collect();
        assert_eq!(plan(&queues, &rows, &running, 0, 0).start, ids(&[]));
    }

    #[test]
    fn start_times_start_when_due_whatever_the_queue_does() {
        let queues = [queue(1, 1, false)];
        let mut rows = [row(1, 1, State::Queued), row(2, 1, State::Queued)];
        rows[0].start_at = Some(10 * H);
        rows[1].start_at = Some(20 * H);
        let p = plan(&queues, &rows, &HashSet::new(), 9 * H, 0);
        assert_eq!(p.start, ids(&[]));
        assert_eq!(p.wake_at, Some(10 * H));
        let p = plan(&queues, &rows, &HashSet::new(), 10 * H, 0);
        assert_eq!(p.start, ids(&[1]));
        assert_eq!(p.wake_at, Some(20 * H));
        // Long after both: both start, even beyond the limit.
        let p = plan(&queues, &rows, &HashSet::new(), 100 * H, 0);
        assert_eq!(p.start, ids(&[1, 2]));
    }

    #[test]
    fn a_timed_download_does_not_take_its_turn_early() {
        let queues = [queue(1, 1, true)];
        let mut rows = [row(1, 1, State::Queued), row(2, 1, State::Queued)];
        rows[0].start_at = Some(10 * H);
        let p = plan(&queues, &rows, &HashSet::new(), 0, 0);
        assert_eq!(p.start, ids(&[2]));
    }

    #[test]
    fn boundaries_follow_the_local_day() {
        // 09:00 to 17:00 in UTC+2. "Now" is 2024-01-02 10:00 local
        // (08:00 UTC), as Unix ms of some day.
        let day = 19_724 * DAY_MS; // 2024-01-02 00:00 UTC
        let offset = 2 * 3600;
        let now = day + 8 * H;
        let w = DailyWindow {
            start: Some(9 * 60),
            stop: Some(17 * 60),
        };
        // Today's 09:00 local is 07:00 UTC.
        assert_eq!(last_boundary(now, offset, w), Some((day + 7 * H, true)));
        // Next: today's 17:00 local, 15:00 UTC.
        assert_eq!(next_boundary(now, offset, w), Some(day + 15 * H));
        // At 01:00 local the latest is yesterday's stop.
        let night = day - H;
        assert_eq!(
            last_boundary(night, offset, w),
            Some((day - DAY_MS + 15 * H, false))
        );
        // A window across midnight: 23:00 to 06:00, at 02:00 UTC (no offset).
        let w = DailyWindow {
            start: Some(23 * 60),
            stop: Some(6 * 60),
        };
        assert_eq!(last_boundary(day + 2 * H, 0, w), Some((day - H, true)));
    }

    #[test]
    fn daily_times_start_and_stop_once_and_survive_a_restart() {
        let w = DailyWindow {
            start: Some(9 * 60),
            stop: Some(17 * 60),
        };
        let day = 100 * DAY_MS;
        let mut q = queue(2, 1, false);
        q.queue.window = w;
        q.window_applied = Some(day - DAY_MS);
        let rows = [row(1, 2, State::Queued)];
        // 10:00: today's 09:00 came due: the queue starts and its download too.
        let p = plan(&[q.clone()], &rows, &HashSet::new(), day + 10 * H, 0);
        assert_eq!(
            p.windows,
            vec![WindowStep {
                queue: QueueId(2),
                started: Some(true),
                applied: day + 9 * H
            }]
        );
        assert_eq!(p.start, ids(&[1]));
        assert_eq!(p.wake_at, Some(day + 17 * H));
        // Applied: the user stops it by hand at 11:00; it stays stopped.
        q.window_applied = Some(day + 9 * H);
        q.queue.started = false;
        let p = plan(&[q.clone()], &rows, &HashSet::new(), day + 11 * H, 0);
        assert!(p.windows.is_empty() && p.start.is_empty());
        // Closed from 08:00 to 18:00: the stop at 17:00 is the latest, so it
        // stays stopped although the start passed too.
        q.window_applied = Some(day + 8 * H - DAY_MS);
        q.queue.started = true;
        let p = plan(&[q.clone()], &rows, &HashSet::new(), day + 18 * H, 0);
        assert_eq!(p.windows[0].started, Some(false));
        assert_eq!(p.start, ids(&[]));
        // A window set just now acts on nothing that already passed.
        q.window_applied = None;
        let p = plan(&[q], &rows, &HashSet::new(), day + 10 * H, 0);
        assert_eq!(p.windows[0].started, None);
        assert_eq!(p.windows[0].applied, day + 10 * H);
    }

    #[test]
    fn finishing_needs_nothing_running_or_waiting() {
        let rows = [
            row(1, 1, State::Paused),
            row(2, 1, State::Failed),
            row(3, 2, State::Queued),
        ];
        assert!(is_finished(QueueId(1), &rows, &HashSet::new(), &[]));
        assert!(!is_finished(QueueId(2), &rows, &HashSet::new(), &[]));
        let running: HashSet<_> = ids(&[1]).into_iter().collect();
        assert!(!is_finished(QueueId(1), &rows, &running, &[]));
        assert!(!is_finished(QueueId(1), &rows, &HashSet::new(), &ids(&[2])));
        assert!(ended_on_its_own(State::Completed) && ended_on_its_own(State::Failed));
        assert!(!ended_on_its_own(State::Paused) && !ended_on_its_own(State::Cancelled));
    }

    #[test]
    fn placing_and_shifting() {
        let order = ids(&[1, 2, 3, 4, 5]);
        assert_eq!(
            place(&order, &ids(&[4, 2]), Some(DownloadId(1))),
            ids(&[4, 2, 1, 3, 5])
        );
        assert_eq!(place(&order, &ids(&[1]), None), ids(&[2, 3, 4, 5, 1]));
        // Before one of the moved ids: the end.
        assert_eq!(
            place(&order, &ids(&[2, 3]), Some(DownloadId(3))),
            ids(&[1, 4, 5, 2, 3])
        );
        // From another queue.
        assert_eq!(
            place(&order, &ids(&[9]), Some(DownloadId(2))),
            ids(&[1, 9, 2, 3, 4, 5])
        );
        assert_eq!(shift(&order, DownloadId(1), Shift::Up), order);
        assert_eq!(
            shift(&order, DownloadId(2), Shift::Up),
            ids(&[2, 1, 3, 4, 5])
        );
        assert_eq!(shift(&order, DownloadId(5), Shift::Down), order);
        assert_eq!(
            shift(&order, DownloadId(3), Shift::Top),
            ids(&[3, 1, 2, 4, 5])
        );
        assert_eq!(
            shift(&order, DownloadId(3), Shift::Bottom),
            ids(&[1, 2, 4, 5, 3])
        );
    }

    #[test]
    fn names_limits_and_windows_are_checked() {
        assert_eq!(clean_name("  Night ").unwrap(), "Night");
        assert!(matches!(
            clean_name("  "),
            Err(Error::QueueRefused(QueueProblem::NameEmpty))
        ));
        assert!(clean_name(&"x".repeat(41)).is_err());
        assert!(check_limit(0).is_err() && check_limit(33).is_err() && check_limit(3).is_ok());
        let w = |start, stop| DailyWindow { start, stop };
        assert!(check_window(w(Some(60), Some(60))).is_err());
        assert!(check_window(w(Some(1440), None)).is_err());
        assert!(check_window(w(None, Some(0))).is_ok());
        assert_eq!(
            FinishAction::parse("shut_down"),
            Some(FinishAction::ShutDown)
        );
    }
}
