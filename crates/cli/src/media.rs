//! `boltget media`: the video tools (yt-dlp and FFmpeg) that Boltget installs
//! on demand, verifies and keeps up to date. See `docs/VIDEO.md`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use boltget_engine::{CookieJar, EngineConfig, ProxyRules};
use boltget_media::{
    AfterFailure, Channel, ContainerPreference, DownloadOptions, DownloadOutcome, Downloader,
    Error, ErrorCategory, InstallEvent, MediaInfo, MediaProgress, Outcome, Phase, PlaylistMode,
    Preset, ProbeOptions, ResolvedPreset, Sidecars, Sources, Status, SubtitleChoice, Tool,
    ToolStatus, codec_label, playlist, presets,
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
  9    a signature or checksum did not verify: nothing was installed

`probe` and `get` also exit with 4 when yt-dlp cannot read the page or the
download fails (its message is printed), and 8 when the tools are not
installed; `get` exits with 130 when stopped with Ctrl-C.";

const GET_HELP: &str = "\
Downloads a page's video or audio with yt-dlp into the folder given with -o
(the current folder by default), named \"<title> [<id>].<ext>\". The page is
probed first, as `probe` does, to resolve the preset (Best quality by
default, preferring MP4 unless --original is given); --format passes a yt-dlp
format expression instead. Progress is shown on stderr; the finished file's
path is printed on stdout and marked as downloaded from the internet, like
Boltget's other downloads. A proxy comes from the environment (HTTPS_PROXY,
ALL_PROXY, NO_PROXY), as for `boltget get`.

Ctrl-C stops the download and keeps yt-dlp's partial files: running the same
command again continues it. The tools are not installed by this command:
install them first with `boltget media install`.

Playlists: a playlist's page (or, with --playlist, a video's page that names
a list) is refused unless --playlist is given. With it, the entries (all, or
those --items names: places from 1 such as 1-5,8 or 10-; only places up to
the last one named are listed) are downloaded one after another into a
folder named after the playlist inside the output folder, each named \"<place> - <title> [<id>].<ext>\", with the same preset
for all: entries are not probed one by one, so a preset is used as yt-dlp's
format expression with its own fallbacks, and sizes are not known before.
Private and deleted entries are skipped. Each finished file's path is
printed; an entry that fails is reported and the others go on (exit code 4
at the end if any failed).";

const PROBE_HELP: &str = "\
Asks yt-dlp what the page offers, without downloading anything: the title,
length, the picker's presets (Best quality, 1080p, 720p, Smaller file, Audio
only; presets that would give the same file are shown once) with their
resolution, container, codecs and size (\"about\" when estimated), and the
subtitle languages. A playlist page lists its entries instead (with
--playlist, also the list a video's page names), each with its place, title
and length, \"(unavailable)\" for private and deleted ones. Due updates of the
tools are installed first (yt-dlp at most once a day).

--cookies passes a Netscape cookies.txt (as browser export extensions write
it); yt-dlp receives a private copy that is deleted afterwards.

With --json, stdout carries one JSON object:
  {title, id, uploader, duration, webpage_url, extractor, thumbnail,
   is_playlist, entry_count, entries: [{index, id, title, url, duration,
   uploader, thumbnail, available}], drm,
   container, presets: [{preset, expression, format_ids, audio_only, width,
   height, fps, dynamic_range, video_codec, audio_codec, container,
   size: {bytes, approximate} | null}],
   subtitles: [{code, name, automatic}]}
preset is best, 1080p, 720p, small or audio; container is mp4 or original
(the --original switch); duration is seconds; drm is true when everything on
the page is DRM-protected (then there are no presets). On failure stdout
carries {\"error\": {key, message}, \"exit_code\"}.";

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
    /// Show what video or audio a web page offers, with the picker's presets.
    #[command(after_long_help = PROBE_HELP)]
    Probe(ProbeArgs),
    /// Download a web page's video or audio.
    #[command(after_long_help = GET_HELP)]
    Get(GetArgs),
}

#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "command-line switches, each its own flag"
)]
pub(crate) struct GetArgs {
    /// The web page.
    url: String,
    /// The preset to download.
    #[arg(long, value_enum, default_value_t = PresetArg::Best, conflicts_with = "format")]
    preset: PresetArg,
    /// A yt-dlp format expression instead of a preset (e.g. "137+140").
    #[arg(long, value_name = "EXPR")]
    format: Option<String>,
    /// Prefer the site's original formats over MP4.
    #[arg(long)]
    original: bool,
    /// Subtitle languages to download, separated by commas (e.g. en,de).
    #[arg(long, value_name = "LANGS")]
    subs: Option<String>,
    /// Use automatic captions for languages without subtitles.
    #[arg(long, requires = "subs")]
    auto_subs: bool,
    /// Embed the subtitles in the video instead of saving them as files.
    #[arg(long, requires = "subs")]
    embed_subs: bool,
    /// The folder to save into (default: the current folder).
    #[arg(short = 'o', long = "output-dir", value_name = "DIR")]
    output_dir: Option<PathBuf>,
    /// A Netscape cookies.txt file for the site.
    #[arg(long, value_name = "FILE")]
    cookies: Option<PathBuf>,
    /// Download a playlist's entries (also the list a video's page names),
    /// one after another, into a folder named after it.
    #[arg(long)]
    playlist: bool,
    /// Only these places of the playlist (from 1): e.g. 1-5,8 or 10-.
    #[arg(long, value_name = "PLACES", requires = "playlist", value_parser = parse_items)]
    items: Option<playlist::Items>,
}

