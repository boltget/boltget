//! `boltget`: the BoltGet command-line downloader.
//!
//! ```text
//! boltget get <url> [-o <path>] [-c <connections>] [--json] [--header 'Name: value']
//! boltget list [--json]
//! boltget pause <id>
//! boltget resume <id> [--restart] [--json]
//! boltget remove <id> [--delete-file]
//! ```
//!
//! Downloads are kept in a database (`--state`, default `%LOCALAPPDATA%\BoltGet`)
//! so they can be paused and resumed. See `boltget help get` for the exit codes
//! and the `--json` event format.
// A CLI prints to stdout by design.
#![allow(clippy::print_stdout)]

mod display;
mod json;
mod list;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use boltget_engine::{
    AddRequest, ConflictPolicy, Destination, DownloadHandle, DownloadId, Engine, EngineConfig,
    EngineEvent, Error, ErrorKind, HeaderName, HeaderValue, RequestContext, ResumeOptions, State,
};
use clap::{Args, Parser, Subcommand};
use tokio::sync::broadcast;

const EXIT_CODES: &str = "\
Exit codes:
  0    success (get, resume: the download completed)
  1    internal or unexpected error
  2    invalid command line (including a malformed --header)
  3    network failure after all retries (timeouts, resets, 5xx, 429)
  4    the request cannot succeed: invalid or unsupported URL, 404, 401/403,
       TLS certificate errors, too many or insecure redirects
  5    local disk problem (no space, permission denied, file system limit,
       partial file missing)
  6    the file changed on the server or the link stopped working mid-download
  7    the download database is in use by another boltget process, or cannot
       be opened
  8    no download with that id, or the command does not apply to it in its
       current state (for example resuming a completed download)
  130  interrupted (Ctrl-C): the download is paused; continue it with
       `boltget resume <id>`";

const JSON_HELP: &str = "\
With --json, stdout carries one JSON object per line. Every object has an
\"event\" field:
  added             {id, url}
  probed            {id, info: {probe: {final_url, size, ranges, filename, ...},
                     path, partial_path, preallocation, initial_connections}}
  resumed           {id, downloaded, total}  continuing from a checkpoint
  state_changed     {id, state, error}  state: probing, downloading, retrying,
                     paused, completed, failed, cancelled
  segments_changed  {id}
  progress          {id, downloaded, total, speed, eta_secs, connections,
                     segments} (every 500 ms)
  completed         {id, path, bytes, resumed_from, elapsed_ms, speed,
                     max_connections, segments}  speed counts this run's bytes
                     (bytes - resumed_from)
  paused            {id, downloaded, total, exit_code}  after Ctrl-C
  failed            {id, error: {kind, key, message}, exit_code}
The last line is always completed, paused or failed. Sizes are bytes, speeds
bytes per second. URLs have their query redacted. `id` is absent from a
failed line when the download never got an id (a bad command line).";

const STATE_HELP: &str = "\
Downloads are recorded in a database in the state directory (default
%LOCALAPPDATA%\\BoltGet on Windows, $XDG_DATA_HOME/boltget on Linux; override
with --state or BOLTGET_DATA_DIR). Only one boltget process at a time can use
it; `boltget list` works alongside.";

/// BoltGet: a fast, segmented download manager.
#[derive(Debug, Parser)]
#[command(
    name = "boltget",
    version,
    about,
    propagate_version = true,
    after_long_help = STATE_HELP
)]
struct Cli {
    /// Directory holding the download database.
    #[arg(long, global = true, value_name = "DIR", env = "BOLTGET_DATA_DIR")]
    state: Option<PathBuf>,

    /// Log engine diagnostics to stderr (repeat for more).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Download a URL (recorded, so it can be paused and resumed).
    #[command(after_long_help = format!("{EXIT_CODES}\n\n{JSON_HELP}"))]
    Get(GetArgs),
    /// List recorded downloads.
    #[command(after_long_help = list::JSON_HELP)]
    List(ListArgs),
    /// Mark a download that is not running as paused.
    ///
    /// A download runs inside the `boltget get` or `boltget resume` that started
    /// it; press Ctrl-C there to pause it. This command cannot reach another
    /// running boltget process yet.
    #[command(after_long_help = EXIT_CODES)]
    Pause(IdArgs),
    /// Continue a paused, interrupted or failed download in the foreground.
    #[command(after_long_help = format!("{EXIT_CODES}\n\n{JSON_HELP}"))]
    Resume(ResumeArgs),
    /// Remove a download from the list.
    #[command(after_long_help = EXIT_CODES)]
    Remove(RemoveArgs),
}

