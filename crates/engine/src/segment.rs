//! The segment table and dynamic segmentation rules.
//!
//! A download of known size `n` is a set of disjoint half-open byte ranges
//! (segments) whose union is `[0, n)`. Each segment has a receive cursor
//! (`start + received`) and a durability cursor (`start + durable`). Workers
//! *claim* bytes at their cursor before handing them to the writer; splitting
//! shrinks a segment's `end` under the same lock. Because claims and splits are
//! linearised by the table, every byte is claimed (and therefore written) by
//! exactly one worker. See `docs/ENGINE.md`, "Dynamic segmentation".
//!
//! The table is pure logic with no I/O, so it is property-tested directly.
//! It is also the unit that checkpointing persists: [`SegmentTable::checkpoint`]
//! takes it apart (from durable offsets only) and
//! [`SegmentTable::from_checkpoint`] puts it back together on resume.

use serde::Serialize;

/// Identifies a worker (one connection) within a download.
pub type WorkerId = u32;

/// One byte range of the download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Segment {
    /// Stable id, unique within the download.
    pub idx: u32,
    /// First byte (inclusive).
    pub start: u64,
    /// One past the last byte (exclusive). Shrinks when the segment is split.
    /// `u64::MAX` while the total size is unknown.
    pub end: u64,
    /// Bytes claimed from the network (`start + received` is the receive cursor).
    pub received: u64,
    /// Bytes known to be durable on disk (`durable <= received`).
    pub durable: u64,
    /// The worker currently responsible for this segment.
    pub worker: Option<WorkerId>,
}

impl Segment {
    /// The receive cursor: the next byte this segment needs.
    pub fn cursor(&self) -> u64 {
        self.start + self.received
    }

    /// Bytes not yet received.
    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.cursor())
    }

    /// Whether every byte of the segment has been received.
    pub fn is_received(&self) -> bool {
        self.cursor() >= self.end
    }
}

/// What a checkpoint stores for one segment.
///
/// `done` is taken from [`Segment::durable`], never from `received`, which is what
/// upholds the checkpoint invariant: persisted progress never exceeds durable bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SegmentCheckpoint {
    /// Segment id.
    pub idx: u32,
    /// First byte.
    pub start: u64,
    /// End (exclusive).
    pub end: u64,
    /// Durable bytes from `start`.
    pub done: u64,
}

/// Tunables for splitting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitConfig {
    /// Boundary alignment (design target 64 KiB).
    pub align: u64,
    /// Smallest segment worth a new connection (design target 1 MiB).
    pub min_split_size: u64,
}

impl Default for SplitConfig {
    fn default() -> Self {
        Self {
            align: 64 * 1024,
            min_split_size: 1024 * 1024,
        }
    }
}

/// Why a claim was rejected. Any of these is an engine bug or a stale worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaimError {
    /// The segment does not exist (any more).
    UnknownSegment,
    /// The worker does not own the segment.
    NotOwner,
    /// The chunk does not start at the receive cursor.
    NotAtCursor { cursor: u64, offset: u64 },
}

/// Work handed to a worker by [`SegmentTable::next_work`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Work {
    /// Download segment `idx` from its cursor to its end.
    Segment { idx: u32 },
    /// Endgame: request `[from, to)` of segment `of` as a duplicate and take it
    /// over with [`SegmentTable::split_at`] when the first bytes arrive.
    Duplicate { of: u32, from: u64, to: u64 },
}

#[derive(Debug, Clone)]
struct Entry {
    seg: Segment,
    /// Estimated bytes the owner has already pulled off the network beyond its
    /// claim cursor, or will before it learns about a split (coalescing buffer
    /// plus throughput × reaction time).
    ahead: u64,
    /// Smoothed throughput of the owner, bytes per second.
    rate: f64,
    /// Endgame already issued a duplicate for this segment.
    duplicated: bool,
}

/// The per-download segment table.
#[derive(Debug, Clone)]
pub struct SegmentTable {
    total: Option<u64>,
    splittable: bool,
    entries: Vec<Entry>,
    next_idx: u32,
    cfg: SplitConfig,
}

/// Rounds `x` down to a multiple of `align` (`align` of 0 or 1 is a no-op).
pub(crate) fn align_down(x: u64, align: u64) -> u64 {
    if align <= 1 { x } else { x - x % align }
}

impl SegmentTable {
    /// Creates the initial split of `[0, total)` into at most `k` segments, with
    /// interior boundaries aligned down to `cfg.align`. The last segment ends at
    /// `total`. `total == 0` yields an empty, already complete table.
    pub fn new_split(total: u64, k: u32, cfg: SplitConfig) -> Self {
        let mut table = Self {
            total: Some(total),
            splittable: true,
            entries: Vec::new(),
            next_idx: 0,
            cfg,
        };
        if total == 0 {
            return table;
        }
        let k = u64::from(k.max(1));
        let mut bounds = vec![0u64];
        for i in 1..k {
            // i * total / k without overflow for any u64 total.
            let b = (u128::from(i) * u128::from(total) / u128::from(k)) as u64;
            let b = align_down(b, cfg.align);
            if b > *bounds.last().expect("bounds starts non-empty") && b < total {
                bounds.push(b);
            }
        }
        bounds.push(total);
        for w in bounds.windows(2) {
            table.push(w[0], w[1]);
        }
        table
    }

