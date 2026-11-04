//! The backend's copy of the list, and the coalescing of everything that
//! changes into one batch per tick (docs/ARCHITECTURE.md, "UI to backend").
//!
//! Engine events and command results mark rows dirty; progress is sampled
//! once per tick for running downloads only. [`Model::tick`] turns all of it
//! into at most one [`EngineMessage::Batch`]: each changed row once, progress
//! only for downloads whose numbers moved, and the file map only for the
//! focused download. With 500 downloads of which 20 run, a tick costs one
//! lock per running download and one message to the page.
//!
//! This is plain data with no I/O, so it is tested directly; `hub.rs` drives
//! it from the engine.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use super::dto::{
    DownloadError, DownloadEvent, DownloadId, DownloadRow, DownloadState, EngineMessage,
    ProgressItem, SegmentSpan,
};
use super::speed::SpeedMeter;

/// A progress reading of one running download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sample {
    pub(crate) id: DownloadId,
    pub(crate) downloaded: u64,
    pub(crate) total: Option<u64>,
    pub(crate) connections: u32,
    pub(crate) segments: Vec<SegmentSpan>,
}

/// What the page last received for a running download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sent {
    downloaded: u64,
    total: Option<u64>,
    bps: u64,
    connections: u32,
}

#[derive(Debug)]
struct Entry {
    row: DownloadRow,
    meter: SpeedMeter,
    sent: Option<Sent>,
    segments_sent: Option<Vec<SegmentSpan>>,
}

