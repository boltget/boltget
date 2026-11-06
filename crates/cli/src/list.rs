//! `boltget list`.

use std::path::Path;

use boltget_engine::{DownloadSummary, Engine, State};
use indicatif::HumanBytes;

use crate::display::percent;
use crate::{ListArgs, data_dir, report};

pub(crate) const JSON_HELP: &str = "\
With --json, stdout is one JSON array with an object per download:
  {id, state, url, file_name, path, partial_path, total, downloaded,
   resumable, error: {kind, key, message} | null,
   checksum: {expected: {algo, hex}, source, actual} | null,
   created_at, updated_at, completed_at,
   queue, queue_name, queue_position, start_at}
state is one of queued, probing, downloading, retrying, verifying, paused,
completed, failed, cancelled, corrupt. checksum.source is \"user\" or
{\"server\": \"<header>\"}; actual is set once the file was hashed.
downloaded counts bytes saved at the last checkpoint. queue is the id of the
queue the download belongs to (1 is Main); within a queue, a smaller
queue_position goes first (compare them, they are not counted from 1).
start_at is when it starts on its own, or null.
Times are Unix milliseconds; URLs have their query redacted.

The table's QUEUE column shows the queue and, for a download that waits its
turn, its place in line (Main #2).

While the Boltget app runs, the list comes from it, and downloaded counts the
bytes received so far for running downloads. Otherwise the list is read from
the database without taking ownership: while another boltget process owns
it, the list shows that process's last checkpoint; a download that a process
which stopped (a crash, a killed app) left running shows as paused, and the
app continues it when it next starts.

Exit codes: 0 success, 7 the database cannot be read.";

pub(crate) fn run(args: &ListArgs, state: Option<&Path>) -> u8 {
    let dir = match data_dir(state) {
        Ok(d) => d,
        Err(e) => return report(&e, false, None, &[]),
    };
    // Read only, never as the owner: opening the database would mark the
    // downloads a crashed app left running as paused, and the app would no
    // longer know to continue them on its next start. Listing also does not
    // create a database where there is none.
    let list = if dir.join(boltget_engine::DB_FILE).exists() {
        Engine::peek(&dir)
    } else {
        Ok(Vec::new())
    };
    let mut list = match list {
        Ok(l) => l,
        Err(e) => return report(&e, false, None, &[]),
    };
    if !lock_held(&dir) {
        // Nothing runs them: they wait for the app's next start, or resume.
        for s in &mut list {
            if s.state.is_active() {
                s.state = State::Paused;
            }
        }
    }
    if args.json {
        crate::json::emit(&list);
    } else {
        print_table(&list.iter().map(Row::from_summary).collect::<Vec<_>>());
    }
    0
}

/// What the table shows of a download, from the database or from the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    id: u64,
    state: String,
    url: String,
    file_name: Option<String>,
    total: Option<u64>,
    downloaded: u64,
    error: Option<String>,
    queue: u64,
    queue_name: String,
    queue_position: i64,
}

impl Row {
    fn from_summary(s: &DownloadSummary) -> Self {
        Self {
            id: s.id.0,
            state: s.state.as_str().to_owned(),
            url: s.url.clone(),
            file_name: s.file_name.clone(),
            total: s.total,
            downloaded: s.downloaded,
            error: s.error.as_ref().map(|e| e.message.clone()),
            queue: s.queue.0,
            queue_name: s.queue_name.clone(),
            queue_position: s.queue_position,
        }
    }

    /// A download as the app sends it (the `list --json` shape).
    pub(crate) fn from_json(v: &serde_json::Value) -> Self {
        let text = |k: &str| v[k].as_str().map(str::to_owned);
        Self {
            id: v["id"].as_u64().unwrap_or(0),
            state: text("state").unwrap_or_default(),
            url: text("url").unwrap_or_default(),
            file_name: text("file_name"),
            total: v["total"].as_u64(),
            downloaded: v["downloaded"].as_u64().unwrap_or(0),
            error: v["error"]["message"].as_str().map(str::to_owned),
            queue: v["queue"].as_u64().unwrap_or(1),
            queue_name: text("queue_name")
                .unwrap_or_else(|| boltget_engine::DEFAULT_QUEUE_NAME.into()),
            queue_position: v["queue_position"].as_i64().unwrap_or(0),
        }
    }
}

/// Whether a process owns the database (holds its lock) now.
fn lock_held(dir: &Path) -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(boltget_engine::LOCK_FILE))
        .is_ok_and(|f| f.try_lock().is_err())
}

fn progress_cell(s: &Row) -> String {
    match (s.state.as_str(), s.total) {
        ("completed", _) => "100%".into(),
        (_, Some(t)) => percent(s.downloaded, t),
        (_, None) if s.downloaded > 0 => HumanBytes(s.downloaded).to_string(),
        _ => "-".into(),
    }
}

/// The QUEUE cell of each row: the queue's name, and for a download that
/// waits its turn its place in line among those waiting there (`Main #2`).
pub(crate) fn queue_cells(list: &[Row]) -> Vec<String> {
    list.iter()
        .map(|s| {
            if s.state != "queued" {
                return s.queue_name.clone();
            }
            let place = 1 + list
                .iter()
                .filter(|o| {
                    o.state == "queued"
                        && o.queue == s.queue
                        && (o.queue_position, o.id) < (s.queue_position, s.id)
                })
                .count();
            format!("{} #{place}", s.queue_name)
        })
        .collect()
}

pub(crate) fn print_table(list: &[Row]) {
    if list.is_empty() {
        println!("No downloads.");
        return;
    }
    let id_w = list
        .iter()
        .map(|s| s.id.to_string().len())
        .max()
        .unwrap_or(1)
        .max(2);
    let queues = queue_cells(list);
    let queue_w = queues
        .iter()
        .map(|q| q.chars().count())
        .max()
        .unwrap_or(0)
        .max(5);
    println!(
        "{:>id_w$}  {:<14}  {:>9}  {:>10}  {:<queue_w$}  NAME",
        "ID", "STATE", "PROGRESS", "SIZE", "QUEUE"
    );
    for (s, queue) in list.iter().zip(&queues) {
        let size = s
            .total
            .map_or_else(|| "?".to_owned(), |t| HumanBytes(t).to_string());
        let name = s.file_name.clone().unwrap_or_else(|| s.url.clone());
        println!(
            "{:>id_w$}  {:<14}  {:>9}  {:>10}  {queue:<queue_w$}  {name}",
            s.id.to_string(),
            s.state,
            progress_cell(s),
            size
        );
        if let Some(e) = &s.error {
            println!("{:>id_w$}  error: {e}", "");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, state: &str, queue: u64, name: &str, position: i64) -> Row {
        Row {
            id,
            state: state.into(),
            url: String::new(),
            file_name: None,
            total: None,
            downloaded: 0,
            error: None,
            queue,
            queue_name: name.into(),
            queue_position: position,
        }
    }

    #[test]
    fn waiting_downloads_show_their_place_in_line() {
        let list = [
            row(1, "completed", 1, "Main", 1),
            row(2, "queued", 1, "Main", 9),
            row(3, "downloading", 1, "Main", 3),
            row(4, "queued", 1, "Main", 4),
            row(5, "queued", 2, "Night", 1),
        ];
        assert_eq!(
            queue_cells(&list),
            ["Main", "Main #2", "Main", "Main #1", "Night #1"]
        );
    }
}
