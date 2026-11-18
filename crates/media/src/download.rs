//! Downloading media: yt-dlp through the guarded runner, with progress read
//! from `--progress-template` lines (docs/VIDEO.md, "Picker and media
//! downloads: as built").
//!
//! A download is stopped (pause, remove, quit) by ending its process tree at
//! once: on Windows by terminating its Job Object. There is no "graceful"
//! step first: yt-dlp writes `.part` files (and `.ytdl` fragment state) as it
//! goes, so a restart with the same arguments continues from them, and a
//! console Ctrl+Break would need the app and yt-dlp to share a console, which
//! a GUI app does not have.
//!
//! **Prefer MP4 that cannot be MP4.** Under "Prefer MP4" a video is merged
//! into MP4 and a single format remuxed into it. When the streams yt-dlp
//! chose cannot go into MP4 as they are (VP8, Theora or Vorbis: see
//! [`crate::presets::mp4_compatible`]) FFmpeg fails ("Conversion failed!");
//! the run is then tried once more with "Prefer original", which finds the
//! downloaded streams already there and keeps their container. The picker
//! avoids this for a single page by starting such a preset with "Prefer
//! original" ([`crate::ResolvedPreset::kept_original`]); the second run is
//! for playlists and typed expressions, whose streams only yt-dlp knows.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use boltget_engine::{ContainerPreference, CookieJar, SubtitleChoice};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use tokio::sync::watch;

use crate::cookies::{CookieFile, JOBS_DIR};
use crate::failure::{ErrorCategory, Failure};
use crate::partial;
use crate::probe::header_value;
use crate::progress::{
    self, DOWNLOAD_TEMPLATE, FILE_TEMPLATE, Line, MediaProgress, Phase, ProgressTracker,
};
use crate::runner::Invocation;
use crate::{Error, Program, Result, Sidecars};

/// How long a download may go without a line of output before it is
/// considered stuck and stopped (yt-dlp reports progress many times a
/// second while it transfers, and post-processing a long video can take a
/// few minutes).
pub const STALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Progress is reported to the caller at most this often (yt-dlp may print
/// hundreds of lines a second on a fast line).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Fragments of an HLS or DASH stream downloaded at once (VIDEO.md, "Engine
/// involvement": the design target is 4).
pub const CONCURRENT_FRAGMENTS: u32 = 4;

/// The most standard error kept for the failure message.
const MAX_STDERR: usize = 64 * 1024;

/// How to download a page's media.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// The yt-dlp format expression (`-f`).
    pub format: String,
    /// "Prefer MP4" or "Prefer original".
    pub container: ContainerPreference,
    /// The result is audio only (the Audio only preset): the audio is
    /// extracted (`-x`), as M4A with "Prefer MP4".
    pub audio_only: bool,
    /// Subtitles to save or embed.
    pub subtitles: Option<SubtitleChoice>,
    /// The output template: a file name, no folders (`-o`).
    pub output_template: String,
    /// The folder (`-P`), absolute.
    pub dest_dir: PathBuf,
    /// Cookies for the site (passed through a temporary file).
    pub cookies: Option<CookieJar>,
    /// The browser's `User-Agent`.
    pub user_agent: Option<String>,
    /// The `Referer`.
    pub referer: Option<String>,
    /// A proxy URL for yt-dlp (`--proxy`): `http`, `https`, `socks5` or
    /// `socks5h`.
    pub proxy: Option<String>,
    /// HLS and DASH fragments at once.
    pub concurrent_fragments: u32,
    /// The picker's size estimate, for the progress total.
    pub expected_size: Option<u64>,
    /// See [`STALL_TIMEOUT`].
    pub stall_timeout: Duration,
    /// Mark the finished file as downloaded from the internet (Windows'
    /// Mark of the Web), as Boltget's own downloads are.
    pub mark_of_the_web: bool,
}