impl Entry {
    fn new(row: DownloadRow) -> Self {
        Self {
            row,
            meter: SpeedMeter::default(),
            sent: None,
            segments_sent: None,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Model {
    rows: BTreeMap<DownloadId, Entry>,
    dirty: BTreeSet<DownloadId>,
    removed: Vec<DownloadId>,
    events: Vec<DownloadEvent>,
    focus: Option<DownloadId>,
}

impl Model {
    /// Replaces everything with a fresh list (startup, or a resync) and drops
    /// whatever was pending: a snapshot follows.
    pub(crate) fn load(&mut self, rows: impl IntoIterator<Item = DownloadRow>) {
        self.rows = rows.into_iter().map(|r| (r.id, Entry::new(r))).collect();
        self.discard_pending();
    }

    /// All rows, oldest first.
    pub(crate) fn snapshot(&self) -> Vec<DownloadRow> {
        self.rows.values().map(|e| e.row.clone()).collect()
    }

    pub(crate) fn row(&self, id: DownloadId) -> Option<&DownloadRow> {
        self.rows.get(&id).map(|e| &e.row)
    }

    /// Every row by id, for the aggregate the tray and taskbar show.
    pub(crate) fn rows(&self) -> BTreeMap<DownloadId, &DownloadRow> {
        self.rows.iter().map(|(id, e)| (*id, &e.row)).collect()
    }

    /// Downloads that have live progress to sample.
    pub(crate) fn active_ids(&self) -> Vec<DownloadId> {
        self.rows
            .values()
            .filter(|e| e.row.state.is_active())
            .map(|e| e.row.id)
            .collect()
    }

    /// Forgets what was pending, after a snapshot made it redundant. The
    /// page's copy of the progress is now the snapshot's, so the next tick
    /// compares against that.
    pub(crate) fn discard_pending(&mut self) {
        self.dirty.clear();
        self.removed.clear();
        self.events.clear();
        for e in self.rows.values_mut() {
            e.sent = Some(Sent {
                downloaded: e.row.downloaded,
                total: e.row.total,
                bps: e.row.bps,
                connections: e.row.connections,
            });
            e.segments_sent = None;
        }
    }

    fn touch(&mut self, id: DownloadId) -> Option<&mut DownloadRow> {
        let entry = self.rows.get_mut(&id)?;
        self.dirty.insert(id);
        Some(&mut entry.row)
    }

    /// A row read again from the database (after a link refresh, say).
    pub(crate) fn replace(&mut self, mut row: DownloadRow) {
        let id = row.id;
        if let Some(e) = self.rows.get_mut(&id) {
            if row.state.is_active() {
                // Keep the live numbers; the database lags behind them.
                row.downloaded = row.downloaded.max(e.row.downloaded);
                row.bps = e.row.bps;
                row.connections = e.row.connections;
            }
            if e.row != row {
                e.row = row;
                self.dirty.insert(id);
            }
        } else {
            self.rows.insert(id, Entry::new(row));
            self.dirty.insert(id);
        }
    }

    pub(crate) fn added(&mut self, row: DownloadRow) {
        let id = row.id;
        if let std::collections::btree_map::Entry::Vacant(slot) = self.rows.entry(id) {
            slot.insert(Entry::new(row));
            self.dirty.insert(id);
            self.events.push(DownloadEvent::Added { id });
        }
    }

    pub(crate) fn probed(
        &mut self,
        id: DownloadId,
        file_name: Option<String>,
        path: String,
        total: Option<u64>,
        resumable: bool,
    ) {
        if let Some(row) = self.touch(id) {
            if file_name.is_some() {
                row.file_name = file_name;
            }
            row.path = Some(path);
            row.total = total;
            row.resumable = resumable;
        }
    }

    pub(crate) fn state_changed(
        &mut self,
        id: DownloadId,
        state: DownloadState,
        error: Option<DownloadError>,
    ) {
        let Some(entry) = self.rows.get_mut(&id) else {
            return;
        };
        let row = &mut entry.row;
        if row.state == state && row.error == error {
            return;
        }
        row.state = state;
        match (&error, state) {
            (Some(_), _) => row.error.clone_from(&error),
            (None, DownloadState::Paused | DownloadState::Cancelled) => {}
            (None, _) => row.error = None,
        }
        if !state.is_active() {
            row.bps = 0;
            row.connections = 0;
            entry.meter.reset();
        }
        self.dirty.insert(id);
        self.events
            .push(DownloadEvent::StateChanged { id, state, error });
    }

    pub(crate) fn resumed(&mut self, id: DownloadId, downloaded: u64, total: Option<u64>) {
        if let Some(entry) = self.rows.get_mut(&id) {
            entry.meter.reset();
            entry.row.downloaded = downloaded;
            entry.row.total = total;
            self.dirty.insert(id);
        }
    }

    pub(crate) fn completed(&mut self, id: DownloadId, path: String, now_ms: u64) {
        let Some(entry) = self.rows.get_mut(&id) else {
            return;
        };
        let row = &mut entry.row;
        row.state = DownloadState::Completed;
        if let Some(total) = row.total {
            row.downloaded = total;
        } else {
            row.total = Some(row.downloaded);
        }
        row.file_name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .or(row.file_name.take());
        row.path = Some(path.clone());
        row.bps = 0;
        row.connections = 0;
        row.error = None;
        row.completed_at = Some(now_ms);
        entry.meter.reset();
        self.dirty.insert(id);
        self.events.push(DownloadEvent::Completed { id, path });
    }

    /// The segment table changed (a split); the focused download's map is
    /// sent on the next tick even if the received counts did not move.
    pub(crate) fn segments_changed(&mut self, id: DownloadId) {
        if self.focus == Some(id)
            && let Some(e) = self.rows.get_mut(&id)
        {
            e.segments_sent = None;
        }
    }

    pub(crate) fn removed(&mut self, id: DownloadId) {
        if self.rows.remove(&id).is_some() {
            self.dirty.remove(&id);
            self.removed.push(id);
            self.events.push(DownloadEvent::Removed { id });
        }
        if self.focus == Some(id) {
            self.focus = None;
        }
    }

    pub(crate) fn set_speed_limit(&mut self, id: DownloadId, limit: Option<u64>) {
        if self.row(id).is_some_and(|r| r.speed_limit != limit)
            && let Some(row) = self.touch(id)
        {
            row.speed_limit = limit;
        }
    }

    /// The download whose file map streams with its progress.
    pub(crate) fn set_focus(&mut self, id: Option<DownloadId>) {
        if self.focus != id {
            if let Some(e) = id.and_then(|id| self.rows.get_mut(&id)) {
                e.segments_sent = None;
            }
            self.focus = id;
        }
    }

    #[cfg(test)]
    pub(crate) fn focus(&self) -> Option<DownloadId> {
        self.focus
    }

    /// Applies one tick's progress samples and returns everything that
    /// changed since the last tick as one batch (`None`: nothing changed).
    pub(crate) fn tick(&mut self, samples: Vec<Sample>, now: Instant) -> Option<EngineMessage> {
        let mut progress = Vec::new();
        for sample in samples {
            let Some(entry) = self.rows.get_mut(&sample.id) else {
                continue;
            };
            if !entry.row.state.is_active() {
                // A sample can race the state change that ended the run.
                continue;
            }
            let bps = entry.meter.record(now, sample.downloaded);
            let row = &mut entry.row;
            row.downloaded = sample.downloaded;
            if sample.total.is_some() {
                row.total = sample.total;
            }
            row.bps = bps;
            row.connections = sample.connections;
            let now_sent = Sent {
                downloaded: row.downloaded,
                total: row.total,
                bps,
                connections: row.connections,
            };
            let segments = (self.focus == Some(sample.id)
                && entry.segments_sent.as_ref() != Some(&sample.segments))
            .then(|| {
                entry.segments_sent = Some(sample.segments.clone());
                sample.segments
            });
            if self.dirty.contains(&sample.id) && segments.is_none() {
                // The row goes out whole in this batch, numbers included.
                continue;
            }
            if entry.sent != Some(now_sent) || segments.is_some() {
                entry.sent = Some(now_sent);
                progress.push(ProgressItem {
                    id: sample.id,
                    downloaded: now_sent.downloaded,
                    total: now_sent.total,
                    bps,
                    connections: now_sent.connections,
                    segments,
                });
            }
        }

        let rows: Vec<DownloadRow> = std::mem::take(&mut self.dirty)
            .into_iter()
            .filter_map(|id| {
                let e = self.rows.get_mut(&id)?;
                // The row carries the latest numbers; progress compares to them.
                e.sent = Some(Sent {
                    downloaded: e.row.downloaded,
                    total: e.row.total,
                    bps: e.row.bps,
                    connections: e.row.connections,
                });
                Some(e.row.clone())
            })
            .collect();
        let removed = std::mem::take(&mut self.removed);
        let events = std::mem::take(&mut self.events);
        if rows.is_empty() && removed.is_empty() && progress.is_empty() && events.is_empty() {
            return None;
        }
        Some(EngineMessage::Batch {
            rows,
            removed,
            progress,
            events,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn row(id: u64, state: DownloadState) -> DownloadRow {
        DownloadRow {
            id: DownloadId(id),
            state,
            url: format!("https://example.com/{id}.bin"),
            file_name: None,
            path: None,
            total: Some(1_000_000),
            downloaded: 0,
            bps: 0,
            connections: 0,
            resumable: true,
            speed_limit: None,
            error: None,
            created_at: 1,
            completed_at: None,
        }
    }

    fn sample(id: u64, downloaded: u64) -> Sample {
        Sample {
            id: DownloadId(id),
            downloaded,
            total: Some(1_000_000),
            connections: 4,
            segments: vec![SegmentSpan {
                start: 0,
                end: Some(1_000_000),
                received: downloaded,
                connection: Some(0),
            }],
        }
    }

    fn batch(
        msg: Option<EngineMessage>,
    ) -> (
        Vec<DownloadRow>,
        Vec<DownloadId>,
        Vec<ProgressItem>,
        Vec<DownloadEvent>,
    ) {
        match msg {
            Some(EngineMessage::Batch {
                rows,
                removed,
                progress,
                events,
            }) => (rows, removed, progress, events),
            other => panic!("expected a batch, got {other:?}"),
        }
    }

    #[test]
    fn quiet_tick_sends_nothing() {
        let mut m = Model::default();
        m.load([
            row(1, DownloadState::Paused),
            row(2, DownloadState::Completed),
        ]);
        assert_eq!(m.tick(Vec::new(), Instant::now()), None);
        assert_eq!(m.active_ids(), Vec::<DownloadId>::new());
    }

    #[test]
    fn many_changes_coalesce_into_one_batch() {
        let mut m = Model::default();
        m.load((1..=500).map(|i| row(i, DownloadState::Paused)));
        let t = Instant::now();
        // Twenty downloads start; each goes through several states and
        // events in the same tick.
        for i in 1..=20 {
            let id = DownloadId(i);
            m.state_changed(id, DownloadState::Probing, None);
            m.probed(
                id,
                Some(format!("{i}.bin")),
                format!("C:\\d\\{i}.bin"),
                Some(1_000_000),
                true,
            );
            m.state_changed(id, DownloadState::Downloading, None);
        }
        let samples: Vec<Sample> = (1..=20).map(|i| sample(i, 1000 * i)).collect();
        let (rows, removed, progress, events) = batch(m.tick(samples, t));
        assert_eq!(rows.len(), 20, "each changed row once");
        assert!(rows.iter().all(|r| r.state == DownloadState::Downloading));
        assert_eq!(rows[0].file_name.as_deref(), Some("1.bin"));
        assert_eq!(removed, Vec::<DownloadId>::new());
        assert_eq!(events.len(), 40, "two state changes each");
        // Progress that the rows already carry is not repeated.
        assert!(progress.is_empty(), "{progress:?}");
        assert_eq!(m.active_ids().len(), 20);

        // Next tick: progress only, only for downloads that moved, no rows.
        let samples: Vec<Sample> = (1..=20)
            .map(|i| {
                sample(
                    i,
                    if i <= 10 {
                        1000 * i + 250_000
                    } else {
                        1000 * i
                    },
                )
            })
            .collect();
        let (rows, _, progress, events) = batch(m.tick(samples, t + Duration::from_millis(250)));
        assert!(rows.is_empty() && events.is_empty());
        // The first ten moved; the other ten have a speed of zero, which is
        // also unchanged (they had no speed yet).
        assert_eq!(progress.len(), 10);
        assert!(
            progress
                .iter()
                .all(|p| p.bps == 1_000_000 && p.segments.is_none())
        );
    }

    #[test]
    fn unchanged_progress_is_not_resent() {
        let mut m = Model::default();
        m.load([row(1, DownloadState::Downloading)]);
        let t = Instant::now();
        let (_, _, p, _) = batch(m.tick(vec![sample(1, 0)], t));
        assert_eq!(p.len(), 1, "connections went from 0 to 4");
        assert_eq!(
            m.tick(vec![sample(1, 0)], t + Duration::from_millis(250)),
            None
        );
    }

    #[test]
    fn segments_only_for_the_focused_download_and_only_when_changed() {
        let mut m = Model::default();
        m.load([
            row(1, DownloadState::Downloading),
            row(2, DownloadState::Downloading),
        ]);
        m.set_focus(Some(DownloadId(2)));
        let t = Instant::now();
        let (_, _, p, _) = batch(m.tick(vec![sample(1, 10), sample(2, 10)], t));
        assert!(p[0].segments.is_none());
        assert!(p[1].segments.is_some());
        // Unchanged: no segments, and nothing at all for download 2 except
        // its speed.
        let (_, _, p, _) = batch(m.tick(
            vec![sample(1, 20), sample(2, 10)],
            t + Duration::from_millis(250),
        ));
        assert!(p.iter().all(|p| p.segments.is_none()), "{p:?}");
        // A split forces the map out again.
        m.segments_changed(DownloadId(2));
        let (_, _, p, _) = batch(m.tick(
            vec![sample(1, 20), sample(2, 10)],
            t + Duration::from_millis(500),
        ));
        let two = p.iter().find(|p| p.id == DownloadId(2)).unwrap();
        assert!(two.segments.is_some());
        // Changing focus sends the new one's map.
        m.set_focus(Some(DownloadId(1)));
        let (_, _, p, _) = batch(m.tick(
            vec![sample(1, 20), sample(2, 10)],
            t + Duration::from_millis(750),
        ));
        let one = p.iter().find(|p| p.id == DownloadId(1)).unwrap();
        assert!(one.segments.is_some());
    }

    #[test]
    fn stopping_clears_speed_and_keeps_the_error() {
        let mut m = Model::default();
        m.load([row(1, DownloadState::Downloading)]);
        let t = Instant::now();
        m.tick(vec![sample(1, 0)], t);
        m.tick(vec![sample(1, 500_000)], t + Duration::from_millis(500));
        assert_eq!(m.row(DownloadId(1)).unwrap().bps, 1_000_000);
        let error = DownloadError {
            kind: super::super::dto::ErrorKind::Permanent,
            key: "error.http.not_found".into(),
            message: "HTTP 404".into(),
        };
        m.state_changed(DownloadId(1), DownloadState::Failed, Some(error.clone()));
        // A late sample after the run ended is ignored.
        let (rows, _, progress, events) =
            batch(m.tick(vec![sample(1, 600_000)], t + Duration::from_millis(750)));
        assert_eq!(progress, Vec::<ProgressItem>::new());
        assert_eq!(rows[0].bps, 0);
        assert_eq!(rows[0].connections, 0);
        assert_eq!(rows[0].error.as_ref(), Some(&error));
        assert_eq!(
            events,
            vec![DownloadEvent::StateChanged {
                id: DownloadId(1),
                state: DownloadState::Failed,
                error: Some(error.clone()),
            }]
        );
        // Pausing keeps the last error; downloading again clears it.
        m.state_changed(DownloadId(1), DownloadState::Paused, None);
        assert!(m.row(DownloadId(1)).unwrap().error.is_some());
        m.state_changed(DownloadId(1), DownloadState::Downloading, None);
        assert!(m.row(DownloadId(1)).unwrap().error.is_none());
    }

    #[test]
    fn added_completed_removed() {
        let mut m = Model::default();
        m.added(row(7, DownloadState::Queued));
        m.added(row(7, DownloadState::Queued));
        m.completed(
            DownloadId(7),
            std::path::Path::new("d")
                .join("file.iso")
                .to_string_lossy()
                .into_owned(),
            99,
        );
        let (rows, _, _, events) = batch(m.tick(Vec::new(), Instant::now()));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, DownloadState::Completed);
        assert_eq!(rows[0].file_name.as_deref(), Some("file.iso"));
        assert_eq!(rows[0].downloaded, 1_000_000);
        assert_eq!(rows[0].completed_at, Some(99));
        assert_eq!(events.len(), 2, "added once, then completed");

        m.set_focus(Some(DownloadId(7)));
        m.removed(DownloadId(7));
        m.removed(DownloadId(7));
        let (rows, removed, _, events) = batch(m.tick(Vec::new(), Instant::now()));
        assert_eq!(rows, Vec::<DownloadRow>::new());
        assert_eq!(removed, vec![DownloadId(7)]);
        assert_eq!(events, vec![DownloadEvent::Removed { id: DownloadId(7) }]);
        assert_eq!(m.focus(), None);
    }

    #[test]
    fn snapshot_makes_pending_changes_redundant() {
        let mut m = Model::default();
        m.load([row(1, DownloadState::Paused)]);
        m.state_changed(DownloadId(1), DownloadState::Downloading, None);
        m.set_speed_limit(DownloadId(1), Some(1000));
        assert_eq!(m.snapshot()[0].speed_limit, Some(1000));
        m.discard_pending();
        assert_eq!(m.tick(Vec::new(), Instant::now()), None);
    }

    #[test]
    fn replace_keeps_live_numbers_of_a_running_download() {
        let mut m = Model::default();
        m.load([row(1, DownloadState::Downloading)]);
        let t = Instant::now();
        m.tick(vec![sample(1, 0)], t);
        m.tick(vec![sample(1, 500_000)], t + Duration::from_millis(500));
        let mut stale = row(1, DownloadState::Downloading);
        stale.downloaded = 100;
        stale.url = "https://example.com/new".into();
        m.replace(stale);
        let r = m.row(DownloadId(1)).unwrap();
        assert_eq!(r.downloaded, 500_000);
        assert_eq!(r.bps, 1_000_000);
        assert_eq!(r.url, "https://example.com/new");
    }
}
