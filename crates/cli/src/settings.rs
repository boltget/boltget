//! `boltget config`: persistent settings stored in the download database.

use std::path::Path;

use boltget_engine::{Engine, EngineConfig, LimitScope, Settings};
use clap::{Args, Subcommand};
use indicatif::HumanBytes;

use crate::{Rate, data_dir, hints_for, report};

pub(crate) const HELP: &str = "\
Keys:
  limit   global bandwidth limit shared by all downloads, in bytes per second
          (500K, 5M, 1048576; K = 1024). Unset: unlimited.

Settings live in the download database (see --state). `set` and `unset` need
the database, so they fail with exit code 7 while another boltget is running;
`get` and `list` work alongside.";

#[derive(Debug, Args)]
pub(crate) struct ConfigArgs {
    #[command(subcommand)]
    pub(crate) action: ConfigAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigAction {
    /// Show every setting.
    List,
    /// Show one setting.
    Get {
        /// Setting name.
        key: Key,
    },
    /// Change a setting.
    Set {
        /// Setting name.
        key: Key,
        /// New value.
        value: String,
    },
    /// Reset a setting to its default.
    Unset {
        /// Setting name.
        key: Key,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum Key {
    /// Global bandwidth limit.
    Limit,
}

fn show_limit(s: &Settings) -> String {
    s.speed_limit.map_or_else(
        || "unlimited".to_owned(),
        |r| format!("{}/s ({r} bytes per second)", HumanBytes(r)),
    )
}

pub(crate) async fn run(args: &ConfigArgs, state: Option<&Path>) -> u8 {
    let dir = match data_dir(state) {
        Ok(d) => d,
        Err(e) => return report(&e, false, None, &[]),
    };
    match &args.action {
        ConfigAction::List | ConfigAction::Get { .. } => match Engine::peek_settings(&dir) {
            Ok(s) => {
                if matches!(args.action, ConfigAction::List) {
                    println!("limit = {}", show_limit(&s));
                } else {
                    println!("{}", show_limit(&s));
                }
                0
            }
            Err(e) => report(&e, false, None, &[]),
        },
        ConfigAction::Set { key, value } => {
            let Key::Limit = key;
            let rate = match crate::parse_limit(value) {
                Ok(Rate(r)) => r,
                Err(e) => {
                    eprintln!("boltget: {e}");
                    return 2;
                }
            };
            set_limit(&dir, rate).await
        }
        ConfigAction::Unset { key } => {
            let Key::Limit = key;
            set_limit(&dir, None).await
        }
    }
}

async fn set_limit(dir: &Path, rate: Option<u64>) -> u8 {
    let engine = match Engine::open(EngineConfig::default(), dir) {
        Ok(e) => e,
        Err(e) => return report(&e, false, None, &hints_for(&e, None, "config")),
    };
    let r = engine.set_limit(LimitScope::Global, rate).await;
    let shown = engine.settings().await.map(|s| show_limit(&s));
    engine.shutdown().await;
    match (r, shown) {
        (Ok(()), Ok(s)) => {
            println!("limit = {s}");
            0
        }
        (Err(e), _) | (_, Err(e)) => report(&e, false, None, &[]),
    }
}