impl DownloadOptions {
    /// Options for `format` into `dest_dir` with the defaults: Prefer MP4,
    /// no subtitles, the default output template.
    pub fn new(format: impl Into<String>, dest_dir: impl Into<PathBuf>) -> Self {
        Self {
            format: format.into(),
            container: ContainerPreference::Mp4,
            audio_only: false,
            subtitles: None,
            output_template: boltget_engine::DEFAULT_OUTPUT_TEMPLATE.to_owned(),
            dest_dir: dest_dir.into(),
            cookies: None,
            user_agent: None,
            referer: None,
            proxy: None,
            concurrent_fragments: CONCURRENT_FRAGMENTS,
            expected_size: None,
            stall_timeout: STALL_TIMEOUT,
            mark_of_the_web: true,
        }
    }
}

/// How a download ended, when it did not fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadOutcome {
    /// The file is complete.
    Completed {
        /// Where it is.
        path: PathBuf,
        /// Its size.
        size: Option<u64>,
        /// "Prefer MP4" was asked for, but the streams could not go into MP4,
        /// so the file kept its original container (a second run with
        /// "Prefer original").
        kept_original: bool,
    },
    /// The caller stopped it; yt-dlp's partial files are kept.
    Stopped,
}

/// Runs yt-dlp to download media.
#[derive(Debug, Clone)]
pub struct Downloader {
    yt_dlp: PathBuf,
    ffmpeg_dir: Option<PathBuf>,
    jobs_dir: PathBuf,
}

impl Sidecars {
    /// A downloader using the active yt-dlp and FFmpeg (FFmpeg must be
    /// installed: merging and converting need it).
    pub fn downloader(&self) -> Result<Downloader> {
        let yt_dlp = self.program(Program::YtDlp)?;
        let ffmpeg = self.program(Program::Ffmpeg)?;
        Ok(Downloader::new(
            yt_dlp,
            ffmpeg.parent().map(Path::to_path_buf),
            self.dir().join(JOBS_DIR),
        ))
    }
}

impl Downloader {
    /// A downloader for the yt-dlp at `yt_dlp`, with FFmpeg in `ffmpeg_dir`,
    /// that keeps its temporary cookie files in `jobs_dir`.
    pub fn new(yt_dlp: PathBuf, ffmpeg_dir: Option<PathBuf>, jobs_dir: PathBuf) -> Self {
        Self {
            yt_dlp,
            ffmpeg_dir,
            jobs_dir,
        }
    }