fn parse_items(s: &str) -> Result<playlist::Items, String> {
    playlist::parse_items(s)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PresetArg {
    Best,
    #[value(name = "1080p")]
    P1080,
    #[value(name = "720p")]
    P720,
    Small,
    Audio,
}

impl PresetArg {
    fn preset(self) -> Preset {
        match self {
            PresetArg::Best => Preset::Best,
            PresetArg::P1080 => Preset::P1080,
            PresetArg::P720 => Preset::P720,
            PresetArg::Small => Preset::Small,
            PresetArg::Audio => Preset::Audio,
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct ProbeArgs {
    /// The web page.
    url: String,
    /// A Netscape cookies.txt file for the site.
    #[arg(long, value_name = "FILE")]
    cookies: Option<PathBuf>,
    /// Prefer the site's original formats over MP4 (the default prefers MP4).
    #[arg(long)]
    original: bool,
    /// List the playlist the page belongs to (a watch page with a list).
    #[arg(long)]
    playlist: bool,
    /// Print JSON instead of text.
    #[arg(long)]
    json: bool,
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
        Error::Release { .. }
        | Error::Unsupported(_)
        | Error::Extraction { .. }
        | Error::Failed { .. } => 4,
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
        MediaAction::Probe(probe_args) => return probe(&sidecars, &shown, probe_args).await,
        MediaAction::Get(get_args) => return get(&sidecars, &shown, get_args).await,
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

/// `boltget media probe`: due updates first (as the app does before using
/// the tools), then yt-dlp on the page.
async fn probe(sidecars: &Sidecars, shown: &Shown, args: ProbeArgs) -> u8 {
    let on_event = |e: InstallEvent| shown.event(e);
    // A failed update check leaves the installed tools in place; the probe
    // goes on with them.
    if let Err(e) = sidecars.update_if_due(SystemTime::now(), &on_event).await {
        eprintln!("boltget: the update check failed: {e}");
    }
    shown.finish();
    let result = after_update(sidecars, async || probe_page(sidecars, &args).await).await;
    match result {
        Ok((info, container)) => {
            if args.json {
                println!("{}", probe_json(&info, container));
            } else {
                print_probe(&info, container);
            }
            0
        }
        Err(e) => {
            let code = exit_code(&e);
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "error": {"key": e.message_key(), "message": e.to_string()},
                        "exit_code": code,
                    })
                );
            } else {
                eprintln!("boltget: {e}");
                if matches!(e, Error::NotInstalled(_)) {
                    eprintln!("hint: install the video tools with `boltget media install`");
                }
            }
            code
        }
    }
}

async fn probe_page(
    sidecars: &Sidecars,
    args: &ProbeArgs,
) -> boltget_media::Result<(MediaInfo, ContainerPreference)> {
    let prober = sidecars.prober()?;
    let cookies = match &args.cookies {
        Some(path) => Some(CookieJar::load(path)?),
        None => None,
    };
    let opts = ProbeOptions {
        cookies,
        proxy: boltget_media::proxy_for(&ProxyRules::from_env(), args.url.trim()),
        playlist: if args.playlist {
            PlaylistMode::Entries
        } else {
            PlaylistMode::Single
        },
        ..ProbeOptions::default()
    };
    let container = if args.original {
        ContainerPreference::Original
    } else {
        ContainerPreference::Mp4
    };
    let info = prober.probe(args.url.trim(), &opts).await?;
    Ok((info, container))
}

/// `boltget media get`: due updates, a probe to resolve the preset, then
/// yt-dlp downloads with a progress bar until done or Ctrl-C.
async fn get(sidecars: &Sidecars, shown: &Shown, args: GetArgs) -> u8 {
    let on_event = |e: InstallEvent| shown.event(e);
    if let Err(e) = sidecars.update_if_due(SystemTime::now(), &on_event).await {
        eprintln!("boltget: the update check failed: {e}");
    }
    shown.finish();
    match after_update(sidecars, async || download(sidecars, &args).await).await {
        Ok(Got::File(path)) => {
            println!("{}", path.display());
            0
        }
        Ok(Got::Playlist { failed: 0 }) => 0,
        Ok(Got::Playlist { failed }) => {
            eprintln!("boltget: {failed} entries failed");
            4
        }
        Ok(Got::Stopped) => {
            eprintln!("boltget: stopped; run the same command again to continue");
            130
        }
        Err(e) => {
            eprintln!("boltget: {e}");
            if matches!(e, Error::NotInstalled(_)) {
                eprintln!("hint: install the video tools with `boltget media install`");
            }
            exit_code(&e)
        }
    }
}

/// How long a "site changed" failure waits for yt-dlp's update before the
/// command gives up on trying again (the update goes on regardless).
const UPDATE_ON_FAILURE_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Runs `work`; when yt-dlp failed because the site changed, checks for a
/// newer yt-dlp (at most once an hour) and, if one was installed, runs
/// `work` once more (docs/VIDEO.md, "Errors and pause/resume: as built").
async fn after_update<T>(
    sidecars: &Sidecars,
    work: impl AsyncFn() -> boltget_media::Result<T>,
) -> boltget_media::Result<T> {
    let first = work().await;
    match &first {
        Err(e) if e.category() == Some(ErrorCategory::SiteChanged) => {
            eprintln!("boltget: {e}");
            eprintln!("boltget: checking for a newer yt-dlp…");
            match sidecars
                .update_after_failure(SystemTime::now(), UPDATE_ON_FAILURE_WAIT)
                .await
            {
                AfterFailure::Updated { version } => {
                    eprintln!("boltget: updated yt-dlp to {version}; trying again");
                    work().await
                }
                AfterFailure::UpToDate => {
                    eprintln!("boltget: yt-dlp is up to date; a fix may take a few days");
                    first
                }
                AfterFailure::CheckedRecently => {
                    eprintln!("boltget: yt-dlp was checked for updates within the hour");
                    first
                }
                AfterFailure::Pending => {
                    eprintln!("boltget: the update check goes on; try again in a few minutes");
                    first
                }
                AfterFailure::CheckFailed(why) => {
                    eprintln!("boltget: the update check failed: {why}");
                    first
                }
                AfterFailure::NotInstalled => first,
            }
        }
        _ => first,
    }
}

/// The format expression, whether it is audio only, the expected size and
/// the container preference to download with, for `get`'s preset (or
/// `--format`): "Prefer MP4" becomes "Prefer original" for a preset whose
/// streams cannot go into MP4.
fn resolve(
    info: &MediaInfo,
    args: &GetArgs,
    container: ContainerPreference,
) -> boltget_media::Result<(String, bool, Option<u64>, ContainerPreference)> {
    let refuse = |category: ErrorCategory, message: &str| Error::Failed {
        category,
        message: message.to_owned(),
    };
    if info.is_playlist() {
        return Err(refuse(
            ErrorCategory::Other,
            "this page is a playlist; add --playlist to download its entries",
        ));
    }
    if info.drm_only() {
        return Err(refuse(
            ErrorCategory::Drm,
            "DRM-protected: Boltget cannot download this",
        ));
    }
    if let Some(expr) = &args.format {
        return Ok((expr.trim().to_owned(), false, None, container));
    }
    let wanted = args.preset.preset();
    let list = presets(info, container);
    // A preset collapsed into another resolves to the same formats, so it
    // is resolved itself; a height above the page's highest video is not
    // offered and resolves to the best video, as yt-dlp would.
    let own = boltget_media::presets::resolve(info, wanted, container);
    let chosen = list
        .iter()
        .find(|r| r.preset == wanted)
        .or(own.as_ref())
        .or_else(|| list.iter().find(|r| r.audio_only == wanted.audio_only()))
        .ok_or_else(|| {
            refuse(
                ErrorCategory::NoMedia,
                "no video or audio for this preset on this page",
            )
        })?;
    eprintln!("{}", preset_line(chosen).trim());
    if chosen.kept_original {
        eprintln!(
            "these streams cannot go into MP4; keeping {}",
            chosen.container.to_ascii_uppercase()
        );
    }
    Ok((
        chosen.expression.clone(),
        chosen.audio_only,
        chosen.size.map(|s| s.bytes),
        if chosen.kept_original {
            ContainerPreference::Original
        } else {
            container
        },
    ))
}

/// What `get` ended with.
enum Got {
    /// The page's file.
    File(PathBuf),
    /// A playlist's entries went one after another; `failed` did not finish.
    Playlist { failed: usize },
    /// Ctrl-C.
    Stopped,
}

/// The expression and whether it is audio only, for a playlist's entries:
/// the preset's expression (its `/` fallbacks choose for each entry), or
/// `--format`.
fn playlist_format(args: &GetArgs, container: ContainerPreference) -> (String, bool) {
    if let Some(expr) = &args.format {
        (expr.trim().to_owned(), false)
    } else {
        let preset = args.preset.preset();
        (preset.expression(container), preset.audio_only())
    }
}

/// Probes, resolves, downloads.
async fn download(sidecars: &Sidecars, args: &GetArgs) -> boltget_media::Result<Got> {
    let downloader = sidecars.downloader()?;
    let cookies = match &args.cookies {
        Some(path) => Some(CookieJar::load(path)?),
        None => None,
    };
    let container = if args.original {
        ContainerPreference::Original
    } else {
        ContainerPreference::Mp4
    };
    let url = args.url.trim();
    let proxy = boltget_media::proxy_for(&ProxyRules::from_env(), url);
    let info = sidecars
        .prober()?
        .probe(
            url,
            &ProbeOptions {
                cookies: cookies.clone(),
                proxy: proxy.clone(),
                playlist: if args.playlist {
                    PlaylistMode::Entries
                } else {
                    PlaylistMode::Single
                },
                // Places up to the last one named are enough to list.
                items: args
                    .items
                    .as_ref()
                    .and_then(playlist::Items::last)
                    .map(|end| boltget_media::ItemRange { start: 1, end }),
                ..ProbeOptions::default()
            },
        )
        .await?;
    let dest = match &args.output_dir {
        Some(d) => d.clone(),
        None => PathBuf::from("."),
    };
    let dest = std::path::absolute(&dest).map_err(|e| Error::Io {
        context: format!("resolving {}", dest.display()),
        source: e,
    })?;
    if args.playlist && info.is_playlist() {
        return download_playlist(&downloader, &info, args, container, cookies, &dest).await;
    }
    let (format, audio_only, expected, container) = resolve(&info, args, container)?;
    let mut opts = DownloadOptions::new(format, dest);
    opts.container = container;
    opts.audio_only = audio_only;
    opts.expected_size = expected;
    opts.cookies = cookies;
    opts.proxy = proxy;
    opts.subtitles = subtitles(args);
    if let Some(title) = &info.title {
        eprintln!("{title}");
    }
    Ok(
        match run_one(&downloader, url, &opts, expected.unwrap_or(0)).await? {
            DownloadOutcome::Completed {
                path,
                kept_original,
                ..
            } => {
                if kept_original {
                    eprintln!("these streams cannot go into MP4; kept the original format");
                }
                Got::File(path)
            }
            DownloadOutcome::Stopped => Got::Stopped,
        },
    )
}

/// The subtitles `--subs` asks for.
fn subtitles(args: &GetArgs) -> Option<SubtitleChoice> {
    args.subs.as_ref().map(|langs| SubtitleChoice {
        languages: langs
            .split(',')
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect(),
        automatic: args.auto_subs,
        embed: args.embed_subs,
    })
}

/// A playlist's chosen entries, one after another, into its folder.
async fn download_playlist(
    downloader: &Downloader,
    info: &MediaInfo,
    args: &GetArgs,
    container: ContainerPreference,
    cookies: Option<CookieJar>,
    dest: &Path,
) -> boltget_media::Result<Got> {
    let entries = playlist::entries(info, 1);
    let chosen: Vec<&boltget_media::PlaylistEntry> = entries
        .iter()
        .filter(|e| args.items.as_ref().is_none_or(|i| i.contains(e.index)))
        .collect();
    let folder = dest.join(playlist::folder_name(
        info.title.as_deref(),
        info.id.as_deref(),
    ));
    let last = u32::try_from(info.entry_count()).unwrap_or(u32::MAX);
    let (format, audio_only) = playlist_format(args, container);
    eprintln!(
        "{}: {} of {} entries into {}",
        info.title.as_deref().unwrap_or("playlist"),
        chosen.len(),
        info.entry_count(),
        folder.display()
    );
    let mut failed = 0;
    for e in chosen {
        let name = e.title.as_deref().unwrap_or("?");
        let Some(url) = e.url.as_deref().filter(|_| e.available) else {
            eprintln!("{:>4}. {name}: unavailable, skipped", e.index);
            continue;
        };
        eprintln!("{:>4}. {name}", e.index);
        let mut opts = DownloadOptions::new(format.clone(), folder.clone());
        opts.container = container;
        opts.audio_only = audio_only;
        opts.cookies.clone_from(&cookies);
        opts.proxy = boltget_media::proxy_for(&ProxyRules::from_env(), url);
        opts.subtitles = subtitles(args);
        opts.output_template = playlist::entry_template(e.index, last);
        match run_one(downloader, url, &opts, 0).await {
            Ok(DownloadOutcome::Completed { path, .. }) => println!("{}", path.display()),
            Ok(DownloadOutcome::Stopped) => return Ok(Got::Stopped),
            Err(err) => {
                eprintln!("      failed: {err}");
                failed += 1;
            }
        }
    }
    Ok(Got::Playlist { failed })
}

/// One download with a progress bar on stderr, until done or Ctrl-C.
async fn run_one(
    downloader: &Downloader,
    url: &str,
    opts: &DownloadOptions,
    expected: u64,
) -> boltget_media::Result<DownloadOutcome> {
    let bar = ProgressBar::new(expected);
    bar.set_style(
        ProgressStyle::with_template("  {msg} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=> "),
    );
    bar.set_message("downloading");
    let on_progress = |p: &MediaProgress| {
        if let Some(total) = p.total {
            bar.set_length(total);
        }
        bar.set_position(p.downloaded);
        if p.phase == Phase::Processing {
            bar.set_message("processing");
        }
    };
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let ctrl_c = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            stop.send_replace(true);
        }
    });
    let outcome = downloader.run(url, opts, &on_progress, stop_rx).await;
    ctrl_c.abort();
    bar.finish_and_clear();
    outcome
}

