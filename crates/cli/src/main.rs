//! `boltget`: the BoltGet command-line downloader.
//!
//! ```text
//! boltget get <url> [-o <path>] [-c <connections>] [--json] [--header 'Name: value']
//! ```
//!
//! See `boltget get --help` for the exit codes and the `--json` event format.
// A CLI prints to stdout by design.
#![allow(clippy::print_stdout)]

mod display;
mod json;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use boltget_engine::{
    AddRequest, ConflictPolicy, Destination, Engine, EngineConfig, Error, ErrorKind, HeaderName,
    HeaderValue, RequestContext,
};
use clap::{Args, Parser, Subcommand};

const EXIT_CODES: &str = "\
Exit codes:
  0    the download completed
  1    internal or unexpected error
  2    invalid command line (including a malformed --header)
  3    network failure after all retries (timeouts, resets, 5xx, 429)
  4    the request cannot succeed: invalid or unsupported URL, 404, 401/403,
       TLS certificate errors, too many or insecure redirects
  5    local disk problem (no space, permission denied, file system limit)
  6    the file changed on the server or the link stopped working mid-download
  130  interrupted (Ctrl-C); the partial file is kept";

const JSON_HELP: &str = "\
With --json, stdout carries one JSON object per line. Every object has an
\"event\" field:
  added          {id, url}
  probed         {id, info: {probe: {final_url, size, ranges, filename, ...},
                  path, partial_path, preallocation, initial_connections}}
  state_changed  {id, state, error}
  progress       {downloaded, total, speed, eta_secs, connections, segments}
                 (every 500 ms)
  completed      {path, bytes, elapsed_ms, speed, max_connections, segments}
  failed         {error: {kind, key, message}, exit_code}
Sizes are bytes, speeds bytes per second. URLs have their query redacted.";

/// BoltGet: a fast, segmented download manager.
#[derive(Debug, Parser)]
#[command(name = "boltget", version, about, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Download a URL.
    #[command(after_long_help = format!("{EXIT_CODES}\n\n{JSON_HELP}"))]
    Get(GetArgs),
}

#[derive(Debug, Args)]
struct GetArgs {
    /// The URL to download (http or https).
    url: String,

    /// Output file, or an existing directory to save into (default: current directory).
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Number of connections (1-32).
    #[arg(short, long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=32))]
    connections: Option<u32>,

    /// Print line-delimited JSON events on stdout instead of a progress display.
    #[arg(long)]
    json: bool,

    /// Extra request header, `Name: value` (User-Agent, Referer, Accept,
    /// Accept-Language, Authorization, Origin). May be repeated.
    #[arg(short = 'H', long = "header", value_name = "HEADER")]
    headers: Vec<String>,

    /// Replace an existing file instead of saving as `name (1).ext`.
    #[arg(long)]
    overwrite: bool,

    /// No progress display; only the final line (or errors).
    #[arg(short, long, conflicts_with = "json")]
    quiet: bool,

    /// Log engine diagnostics to stderr (repeat for more).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

/// Maps an engine error to the documented exit code.
fn exit_code_for(e: &Error) -> u8 {
    match e.kind() {
        ErrorKind::Transient | ErrorKind::Throttled => 3,
        ErrorKind::Permanent => 4,
        ErrorKind::Disk => 5,
        ErrorKind::ResourceChanged | ErrorKind::RangeUnsupported | ErrorKind::LinkExpired => 6,
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

fn build_request(args: &GetArgs) -> Result<AddRequest> {
    let mut ctx = RequestContext::new(&args.url)?;
    for h in &args.headers {
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
    if let Some(n) = args.connections {
        req = req.connections(n);
    }
    if args.overwrite {
        req = req.conflict(ConflictPolicy::Overwrite);
    }
    Ok(req)
}

async fn get(args: GetArgs) -> Result<u8> {
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
            if args.json {
                json::failed_usage(&e, code);
            } else {
                eprintln!("boltget: {e:#}");
            }
            return Ok(code);
        }
    };
    let mut cfg = EngineConfig::default();
    if let Some(n) = args.connections {
        cfg.initial_connections = n;
        cfg.max_connections = cfg.max_connections.max(n);
    }
    let engine = Engine::new(cfg)?;
    let events = engine.subscribe();
    let handle = engine.add(req);
    let cancel = handle.cancellation_token();
    let interrupted = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel.cancel();
        }
    });

    let result = if args.json {
        json::run(handle, events).await
    } else {
        let show_bar = !args.quiet && args.verbose == 0;
        display::run(handle, show_bar).await
    };
    interrupted.abort();

    Ok(match result {
        Ok(done) => {
            if !args.json {
                display::summary(&done);
            }
            0
        }
        Err((e, partial)) => {
            let code = exit_code_for(&e);
            if args.json {
                json::failed(&e, code);
            } else {
                if matches!(e, Error::Cancelled) {
                    eprintln!("boltget: interrupted");
                } else {
                    eprintln!("boltget: {e}");
                }
                if let Some(p) = partial.filter(|p| p.exists()) {
                    eprintln!("boltget: partial file kept at {}", p.display());
                }
            }
            code
        }
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let Command::Get(args) = cli.command;
    init_logging(args.verbose);
    match get(args).await {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("boltget: {e:#}");
            ExitCode::from(1)
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_get() {
        let cli = Cli::try_parse_from(["boltget", "get", "https://x.test/a", "-c", "4", "--json"])
            .unwrap();
        let Command::Get(a) = cli.command;
        assert_eq!(a.connections, Some(4));
        assert!(a.json);
        assert!(Cli::try_parse_from(["boltget", "get", "u", "-c", "0"]).is_err());
        assert!(Cli::try_parse_from(["boltget", "get", "u", "-c", "33"]).is_err());
    }

    #[test]
    fn exit_codes() {
        assert_eq!(exit_code_for(&Error::Cancelled), 130);
        assert_eq!(exit_code_for(&Error::Timeout("x")), 3);
        assert_eq!(exit_code_for(&Error::TooManyRedirects(10)), 4);
        assert_eq!(exit_code_for(&Error::ResourceChanged(String::new())), 6);
    }
}