    /// The arguments for downloading `url` (after the runner's fixed prefix
    /// and `--ffmpeg-location`).
    pub fn args(
        &self,
        url: &str,
        opts: &DownloadOptions,
        cookie_file: Option<&Path>,
    ) -> Result<Vec<String>> {
        let text = |p: &Path, what: &str| {
            p.to_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::Argument {
                    program: Program::YtDlp.name(),
                    detail: format!("the {what} is not valid Unicode"),
                })
        };
        let mut args: Vec<String> = [
            "--newline",
            // `--print` implies `--quiet`, which would hide the progress.
            "--progress",
            "--no-simulate",
            "--no-playlist",
            "--no-mtime",
            "--encoding",
            "utf-8",
            "--socket-timeout",
            "20",
        ]
        .map(str::to_owned)
        .to_vec();
        args.extend([
            "--progress-template".into(),
            format!("download:{DOWNLOAD_TEMPLATE}"),
            "--progress-template".into(),
            format!("postprocess:{}", progress::POSTPROCESS_TEMPLATE),
            "--print".into(),
            format!("after_move:{FILE_TEMPLATE}"),
        ]);
        args.extend(["-f".into(), opts.format.clone()]);
        args.extend(["-P".into(), text(&opts.dest_dir, "folder")?]);
        args.extend(["-o".into(), opts.output_template.clone()]);
        if opts.concurrent_fragments > 1 {
            args.extend([
                "--concurrent-fragments".into(),
                opts.concurrent_fragments.min(16).to_string(),
            ]);
        }
        let mp4 = opts.container == ContainerPreference::Mp4;
        match (opts.audio_only, mp4) {
            (true, true) => args.extend(["-x".into(), "--audio-format".into(), "m4a".into()]),
            (true, false) => args.push("-x".into()),
            (false, true) => args.extend([
                "--merge-output-format".into(),
                "mp4".into(),
                "--remux-video".into(),
                "mp4".into(),
            ]),
            (false, false) => {}
        }
        if let Some(subs) = opts.subtitles.as_ref().filter(|s| !s.languages.is_empty()) {
            args.push("--write-subs".into());
            if subs.automatic {
                args.push("--write-auto-subs".into());
            }
            args.extend(["--sub-langs".into(), subs.languages.join(",")]);
            if subs.embed && !opts.audio_only {
                // With `--write-subs`, yt-dlp keeps the subtitle file next to
                // the video after embedding it; this makes it delete the
                // file, as it does without `--write-subs`.
                args.extend([
                    "--embed-subs".into(),
                    "--compat-options".into(),
                    "no-keep-subs".into(),
                ]);
            } else {
                // Files next to the video: SubRip, which every player reads.
                args.extend(["--convert-subs".into(), "srt".into()]);
            }
        }
        if let Some(path) = cookie_file {
            args.extend(["--cookies".into(), text(path, "cookie file's path")?]);
        }
        if let Some(referer) = &opts.referer {
            args.extend(["--referer".into(), header_value(referer)?]);
        }
        if let Some(ua) = &opts.user_agent {
            args.extend([
                "--add-headers".into(),
                format!("User-Agent:{}", header_value(ua)?),
            ]);
        }
        if let Some(proxy) = &opts.proxy {
            args.extend(["--proxy".into(), header_value(proxy)?]);
        }
        args.extend(["--".into(), url.to_owned()]);
        crate::args::check_yt_dlp(&args)?;
        Ok(args)
    }

    /// Downloads the media of `url`, calling `on_progress` as it goes (at
    /// most every [`PROGRESS_INTERVAL`], and whenever the phase changes),
    /// until it completes, fails, or `stop` becomes `true`. Cookies, if any,
    /// go through a temporary file deleted when this returns. A video whose
    /// streams cannot go into MP4 under "Prefer MP4" keeps its original
    /// container (see the module documentation).
    pub async fn run(
        &self,
        url: &str,
        opts: &DownloadOptions,
        on_progress: &(dyn Fn(&MediaProgress) + Sync),
        stop: watch::Receiver<bool>,
    ) -> Result<DownloadOutcome> {
        let first = self.run_once(url, opts, on_progress, stop.clone()).await;
        let mp4_video = opts.container == ContainerPreference::Mp4 && !opts.audio_only;
        match first {
            Err(Error::Failed {
                category: ErrorCategory::Processing,
                message,
            }) if mp4_video => {
                tracing::info!(%message, "MP4 failed; keeping the original container");
                let original = DownloadOptions {
                    container: ContainerPreference::Original,
                    ..opts.clone()
                };
                Ok(
                    match self.run_once(url, &original, on_progress, stop).await? {
                        DownloadOutcome::Completed { path, size, .. } => {
                            DownloadOutcome::Completed {
                                path,
                                size,
                                kept_original: true,
                            }
                        }
                        DownloadOutcome::Stopped => DownloadOutcome::Stopped,
                    },
                )
            }
            other => other,
        }
    }

    async fn run_once(
        &self,
        url: &str,
        opts: &DownloadOptions,
        on_progress: &(dyn Fn(&MediaProgress) + Sync),
        mut stop: watch::Receiver<bool>,
    ) -> Result<DownloadOutcome> {
        if *stop.borrow() {
            return Ok(DownloadOutcome::Stopped);
        }
        std::fs::create_dir_all(&opts.dest_dir)
            .map_err(|e| Error::io(format!("creating {}", opts.dest_dir.display()), e))?;
        let cookie_file = match &opts.cookies {
            Some(jar) if !jar.is_empty() => Some(CookieFile::create(&self.jobs_dir, jar)?),
            _ => None,
        };
        let args = self.args(url, opts, cookie_file.as_ref().map(CookieFile::path))?;
        let started = std::time::SystemTime::now();
        let mut running = Invocation::yt_dlp(&self.yt_dlp, self.ffmpeg_dir.as_deref(), &args)?
            .current_dir(&opts.dest_dir)
            .spawn()?;
        let stdout = running.child().stdout.take();
        let stderr = running.child().stderr.take();
        let stderr_task = tokio::spawn(async move {
            let mut kept: Vec<u8> = Vec::new();
            let Some(mut stream) = stderr else {
                return kept;
            };
            let mut buf = vec![0u8; 8192];
            while let Ok(n) = stream.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > MAX_STDERR {
                    kept.drain(..kept.len() - MAX_STDERR);
                }
            }
            kept
        });
        let mut tracker = ProgressTracker::new(opts.expected_size);
        let mut file: Option<PathBuf> = None;
        let mut stems: Vec<String> = Vec::new();
        let mut last_sent: Option<(Instant, Phase)> = None;
        let mut send = |tracker: &ProgressTracker, force: bool| {
            let p = tracker.progress();
            let due = match last_sent {
                None => true,
                Some((at, phase)) => force || phase != p.phase || at.elapsed() >= PROGRESS_INTERVAL,
            };
            if due {
                on_progress(p);
                last_sent = Some((Instant::now(), p.phase));
            }
        };
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                tokio::select! {
                    read = reader.read_until(b'\n', &mut line) => {
                        match read {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                let text = String::from_utf8_lossy(&line);
                                let line = progress::parse_line(&text);
                                if let Some(Line::Download(p)) = &line
                                    && let Some(stem) = p
                                        .filename
                                        .as_deref()
                                        .and_then(|f| partial::stem_of(Path::new(f)))
                                    && !stems.contains(&stem)
                                {
                                    stems.push(stem);
                                }
                                match line {
                                    Some(Line::File(path)) => file = Some(path),
                                    Some(l) if tracker.apply(&l) => send(&tracker, false),
                                    _ => {}
                                }
                            }
                        }
                    }
                    () = stopped(&mut stop) => {
                        running.stop();
                        let _ = running.child().wait().await;
                        return Ok(DownloadOutcome::Stopped);
                    }
                    () = tokio::time::sleep(opts.stall_timeout) => {
                        running.stop();
                        let _ = running.child().wait().await;
                        return Err(Error::Timeout {
                            program: Program::YtDlp.name().to_owned(),
                            timeout: opts.stall_timeout,
                        });
                    }
                }
            }
        }
        let status = tokio::select! {
            status = running.child().wait() => {
                status.map_err(|e| Error::io("waiting for yt-dlp", e))?
            }
            () = stopped(&mut stop) => {
                running.stop();
                let _ = running.child().wait().await;
                return Ok(DownloadOutcome::Stopped);
            }
        };
        let stderr = stderr_task.await.unwrap_or_default();
        drop(cookie_file);
        if !status.success() {
            let failure = Failure::from_stderr(&String::from_utf8_lossy(&stderr), status);
            if failure.category == ErrorCategory::Processing {
                // FFmpeg's unfinished output is of no use; the downloaded
                // streams stay for the next run.
                for stem in &stems {
                    remove_failed_outputs(&opts.dest_dir, stem, started);
                }
            }
            return Err(Error::Failed {
                category: failure.category,
                message: failure.message,
            });
        }
        let path = file.ok_or_else(|| Error::Failed {
            category: ErrorCategory::Other,
            message: "yt-dlp finished without naming the file it wrote".into(),
        })?;
        // yt-dlp prints the path as it saw it; relative only if -P was.
        let path = if path.is_absolute() {
            path
        } else {
            opts.dest_dir.join(path)
        };
        let size = std::fs::metadata(&path).ok().map(|m| m.len());
        if opts.mark_of_the_web && path.is_file() {
            boltget_engine::mark_of_the_web(&path, &redacted(url), None);
        }
        send(&tracker, true);
        Ok(DownloadOutcome::Completed {
            path,
            size,
            kept_original: false,
        })
    }
}

