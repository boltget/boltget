//! Reading yt-dlp's progress while it downloads.
//!
//! [`Downloader`](crate::Downloader) asks yt-dlp for machine-readable lines
//! only: `--progress-template` turns each progress report into one line,
//! `[boltget] download <JSON>` (yt-dlp's progress dictionary, `%(progress)j`)
//! or `[boltget] postprocess <JSON>` (a post-processor starting or finishing,
//! such as the merger), and `--print after_move:...` reports the finished
//! file as `[boltget] file <JSON string>`. Every other line is yt-dlp's own
//! and ignored here; nothing human-readable is scraped (docs/VIDEO.md,
//! "Progress and errors").
//!
//! [`ProgressTracker`] adds the files of one job up: a video and its audio
//! are downloaded one after the other, each with its own progress, and the
//! job's progress is their sum.

use std::path::PathBuf;

use serde::Deserialize;

/// What every machine-readable line starts with.
pub(crate) const PREFIX: &str = "[boltget] ";

/// The template for download progress (after `download:`).
pub(crate) const DOWNLOAD_TEMPLATE: &str = "[boltget] download %(progress)j";

/// The template for post-processing (after `postprocess:`).
pub(crate) const POSTPROCESS_TEMPLATE: &str = "[boltget] postprocess %(progress)j";

/// The template that prints the finished file (after `after_move:`).
pub(crate) const FILE_TEMPLATE: &str = "[boltget] file %(filepath)j";

/// One machine-readable line.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// A download progress report for one file.
    Download(FileProgress),
    /// A post-processor started or finished.
    PostProcess {
        /// yt-dlp's name for it (`Merger`, `FFmpegVideoRemuxer`, ...).
        postprocessor: String,
        /// `started`, `processing` or `finished`.
        status: String,
    },
    /// The finished file.
    File(PathBuf),
}

/// One file's progress as yt-dlp reports it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileProgress {
    /// `downloading`, `finished` or `error`.
    pub status: String,
    /// The file being written (its final name).
    pub filename: Option<String>,
    /// Bytes so far.
    pub downloaded: u64,
    /// The size, when the server said.
    pub total: Option<u64>,
    /// yt-dlp's estimate of the size (fragmented downloads).
    pub total_estimate: Option<u64>,
    /// Bytes per second.
    pub speed: Option<f64>,
    /// Seconds left.
    pub eta: Option<u64>,
    /// The fragment being downloaded (fragmented downloads).
    pub fragment_index: Option<u64>,
    /// How many fragments there are.
    pub fragment_count: Option<u64>,
}

/// yt-dlp's progress dictionary: numbers may be integers, floats or `null`.
#[derive(Debug, Deserialize)]
struct RawProgress {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    downloaded_bytes: Option<f64>,
    #[serde(default)]
    total_bytes: Option<f64>,
    #[serde(default)]
    total_bytes_estimate: Option<f64>,
    #[serde(default)]
    speed: Option<f64>,
    #[serde(default)]
    eta: Option<f64>,
    #[serde(default)]
    fragment_index: Option<f64>,
    #[serde(default)]
    fragment_count: Option<f64>,
    #[serde(default)]
    postprocessor: Option<String>,
}

/// A non-negative whole number from yt-dlp's float, if it is one.
fn count(v: Option<f64>) -> Option<u64> {
    v.filter(|n| n.is_finite() && *n >= 0.0)
        .map(|n| n.round() as u64)
}

/// Parses one line of yt-dlp's standard output; `None` for anything that is
/// not one of Boltget's lines (or is malformed).
pub fn parse_line(line: &str) -> Option<Line> {
    let rest = line.trim_end_matches(['\r', '\n']).strip_prefix(PREFIX)?;
    let (kind, json) = rest.split_once(' ')?;
    match kind {
        "download" => {
            let raw: RawProgress = serde_json::from_str(json).ok()?;
            Some(Line::Download(FileProgress {
                status: raw.status.unwrap_or_default(),
                filename: raw.filename,
                downloaded: count(raw.downloaded_bytes).unwrap_or(0),
                total: count(raw.total_bytes).filter(|t| *t > 0),
                total_estimate: count(raw.total_bytes_estimate).filter(|t| *t > 0),
                speed: raw.speed.filter(|s| s.is_finite() && *s >= 0.0),
                eta: count(raw.eta),
                fragment_index: count(raw.fragment_index),
                fragment_count: count(raw.fragment_count),
            }))
        }
        "postprocess" => {
            let raw: RawProgress = serde_json::from_str(json).ok()?;
            Some(Line::PostProcess {
                postprocessor: raw.postprocessor.unwrap_or_default(),
                status: raw.status.unwrap_or_default(),
            })
        }
        "file" => {
            let path: String = serde_json::from_str(json).ok()?;
            (!path.is_empty()).then(|| Line::File(PathBuf::from(path)))
        }
        _ => None,
    }
}

/// Where a media download is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// yt-dlp is downloading streams.
    Downloading,
    /// FFmpeg is merging or converting them (no byte progress).
    Processing,
}