fn preset_name(p: Preset) -> &'static str {
    match p {
        Preset::Best => "Best quality",
        Preset::P1080 => "1080p",
        Preset::P720 => "720p",
        Preset::Small => "Smaller file",
        Preset::Audio => "Audio only",
    }
}

fn duration_text(secs: f64) -> String {
    let s = secs.round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// One preset as a line: resolution, container, codecs, size.
fn preset_line(r: &ResolvedPreset) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(h) = r.height {
        let mut res = format!("{h}p");
        if let Some(fps) = r.fps.filter(|f| *f > 30.5) {
            let _ = write!(res, "{}", fps.round());
        }
        if let Some(hdr) = &r.dynamic_range {
            let _ = write!(res, " {hdr}");
        }
        parts.push(res);
    }
    parts.push(r.container.to_uppercase());
    let codecs: Vec<String> = [&r.video_codec, &r.audio_codec]
        .into_iter()
        .flatten()
        .map(|c| codec_label(c))
        .collect();
    if !codecs.is_empty() {
        parts.push(codecs.join(" + "));
    }
    if let Some(size) = r.size {
        let about = if size.approximate { "about " } else { "" };
        parts.push(format!("{about}{}", indicatif::HumanBytes(size.bytes)));
    }
    format!("  {:<14}{}", preset_name(r.preset), parts.join("  "))
}