/// Deletes what FFmpeg left of a failed post-processing step of the job
/// named `stem` in `dir`: the merger's `<stem>.temp.<ext>`, and the MP4 the
/// remuxer writes directly (`<stem>.mp4`) if this run, started at `started`,
/// wrote it. Nothing else.
fn remove_failed_outputs(dir: &Path, stem: &str, started: std::time::SystemTime) {
    let mut doomed: Vec<PathBuf> = partial::files(dir, stem, partial::Scope::Temporary)
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix(stem))
                .is_some_and(|rest| rest.starts_with(".temp."))
        })
        .collect();
    let mp4 = dir.join(format!("{stem}.mp4"));
    let ours = std::fs::metadata(&mp4)
        .and_then(|m| m.modified())
        .is_ok_and(|at| at >= started);
    if ours {
        doomed.push(mp4);
    }
    for p in doomed {
        if let Err(e) = std::fs::remove_file(&p) {
            tracing::debug!(path = %p.display(), error = %e, "could not delete FFmpeg's output");
        }
    }
}

/// The proxy yt-dlp should use for `url` under `rules` (Boltget's proxy
/// settings), as a `--proxy` value: `None` goes direct. SOCKS5 becomes
/// `socks5h://`, so names are resolved by the proxy as in Boltget's own
/// downloads. Credentials are never stored with proxies, so none are passed.
pub fn proxy_for(rules: &boltget_engine::ProxyRules, url: &str) -> Option<String> {
    let p = rules.for_url(url)?;
    let plain = p.without_credentials();
    Some(match p.kind() {
        boltget_engine::ProxyKind::Socks5 => plain.replacen("socks5://", "socks5h://", 1),
        _ => plain,
    })
}

