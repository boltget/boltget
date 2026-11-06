//! What the app shows outside its window (docs/APP-DESIGN.md #13, #14, #16):
//! the aggregate the tray tooltip and the taskbar button show, and the
//! moments worth a notification.
//!
//! **The batch.** The taskbar button shows the downloads that ran since the
//! user last looked: every download that runs joins it, and it is cleared
//! when nothing runs and either everything in it completed or the window
//! gains focus. So one paused or failed download colours the button yellow
//! or red until the user comes back, and old paused downloads never do.
//!
//! Plain data with no I/O, so it is tested directly; the hub drives it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::dto::{DownloadError, DownloadId, DownloadRow, DownloadState};

/// The taskbar button's progress (Tauri's `ProgressBarStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Taskbar {
    /// No progress shown.
    #[default]
    None,
    /// Downloading, percent done.
    Normal(u8),
    /// Downloading, sizes unknown.
    Indeterminate,
    /// Nothing runs and something is paused: yellow.
    Paused(u8),
    /// Something failed or needs a new link: red.
    Error(u8),
}

/// The aggregate of what runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Activity {
    /// Downloads with live progress.
    pub(crate) running: usize,
    /// Their combined speed (3 s average), bytes per second.
    pub(crate) bps: u64,
    /// Seconds until the running downloads finish, when known.
    pub(crate) eta_secs: Option<u64>,
    /// The taskbar button.
    pub(crate) taskbar: Taskbar,
}

impl Activity {
    /// Whether bytes are arriving: the tray mark animates only then (#13).
    pub(crate) fn flowing(&self) -> bool {
        self.running > 0 && self.bps > 0
    }
}

/// Something worth a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Notice {
    /// A download completed.
    Completed {
        id: DownloadId,
        name: String,
        size: Option<u64>,
        /// How long it took, when this session saw all of it.
        elapsed: Option<Duration>,
    },
    /// A download stopped and needs the user: it failed, its link expired,
    /// or its checksum does not match.
    Attention {
        id: DownloadId,
        name: String,
        state: DownloadState,
        error: Option<DownloadError>,
    },
}

impl Notice {
    pub(crate) fn id(&self) -> DownloadId {
        match self {
            Self::Completed { id, .. } | Self::Attention { id, .. } => *id,
        }
    }
}

/// States that need the user.
pub(crate) fn needs_attention(state: DownloadState) -> bool {
    matches!(
        state,
        DownloadState::Failed | DownloadState::NeedsNewLink | DownloadState::Corrupt
    )
}

/// A row's name for people: its file name, else the last part of its link.
pub(crate) fn display_name(row: &DownloadRow) -> String {
    if let Some(name) = row.file_name.as_deref().filter(|n| !n.is_empty()) {
        return name.to_owned();
    }
    let path = row.url.split(['?', '#']).next().unwrap_or_default();
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty() && !s.contains(':'))
        .unwrap_or(&row.url)
        .to_owned()
}

#[derive(Debug, Default)]
pub(crate) struct Tracker {
    batch: BTreeSet<DownloadId>,
    /// When a download started running this session, and whether it started
    /// from nothing (so its whole time is known).
    started: BTreeMap<DownloadId, (Instant, bool)>,
}

impl Tracker {
    /// A download changed state: remembers when runs start, and turns
    /// completions and failures into notices.
    pub(crate) fn state_changed(
        &mut self,
        row: &DownloadRow,
        previous: Option<DownloadState>,
        now: Instant,
    ) -> Option<Notice> {
        let id = row.id;
        if row.state.is_active() {
            self.started.entry(id).or_insert((now, row.downloaded == 0));
            self.batch.insert(id);
            return None;
        }
        let started = self.started.remove(&id);
        if row.state == DownloadState::Completed {
            return Some(Notice::Completed {
                id,
                name: display_name(row),
                size: row.total,
                elapsed: started.and_then(|(at, whole)| whole.then(|| now - at)),
            });
        }
        if needs_attention(row.state) && previous != Some(row.state) {
            return Some(Notice::Attention {
                id,
                name: display_name(row),
                state: row.state,
                error: row.error.clone(),
            });
        }
        None
    }

    /// A download left the list.
    pub(crate) fn removed(&mut self, id: DownloadId) {
        self.batch.remove(&id);
        self.started.remove(&id);
    }

    /// The window gained focus: the user has seen how things stand.
    pub(crate) fn acknowledge(&mut self, rows: &BTreeMap<DownloadId, &DownloadRow>) {
        if !rows.values().any(|r| r.state.is_active()) {
            self.batch.clear();
        }
    }