    /// A single segment `[0, total)` that is never split (no range support).
    pub fn new_single(total: u64, cfg: SplitConfig) -> Self {
        let mut t = Self::new_split(total, 1, cfg);
        t.splittable = false;
        t
    }

    /// A single open-ended segment for a resource of unknown size.
    pub fn new_open(cfg: SplitConfig) -> Self {
        let mut t = Self {
            total: None,
            splittable: false,
            entries: Vec::new(),
            next_idx: 0,
            cfg,
        };
        t.push(0, u64::MAX);
        t
    }

    /// Rebuilds a table from a persisted checkpoint (resume). Every segment
    /// restarts at its durable cursor (`received = durable = done`) with no
    /// worker. A fully done segment is merged into the one after it, which keeps
    /// each segment's progress contiguous from its start and stops the table from
    /// growing over many resume cycles; ids are renumbered from 0.
    ///
    /// `total` is `None` for a resource of unknown size, whose checkpoint must be
    /// a single open segment from 0. Fails if the checkpoint is not a valid
    /// partition of `[0, total)`.
    pub fn from_checkpoint(
        total: Option<u64>,
        splittable: bool,
        checkpoint: &[SegmentCheckpoint],
        cfg: SplitConfig,
    ) -> Result<Self, String> {
        let mut cps = checkpoint.to_vec();
        cps.sort_by_key(|c| c.start);
        let mut table = Self {
            total,
            splittable: splittable && total.is_some(),
            entries: Vec::new(),
            next_idx: 0,
            cfg,
        };
        let Some(total) = total else {
            return match cps.as_slice() {
                [c] if c.start == 0 && c.end == u64::MAX => {
                    table.push_with(0, u64::MAX, c.done);
                    Ok(table)
                }
                _ => Err("an unknown-size checkpoint must be one open segment".into()),
            };
        };
        let mut expect = 0u64;
        for c in &cps {
            if c.start != expect || c.end <= c.start || c.done > c.end - c.start {
                return Err(format!(
                    "invalid checkpoint segment [{}, {}) done {} (expected start {expect})",
                    c.start, c.end, c.done
                ));
            }
            expect = c.end;
        }
        if expect != total {
            return Err(format!(
                "checkpoint covers [0, {expect}) instead of [0, {total})"
            ));
        }
        let mut pending: Option<(u64, u64, u64)> = None;
        for c in &cps {
            pending = Some(match pending {
                // The previous range is complete: extend it over this one.
                Some((start, end, done)) if done == end - start => (start, c.end, done + c.done),
                Some((start, end, done)) => {
                    table.push_with(start, end, done);
                    (c.start, c.end, c.done)
                }
                None => (c.start, c.end, c.done),
            });
        }
        if let Some((start, end, done)) = pending {
            table.push_with(start, end, done);
        }
        table.check_invariants()?;
        Ok(table)
    }

    fn push(&mut self, start: u64, end: u64) -> u32 {
        self.push_with(start, end, 0)
    }

    /// Adds a segment whose first `done` bytes are already received and durable.
    fn push_with(&mut self, start: u64, end: u64, done: u64) -> u32 {
        let idx = self.next_idx;
        self.next_idx += 1;
        self.entries.push(Entry {
            seg: Segment {
                idx,
                start,
                end,
                received: done,
                durable: done,
                worker: None,
            },
            ahead: 0,
            rate: 0.0,
            duplicated: false,
        });
        idx
    }

    fn entry(&self, idx: u32) -> Option<&Entry> {
        self.entries.iter().find(|e| e.seg.idx == idx)
    }