/// A playlist's entry as a line: place, title, length.
fn entry_line(e: &boltget_media::PlaylistEntry) -> String {
    let title = e.title.as_deref().or(e.url.as_deref()).unwrap_or("?");
    let mut line = format!("  {:>3}. {title}", e.index);
    if let Some(d) = e.duration {
        let _ = write!(line, "  {}", duration_text(d));
    }
    if !e.available {
        line.push_str("  (unavailable)");
    }
    line
}

fn print_probe(info: &MediaInfo, container: ContainerPreference) {
    println!("{}", info.title.as_deref().unwrap_or("(untitled)"));
    if let Some(who) = info.publisher() {
        println!("  by {who}");
    }
    if let Some(d) = info.duration {
        println!("  length {}", duration_text(d));
    }
    if info.is_playlist() {
        println!("playlist, {} entries:", info.entry_count());
        for e in playlist::entries(info, 1) {
            println!("{}", entry_line(&e));
        }
        return;
    }
    if info.drm_only() {
        println!("DRM-protected: Boltget cannot download this.");
        return;
    }
    let list = presets(info, container);
    if list.is_empty() {
        println!("no media found on this page");
        return;
    }
    let which = match container {
        ContainerPreference::Mp4 => "prefer MP4",
        ContainerPreference::Original => "prefer original",
    };
    println!("presets ({which}):");
    for r in &list {
        println!("{}", preset_line(r));
    }
    let langs = info.subtitle_languages();
    let (auto, own): (Vec<_>, Vec<_>) = langs.iter().partition(|l| l.automatic);
    if !own.is_empty() {
        let codes: Vec<&str> = own.iter().map(|l| l.code.as_str()).collect();
        println!("subtitles: {}", codes.join(", "));
    }
    if !auto.is_empty() {
        println!("automatic captions: {} languages", auto.len());
    }
}

