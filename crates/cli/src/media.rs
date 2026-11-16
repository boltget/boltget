//! `boltget media`: the video tools (yt-dlp and FFmpeg) that Boltget installs
//! on demand, verifies and keeps up to date. See `docs/VIDEO.md`.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use boltget_engine::{EngineConfig, ProxyRules};
use boltget_media::{
    Channel, Error, InstallEvent, Outcome, Sidecars, Sources, Status, Tool, ToolStatus,
};
use clap::{Args, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};

pub(crate) const HELP: &str = "\
Video downloads need two tools that Boltget does not bundle: yt-dlp and FFmpeg.
They are installed into the tools directory (the state directory's `tools`,
%LOCALAPPDATA%\\Boltget\\tools by default) only when asked for, downloaded
from their pinned sources over HTTPS, and verified before they ever run:
yt-dlp by its release key's signature over its checksum file and then its
checksum, FFmpeg by the checksum Boltget pins for each release. A new version
becomes active only after it starts and reports its version; the previous one
is kept so a bad update can be undone. yt-dlp's own updater is never used.

Exit codes:
  0    success
  1    the new version failed its smoke test (the previous one stays active)
  2    invalid command line
  3    network failure after all retries
  4    the release could not be used, or the tool has no build for this
       platform
  5    local disk problem
  7    another Boltget process is installing tools right now
  8    the tool is not installed, or there is no earlier version to go back to
  9    a signature or checksum did not verify: nothing was installed";