    fn entry_mut(&mut self, idx: u32) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.seg.idx == idx)
    }

    /// Total size, if known.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// Whether segments may be split (range support and known size).
    pub fn is_splittable(&self) -> bool {
        self.splittable
    }

    /// Disables splitting (for example after the server stopped honouring ranges).
    pub fn set_splittable(&mut self, splittable: bool) {
        self.splittable = splittable && self.total.is_some();
    }

    /// The split tunables.
    pub fn config(&self) -> SplitConfig {
        self.cfg
    }

    /// A segment by id.
    pub fn get(&self, idx: u32) -> Option<&Segment> {
        self.entry(idx).map(|e| &e.seg)
    }

    /// All segments, ordered by `start`.
    pub fn segments(&self) -> Vec<Segment> {
        let mut v: Vec<Segment> = self.entries.iter().map(|e| e.seg.clone()).collect();
        v.sort_by_key(|s| s.start);
        v
    }

    /// Sum of received bytes over all segments.
    pub fn received_total(&self) -> u64 {
        self.entries.iter().map(|e| e.seg.received).sum()
    }

    /// Sum of durable bytes over all segments.
    pub fn durable_total(&self) -> u64 {
        self.entries.iter().map(|e| e.seg.durable).sum()
    }

    /// Number of segments that have a worker.
    pub fn active_workers(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.seg.worker.is_some())
            .count()
    }

    /// Whether every byte has been received. Always false while the size is unknown.
    pub fn is_complete(&self) -> bool {
        self.total.is_some() && self.entries.iter().all(|e| e.seg.is_received())
    }

    /// Whether every byte has been received *and* is durable.
    pub fn is_durable(&self) -> bool {
        self.is_complete()
            && self
                .entries
                .iter()
                .all(|e| e.seg.durable == e.seg.end - e.seg.start)
    }

    /// The segment `worker` currently owns, if any.
    pub fn owned_by(&self, worker: WorkerId) -> Option<u32> {
        self.entries
            .iter()
            .find(|e| e.seg.worker == Some(worker) && !e.seg.is_received())
            .map(|e| e.seg.idx)
    }

    /// Assigns an unowned, unfinished segment to `worker`. Returns false if the
    /// segment is unknown, finished, or owned by someone else.
    pub fn assign(&mut self, idx: u32, worker: WorkerId) -> bool {
        match self.entry_mut(idx) {
            Some(e) if !e.seg.is_received() && e.seg.worker.is_none_or(|w| w == worker) => {
                e.seg.worker = Some(worker);
                true
            }
            _ => false,
        }
    }

    /// Releases every segment owned by `worker` (it finished, failed or exited).
    pub fn release(&mut self, worker: WorkerId) {
        for e in &mut self.entries {
            if e.seg.worker == Some(worker) {
                e.seg.worker = None;
                e.ahead = 0;
                e.rate = 0.0;
            }
        }
    }

    /// Finds work for an idle `worker`: an unassigned unfinished segment first,
    /// otherwise a split of the largest remaining active segment. The returned
    /// segment is already assigned to `worker`.
    pub(crate) fn next_work(&mut self, worker: WorkerId) -> Option<Work> {
        self.release(worker);
        if let Some(idx) = self
            .entries
            .iter()
            .filter(|e| e.seg.worker.is_none() && !e.seg.is_received())
            .min_by_key(|e| e.seg.start)
            .map(|e| e.seg.idx)
        {
            self.assign(idx, worker);
            return Some(Work::Segment { idx });
        }
        self.split_largest(worker).map(|idx| Work::Segment { idx })
    }

    /// The split rule from `docs/ENGINE.md`: pick the active segment with the most
    /// remaining bytes, skip the bytes its owner will receive before reacting
    /// (`ahead`), and split the rest in half (aligned), if each half is at least
    /// `min_split_size`. Returns the new segment, assigned to `worker`.
    pub(crate) fn split_largest(&mut self, worker: WorkerId) -> Option<u32> {
        if !self.splittable {
            return None;
        }
        let mut candidates: Vec<(u64, u32)> = self
            .entries
            .iter()
            .filter(|e| e.seg.worker.is_some() && e.seg.worker != Some(worker))
            .map(|e| (e.seg.remaining(), e.seg.idx))
            .filter(|(r, _)| *r > 0)
            .collect();
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        for (_, idx) in candidates {
            let e = self.entry(idx).expect("candidate exists");
            let effective = e.seg.remaining().saturating_sub(e.ahead);
            if effective < 2 * self.cfg.min_split_size {
                continue;
            }
            let mid = align_down(e.seg.cursor() + e.ahead + effective / 2, self.cfg.align);
            if let Some(new) = self.split_at(idx, mid, worker) {
                return Some(new);
            }
        }
        None
    }

    /// Splits segment `idx` at `mid`: the segment keeps `[start, mid)` and a new
    /// segment `[mid, old_end)` is created and assigned to `worker`.
    ///
    /// Fails (returns `None`) if `mid` is not strictly between the segment's receive
    /// cursor and its end: bytes already claimed are never given away.
    pub(crate) fn split_at(&mut self, idx: u32, mid: u64, worker: WorkerId) -> Option<u32> {
        self.total?;
        let e = self.entry_mut(idx)?;
        if mid <= e.seg.cursor() || mid >= e.seg.end {
            return None;
        }
        let old_end = e.seg.end;
        e.seg.end = mid;
        let new = self.push(mid, old_end);
        self.assign(new, worker);
        Some(new)
    }

    /// Endgame candidate for an idle `worker` (see `docs/ENGINE.md`, "Endgame"):
    /// only when nothing is unassigned or splittable, the slowest active segment
    /// would take longer than `threshold_secs`, and more than twice the median of
    /// the others. At most one duplicate per segment.
    pub(crate) fn endgame_work(&mut self, threshold_secs: f64) -> Option<Work> {
        if !self.splittable {
            return None;
        }
        let etas: Vec<(f64, u32)> = self
            .entries
            .iter()
            .filter(|e| e.seg.worker.is_some() && !e.seg.is_received())
            .map(|e| {
                let eta = if e.rate > 0.0 {
                    e.seg.remaining() as f64 / e.rate
                } else {
                    f64::INFINITY
                };
                (eta, e.seg.idx)
            })
            .collect();
        let (slow_eta, slow_idx) = etas.iter().copied().max_by(|a, b| a.0.total_cmp(&b.0))?;
        let mut others: Vec<f64> = etas
            .iter()
            .filter(|(_, i)| *i != slow_idx)
            .map(|(eta, _)| *eta)
            .collect();
        others.sort_by(f64::total_cmp);
        let median = others.get(others.len() / 2).copied().unwrap_or(0.0);
        if slow_eta <= threshold_secs || slow_eta <= 2.0 * median {
            return None;
        }
        let align = self.cfg.align;
        let e = self.entry_mut(slow_idx)?;
        if e.duplicated {
            return None;
        }
        let from = align_down(e.seg.cursor() + e.seg.remaining() / 2, align);
        if from <= e.seg.cursor() || from >= e.seg.end {
            return None;
        }
        e.duplicated = true;
        Some(Work::Duplicate {
            of: slow_idx,
            from,
            to: e.seg.end,
        })
    }

    /// The current end of segment `idx` (it may shrink at any time).
    pub fn end_of(&self, idx: u32) -> Option<u64> {
        self.entry(idx).map(|e| e.seg.end)
    }

    /// Claims `len` bytes at `offset` for `worker` in segment `idx`.
    ///
    /// Returns how many bytes were accepted: fewer than `len` when the segment
    /// was shortened by a split (the caller discards the rest). The caller must
    /// write exactly the accepted bytes at `offset`.
    pub(crate) fn claim(
        &mut self,
        idx: u32,
        worker: WorkerId,
        offset: u64,
        len: u64,
    ) -> Result<u64, ClaimError> {
        let e = self.entry_mut(idx).ok_or(ClaimError::UnknownSegment)?;
        if e.seg.worker != Some(worker) {
            return Err(ClaimError::NotOwner);
        }
        let cursor = e.seg.cursor();
        if offset != cursor {
            return Err(ClaimError::NotAtCursor { cursor, offset });
        }
        let accepted = len.min(e.seg.end - cursor);
        e.seg.received += accepted;
        Ok(accepted)
    }

    /// Updates the owner's throughput estimate and look-ahead for split decisions.
    pub(crate) fn update_stats(&mut self, idx: u32, rate: f64, ahead: u64) {
        if let Some(e) = self.entry_mut(idx) {
            e.rate = rate;
            e.ahead = ahead;
        }
    }

    /// Records that the file is durable up to absolute offset `upto` for segment
    /// `idx`. Durability never exceeds what was received and never goes backwards.
    pub(crate) fn mark_durable(&mut self, idx: u32, upto: u64) {
        if let Some(e) = self.entry_mut(idx) {
            let d = upto.saturating_sub(e.seg.start).min(e.seg.received);
            e.seg.durable = e.seg.durable.max(d);
        }
    }

    /// Ends the open segment of an unknown-size download at its cursor, fixing the
    /// total size.
    pub(crate) fn finish_open(&mut self, idx: u32) -> Option<u64> {
        if self.total.is_some() {
            return self.total;
        }
        let e = self.entry_mut(idx)?;
        e.seg.end = e.seg.cursor();
        let total = e.seg.end;
        self.total = Some(total);
        Some(total)
    }

    /// Throws away all progress of segment `idx` and replaces it with a fresh
    /// segment over the same range, owned by `worker`. Used when a non-resumable
    /// download has to start over. Returns the new segment id.
    pub(crate) fn restart(&mut self, idx: u32, worker: WorkerId) -> Option<u32> {
        let pos = self.entries.iter().position(|e| e.seg.idx == idx)?;
        let old = self.entries.remove(pos);
        let new = self.push(old.seg.start, old.seg.end);
        self.assign(new, worker);
        Some(new)
    }

    /// What a checkpoint would persist right now.
    pub fn checkpoint(&self) -> Vec<SegmentCheckpoint> {
        self.segments()
            .into_iter()
            .map(|s| SegmentCheckpoint {
                idx: s.idx,
                start: s.start,
                end: s.end,
                done: s.durable,
            })
            .collect()
    }

    /// Checks the segment-table invariants from `docs/ENGINE.md`:
    /// disjoint segments covering `[0, n)` (when `n` is known) and
    /// `start + durable <= start + received <= end`.
    pub fn check_invariants(&self) -> Result<(), String> {
        let segs = self.segments();
        for s in &segs {
            if s.durable > s.received {
                return Err(format!("segment {} durable > received", s.idx));
            }
            if s.start > s.end || s.received > s.end - s.start {
                return Err(format!("segment {} received past end", s.idx));
            }
        }
        let Some(total) = self.total else {
            return if segs.len() == 1 && segs[0].start == 0 {
                Ok(())
            } else {
                Err("open table must have exactly one segment from 0".into())
            };
        };
        let mut expect = 0u64;
        for s in &segs {
            if s.start != expect {
                return Err(format!(
                    "gap or overlap at {expect}: segment {} starts at {}",
                    s.idx, s.start
                ));
            }
            if s.end <= s.start {
                return Err(format!("segment {} is empty", s.idx));
            }
            expect = s.end;
        }
        if expect != total {
            return Err(format!(
                "segments cover [0, {expect}) instead of [0, {total})"
            ));
        }
        let mut owners: Vec<WorkerId> = self
            .entries
            .iter()
            .filter(|e| !e.seg.is_received())
            .filter_map(|e| e.seg.worker)
            .collect();
        owners.sort_unstable();
        if owners.windows(2).any(|w| w[0] == w[1]) {
            return Err("a worker owns two unfinished segments".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;

    #[test]
    fn initial_split_aligned() {
        let t = SegmentTable::new_split(100 * MIB + 7, 8, SplitConfig::default());
        let segs = t.segments();
        assert_eq!(segs.len(), 8);
        assert_eq!(segs.last().unwrap().end, 100 * MIB + 7);
        for s in &segs[1..] {
            assert_eq!(s.start % (64 * KIB), 0);
        }
        t.check_invariants().unwrap();
    }

    #[test]
    fn initial_split_tiny_file_collapses() {
        let t = SegmentTable::new_split(10 * KIB, 8, SplitConfig::default());
        assert_eq!(t.segments().len(), 1);
        t.check_invariants().unwrap();
        let empty = SegmentTable::new_split(0, 8, SplitConfig::default());
        assert!(empty.is_complete());
        empty.check_invariants().unwrap();
    }

    #[test]
    fn split_largest_halves_effective_remaining() {
        let cfg = SplitConfig::default();
        let mut t = SegmentTable::new_split(64 * MIB, 1, cfg);
        assert_eq!(t.next_work(0), Some(Work::Segment { idx: 0 }));
        t.update_stats(0, 0.0, 2 * MIB);
        let new = t.split_largest(1).unwrap();
        let s0 = t.get(0).unwrap().clone();
        let s1 = t.get(new).unwrap().clone();
        // cursor 0 + ahead 2 MiB + (62 MiB / 2) = 33 MiB
        assert_eq!(s0.end, 33 * MIB);
        assert_eq!(s1.start, 33 * MIB);
        assert_eq!(s1.end, 64 * MIB);
        assert_eq!(s1.worker, Some(1));
        t.check_invariants().unwrap();
    }

    #[test]
    fn no_split_below_twice_min_split() {
        let cfg = SplitConfig::default();
        let mut t = SegmentTable::new_split(2 * MIB - 1, 1, cfg);
        t.next_work(0);
        assert_eq!(t.split_largest(1), None);
        assert_eq!(t.next_work(1), None);
    }

    #[test]
    fn claims_are_truncated_after_split() {
        let cfg = SplitConfig {
            align: 1,
            min_split_size: 10,
        };
        let mut t = SegmentTable::new_split(100, 1, cfg);
        t.next_work(0);
        assert_eq!(t.claim(0, 0, 0, 30), Ok(30));
        let new = t.split_at(0, 40, 1).unwrap();
        assert_eq!(t.claim(0, 0, 30, 30), Ok(10));
        assert_eq!(t.claim(0, 0, 40, 1), Ok(0));
        assert_eq!(t.claim(new, 0, 40, 1), Err(ClaimError::NotOwner));
        assert_eq!(
            t.claim(new, 1, 41, 1),
            Err(ClaimError::NotAtCursor {
                cursor: 40,
                offset: 41
            })
        );
        assert_eq!(t.claim(new, 1, 40, 100), Ok(60));
        assert!(t.is_complete());
        // Cannot split behind the cursor.
        assert_eq!(t.split_at(0, 20, 2), None);
    }

    #[test]
    fn unassigned_before_split() {
        let mut t = SegmentTable::new_split(64 * MIB, 4, SplitConfig::default());
        for w in 0..4 {
            assert!(matches!(t.next_work(w), Some(Work::Segment { .. })));
        }
        t.release(2);
        assert_eq!(t.next_work(9), Some(Work::Segment { idx: 2 }));
    }

    #[test]
    fn open_table_finishes_at_cursor() {
        let mut t = SegmentTable::new_open(SplitConfig::default());
        t.next_work(0);
        assert_eq!(t.claim(0, 0, 0, 500), Ok(500));
        assert!(!t.is_complete());
        assert_eq!(t.split_largest(1), None);
        assert_eq!(t.finish_open(0), Some(500));
        assert!(t.is_complete());
        t.check_invariants().unwrap();
    }

    #[test]
    fn restart_resets_progress() {
        let mut t = SegmentTable::new_single(1000, SplitConfig::default());
        t.next_work(0);
        t.claim(0, 0, 0, 400).unwrap();
        t.mark_durable(0, 400);
        let new = t.restart(0, 0).unwrap();
        assert_eq!(t.received_total(), 0);
        assert_eq!(t.durable_total(), 0);
        assert_eq!(t.claim(new, 0, 0, 1000), Ok(1000));
        t.check_invariants().unwrap();
    }

    #[test]
    fn durable_is_clamped_and_monotonic() {
        let mut t = SegmentTable::new_single(1000, SplitConfig::default());
        t.next_work(0);
        t.claim(0, 0, 0, 100).unwrap();
        t.mark_durable(0, 5000);
        assert_eq!(t.get(0).unwrap().durable, 100);
        t.mark_durable(0, 50);
        assert_eq!(t.get(0).unwrap().durable, 100);
        assert_eq!(t.checkpoint()[0].done, 100);
    }

    #[test]
    fn endgame_targets_slow_segment_once() {
        let cfg = SplitConfig {
            align: 1,
            min_split_size: 1000,
        };
        let mut t = SegmentTable::new_split(3000, 3, cfg);
        for w in 0..3 {
            t.next_work(w);
        }
        t.update_stats(0, 1000.0, 0);
        t.update_stats(1, 1000.0, 0);
        t.update_stats(2, 10.0, 0); // 1000 bytes at 10 B/s = 100 s
        let w = t.endgame_work(2.0).unwrap();
        let Work::Duplicate { of, from, to } = w else {
            panic!("expected duplicate")
        };
        assert_eq!(of, 2);
        assert_eq!((from, to), (2500, 3000));
        assert_eq!(t.endgame_work(2.0), None, "one duplicate per segment");
        let new = t.split_at(of, from, 3).unwrap();
        assert_eq!(t.get(new).unwrap().worker, Some(3));
        t.check_invariants().unwrap();
    }

    #[test]
    fn checkpoint_round_trip_merges_done_segments() {
        let cfg = SplitConfig {
            align: 1,
            min_split_size: 10,
        };
        let cp = |idx, start, end, done| SegmentCheckpoint {
            idx,
            start,
            end,
            done,
        };
        let cps = [
            cp(4, 0, 100, 100),
            cp(7, 100, 200, 30),
            cp(2, 200, 300, 100),
            cp(9, 300, 400, 100),
            cp(1, 400, 500, 0),
        ];
        let t = SegmentTable::from_checkpoint(Some(500), true, &cps, cfg).unwrap();
        let segs = t.segments();
        let shape: Vec<(u64, u64, u64)> =
            segs.iter().map(|s| (s.start, s.end, s.received)).collect();
        assert_eq!(shape, vec![(0, 200, 130), (200, 500, 200)]);
        assert!(
            segs.iter()
                .all(|s| s.durable == s.received && s.worker.is_none())
        );
        assert_eq!(t.durable_total(), 330);
        assert!(t.is_splittable());
        // Everything done: one complete segment.
        let all = [cp(0, 0, 10, 10), cp(1, 10, 20, 10)];
        let t = SegmentTable::from_checkpoint(Some(20), true, &all, cfg).unwrap();
        assert!(t.is_durable());
        assert_eq!(t.segments().len(), 1);
    }

    #[test]
    fn invalid_checkpoints_are_rejected() {
        let cfg = SplitConfig::default();
        let cp = |start, end, done| SegmentCheckpoint {
            idx: 0,
            start,
            end,
            done,
        };
        let bad: [&[SegmentCheckpoint]; 5] = [
            &[],
            &[cp(0, 10, 0)],
            &[cp(0, 50, 0), cp(60, 100, 0)],
            &[cp(0, 60, 0), cp(50, 100, 0)],
            &[cp(0, 100, 101)],
        ];
        for b in bad {
            assert!(
                SegmentTable::from_checkpoint(Some(100), true, b, cfg).is_err(),
                "{b:?}"
            );
        }
        let open = [cp(0, u64::MAX, 77)];
        let t = SegmentTable::from_checkpoint(None, true, &open, cfg).unwrap();
        assert!(!t.is_splittable());
        assert_eq!(t.get(0).unwrap().cursor(), 77);
        assert!(SegmentTable::from_checkpoint(None, false, &[cp(0, 100, 0)], cfg).is_err());
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        #[derive(Debug, Clone)]
        enum Op {
            /// Worker `w` receives `len` bytes on its segment.
            Receive { w: u32, len: u64 },
            /// Worker `w` asks for work (may split).
            Idle { w: u32 },
            /// Worker `w` fails and releases its segment.
            Fail { w: u32 },
            /// The writer flushes everything claimed so far.
            Flush,
            /// Worker `w` sets its look-ahead estimate.
            Ahead { w: u32, ahead: u64 },
            /// An explicit split of `w`'s segment at a fraction of its remainder.
            SplitAt { w: u32, by: u32, frac: u8 },
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                6 => (0..6u32, 1..5_000u64).prop_map(|(w, len)| Op::Receive { w, len }),
                3 => (0..6u32).prop_map(|w| Op::Idle { w }),
                1 => (0..6u32).prop_map(|w| Op::Fail { w }),
                1 => Just(Op::Flush),
                1 => (0..6u32, 0..3_000u64).prop_map(|(w, ahead)| Op::Ahead { w, ahead }),
                1 => (0..6u32, 0..6u32, 0..=255u8).prop_map(|(w, by, frac)| Op::SplitAt { w, by, frac }),
            ]
        }

        fn byte_at(offset: u64) -> u8 {
            (offset.wrapping_mul(2_654_435_761) >> 13) as u8
        }

        /// Simulated file: each byte records how many times it was written.
        struct Model {
            data: Vec<u8>,
            writes: Vec<u8>,
            claimed: Vec<(u32, u64)>, // (segment, absolute end) for the flush model
        }

        impl Model {
            fn write(&mut self, offset: u64, len: u64) {
                for o in offset..offset + len {
                    self.data[o as usize] = byte_at(o);
                    self.writes[o as usize] += 1;
                }
            }
        }

        fn apply(t: &mut SegmentTable, m: &mut Model, op: &Op) {
            match *op {
                Op::Receive { w, len } => {
                    if let Some(idx) = t.owned_by(w) {
                        let cursor = t.get(idx).unwrap().cursor();
                        let accepted = t.claim(idx, w, cursor, len).unwrap();
                        assert!(accepted <= len);
                        m.write(cursor, accepted);
                        m.claimed.push((idx, cursor + accepted));
                    }
                }
                Op::Idle { w } => {
                    if t.owned_by(w).is_none() {
                        t.next_work(w);
                    }
                }
                Op::Fail { w } => t.release(w),
                Op::Flush => {
                    for (idx, upto) in m.claimed.drain(..) {
                        t.mark_durable(idx, upto);
                    }
                }
                Op::Ahead { w, ahead } => {
                    if let Some(idx) = t.owned_by(w) {
                        t.update_stats(idx, 1.0, ahead);
                    }
                }
                Op::SplitAt { w, by, frac } => {
                    if let (Some(idx), None) = (t.owned_by(w), t.owned_by(by))
                        && w != by
                    {
                        let s = t.get(idx).unwrap().clone();
                        let mid = s.cursor() + s.remaining() * u64::from(frac) / 255;
                        let _ = t.split_at(idx, mid, by);
                    }
                }
            }
        }

        /// Simulates a crash: keep only what the checkpoint calls durable,
        /// rebuild the table from it.
        fn crash(
            t: &mut SegmentTable,
            m: &mut Model,
            cfg: SplitConfig,
        ) -> Result<(), TestCaseError> {
            let cps = t.checkpoint();
            let total = t.total().unwrap_or(0) as usize;
            let mut durable = vec![false; total];
            for c in &cps {
                for o in c.start..c.start + c.done {
                    durable[o as usize] = true;
                }
            }
            for (o, d) in durable.iter().enumerate() {
                if *d {
                    prop_assert_eq!(m.data[o], byte_at(o as u64), "durable byte {} is wrong", o);
                } else {
                    m.data[o] = 0xAA;
                }
            }
            m.claimed.clear();
            let before = t.durable_total();
            *t = SegmentTable::from_checkpoint(t.total(), t.is_splittable(), &cps, cfg)
                .map_err(TestCaseError::fail)?;
            prop_assert_eq!(t.durable_total(), before);
            prop_assert_eq!(t.received_total(), before);
            Ok(())
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(512),
                ..ProptestConfig::default()
            })]

            #[test]
            fn invariants_hold_and_file_is_exact(
                total in 0u64..60_000,
                k in 1u32..8,
                align in prop_oneof![Just(1u64), Just(16), Just(64)],
                min_split in 1u64..2_000,
                ops in proptest::collection::vec(op(), 0..300),
            ) {
                let cfg = SplitConfig { align, min_split_size: min_split };
                let mut t = SegmentTable::new_split(total, k, cfg);
                let mut m = Model {
                    data: vec![0; total as usize],
                    writes: vec![0; total as usize],
                    claimed: Vec::new(),
                };
                t.check_invariants().map_err(TestCaseError::fail)?;
                for op in &ops {
                    let before = t.received_total();
                    apply(&mut t, &mut m, op);
                    t.check_invariants().map_err(TestCaseError::fail)?;
                    prop_assert!(t.received_total() >= before, "received bytes were lost");
                    prop_assert!(t.durable_total() <= t.received_total());
                    // Checkpoint never claims more than is durable.
                    let segs = t.segments();
                    for (cp, s) in t.checkpoint().iter().zip(segs.iter()) {
                        prop_assert!(cp.done <= s.durable);
                    }
                }
                // The original crew fails; a fresh crew drives the download to completion.
                for w in 0..6u32 {
                    apply(&mut t, &mut m, &Op::Fail { w });
                }
                let mut guard = 0;
                while !t.is_complete() {
                    guard += 1;
                    prop_assert!(guard < 100_000, "no progress towards completion");
                    for w in 10..14u32 {
                        if t.owned_by(w).is_none() {
                            t.next_work(w);
                        }
                        apply(&mut t, &mut m, &Op::Receive { w, len: 4_096 });
                    }
                    t.check_invariants().map_err(TestCaseError::fail)?;
                }
                apply(&mut t, &mut m, &Op::Flush);
                prop_assert!(t.is_durable());
                prop_assert_eq!(t.received_total(), total);
                for (o, &c) in m.writes.iter().enumerate() {
                    prop_assert_eq!(c, 1, "byte {} written {} times", o, c);
                    prop_assert_eq!(m.data[o], byte_at(o as u64));
                }
            }

            /// Crashes at random points: the table is rebuilt from its last
            /// checkpoint and every byte that was not durable is lost (garbage),
            /// as after a power failure. Durable bytes must survive intact, and
            /// the download must still end byte-exact.
            #[test]
            fn crash_and_resume_from_checkpoint(
                total in 1u64..60_000,
                k in 1u32..8,
                min_split in 1u64..2_000,
                ops in proptest::collection::vec(
                    prop_oneof![9 => op().prop_map(Some), 1 => Just(None)],
                    0..300,
                ),
            ) {
                let cfg = SplitConfig { align: 16, min_split_size: min_split };
                let mut t = SegmentTable::new_split(total, k, cfg);
                let mut m = Model {
                    data: vec![0xAA; total as usize],
                    writes: vec![0; total as usize],
                    claimed: Vec::new(),
                };
                let mut crashes = 0u32;
                for op in ops.iter().map(Option::as_ref).chain(std::iter::once(None)) {
                    if let Some(op) = op {
                        apply(&mut t, &mut m, op);
                    } else {
                        crash(&mut t, &mut m, cfg)?;
                        crashes += 1;
                    }
                    t.check_invariants().map_err(TestCaseError::fail)?;
                }
                let mut guard = 0;
                while !t.is_complete() {
                    guard += 1;
                    prop_assert!(guard < 100_000, "no progress towards completion");
                    for w in 10..14u32 {
                        if t.owned_by(w).is_none() {
                            t.next_work(w);
                        }
                        apply(&mut t, &mut m, &Op::Receive { w, len: 4_096 });
                    }
                    t.check_invariants().map_err(TestCaseError::fail)?;
                }
                apply(&mut t, &mut m, &Op::Flush);
                prop_assert!(t.is_durable());
                prop_assert!(crashes >= 1);
                for o in 0..total as usize {
                    prop_assert_eq!(m.data[o], byte_at(o as u64), "byte {} wrong after resume", o);
                }
            }

            #[test]
            fn initial_split_covers_everything(total in 0u64..u64::MAX / 2, k in 1u32..64) {
                let t = SegmentTable::new_split(total, k, SplitConfig::default());
                t.check_invariants().map_err(TestCaseError::fail)?;
                prop_assert!(t.segments().len() <= k as usize);
            }

            #[test]
            fn split_never_loses_bytes(
                total in 1u64..1_000_000,
                claimed_frac in 0u8..=255,
                ahead in 0u64..100_000,
                min_split in 1u64..50_000,
            ) {
                let cfg = SplitConfig { align: 64, min_split_size: min_split };
                let mut t = SegmentTable::new_split(total, 1, cfg);
                t.next_work(0);
                let claim = total * u64::from(claimed_frac) / 255;
                t.claim(0, 0, 0, claim).unwrap();
                t.update_stats(0, 1.0, ahead);
                let before: u64 = t.segments().iter().map(|s| s.end - s.start).sum();
                if let Some(new) = t.split_largest(1) {
                    let s = t.get(new).unwrap();
                    prop_assert!(s.end - s.start >= min_split);
                    prop_assert!(s.start >= claim);
                }
                let after: u64 = t.segments().iter().map(|s| s.end - s.start).sum();
                prop_assert_eq!(before, after);
                prop_assert_eq!(t.received_total(), claim);
                t.check_invariants().map_err(TestCaseError::fail)?;
            }
        }
    }
}