fn probe_json(info: &MediaInfo, container: ContainerPreference) -> serde_json::Value {
    serde_json::json!({
        "title": info.title,
        "id": info.id,
        "uploader": info.publisher(),
        "duration": info.duration,
        "webpage_url": info.webpage_url,
        "extractor": info.extractor,
        "thumbnail": info.best_thumbnail(),
        "is_playlist": info.is_playlist(),
        "entry_count": info.entry_count(),
        "entries": playlist::entries(info, 1),
        "drm": info.drm_only(),
        "container": container,
        "presets": presets(info, container),
        "subtitles": info.subtitle_languages(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_probe() {
        use clap::Parser as _;
        let cli = crate::Cli::try_parse_from([
            "boltget",
            "media",
            "probe",
            "https://x.test/v",
            "--cookies",
            "c.txt",
            "--original",
            "--json",
        ])
        .unwrap();
        let crate::Command::Media(MediaArgs {
            action: MediaAction::Probe(p),
        }) = cli.command
        else {
            panic!("expected media probe");
        };
        assert_eq!(p.url, "https://x.test/v");
        assert_eq!(p.cookies.as_deref(), Some(Path::new("c.txt")));
        assert!(p.original && p.json && !p.playlist);
    }

    #[test]
    fn parses_get_with_a_playlist() {
        use clap::Parser as _;
        let cli = crate::Cli::try_parse_from([
            "boltget",
            "media",
            "get",
            "https://x.test/playlist?list=PL1",
            "--playlist",
            "--items",
            "1-5,8",
            "--preset",
            "720p",
        ])
        .unwrap();
        let crate::Command::Media(MediaArgs {
            action: MediaAction::Get(g),
        }) = cli.command
        else {
            panic!("expected media get");
        };
        assert!(g.playlist);
        let items = g.items.as_ref().unwrap();
        assert!(items.contains(8) && !items.contains(6));
        assert_eq!(
            playlist_format(&g, ContainerPreference::Mp4),
            (Preset::P720.expression(ContainerPreference::Mp4), false)
        );
        // --items needs --playlist, and only places.
        for bad in [
            vec![
                "boltget",
                "media",
                "get",
                "https://x.test/p",
                "--items",
                "1-3",
            ],
            vec![
                "boltget",
                "media",
                "get",
                "https://x.test/p",
                "--playlist",
                "--items",
                "1;calc",
            ],
        ] {
            assert!(crate::Cli::try_parse_from(bad).is_err());
        }
    }

    #[test]
    fn playlist_entries_as_lines_and_json() {
        let info = MediaInfo::from_json(
            br#"{"_type": "playlist", "title": "P", "entries": [
                {"id": "a", "url": "https://v.test/w?v=a", "title": "First", "duration": 61},
                {"id": "b", "url": "https://v.test/w?v=b", "title": "[Private video]"}
            ]}"#,
        )
        .unwrap();
        let list = playlist::entries(&info, 1);
        assert_eq!(entry_line(&list[0]), "    1. First  1:01");
        assert_eq!(
            entry_line(&list[1]),
            "    2. [Private video]  (unavailable)"
        );
        let json = probe_json(&info, ContainerPreference::Mp4);
        assert_eq!(json["entries"][1]["index"], 2);
        assert_eq!(json["entries"][1]["available"], false);
    }

    #[test]
    fn durations_and_preset_lines() {
        assert_eq!(duration_text(212.0), "3:32");
        assert_eq!(duration_text(3725.4), "1:02:05");
        assert_eq!(duration_text(0.4), "0:00");
        let info = MediaInfo::from_json(
            br#"{"duration": 100, "formats": [
                {"format_id": "a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "filesize": 1048576},
                {"format_id": "v", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "fps": 60, "dynamic_range": "HDR10", "filesize_approx": 10485760}
            ]}"#,
        )
        .unwrap();
        let list = presets(&info, ContainerPreference::Mp4);
        assert_eq!(
            preset_line(&list[0]),
            "  Best quality  1080p60 HDR10  MP4  H.264 + AAC  about 11.00 MiB"
        );
        assert_eq!(
            preset_line(list.last().unwrap()),
            "  Audio only    M4A  AAC  1.00 MiB"
        );
        let json = probe_json(&info, ContainerPreference::Mp4);
        assert_eq!(json["container"], "mp4");
        assert_eq!(json["presets"][0]["preset"], "best");
        assert_eq!(json["presets"][0]["size"]["approximate"], true);
        assert_eq!(json["drm"], false);
    }
}
