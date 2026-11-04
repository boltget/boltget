//! `boltget list`.

use std::path::Path;

use boltget_engine::{DownloadSummary, Engine, EngineConfig, Error};
use indicatif::HumanBytes;

use crate::display::percent;
use crate::{ListArgs, data_dir, report};

pub(crate) const JSON_HELP: &str = "\
With --json, stdout is one JSON array with an object per download:
  {id, state, url, file_name, path, partial_path, total, downloaded,
   resumable, error: {kind, key, message} | null,
   checksum: {expected: {algo, hex}, source, actual} | null,
   created_at, updated_at, completed_at}
state is one of queued, probing, downloading, retrying, verifying, paused,
completed, failed, cancelled, corrupt. checksum.source is \"user\" or
{\"server\": \"<header>\"}; actual is set once the file was hashed.
downloaded counts bytes saved at the last checkpoint.
Times are Unix milliseconds; URLs have their query redacted.

While the Boltget app runs, the list comes from it, and downloaded counts the
bytes received so far for running downloads. While another boltget process
owns the database, the list is read without taking ownership and shows that
process's last checkpoint.

Exit codes: 0 success, 7 the database cannot be read.";

pub(crate) async fn run(args: &ListArgs, state: Option<&Path>) -> u8 {
    let dir = match data_dir(state) {
        Ok(d) => d,
        Err(e) => return report(&e, false, None, &[]),
    };
    let list = if dir.join(boltget_engine::DB_FILE).exists() {
        match Engine::open(EngineConfig::default(), &dir) {
            Ok(engine) => {
                let l = engine.list().await;
                engine.shutdown().await;
                l
            }
            // Another process is downloading: read its last checkpoint instead.
            Err(Error::DatabaseLocked { .. }) => Engine::peek(&dir),
            Err(e) => Err(e),
        }
    } else {
        // Listing does not create a database where there is none.
        Ok(Vec::new())
    };
    let list = match list {
        Ok(l) => l,
        Err(e) => return report(&e, false, None, &[]),
    };
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
        }
    }
}

fn progress_cell(s: &Row) -> String {
    match (s.state.as_str(), s.total) {
        ("completed", _) => "100%".into(),
        (_, Some(t)) => percent(s.downloaded, t),
        (_, None) if s.downloaded > 0 => HumanBytes(s.downloaded).to_string(),
        _ => "-".into(),
    }
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
    println!(
        "{:>id_w$}  {:<14}  {:>9}  {:>10}  NAME",
        "ID", "STATE", "PROGRESS", "SIZE"
    );
    for s in list {
        let size = s
            .total
            .map_or_else(|| "?".to_owned(), |t| HumanBytes(t).to_string());
        let name = s.file_name.clone().unwrap_or_else(|| s.url.clone());
        println!(
            "{:>id_w$}  {:<14}  {:>9}  {:>10}  {name}",
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