#[derive(Debug, Args)]
pub(crate) struct MediaArgs {
    #[command(subcommand)]
    pub(crate) action: MediaAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum MediaAction {
    /// Show which versions are installed and active.
    Status {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Install the tools that are missing (both by default).
    Install {
        /// Which tool.
        #[arg(value_enum, default_value_t = Which::All)]
        tool: Which,
    },
    /// Install the latest release of the installed tools (or of one tool,
    /// installing it if missing).
    Update {
        /// Which tool.
        #[arg(value_enum, default_value_t = Which::All)]
        tool: Which,
        /// Only check what is due: yt-dlp once a day, FFmpeg when this
        /// Boltget pins a different build.
        #[arg(long)]
        if_due: bool,
    },
    /// Run each installed tool's smoke test; a tool that fails it goes back
    /// to its previous version.
    Check,
    /// Go back to the previous version of a tool.
    Rollback {
        /// Which tool.
        #[arg(value_enum)]
        tool: One,
    },
    /// Choose yt-dlp's release channel (stable by default).
    Channel {
        /// The channel.
        #[arg(value_enum)]
        channel: ChannelArg,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Which {
    All,
    YtDlp,
    Ffmpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum One {
    YtDlp,
    Ffmpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ChannelArg {
    Stable,
    Nightly,
}

impl Which {
    fn tools(self) -> Vec<Tool> {
        match self {
            Which::All => Tool::ALL.to_vec(),
            Which::YtDlp => vec![Tool::YtDlp],
            Which::Ffmpeg => vec![Tool::Ffmpeg],
        }
    }
}

impl One {
    fn tool(self) -> Tool {
        match self {
            One::YtDlp => Tool::YtDlp,
            One::Ffmpeg => Tool::Ffmpeg,
        }
    }
}

/// The exit code for a media error.
fn exit_code(e: &Error) -> u8 {
    match e {
        Error::Download(e) => crate::exit_code_for(e),
        Error::Signature { .. } | Error::Checksum { .. } => 9,
        Error::Release { .. } | Error::Unsupported(_) => 4,
        Error::NotInstalled(_) | Error::NoPrevious(_) => 8,
        Error::Busy(_) => 7,
        Error::Argument { .. } => 2,
        Error::Timeout { .. } => 3,
        Error::Io { .. } | Error::Manifest { .. } | Error::Archive { .. } => 5,
        _ => 1,
    }
}

fn fail(e: &Error) -> u8 {
    eprintln!("boltget: {e}");
    exit_code(e)
}

pub(crate) async fn run(args: MediaArgs, state: Option<&Path>) -> u8 {
    let Some(dir) = state
        .map(|s| s.join("tools"))
        .or_else(Sidecars::default_dir)
    else {
        eprintln!("boltget: cannot find a data directory; pass --state <DIR>");
        return 5;
    };
    let mut engine = EngineConfig::default();
    engine.proxy = ProxyRules::from_env();
    let sidecars = Sidecars::new(dir, Sources::official(), engine);
    let shown = Shown::default();
    let on_event = |e: InstallEvent| shown.event(e);
    let result = match args.action {
        MediaAction::Status { json } => status(&sidecars, json),
        MediaAction::Install { tool } => {
            each(&tool.tools(), async |t| {
                sidecars.install(t, &on_event).await
            })
            .await
        }
        MediaAction::Update { tool, if_due } if if_due && tool == Which::All => sidecars
            .update_if_due(SystemTime::now(), &on_event)
            .await
            .map(|outs| outs.iter().for_each(report)),
        MediaAction::Update { tool, .. } => {
            let installed = sidecars.status().map(|s| s.tools).unwrap_or_default();
            let wanted: Vec<Tool> = tool
                .tools()
                .into_iter()
                .filter(|t| {
                    tool != Which::All
                        || installed.iter().any(|s| s.tool == *t && s.active.is_some())
                })
                .collect();
            if wanted.is_empty() {
                println!("nothing installed yet; install with: boltget media install");
            }
            each(&wanted, async |t| sidecars.update(t, &on_event).await).await
        }
        MediaAction::Check => check(&sidecars, &on_event).await,
        MediaAction::Rollback { tool } => sidecars.rollback(tool.tool()).map(|v| {
            println!("{}: {v} is active again", tool.tool());
        }),
        MediaAction::Channel { channel } => {
            let channel = match channel {
                ChannelArg::Stable => Channel::Stable,
                ChannelArg::Nightly => Channel::Nightly,
            };
            sidecars.set_channel(channel).map(|()| {
                println!(
                    "yt-dlp follows the {} channel from the next update",
                    channel.as_str()
                );
            })
        }
    };
    shown.finish();
    match result {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

async fn each(
    tools: &[Tool],
    f: impl AsyncFn(Tool) -> boltget_media::Result<Outcome>,
) -> boltget_media::Result<()> {
    for &t in tools {
        report(&f(t).await?);
    }
    Ok(())
}

fn report(o: &Outcome) {
    match o {
        Outcome::Installed {
            tool,
            version,
            previous: Some(p),
        } => println!("{tool}: {version} installed and active (was {p})"),
        Outcome::Installed { tool, version, .. } => {
            println!("{tool}: {version} installed and active");
        }
        Outcome::UpToDate { tool, version } => println!("{tool}: {version} is up to date"),
        Outcome::NotDue { tool } => println!("{tool}: no check due"),
        Outcome::NotInstalled { tool } => println!("{tool}: not installed"),
        _ => {}
    }
}

async fn check(
    sidecars: &Sidecars,
    on_event: &(dyn Fn(InstallEvent) + Sync),
) -> boltget_media::Result<()> {
    for tool in Tool::ALL {
        match sidecars.verify_active(tool, on_event).await {
            Ok(v) => println!("{tool}: {v} works"),
            Err(Error::NotInstalled(_)) => println!("{tool}: not installed"),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn status(sidecars: &Sidecars, json: bool) -> boltget_media::Result<()> {
    let s = sidecars.status()?;
    if json {
        println!("{}", status_json(&s));
        return Ok(());
    }
    println!("tools directory: {}", s.dir.display());
    for t in &s.tools {
        println!("{}", status_line(t));
        for (p, path) in &t.programs {
            println!("  {p}: {}", path.display());
        }
    }
    Ok(())
}

fn status_line(t: &ToolStatus) -> String {
    let mut line = match (&t.active, t.supported) {
        (Some(v), _) => format!("{}: {v}", t.tool),
        (None, true) => format!("{}: not installed", t.tool),
        (None, false) => format!("{}: no build for this platform", t.tool),
    };
    if let Some(p) = &t.previous {
        let _ = write!(line, " (previous: {p})");
    }
    if t.tool == Tool::YtDlp && t.channel == Channel::Nightly {
        line.push_str(" [nightly]");
    }
    if let Some(at) = t.last_check
        && let Ok(ago) = SystemTime::now().duration_since(at)
    {
        let _ = write!(line, ", checked {} ago", human_ago(ago));
    }
    line
}

fn human_ago(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        3600..86_400 => format!("{} h", s / 3600),
        _ => format!("{} days", s / 86_400),
    }
}

fn status_json(s: &Status) -> serde_json::Value {
    let tools: Vec<serde_json::Value> = s
        .tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "tool": t.tool.id(),
                "active": t.active,
                "previous": t.previous,
                "channel": t.channel.as_str(),
                "last_check": t.last_check.and_then(|at| at.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
                "supported": t.supported,
                "programs": t.programs.iter().map(|(p, path)| (p.name().to_owned(), serde_json::Value::from(path.display().to_string()))).collect::<serde_json::Map<_, _>>(),
            })
        })
        .collect();
    serde_json::json!({ "dir": s.dir.display().to_string(), "tools": tools })
}

/// Shows install progress on stderr: a line per step, a bar per large file.
#[derive(Default)]
struct Shown {
    bar: Mutex<Option<(String, ProgressBar)>>,
}

impl Shown {
    fn event(&self, e: InstallEvent) {
        let mut bar = self
            .bar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match e {
            InstallEvent::Downloading {
                tool,
                file,
                done,
                total,
            } => {
                // Metadata files are small: no bar for them.
                if total.is_none_or(|t| t < 1024 * 1024) {
                    return;
                }
                if bar.as_ref().is_none_or(|(f, _)| *f != file) {
                    if let Some((_, b)) = bar.take() {
                        b.finish_and_clear();
                    }
                    let b = ProgressBar::new(total.unwrap_or(0));
                    b.set_style(
                        ProgressStyle::with_template(
                            "  {msg} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec}",
                        )
                        .unwrap_or_else(|_| ProgressStyle::default_bar())
                        .progress_chars("=> "),
                    );
                    b.set_message(format!("{tool}: {file}"));
                    *bar = Some((file, b));
                }
                if let Some((_, b)) = bar.as_ref() {
                    b.set_position(done);
                }
            }
            other => {
                if let Some((_, b)) = bar.take() {
                    b.finish_and_clear();
                }
                let line = match other {
                    InstallEvent::Checking { tool } => {
                        format!("{tool}: checking for a new version")
                    }
                    InstallEvent::Verifying { tool } => format!("{tool}: verifying"),
                    InstallEvent::Unpacking { tool } => format!("{tool}: unpacking"),
                    InstallEvent::SmokeTesting { tool, version } => {
                        format!("{tool}: testing {version}")
                    }
                    InstallEvent::RolledBack { tool, from, to } => {
                        format!("{tool}: {from} failed its test; back to {to}")
                    }
                    _ => return,
                };
                eprintln!("{line}");
            }
        }
    }

    fn finish(&self) {
        if let Some((_, b)) = self
            .bar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            b.finish_and_clear();
        }
    }
}
