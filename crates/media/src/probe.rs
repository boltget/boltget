//! Probing a web page: `yt-dlp -J` through the guarded runner, parsed into
//! [`MediaInfo`] (`docs/VIDEO.md`, "How a media download flows" and
//! "Probing and presets: as built").

use std::path::{Path, PathBuf};
use std::time::Duration;

use boltget_engine::CookieJar;

use crate::cookies::{CookieFile, JOBS_DIR};
use crate::info::MediaInfo;
use crate::playlist::ItemRange;
use crate::runner::Invocation;
use crate::{Error, Program, Result, Sidecars};

/// How long a probe may take. Generous: yt-dlp's first start can be slow
/// while an antivirus product scans it, and some sites answer slowly.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// The most info JSON read from yt-dlp. A video's is usually well under a
/// megabyte; a flat playlist of thousands of entries a few.
pub const MAX_INFO_JSON: usize = 32 * 1024 * 1024;

/// yt-dlp's own network timeout for each request (seconds).
const SOCKET_TIMEOUT: &str = "20";

/// What to do with a page that is (or belongs to) a playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaylistMode {
    /// The video itself when the address names one (`--no-playlist`); a page
    /// that is only a playlist still lists its entries, without resolving
    /// them (`--flat-playlist`), which keeps the probe fast.
    #[default]
    Single,
    /// The playlist the address belongs to, its entries unresolved
    /// (`--yes-playlist --flat-playlist`).
    Entries,
}

/// How to probe a page.
#[derive(Debug, Clone, Default)]
pub struct ProbeOptions {
    /// Cookies for the site (passed through a temporary file).
    pub cookies: Option<CookieJar>,
    /// The browser's `User-Agent`.
    pub user_agent: Option<String>,
    /// The `Referer`.
    pub referer: Option<String>,
    /// A proxy URL for yt-dlp (`--proxy`).
    pub proxy: Option<String>,
    /// Playlist handling.
    pub playlist: PlaylistMode,
    /// Which of a playlist's entries to list (`--playlist-items`); `None`
    /// lists them all. A probe lists at most a page of them
    /// ([`crate::playlist::PAGE_SIZE`]) when the caller asks for a range.
    pub items: Option<ItemRange>,
    /// The time limit ([`PROBE_TIMEOUT`] when `None`).
    pub timeout: Option<Duration>,
}

/// Runs yt-dlp to describe pages.
#[derive(Debug, Clone)]
pub struct Prober {
    yt_dlp: PathBuf,
    ffmpeg_dir: Option<PathBuf>,
    jobs_dir: PathBuf,
}

impl Sidecars {
    /// A prober using the active yt-dlp, and the active FFmpeg if there is
    /// one (yt-dlp then knows it can merge, which its format choices
    /// depend on). Cookie files go to the tools directory's `.jobs` folder.
    pub fn prober(&self) -> Result<Prober> {
        let yt_dlp = self.program(Program::YtDlp)?;
        let ffmpeg_dir = self
            .program(Program::Ffmpeg)
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        Ok(Prober::new(yt_dlp, ffmpeg_dir, self.dir().join(JOBS_DIR)))
    }
}

impl Prober {
    /// A prober for the yt-dlp at `yt_dlp`, with FFmpeg in `ffmpeg_dir`, that
    /// keeps its temporary cookie files in `jobs_dir`.
    pub fn new(yt_dlp: PathBuf, ffmpeg_dir: Option<PathBuf>, jobs_dir: PathBuf) -> Self {
        Self {
            yt_dlp,
            ffmpeg_dir,
            jobs_dir,
        }
    }