/// Options shared by the commands that run a download.
#[derive(Debug, Args)]
struct RunArgs {
    /// Number of connections (1-32).
    #[arg(short, long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=32))]
    connections: Option<u32>,

    /// Print line-delimited JSON events on stdout instead of a progress display.
    #[arg(long)]
    json: bool,

    /// Extra request header, `Name: value` (User-Agent, Referer, Accept,
    /// Accept-Language, Authorization, Origin). May be repeated. Authorization is
    /// not saved: pass it again to `resume`.
    #[arg(short = 'H', long = "header", value_name = "HEADER")]
    headers: Vec<String>,

    /// No progress display; only the final line (or errors).
    #[arg(short, long, conflicts_with = "json")]
    quiet: bool,

    /// Flush (and checkpoint) interval in milliseconds. For tests.
    #[arg(long, hide = true, value_name = "MS")]
    flush_interval_ms: Option<u64>,

    /// Smallest segment worth splitting, in bytes. For tests.
    #[arg(long, hide = true, value_name = "BYTES")]
    min_split_size: Option<u64>,
}

#[derive(Debug, Args)]
struct GetArgs {
    /// The URL to download (http or https).
    url: String,

    /// Output file, or an existing directory to save into (default: current directory).
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Replace an existing file instead of saving as `name (1).ext`.
    #[arg(long)]
    overwrite: bool,

    /// Do not record the download (it cannot be resumed).
    #[arg(long)]
    no_state: bool,

    #[command(flatten)]
    run: RunArgs,
}

#[derive(Debug, Args)]
struct ListArgs {
    /// Print a JSON array instead of a table.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct IdArgs {
    /// Download id (see `boltget list`).
    id: u64,
}

#[derive(Debug, Args)]
struct ResumeArgs {
    /// Download id (see `boltget list`).
    id: u64,

    /// Discard the saved progress and download again from the beginning, to the
    /// same file name (needed when the file changed on the server).
    #[arg(long)]
    restart: bool,

    #[command(flatten)]
    run: RunArgs,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    /// Download id (see `boltget list`).
    id: u64,

    /// Also delete the partial file, or the downloaded file if it completed.
    #[arg(long)]
    delete_file: bool,
}

/// Maps an engine error to the documented exit code.
fn exit_code_for(e: &Error) -> u8 {
    match e.kind() {
        ErrorKind::Transient | ErrorKind::Throttled => 3,
        ErrorKind::Permanent => 4,
        ErrorKind::Disk => 5,
        ErrorKind::ResourceChanged | ErrorKind::RangeUnsupported | ErrorKind::LinkExpired => 6,
        ErrorKind::Busy | ErrorKind::Database => 7,
        ErrorKind::NotFound | ErrorKind::InvalidState => 8,
        ErrorKind::Cancelled => 130,
        _ => 1,
    }
}

