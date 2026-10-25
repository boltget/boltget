//! `boltget list`.

use std::path::Path;

use boltget_engine::{DownloadSummary, Engine, EngineConfig, Error, State};
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

While another boltget process owns the database, the list is read without
taking ownership and shows that process's last checkpoint.

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
        print_table(&list);
    }
    0
}

fn progress_cell(s: &DownloadSummary) -> String {
    match (s.state, s.total) {
        (State::Completed, _) => "100%".into(),
        (_, Some(t)) => percent(s.downloaded, t),
        (_, None) if s.downloaded > 0 => HumanBytes(s.downloaded).to_string(),
        _ => "-".into(),
    }
}

fn print_table(list: &[DownloadSummary]) {
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
            s.state.as_str(),
            progress_cell(s),
            size
        );
        if let Some(e) = &s.error {
            println!("{:>id_w$}  error: {}", "", e.message);
        }
    }
}