    /// The arguments for probing `url` (after the runner's fixed prefix and
    /// `--ffmpeg-location`).
    pub fn args(
        &self,
        url: &str,
        opts: &ProbeOptions,
        cookie_file: Option<&Path>,
    ) -> Result<Vec<String>> {
        let mut args: Vec<String> = vec!["-J".into(), "--flat-playlist".into()];
        args.push(
            match opts.playlist {
                PlaylistMode::Single => "--no-playlist",
                PlaylistMode::Entries => "--yes-playlist",
            }
            .into(),
        );
        if let Some(items) = opts.items {
            args.extend(["--playlist-items".into(), items.to_string()]);
        }
        // A page whose formats are all DRM-protected (or that has none) is
        // still described, with `_has_drm` set, instead of failing.
        args.push("--ignore-no-formats-error".into());
        args.extend(["--encoding".into(), "utf-8".into()]);
        args.extend(["--socket-timeout".into(), SOCKET_TIMEOUT.into()]);
        if let Some(path) = cookie_file {
            let path = path.to_str().ok_or_else(|| Error::Argument {
                program: Program::YtDlp.name(),
                detail: "the cookie file's path is not valid Unicode".into(),
            })?;
            args.extend(["--cookies".into(), path.to_owned()]);
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
        Ok(args)
    }

    /// Describes the page at `url`. Cookies, if any, go through a temporary
    /// file that is deleted when the probe ends, however it ends.
    pub async fn probe(&self, url: &str, opts: &ProbeOptions) -> Result<MediaInfo> {
        let cookie_file = match &opts.cookies {
            Some(jar) if !jar.is_empty() => Some(CookieFile::create(&self.jobs_dir, jar)?),
            _ => None,
        };
        let args = self.args(url, opts, cookie_file.as_ref().map(CookieFile::path))?;
        let out = Invocation::yt_dlp(&self.yt_dlp, self.ffmpeg_dir.as_deref(), &args)?
            .timeout(opts.timeout.unwrap_or(PROBE_TIMEOUT))
            .max_output(MAX_INFO_JSON + 1)
            .run()
            .await?;
        drop(cookie_file);
        if !out.status.success() {
            return Err(Error::Extraction {
                message: error_message(&out.stderr_text(), out.status),
            });
        }
        if out.stdout.len() > MAX_INFO_JSON {
            return Err(Error::InfoJson {
                detail: format!("more than {MAX_INFO_JSON} bytes"),
            });
        }
        MediaInfo::from_json(&out.stdout).map_err(|detail| Error::InfoJson { detail })
    }
}

/// A header (or proxy) value for the command line: no control characters,
/// which could otherwise smuggle a second header in.
pub(crate) fn header_value(v: &str) -> Result<String> {
    if v.chars().any(char::is_control) {
        return Err(Error::Argument {
            program: Program::YtDlp.name(),
            detail: "a header value contains a control character".into(),
        });
    }
    Ok(v.to_owned())
}

/// The message of a failed yt-dlp: its last `ERROR:` line without the
/// prefix, else its last line of output, else the exit status.
pub(crate) fn error_message(stderr: &str, status: std::process::ExitStatus) -> String {
    const MAX: usize = 2000;
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let message = lines
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("ERROR:"))
        .or_else(|| lines.last().copied())
        .map_or_else(
            || format!("yt-dlp exited with {status}"),
            |m| m.trim().to_owned(),
        );
    match message.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", &message[..cut]),
        None => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: i32) -> std::process::ExitStatus {
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(code as u32)
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(code << 8)
        }
    }

    #[test]
    fn error_messages() {
        let s = status(1);
        assert_eq!(
            error_message(
                "WARNING: [youtube] something\nERROR: [youtube] abc: Video unavailable\n",
                s
            ),
            "[youtube] abc: Video unavailable"
        );
        assert_eq!(error_message("Traceback\n  oops\n\n", s), "oops");
        assert!(error_message("", s).starts_with("yt-dlp exited with"));
        let long = format!("ERROR: {}", "x".repeat(5000));
        assert_eq!(error_message(&long, s).chars().count(), 2001);
    }

    #[test]
    fn arguments() {
        let p = Prober::new("/t/yt-dlp".into(), None, "/t/.jobs".into());
        let opts = ProbeOptions {
            user_agent: Some("Mozilla/5.0 (Windows NT 10.0)".into()),
            referer: Some("https://example.com/".into()),
            playlist: PlaylistMode::Entries,
            items: Some(ItemRange::page(201)),
            ..ProbeOptions::default()
        };
        let args = p
            .args(
                "https://v.test/w?v=1",
                &opts,
                Some(Path::new("/t/.jobs/c.txt")),
            )
            .unwrap();
        assert_eq!(
            args,
            [
                "-J",
                "--flat-playlist",
                "--yes-playlist",
                "--playlist-items",
                "201:400",
                "--ignore-no-formats-error",
                "--encoding",
                "utf-8",
                "--socket-timeout",
                "20",
                "--cookies",
                "/t/.jobs/c.txt",
                "--referer",
                "https://example.com/",
                "--add-headers",
                "User-Agent:Mozilla/5.0 (Windows NT 10.0)",
                "--",
                "https://v.test/w?v=1",
            ]
        );
        crate::args::check_yt_dlp(&args).unwrap();
        let sneaky = ProbeOptions {
            user_agent: Some("x\r\nCookie: a=b".into()),
            ..ProbeOptions::default()
        };
        assert!(p.args("https://v.test/", &sneaky, None).is_err());
        // The URL always comes after `--`, so a URL cannot be an option.
        let args = p
            .args("--exec=calc", &ProbeOptions::default(), None)
            .unwrap();
        assert!(crate::args::check_yt_dlp(&args).is_err());
    }
}