fn init_logging(verbose: u8) {
    use tracing_subscriber::EnvFilter;
    let default = match verbose {
        0 => "warn",
        1 => "boltget_engine=debug,warn",
        _ => "boltget_engine=trace,debug",
    };
    let filter = EnvFilter::try_from_env("BOLTGET_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

fn parse_headers(headers: &[String]) -> Result<Vec<(HeaderName, HeaderValue)>> {
    headers
        .iter()
        .map(|h| {
            let (name, value) = h
                .split_once(':')
                .with_context(|| format!("header {h:?} is not `Name: value`"))?;
            let name: HeaderName = name
                .trim()
                .parse()
                .with_context(|| format!("bad header name in {h:?}"))?;
            let value: HeaderValue = value
                .trim()
                .parse()
                .with_context(|| format!("bad header value in {h:?}"))?;
            Ok((name, value))
        })
        .collect()
}

fn build_request(args: &GetArgs) -> Result<AddRequest> {
    let mut ctx = RequestContext::new(&args.url)?;
    for (name, value) in parse_headers(&args.run.headers)? {
        if !ctx.header(name.clone(), value) {
            bail!("header {name} is not allowed (the engine controls it)");
        }
    }
    let destination = match &args.output {
        None => Destination::Directory(std::env::current_dir()?),
        Some(p) if p.is_dir() => Destination::Directory(p.clone()),
        Some(p) if p.as_os_str().to_string_lossy().ends_with(['/', '\\']) => {
            bail!("directory {} does not exist", p.display())
        }
        Some(p) => Destination::File(p.clone()),
    };
    let mut req = AddRequest::new(ctx, destination);
    if let Some(n) = args.run.connections {
        req = req.connections(n);
    }
    if args.overwrite {
        req = req.conflict(ConflictPolicy::Overwrite);
    }
    Ok(req)
}

fn engine_config(run: &RunArgs) -> EngineConfig {
    let mut cfg = EngineConfig::default();
    if let Some(n) = run.connections {
        cfg.initial_connections = n;
        cfg.max_connections = cfg.max_connections.max(n);
    }
    if let Some(ms) = run.flush_interval_ms {
        cfg.flush_interval = Duration::from_millis(ms);
    }
    if let Some(n) = run.min_split_size {
        cfg.min_split_size = n.max(1);
    }
    cfg
}

fn data_dir(state: Option<&Path>) -> Result<PathBuf, Error> {
    state
        .map(Path::to_path_buf)
        .or_else(boltget_engine::default_data_dir)
        .ok_or_else(|| Error::Database("cannot find a data directory; pass --state <DIR>".into()))
}

/// Prints an error (with optional hint lines) and returns its exit code.
fn report(e: &Error, json: bool, id: Option<DownloadId>, hints: &[String]) -> u8 {
    let code = exit_code_for(e);
    if json {
        json::failed(e, id, code);
    } else {
        eprintln!("boltget: {e}");
        for h in hints {
            eprintln!("boltget: {h}");
        }
    }
    code
}

/// Hints for errors that `resume` (or another flag) can fix.
fn hints_for(e: &Error, id: Option<DownloadId>, command: &str) -> Vec<String> {
    match (e, id) {
        (Error::DatabaseLocked { .. }, _) => {
            let mut v = vec![
                "wait for the other download to finish, or press Ctrl-C in its terminal to pause it"
                    .to_owned(),
            ];
            if command == "get" {
                v.push("or download without recording it: --no-state".into());
            }
            v
        }
        (_, Some(id)) => match e.kind() {
            ErrorKind::ResourceChanged | ErrorKind::RangeUnsupported => vec![format!(
                "the saved progress cannot be used; start over with: boltget resume {id} --restart"
            )],
            _ if matches!(e, Error::PartialMissing { .. }) => {
                vec![format!("start over with: boltget resume {id} --restart")]
            }
            ErrorKind::Transient
            | ErrorKind::Throttled
            | ErrorKind::LinkExpired
            | ErrorKind::Disk => {
                vec![format!(
                    "the progress is saved; try again with: boltget resume {id}"
                )]
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Runs a started download in the foreground until it completes, fails or is
/// paused with Ctrl-C, then shuts the engine down. Returns the exit code.
async fn run_download(
    engine: &Engine,
    handle: DownloadHandle,
    events: broadcast::Receiver<EngineEvent>,
    run: &RunArgs,
    verbose: u8,
) -> u8 {
    let id = handle.id();
    let persistent = engine.data_dir().is_some();
    let ctrl_c = {
        let engine = engine.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                // Pause: workers stop, the writer flushes, the checkpoint is saved.
                engine.request_pause(id);
                if tokio::signal::ctrl_c().await.is_ok() {
                    // A second Ctrl-C does not wait. The checkpoint never claims
                    // bytes that are not on disk, so this is still safe to resume.
                    std::process::exit(130);
                }
            }
        })
    };
    let result = if run.json {
        json::run(handle, events).await
    } else {
        let show_bar = !run.quiet && verbose == 0;
        display::run(handle, show_bar, persistent.then_some(id)).await
    };
    engine.shutdown().await;
    ctrl_c.abort();

    match result {
        Ok(done) => {
            if !run.json {
                display::summary(&done);
            }
            0
        }
        Err(e) => {
            let paused = matches!(e.error, Error::Cancelled) && e.progress.state == State::Paused;
            if paused {
                if run.json {
                    json::paused(&e.progress, 130);
                } else {
                    display::paused(&e.progress, e.partial.as_deref(), persistent);
                }
                return 130;
            }
            let hints = if persistent {
                hints_for(&e.error, Some(id), "")
            } else {
                Vec::new()
            };
            let code = report(&e.error, run.json, Some(id), &hints);
            if !run.json
                && let Some(p) = e.partial.filter(|p| p.exists())
            {
                eprintln!("boltget: partial file kept at {}", p.display());
            }
            code
        }
    }
}

/// Opens the engine with the database, or reports why not.
fn open(state: Option<&Path>, cfg: EngineConfig, json: bool, command: &str) -> Result<Engine, u8> {
    data_dir(state)
        .and_then(|dir| Engine::open(cfg, dir))
        .map_err(|e| report(&e, json, None, &hints_for(&e, None, command)))
}

async fn get(args: GetArgs, state: Option<&Path>, verbose: u8) -> u8 {
    let req = match build_request(&args) {
        Ok(r) => r,
        Err(e) => {
            // A URL the engine rejects is a refused request (4); anything else
            // about the command line is a usage error (2).
            let code = if e.downcast_ref::<Error>().is_some() {
                4
            } else {
                2
            };
            if args.run.json {
                json::failed_usage(&e, code);
            } else {
                eprintln!("boltget: {e:#}");
            }
            return code;
        }
    };
    let cfg = engine_config(&args.run);
    let engine = if args.no_state {
        match Engine::new(cfg) {
            Ok(e) => e,
            Err(e) => return report(&e, args.run.json, None, &[]),
        }
    } else {
        match open(state, cfg, args.run.json, "get") {
            Ok(e) => e,
            Err(code) => return code,
        }
    };
    // Subscribe first so the `added` event is not missed.
    let events = engine.subscribe();
    let handle = engine.add(req);
    run_download(&engine, handle, events, &args.run, verbose).await
}

async fn resume(args: ResumeArgs, state: Option<&Path>, verbose: u8) -> u8 {
    let headers = match parse_headers(&args.run.headers) {
        Ok(h) => h,
        Err(e) => {
            if args.run.json {
                json::failed_usage(&e, 2);
            } else {
                eprintln!("boltget: {e:#}");
            }
            return 2;
        }
    };
    let engine = match open(state, engine_config(&args.run), args.run.json, "resume") {
        Ok(e) => e,
        Err(code) => return code,
    };
    let id = DownloadId(args.id);
    let mut opts = ResumeOptions::default().restart(args.restart);
    opts.connections = args.run.connections;
    for (name, value) in headers {
        opts = opts.header(name, value);
    }
    let events = engine.subscribe();
    match engine.resume(id, opts).await {
        Ok(handle) => run_download(&engine, handle, events, &args.run, verbose).await,
        Err(e) => {
            engine.shutdown().await;
            report(
                &e,
                args.run.json,
                Some(id),
                &hints_for(&e, Some(id), "resume"),
            )
        }
    }
}

async fn pause(args: &IdArgs, state: Option<&Path>) -> u8 {
    let id = DownloadId(args.id);
    let dir = match data_dir(state) {
        Ok(d) => d,
        Err(e) => return report(&e, false, None, &[]),
    };
    let engine = match Engine::open(EngineConfig::default(), &dir) {
        Ok(e) => e,
        Err(e @ Error::DatabaseLocked { .. }) => {
            // Say what we can from a read-only look at the database.
            let found = Engine::peek(&dir)
                .ok()
                .and_then(|l| l.into_iter().find(|s| s.id == id));
            return match found {
                None => report(&Error::UnknownDownload(id), false, Some(id), &[]),
                Some(s) if s.state.is_active() => report(
                    &e,
                    false,
                    Some(id),
                    &[format!(
                        "download {id} is running in that process; press Ctrl-C there to pause it"
                    )],
                ),
                Some(_) => report(&e, false, Some(id), &hints_for(&e, None, "pause")),
            };
        }
        Err(e) => return report(&e, false, None, &[]),
    };
    let r = engine.pause(id).await;
    engine.shutdown().await;
    match r {
        Ok(State::Paused) => {
            println!("Paused download {id}");
            0
        }
        Ok(other) => {
            println!("Download {id} is not running ({other})");
            0
        }
        Err(e) => report(&e, false, Some(id), &[]),
    }
}

async fn remove(args: &RemoveArgs, state: Option<&Path>) -> u8 {
    let id = DownloadId(args.id);
    let engine = match open(state, EngineConfig::default(), false, "remove") {
        Ok(e) => e,
        Err(code) => return code,
    };
    let r = engine.remove(id, args.delete_file).await;
    engine.shutdown().await;
    match r {
        Ok(removed) => {
            println!("Removed download {id}");
            for p in &removed.deleted {
                println!("Deleted {}", p.display());
            }
            0
        }
        Err(e) => report(&e, false, Some(id), &[]),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);
    let state = cli.state.as_deref();
    let code = match cli.command {
        Command::Get(args) => get(args, state, cli.verbose).await,
        Command::List(args) => list::run(&args, state).await,
        Command::Pause(args) => pause(&args, state).await,
        Command::Resume(args) => resume(args, state, cli.verbose).await,
        Command::Remove(args) => remove(&args, state).await,
    };
    ExitCode::from(code)
}

/// Bytes per second over a sliding window of samples.
#[derive(Debug, Default)]
pub(crate) struct Speed {
    samples: std::collections::VecDeque<(std::time::Instant, u64)>,
}

impl Speed {
    const WINDOW: Duration = Duration::from_secs(5);

    pub(crate) fn sample(&mut self, bytes: u64) -> f64 {
        let now = std::time::Instant::now();
        self.samples.push_back((now, bytes));
        while self
            .samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > Self::WINDOW)
        {
            self.samples.pop_front();
        }
        match (self.samples.front(), self.samples.back()) {
            (Some((t0, b0)), Some((t1, b1))) if t1 > t0 => {
                b1.saturating_sub(*b0) as f64 / t1.duration_since(*t0).as_secs_f64()
            }
            _ => 0.0,
        }
    }
}

/// Waits for a download task whose state is already terminal, keeping its
/// final progress for reporting how it stopped.
pub(crate) async fn finish(
    handle: DownloadHandle,
    partial: Option<PathBuf>,
) -> Result<boltget_engine::Completed, Box<Stopped>> {
    let progress = handle.progress();
    handle.wait().await.map_err(|error| {
        Box::new(Stopped {
            error,
            progress,
            partial,
        })
    })
}

/// How a foreground download ended without completing.
#[derive(Debug)]
pub(crate) struct Stopped {
    pub(crate) error: Error,
    /// The last progress snapshot (its state tells pause from failure).
    pub(crate) progress: boltget_engine::Progress,
    pub(crate) partial: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_commands() {
        let cli = Cli::try_parse_from(["boltget", "get", "https://x.test/a", "-c", "4", "--json"])
            .unwrap();
        let Command::Get(a) = cli.command else {
            panic!("expected get");
        };
        assert_eq!(a.run.connections, Some(4));
        assert!(a.run.json);
        assert!(Cli::try_parse_from(["boltget", "get", "u", "-c", "0"]).is_err());
        assert!(Cli::try_parse_from(["boltget", "get", "u", "-c", "33"]).is_err());

        let cli =
            Cli::try_parse_from(["boltget", "resume", "7", "--restart", "--state", "d"]).unwrap();
        assert_eq!(cli.state.as_deref(), Some(Path::new("d")));
        let Command::Resume(r) = cli.command else {
            panic!("expected resume");
        };
        assert_eq!(r.id, 7);
        assert!(r.restart);
        assert!(Cli::try_parse_from(["boltget", "pause", "x"]).is_err());
        let cli = Cli::try_parse_from(["boltget", "--state", "d", "remove", "3", "--delete-file"])
            .unwrap();
        let Command::Remove(r) = cli.command else {
            panic!("expected remove");
        };
        assert!(r.delete_file);
    }

    #[test]
    fn exit_codes() {
        assert_eq!(exit_code_for(&Error::Cancelled), 130);
        assert_eq!(exit_code_for(&Error::Timeout("x")), 3);
        assert_eq!(exit_code_for(&Error::TooManyRedirects(10)), 4);
        assert_eq!(exit_code_for(&Error::ResourceChanged(String::new())), 6);
        assert_eq!(
            exit_code_for(&Error::DatabaseLocked {
                dir: PathBuf::new()
            }),
            7
        );
        assert_eq!(exit_code_for(&Error::Database(String::new())), 7);
        assert_eq!(exit_code_for(&Error::UnknownDownload(DownloadId(1))), 8);
        assert_eq!(
            exit_code_for(&Error::InvalidState {
                id: DownloadId(1),
                reason: String::new()
            }),
            8
        );
        assert_eq!(
            exit_code_for(&Error::PartialMissing {
                path: PathBuf::new(),
                detail: String::new()
            }),
            5
        );
    }
}