    /// The aggregate over `rows` (all of them, by id).
    pub(crate) fn activity(&mut self, rows: &BTreeMap<DownloadId, &DownloadRow>) -> Activity {
        for (id, row) in rows {
            if row.state.is_active() {
                self.batch.insert(*id);
            }
        }
        self.batch.retain(|id| rows.contains_key(id));
        let members: Vec<&DownloadRow> = self
            .batch
            .iter()
            .filter_map(|id| rows.get(id).copied())
            .collect();
        let running: Vec<&&DownloadRow> = members.iter().filter(|r| r.state.is_active()).collect();
        let bps: u64 = running.iter().map(|r| r.bps).sum();

        let mut done = 0u64;
        let mut total = Some(0u64);
        for r in &members {
            match (r.state, r.total) {
                (DownloadState::Completed, t) => {
                    let size = t.unwrap_or(r.downloaded);
                    done += size;
                    total = total.map(|s| s + size);
                }
                (_, Some(t)) => {
                    done += r.downloaded.min(t);
                    total = total.map(|s| s + t);
                }
                (_, None) => total = None,
            }
        }
        let percent = total
            .filter(|&t| t > 0)
            .map_or(0, |t| (u128::from(done) * 100 / u128::from(t)) as u8);
        let left: Option<u64> = running
            .iter()
            .map(|r| r.total.map(|t| t.saturating_sub(r.downloaded)))
            .sum();
        let eta_secs = left.filter(|_| bps > 0).map(|l| l.div_ceil(bps));

        let attention = members.iter().any(|r| needs_attention(r.state));
        let stopped = members.iter().any(|r| {
            matches!(
                r.state,
                DownloadState::Paused | DownloadState::Cancelled | DownloadState::Queued
            )
        });
        let taskbar = if !running.is_empty() {
            match (attention, total) {
                (true, _) => Taskbar::Error(percent),
                (false, Some(_)) => Taskbar::Normal(percent),
                (false, None) => Taskbar::Indeterminate,
            }
        } else if attention {
            Taskbar::Error(percent)
        } else if stopped {
            Taskbar::Paused(percent)
        } else {
            // Everything in it completed (or it is empty).
            self.batch.clear();
            Taskbar::None
        };
        Activity {
            running: running.len(),
            bps,
            eta_secs,
            taskbar,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, state: DownloadState, downloaded: u64, total: Option<u64>) -> DownloadRow {
        DownloadRow {
            id: DownloadId(id),
            state,
            url: format!("https://example.com/files/{id}.bin?token=x"),
            file_name: None,
            path: None,
            total,
            downloaded,
            bps: if state.is_active() { 1000 } else { 0 },
            connections: 0,
            resumable: true,
            speed_limit: None,
            error: None,
            created_at: 1,
            completed_at: None,
            queue: crate::bridge::dto::QueueId(1),
            queue_position: 0,
            start_at: None,
        }
    }

    fn by_id(rows: &[DownloadRow]) -> BTreeMap<DownloadId, &DownloadRow> {
        rows.iter().map(|r| (r.id, r)).collect()
    }

    #[test]
    fn idle_shows_nothing_and_old_paused_downloads_stay_out() {
        let mut t = Tracker::default();
        let rows = [row(1, DownloadState::Paused, 10, Some(100))];
        let a = t.activity(&by_id(&rows));
        assert_eq!(a, Activity::default());
        assert!(!a.flowing());
    }

    #[test]
    fn running_downloads_add_up() {
        let mut t = Tracker::default();
        let rows = [
            row(1, DownloadState::Downloading, 250, Some(1000)),
            row(2, DownloadState::Downloading, 250, Some(1000)),
            row(3, DownloadState::Completed, 0, Some(5000)),
        ];
        let a = t.activity(&by_id(&rows));
        assert_eq!(a.running, 2);
        assert_eq!(a.bps, 2000);
        assert!(a.flowing());
        // (2000 - 500) bytes left at 2000 B/s.
        assert_eq!(a.eta_secs, Some(1));
        assert_eq!(a.taskbar, Taskbar::Normal(25));
    }

    #[test]
    fn unknown_sizes_are_indeterminate() {
        let mut t = Tracker::default();
        let rows = [
            row(1, DownloadState::Downloading, 250, None),
            row(2, DownloadState::Downloading, 250, Some(1000)),
        ];
        let a = t.activity(&by_id(&rows));
        assert_eq!(a.taskbar, Taskbar::Indeterminate);
        assert_eq!(a.eta_secs, None);
    }

    #[test]
    fn the_batch_keeps_finished_downloads_until_all_are_done() {
        let mut t = Tracker::default();
        let mut rows = vec![
            row(1, DownloadState::Downloading, 500, Some(1000)),
            row(2, DownloadState::Downloading, 0, Some(1000)),
        ];
        t.activity(&by_id(&rows));
        rows[0] = row(1, DownloadState::Completed, 1000, Some(1000));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::Normal(50));
        rows[1] = row(2, DownloadState::Completed, 1000, Some(1000));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::None);
        // A new run starts a new batch.
        rows.push(row(3, DownloadState::Downloading, 100, Some(1000)));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::Normal(10));
    }

    #[test]
    fn paused_is_yellow_and_failed_is_red_until_the_user_looks() {
        let mut t = Tracker::default();
        let mut rows = vec![
            row(1, DownloadState::Downloading, 500, Some(1000)),
            row(2, DownloadState::Downloading, 0, Some(1000)),
        ];
        t.activity(&by_id(&rows));
        rows[0] = row(1, DownloadState::Paused, 500, Some(1000));
        rows[1] = row(2, DownloadState::Paused, 0, Some(1000));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::Paused(25));
        rows[1] = row(2, DownloadState::Failed, 0, Some(1000));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::Error(25));
        // A failure while others run shows red too.
        rows[0] = row(1, DownloadState::Downloading, 600, Some(1000));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::Error(30));
        rows[0] = row(1, DownloadState::Paused, 600, Some(1000));
        t.acknowledge(&by_id(&rows));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::None);
    }

    #[test]
    fn acknowledging_while_running_keeps_the_batch() {
        let mut t = Tracker::default();
        let rows = [
            row(1, DownloadState::Downloading, 500, Some(1000)),
            row(2, DownloadState::Failed, 0, Some(1000)),
        ];
        t.activity(&by_id(&rows));
        t.acknowledge(&by_id(&rows));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::Normal(50));
    }

    #[test]
    fn removed_downloads_leave_the_batch() {
        let mut t = Tracker::default();
        let rows = vec![
            row(1, DownloadState::Downloading, 500, Some(1000)),
            row(2, DownloadState::Paused, 0, Some(1000)),
        ];
        t.activity(&by_id(&rows));
        let rows = vec![row(2, DownloadState::Paused, 0, Some(1000))];
        t.removed(DownloadId(1));
        assert_eq!(t.activity(&by_id(&rows)).taskbar, Taskbar::None);
    }

    #[test]
    fn completions_and_failures_become_notices() {
        let mut t = Tracker::default();
        let start = Instant::now();
        let mut r = row(1, DownloadState::Downloading, 0, Some(1000));
        assert_eq!(
            t.state_changed(&r, Some(DownloadState::Queued), start),
            None
        );
        r.state = DownloadState::Completed;
        r.file_name = Some("ubuntu.iso".into());
        let n = t.state_changed(
            &r,
            Some(DownloadState::Downloading),
            start + Duration::from_secs(8),
        );
        assert_eq!(
            n,
            Some(Notice::Completed {
                id: DownloadId(1),
                name: "ubuntu.iso".into(),
                size: Some(1000),
                elapsed: Some(Duration::from_secs(8)),
            })
        );

        // Resumed from a checkpoint: the time of this run alone would mislead.
        let mut r = row(2, DownloadState::Downloading, 400, Some(1000));
        t.state_changed(&r, Some(DownloadState::Paused), start);
        r.state = DownloadState::Completed;
        assert!(matches!(
            t.state_changed(&r, None, start),
            Some(Notice::Completed { elapsed: None, ref name, .. }) if name == "2.bin"
        ));

        let mut r = row(3, DownloadState::Downloading, 0, Some(1000));
        t.state_changed(&r, None, start);
        r.state = DownloadState::NeedsNewLink;
        assert!(matches!(
            t.state_changed(&r, Some(DownloadState::Downloading), start),
            Some(Notice::Attention {
                state: DownloadState::NeedsNewLink,
                ..
            })
        ));
        // A pause is the user's own doing: no notice.
        r.state = DownloadState::Paused;
        assert_eq!(
            t.state_changed(&r, Some(DownloadState::Downloading), start),
            None
        );
    }

    #[test]
    fn names_fall_back_to_the_link() {
        let mut r = row(1, DownloadState::Queued, 0, None);
        assert_eq!(display_name(&r), "1.bin");
        r.url = "https://example.com/".into();
        assert_eq!(display_name(&r), "example.com");
        r.file_name = Some("a.zip".into());
        assert_eq!(display_name(&r), "a.zip");
    }
}