/// A job's progress, all of its files together.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaProgress {
    /// Downloading or processing.
    pub phase: Phase,
    /// Bytes downloaded, all files.
    pub downloaded: u64,
    /// The total, if known or estimated.
    pub total: Option<u64>,
    /// Whether `total` is an estimate.
    pub estimated: bool,
    /// Bytes per second of the file being downloaded.
    pub speed: Option<f64>,
    /// Seconds left for the file being downloaded.
    pub eta: Option<u64>,
    /// Fragment `(index, count)` of the file being downloaded.
    pub fragments: Option<(u64, u64)>,
    /// The post-processor running, while processing.
    pub step: Option<String>,
    /// The file being downloaded (its final name, as yt-dlp reports it).
    pub file: Option<String>,
}

#[derive(Debug, Clone)]
struct FileState {
    name: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    estimated: bool,
}

/// Adds up the progress of a job's files.
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    files: Vec<FileState>,
    expected: Option<u64>,
    current: MediaProgress,
}

impl ProgressTracker {
    /// A tracker for a job whose result is expected to be about `expected`
    /// bytes (the picker's estimate), if known. The total shown is never
    /// less than what the files report, so a job whose estimate was low
    /// still reaches 100%.
    pub fn new(expected: Option<u64>) -> Self {
        Self {
            files: Vec::new(),
            expected,
            current: MediaProgress {
                phase: Phase::Downloading,
                downloaded: 0,
                total: expected,
                estimated: true,
                speed: None,
                eta: None,
                fragments: None,
                step: None,
                file: None,
            },
        }
    }

    /// The progress now.
    pub fn progress(&self) -> &MediaProgress {
        &self.current
    }

    /// Takes one line in; returns whether the progress changed.
    pub fn apply(&mut self, line: &Line) -> bool {
        let before = self.current.clone();
        match line {
            Line::Download(p) => self.download(p),
            Line::PostProcess {
                postprocessor,
                status,
            } => {
                if status == "started" || status == "processing" {
                    self.current.phase = Phase::Processing;
                    self.current.step = Some(postprocessor.clone());
                    self.current.speed = None;
                    self.current.eta = None;
                }
            }
            Line::File(_) => {}
        }
        before != self.current
    }