/// Resolves once `stop` is `true`; never, if its sender is gone.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The page's address without query, fragment or credentials, for the Mark
/// of the Web.
fn redacted(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string()
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\you\Videos")
        } else {
            PathBuf::from("/home/you/Videos")
        }
    }

    fn d() -> Downloader {
        Downloader::new(
            "/t/yt-dlp".into(),
            Some("/t/ffmpeg".into()),
            "/t/.jobs".into(),
        )
    }

    fn after<'a>(args: &'a [String], opt: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == opt)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    #[test]
    fn a_video_in_mp4_with_embedded_subtitles() {
        let mut opts = DownloadOptions::new("bv*[ext=mp4]+ba[ext=m4a]/b", folder());
        opts.subtitles = Some(SubtitleChoice {
            languages: vec!["en".into(), "pt-BR".into()],
            automatic: true,
            embed: true,
        });
        opts.proxy = Some("socks5h://127.0.0.1:1080".into());
        let args = d().args("https://v.test/w?v=1", &opts, None).unwrap();
        assert_eq!(after(&args, "-f"), Some("bv*[ext=mp4]+ba[ext=m4a]/b"));
        assert_eq!(after(&args, "-o"), Some("%(title)s [%(id)s].%(ext)s"));
        assert_eq!(after(&args, "-P"), folder().to_str());
        assert_eq!(after(&args, "--merge-output-format"), Some("mp4"));
        assert_eq!(after(&args, "--remux-video"), Some("mp4"));
        assert_eq!(after(&args, "--concurrent-fragments"), Some("4"));
        assert_eq!(after(&args, "--sub-langs"), Some("en,pt-BR"));
        // The subtitle file goes once it is in the video.
        assert_eq!(after(&args, "--compat-options"), Some("no-keep-subs"));
        assert_eq!(after(&args, "--proxy"), Some("socks5h://127.0.0.1:1080"));
        assert_eq!(
            after(&args, "--print"),
            Some("after_move:[boltget] file %(filepath)j")
        );
        for flag in [
            "--write-subs",
            "--write-auto-subs",
            "--embed-subs",
            "--newline",
            "--progress",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        assert!(!args.iter().any(|a| a == "-x" || a == "--convert-subs"));
        let templates: Vec<&str> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--progress-template")
            .filter_map(|(i, _)| args.get(i + 1).map(String::as_str))
            .collect();
        assert_eq!(
            templates,
            [
                "download:[boltget] download %(progress)j",
                "postprocess:[boltget] postprocess %(progress)j"
            ]
        );
        assert_eq!(&args[args.len() - 2..], ["--", "https://v.test/w?v=1"]);
    }

    #[test]
    fn audio_only_and_original_formats() {
        let mut opts = DownloadOptions::new("ba[ext=m4a]/ba/b", folder());
        opts.audio_only = true;
        opts.subtitles = Some(SubtitleChoice {
            languages: vec!["de".into()],
            automatic: false,
            embed: true,
        });
        let args = d().args("https://v.test/a", &opts, None).unwrap();
        assert!(args.iter().any(|a| a == "-x"));
        assert_eq!(after(&args, "--audio-format"), Some("m4a"));
        // Audio files cannot carry subtitles: they are saved next to it.
        assert_eq!(after(&args, "--convert-subs"), Some("srt"));
        assert!(
            !args.iter().any(|a| a == "--write-auto-subs"
                || a == "--embed-subs"
                || a == "--compat-options")
        );
        opts.container = ContainerPreference::Original;
        let args = d().args("https://v.test/a", &opts, None).unwrap();
        assert!(args.iter().any(|a| a == "-x"));
        assert_eq!(after(&args, "--audio-format"), None);
        opts.audio_only = false;
        opts.subtitles = None;
        opts.concurrent_fragments = 1;
        let args = d().args("https://v.test/a", &opts, None).unwrap();
        for opt in [
            "-x",
            "--merge-output-format",
            "--remux-video",
            "--write-subs",
            "--concurrent-fragments",
        ] {
            assert!(!args.iter().any(|a| a == opt), "{opt}");
        }
    }

    #[test]
    fn unsafe_values_are_refused() {
        let base = DownloadOptions::new("b", folder());
        let mut o = base.clone();
        o.output_template = "../%(title)s.%(ext)s".into();
        assert!(d().args("https://v.test/", &o, None).is_err());
        let mut o = base.clone();
        o.dest_dir = PathBuf::from("relative");
        assert!(d().args("https://v.test/", &o, None).is_err());
        let mut o = base.clone();
        o.proxy = Some("ftp://p.test".into());
        assert!(d().args("https://v.test/", &o, None).is_err());
        let mut o = base.clone();
        o.subtitles = Some(SubtitleChoice {
            languages: vec!["en.*".into()],
            automatic: false,
            embed: false,
        });
        assert!(d().args("https://v.test/", &o, None).is_err());
        let mut o = base;
        o.user_agent = Some("x\r\nCookie: a".into());
        assert!(d().args("https://v.test/", &o, None).is_err());
        assert!(
            d().args("--exec=calc", &DownloadOptions::new("b", folder()), None)
                .is_err()
        );
    }

    #[test]
    fn proxies_for_yt_dlp() {
        use boltget_engine::{Proxy, ProxyRules};
        let socks = ProxyRules::all(Proxy::parse("socks5://127.0.0.1:9050").unwrap());
        assert_eq!(
            proxy_for(&socks, "https://v.test/").as_deref(),
            Some("socks5h://127.0.0.1:9050")
        );
        let http = ProxyRules::from_windows("proxy.corp:8080", "<local>");
        assert_eq!(
            proxy_for(&http, "https://v.test/").as_deref(),
            Some("http://proxy.corp:8080")
        );
        assert_eq!(proxy_for(&ProxyRules::direct(), "https://v.test/"), None);
        let args = d()
            .args(
                "https://v.test/",
                &DownloadOptions {
                    proxy: proxy_for(&socks, "https://v.test/"),
                    ..DownloadOptions::new("b", folder())
                },
                None,
            )
            .unwrap();
        assert!(args.iter().any(|a| a == "socks5h://127.0.0.1:9050"));
    }

    #[test]
    fn the_mark_of_the_web_names_the_page_without_secrets() {
        assert_eq!(
            redacted("https://u:p@v.test/watch?v=1&t=secret#x"),
            "https://v.test/watch"
        );
        assert_eq!(redacted("not a url"), "");
    }
}
