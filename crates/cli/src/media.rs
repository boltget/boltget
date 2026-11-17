//! `boltget media`: the video tools (yt-dlp and FFmpeg) that Boltget installs
//! on demand, verifies and keeps up to date. See `docs/VIDEO.md`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use boltget_engine::{CookieJar, EngineConfig, ProxyRules};
use boltget_media::{
    Channel, ContainerPreference, DownloadOptions, DownloadOutcome, Error, InstallEvent, MediaInfo,
    MediaProgress, Outcome, Phase, PlaylistMode, Preset, ProbeOptions, ResolvedPreset, Sidecars,
    Sources, Status, SubtitleChoice, Tool, ToolStatus, codec_label, presets,
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
install them first with `boltget media install`.";

const PROBE_HELP: &str = "\
Asks yt-dlp what the page offers, without downloading anything: the title,
length, the picker's presets (Best quality, 1080p, 720p, Smaller file, Audio
only; presets that would give the same file are shown once) with their
resolution, container, codecs and size (\"about\" when estimated), and the
subtitle languages. A playlist page lists its entries instead. Due updates of
the tools are installed first (yt-dlp at most once a day).

--cookies passes a Netscape cookies.txt (as browser export extensions write
it); yt-dlp receives a private copy that is deleted afterwards.

With --json, stdout carries one JSON object:
  {title, id, uploader, duration, webpage_url, extractor, thumbnail,
   is_playlist, entry_count, entries: [{id, title, url, duration}], drm,
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
    let result = probe_page(sidecars, &args).await;
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
    match download(sidecars, &args).await {
        Ok(Some(path)) => {
            println!("{}", path.display());
            0
        }
        Ok(None) => {
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

/// The format expression, whether it is audio only, and the expected size,
/// for `get`'s preset (or `--format`).
fn resolve(
    info: &MediaInfo,
    args: &GetArgs,
    container: ContainerPreference,
) -> boltget_media::Result<(String, bool, Option<u64>)> {
    let refuse = |message: &str| Error::Failed {
        message: message.to_owned(),
    };
    if info.is_playlist() {
        return Err(refuse(
            "this page is a playlist; playlists come in a later version (try the address of one video)",
        ));
    }
    if info.drm_only() {
        return Err(refuse("DRM-protected: Boltget cannot download this"));
    }
    if let Some(expr) = &args.format {
        return Ok((expr.trim().to_owned(), false, None));
    }
    let wanted = args.preset.preset();
    let list = presets(info, container);
    // A preset collapsed into another resolves to the same file.
    let chosen = list
        .iter()
        .find(|r| r.preset == wanted)
        .or_else(|| list.iter().find(|r| r.audio_only == wanted.audio_only()))
        .ok_or_else(|| refuse("no video or audio for this preset on this page"))?;
    eprintln!("{}", preset_line(chosen).trim());
    Ok((
        chosen.expression.clone(),
        chosen.audio_only,
        chosen.size.map(|s| s.bytes),
    ))
}

/// Probes, resolves, downloads. `None`: stopped with Ctrl-C.
async fn download(sidecars: &Sidecars, args: &GetArgs) -> boltget_media::Result<Option<PathBuf>> {
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
                ..ProbeOptions::default()
            },
        )
        .await?;
    let (format, audio_only, expected) = resolve(&info, args, container)?;
    let dest = match &args.output_dir {
        Some(d) => d.clone(),
        None => PathBuf::from("."),
    };
    let dest = std::path::absolute(&dest).map_err(|e| Error::Io {
        context: format!("resolving {}", dest.display()),
        source: e,
    })?;
    let mut opts = DownloadOptions::new(format, dest);
    opts.container = container;
    opts.audio_only = audio_only;
    opts.expected_size = expected;
    opts.cookies = cookies;
    opts.proxy = proxy;
    opts.subtitles = args.subs.as_ref().map(|langs| SubtitleChoice {
        languages: langs
            .split(',')
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect(),
        automatic: args.auto_subs,
        embed: args.embed_subs,
    });
    if let Some(title) = &info.title {
        eprintln!("{title}");
    }
    let bar = ProgressBar::new(expected.unwrap_or(0));
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
    let outcome = downloader.run(url, &opts, &on_progress, stop_rx).await;
    ctrl_c.abort();
    bar.finish_and_clear();
    Ok(match outcome? {
        DownloadOutcome::Completed { path, .. } => Some(path),
        DownloadOutcome::Stopped => None,
    })
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
        for (i, e) in info.entries.iter().enumerate() {
            let length = e.duration.map(duration_text).unwrap_or_default();
            println!(
                "  {:>3}. {}  {length}",
                i + 1,
                e.title.as_deref().or(e.url.as_deref()).unwrap_or("?")
            );
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
        "entries": info.entries,
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