    fn download(&mut self, p: &FileProgress) {
        let found = self
            .files
            .iter()
            .position(|f| f.name.is_some() && f.name == p.filename);
        let index = found.unwrap_or_else(|| {
            self.files.push(FileState {
                name: p.filename.clone(),
                downloaded: 0,
                total: None,
                estimated: false,
            });
            self.files.len() - 1
        });
        let file = &mut self.files[index];
        file.downloaded = p.downloaded;
        match (p.total, p.total_estimate) {
            (Some(t), _) => {
                file.total = Some(t);
                file.estimated = false;
            }
            (None, Some(t)) => {
                file.total = Some(t);
                file.estimated = true;
            }
            (None, None) => {}
        }
        if p.status == "finished" {
            // The finished report carries the final size.
            let size = p.total.unwrap_or(p.downloaded).max(p.downloaded);
            file.downloaded = size;
            file.total = Some(size);
            file.estimated = false;
        }
        let downloaded: u64 = self.files.iter().map(|f| f.downloaded).sum();
        let reported: Option<u64> = self.files.iter().map(|f| f.total).sum();
        let any_estimate = self.files.iter().any(|f| f.estimated || f.total.is_none());
        let (total, estimated) = match (self.expected, reported) {
            // The picker's estimate covers files not started yet (the audio
            // after the video); what the files report wins once it is more.
            (Some(e), Some(r)) => (Some(e.max(r)), e > r || any_estimate),
            (Some(e), None) => (Some(e.max(downloaded)), true),
            (None, Some(r)) => (Some(r), any_estimate),
            (None, None) => (None, true),
        };
        let c = &mut self.current;
        c.phase = Phase::Downloading;
        c.step = None;
        c.downloaded = downloaded;
        c.total = total.map(|t| t.max(downloaded));
        c.estimated = estimated;
        c.speed = if p.status == "finished" {
            None
        } else {
            p.speed
        };
        c.eta = if p.status == "finished" { None } else { p.eta };
        c.fragments = p.fragment_index.zip(p.fragment_count);
        c.file.clone_from(&p.filename);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines as yt-dlp 2026.10.26 prints them with Boltget's templates
    /// (shortened: the dictionaries carry more keys, which are ignored).
    const RECORDED: &str = r#"[youtube] Extracting URL: https://www.youtube.com/watch?v=abc
[info] abc: Downloading 1 format(s): 137+140
[boltget] download {"status": "downloading", "downloaded_bytes": 1024, "total_bytes": 4096, "filename": "C:\\V\\Clip [abc].f137.mp4", "tmpfilename": "C:\\V\\Clip [abc].f137.mp4.part", "eta": 3, "speed": 1024.5, "elapsed": 1.0, "ctx_id": null, "_eta_str": "00:03", "_speed_str": "1.00KiB/s", "_percent_str": " 25.0%", "_total_bytes_str": "4.00KiB", "_default_template": " 25.0% of 4.00KiB"}
[boltget] download {"status": "finished", "downloaded_bytes": 4096, "total_bytes": 4096, "filename": "C:\\V\\Clip [abc].f137.mp4", "elapsed": 4.2}
[boltget] download {"status": "downloading", "downloaded_bytes": 512, "total_bytes": null, "total_bytes_estimate": 2048.7, "filename": "C:\\V\\Clip [abc].f140.m4a", "eta": null, "speed": null, "fragment_index": 3, "fragment_count": 12}
[boltget] download {"status": "finished", "downloaded_bytes": 2000, "total_bytes": 2000, "filename": "C:\\V\\Clip [abc].f140.m4a"}
[boltget] postprocess {"status": "started", "postprocessor": "Merger", "_default_template": "Merger started"}
[Merger] Merging formats into "C:\V\Clip [abc].mp4"
[boltget] postprocess {"status": "finished", "postprocessor": "Merger"}
[boltget] file "C:\\V\\Clip [abc].mp4"
"#;

    #[test]
    fn recorded_lines_parse() {
        let lines: Vec<Line> = RECORDED.lines().filter_map(parse_line).collect();
        assert_eq!(lines.len(), 7);
        let Line::Download(first) = &lines[0] else {
            panic!("{:?}", lines[0])
        };
        assert_eq!(first.status, "downloading");
        assert_eq!(
            (first.downloaded, first.total, first.eta),
            (1024, Some(4096), Some(3))
        );
        assert_eq!(first.speed, Some(1024.5));
        assert_eq!(first.filename.as_deref(), Some(r"C:\V\Clip [abc].f137.mp4"));
        let Line::Download(frag) = &lines[2] else {
            panic!()
        };
        assert_eq!((frag.total, frag.total_estimate), (None, Some(2049)));
        assert_eq!(
            (frag.fragment_index, frag.fragment_count),
            (Some(3), Some(12))
        );
        assert_eq!(
            lines[4],
            Line::PostProcess {
                postprocessor: "Merger".into(),
                status: "started".into()
            }
        );
        assert_eq!(lines[6], Line::File(PathBuf::from(r"C:\V\Clip [abc].mp4")));
    }

    #[test]
    fn other_lines_and_garbage_are_ignored() {
        for line in [
            "",
            "[download] 25.0% of 4.00KiB",
            "[boltget]",
            "[boltget] download",
            "[boltget] download {not json",
            "[boltget] download [1, 2]",
            "[boltget] file 42",
            "[boltget] file \"\"",
            "[boltget] something {}",
            " [boltget] download {}",
        ] {
            assert_eq!(parse_line(line), None, "{line:?}");
        }
        // Unknown keys, nulls and negative numbers are tolerated.
        let Some(Line::Download(p)) =
            parse_line(r#"[boltget] download {"downloaded_bytes": -5, "speed": null, "x": 1}"#)
        else {
            panic!("a lenient line was refused")
        };
        assert_eq!((p.downloaded, p.speed), (0, None));
        // A string where a number belongs makes the line unusable.
        assert_eq!(
            parse_line(r#"[boltget] download {"downloaded_bytes": "many"}"#),
            None
        );
    }

    #[test]
    fn a_job_adds_its_files_up_and_ends_processing() {
        let mut t = ProgressTracker::new(Some(5000));
        let mut seen = Vec::new();
        for line in RECORDED.lines().filter_map(parse_line) {
            if t.apply(&line) {
                seen.push(t.progress().clone());
            }
        }
        // Video at 25%: the picker's estimate is the total.
        assert_eq!(
            (seen[0].downloaded, seen[0].total, seen[0].estimated),
            (1024, Some(5000), true)
        );
        assert_eq!(seen[0].speed, Some(1024.5));
        // Video done, audio fragment 3 of 12: 4096 + 512, total from the
        // files (6145 > 5000), still an estimate.
        assert_eq!(
            (seen[2].downloaded, seen[2].total, seen[2].estimated),
            (4608, Some(6145), true)
        );
        assert_eq!(seen[2].fragments, Some((3, 12)));
        // Both done: exact.
        assert_eq!(
            (seen[3].downloaded, seen[3].total, seen[3].estimated),
            (6096, Some(6096), false)
        );
        let last = t.progress();
        assert_eq!(last.phase, Phase::Processing);
        assert_eq!(last.step.as_deref(), Some("Merger"));
        assert_eq!(last.downloaded, 6096);
    }

    #[test]
    fn without_an_estimate_the_total_comes_from_the_files() {
        let mut t = ProgressTracker::new(None);
        assert_eq!(t.progress().total, None);
        t.apply(&Line::Download(FileProgress {
            status: "downloading".into(),
            filename: Some("a.mp4".into()),
            downloaded: 10,
            ..FileProgress::default()
        }));
        assert_eq!((t.progress().downloaded, t.progress().total), (10, None));
        t.apply(&Line::Download(FileProgress {
            status: "downloading".into(),
            filename: Some("a.mp4".into()),
            downloaded: 20,
            total: Some(100),
            ..FileProgress::default()
        }));
        assert_eq!(
            (
                t.progress().downloaded,
                t.progress().total,
                t.progress().estimated
            ),
            (20, Some(100), false)
        );
    }
}
