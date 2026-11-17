//! The argument allowlists for sidecars.
//!
//! Every argument vector passed to yt-dlp, ffmpeg or ffprobe is checked here
//! first. Only options on an allowlist pass, each with its exact name (never
//! an abbreviation, never `-xVALUE`); anything else is refused. The lists
//! leave out, on purpose, every option that runs commands (`--exec`,
//! `--netrc-cmd`, external downloaders, post-processor plugins), loads code or
//! configuration (`--plugin-dirs`, `--config-locations`, `--alias`), updates
//! the tool (`-U`), reads browser profiles or arbitrary local files, or weakens
//! TLS. [`YT_DLP_FORBIDDEN`] and [`FFMPEG_FORBIDDEN`] name them, and the tests
//! check that each of them is refused. See `docs/ARCHITECTURE.md`, "Sidecars".

use crate::{Error, Program, Result};

/// Options Boltget puts first on every yt-dlp command line: no configuration
/// files, no plugins, no update check.
pub const YT_DLP_PREFIX: &[&str] = &["--ignore-config", "--no-plugin-dirs", "--no-update"];

/// yt-dlp options without a value that may be passed.
const YT_DLP_FLAGS: &[&str] = &[
    "--version",
    "-J",
    "--dump-single-json",
    "-j",
    "--dump-json",
    "-s",
    "--simulate",
    "--no-simulate",
    "--skip-download",
    "--no-playlist",
    "--yes-playlist",
    "--flat-playlist",
    "--no-flat-playlist",
    "--newline",
    "--progress",
    "--no-progress",
    "-q",
    "--quiet",
    "--no-quiet",
    "--no-warnings",
    "-v",
    "--verbose",
    "--ignore-config",
    "--no-config",
    "--no-config-locations",
    "--no-plugin-dirs",
    "--no-update",
    "--no-exec",
    "--no-cookies",
    "--no-cookies-from-browser",
    "--no-batch-file",
    "--no-download-archive",
    "--no-cache-dir",
    "-c",
    "--continue",
    "--no-continue",
    "--part",
    "--no-part",
    "--mtime",
    "--no-mtime",
    "--restrict-filenames",
    "--no-restrict-filenames",
    "--windows-filenames",
    "--no-windows-filenames",
    "-w",
    "--no-overwrites",
    "--force-overwrites",
    "--no-force-overwrites",
    "--write-subs",
    "--no-write-subs",
    "--write-auto-subs",
    "--no-write-auto-subs",
    "--embed-subs",
    "--no-embed-subs",
    "--write-thumbnail",
    "--no-write-thumbnail",
    "--embed-thumbnail",
    "--no-embed-thumbnail",
    "--embed-metadata",
    "--no-embed-metadata",
    "--embed-chapters",
    "--no-embed-chapters",
    "-x",
    "--extract-audio",
    "-k",
    "--keep-video",
    "--no-keep-video",
    "--no-write-info-json",
    "--no-write-comments",
    "--no-write-playlist-metafiles",
    "--abort-on-error",
    "-i",
    "--ignore-errors",
    "--live-from-start",
    "--no-live-from-start",
    "--hls-use-mpegts",
    "--no-hls-use-mpegts",
    "--prefer-free-formats",
    "--no-prefer-free-formats",
    "--check-formats",
    "--no-check-formats",
    "--ignore-no-formats-error",
    "--no-ignore-no-formats-error",
    "-4",
    "--force-ipv4",
    "-6",
    "--force-ipv6",
];

/// yt-dlp options that take one value (the next argument) and may be passed.
const YT_DLP_VALUED: &[&str] = &[
    "-f",
    "--format",
    "-S",
    "--format-sort",
    "-o",
    "--output",
    "-P",
    "--paths",
    "--cookies",
    "--merge-output-format",
    "--remux-video",
    "--recode-video",
    "--audio-format",
    "--audio-quality",
    "--sub-langs",
    "--sub-format",
    "--convert-subs",
    "--convert-thumbnails",
    "--progress-template",
    "-N",
    "--concurrent-fragments",
    "-r",
    "--limit-rate",
    "--throttled-rate",
    "-R",
    "--retries",
    "--fragment-retries",
    "--extractor-retries",
    "--file-access-retries",
    "--retry-sleep",
    "--socket-timeout",
    "--proxy",
    "--add-headers",
    "--referer",
    "-I",
    "--playlist-items",
    "--max-downloads",
    "--encoding",
    "--color",
    "--impersonate",
    "-O",
    "--print",
    "--output-na-placeholder",
    "--trim-filenames",
    "--download-sections",
    "--http-chunk-size",
    "--buffer-size",
    "--max-filesize",
    "--min-filesize",
    "--sleep-requests",
    "--sleep-interval",
    "--max-sleep-interval",
    "--sleep-subtitles",
];

/// yt-dlp options that are never passed, whatever the caller asks for. Not
/// consulted by the check (the allowlist decides); the tests prove each one
/// is refused. yt-dlp's parser accepts unambiguous abbreviations of long
/// options, but an exact name always wins, and only exact names (each a real
/// yt-dlp option) pass the allowlist.
pub const YT_DLP_FORBIDDEN: &[&str] = &[
    // Run commands.
    "--exec",
    "--exec-before-download",
    "--netrc-cmd",
    "--use-postprocessor",
    "--postprocessor-args",
    "--ppa",
    "--external-downloader",
    "--downloader",
    "--external-downloader-args",
    "--downloader-args",
    "--js-runtimes",
    "--extractor-args",
    // Load code, configuration or aliases.
    "--config-locations",
    "--config-location",
    "--plugin-dirs",
    "--alias",
    "--remote-components",
    "--compat-options",
    // Replace the verified binary.
    "-U",
    "--update",
    "--update-to",
    // Read local files or browser profiles, or credentials.
    "-a",
    "--batch-file",
    "--load-info-json",
    "--cookies-from-browser",
    "--enable-file-urls",
    "-n",
    "--netrc",
    "--netrc-location",
    "-u",
    "--username",
    "-p",
    "--password",
    "--video-password",
    "--ap-mso",
    "--ap-username",
    "--ap-password",
    "--client-certificate",
    "--client-certificate-key",
    "--client-certificate-password",
    // Weaken TLS.
    "--no-check-certificates",
    "--no-check-certificate",
    "--legacy-server-connect",
    // Write to places Boltget does not choose, or send data elsewhere.
    "--print-to-file",
    "--cache-dir",
    "--download-archive",
    "--write-pages",
    "--sponsorblock-api",
    "--geo-verification-proxy",
    // Set by Boltget itself, to the verified FFmpeg.
    "--ffmpeg-location",
];

/// ffmpeg and ffprobe options without a value that may be passed.
const FFMPEG_FLAGS: &[&str] = &[
    "-version",
    "-hide_banner",
    "-nostdin",
    "-nostats",
    "-y",
    "-n",
    "-show_format",
    "-show_streams",
];

/// ffmpeg and ffprobe options that take one value. `-i` and `-f` have their
/// values checked further.
const FFMPEG_VALUED: &[&str] = &[
    "-loglevel",
    "-v",
    "-i",
    "-f",
    "-c",
    "-c:v",
    "-c:a",
    "-c:s",
    "-codec",
    "-map",
    "-map_metadata",
    "-metadata",
    "-movflags",
    "-threads",
    "-print_format",
    "-of",
];

/// Container formats `-f` may name. Demuxers that read other files (`concat`,
/// `hls` playlists, `lavfi`) are not among them.
const FFMPEG_FORMATS: &[&str] = &[
    "mp4", "mov", "matroska", "webm", "mp3", "ipod", "ogg", "opus", "flac", "wav", "null", "json",
];

/// ffmpeg options that are never passed: filters (which can read files),
/// protocol lists, attachments and reports. As for yt-dlp, the allowlist
/// decides; the tests prove these are refused.
pub const FFMPEG_FORBIDDEN: &[&str] = &[
    "-filter_complex",
    "-filter_complex_script",
    "-lavfi",
    "-vf",
    "-af",
    "-filter",
    "-filter_script",
    "-dump_attachment",
    "-attach",
    "-protocol_whitelist",
    "-protocol_blacklist",
    "-report",
    "-progress",
    "-sdp_file",
    "-vstats_file",
    "-passlogfile",
    "-init_hw_device",
];

/// Checks a yt-dlp argument vector (without [`YT_DLP_PREFIX`], which the
/// runner adds). URLs must come after `--` and be `http` or `https`.
pub fn check_yt_dlp(args: &[String]) -> Result<()> {
    let program = Program::YtDlp.name();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        no_nul(program, arg)?;
        if arg == "--" {
            for url in it.by_ref() {
                no_nul(program, url)?;
                let ok = url::Url::parse(url)
                    .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.has_host());
                if !ok {
                    return Err(refused(program, format!("{url:?} is not an http(s) URL")));
                }
            }
            break;
        }
        option(
            program,
            arg,
            &mut it,
            YT_DLP_FLAGS,
            YT_DLP_VALUED,
            yt_dlp_value,
        )?;
    }
    Ok(())
}

/// Proxy schemes yt-dlp may be given (`socks5h` resolves names at the
/// proxy, as Boltget's own SOCKS5 does).
const PROXY_SCHEMES: &[&str] = &["http", "https", "socks5", "socks5h"];

/// Containers for `--merge-output-format`.
const MERGE_FORMATS: &[&str] = &["mp4", "mkv", "webm", "mov", "flv", "avi"];

/// Formats for `--audio-format`.
const AUDIO_FORMATS: &[&str] = &[
    "best", "aac", "alac", "flac", "m4a", "mp3", "opus", "vorbis", "wav",
];

/// The headers `--add-headers` may set: what the browser extension hands
/// over (docs/VIDEO.md, "The extension's media panel: as built").
const HEADERS: &[&str] = &["User-Agent", "Referer"];

/// Formats for `--convert-subs` and `--sub-format` alternatives.
const SUBTITLE_FORMATS: &[&str] = &["srt", "vtt", "ass", "lrc", "none", "best"];

/// The values of yt-dlp options that name places, proxies or languages are
/// checked further (docs/VIDEO.md, "Picker and media downloads: as built"):
///
/// - `--proxy`: `http`, `https`, `socks5` or `socks5h` with a host, or empty
///   (direct).
/// - `-o`/`--output`: a file name template only: no path separators, no
///   `..`, no drive or `TYPES:` prefix (`:`), not absolute. The folder comes
///   from `-P`.
/// - `-P`/`--paths`: one absolute folder, without a `TYPES:` prefix.
/// - `--sub-langs`: language codes separated by commas: letters, digits,
///   `-` and `_` (no regular expressions, no `all`).
/// - `--merge-output-format`, `--audio-format`, `--convert-subs`: known
///   formats only.
/// - `--add-headers`: `User-Agent:` or `Referer:` with a value, nothing else
///   (the browser's two; no cookies or credentials this way).
/// - `--referer`: an `http` or `https` URL.
/// - `-I`/`--playlist-items`: places and ranges of places only: digits,
///   `:` and `-` between them, `,` between items (`1:200`, `1-5,8`); no
///   negative places, nothing else.
fn yt_dlp_value(option: &str, value: &str) -> Result<()> {
    let program = Program::YtDlp.name();
    let bad = |what: &str| Err(refused(program, format!("{option} {value:?}: {what}")));
    match option {
        "--proxy" => {
            if value.is_empty() {
                return Ok(());
            }
            let ok = url::Url::parse(value).is_ok_and(|u| {
                PROXY_SCHEMES.contains(&u.scheme()) && u.host_str().is_some_and(|h| !h.is_empty())
            });
            if !ok || value.chars().any(char::is_control) {
                return bad("not an http, https or socks5 proxy URL");
            }
        }
        "-o" | "--output" => {
            let plain = !value.contains(['/', '\\', ':']) && !value.contains("..");
            if value.trim().is_empty() || !plain || value.chars().any(char::is_control) {
                return bad("an output template is a file name, without folders");
            }
        }
        "-P" | "--paths" => {
            let path = std::path::Path::new(value);
            let prefixed = value
                .split_once(':')
                .is_some_and(|(head, _)| head.len() > 1);
            if !path.is_absolute() || prefixed || value.chars().any(char::is_control) {
                return bad("the folder must be an absolute path");
            }
        }
        "--sub-langs" => {
            let ok = !value.is_empty()
                && value.split(',').all(|l| {
                    !l.is_empty()
                        && l.len() <= 32
                        && l.chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                });
            if !ok || value.eq_ignore_ascii_case("all") {
                return bad("language codes separated by commas");
            }
        }
        "--add-headers" => {
            // `Name:value`; only the two headers the browser hands over.
            let ok = value.split_once(':').is_some_and(|(name, v)| {
                HEADERS.iter().any(|h| h.eq_ignore_ascii_case(name.trim())) && !v.trim().is_empty()
            });
            if !ok || value.chars().any(char::is_control) {
                return bad("only User-Agent and Referer headers");
            }
        }
        "--referer" => {
            let ok = url::Url::parse(value)
                .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.has_host());
            if !ok || value.chars().any(char::is_control) {
                return bad("not an http(s) URL");
            }
        }
        "-I" | "--playlist-items" => {
            let ok = !value.is_empty()
                && value.len() <= 256
                && value.split(',').all(|item| {
                    item.split([':', '-']).enumerate().all(|(i, n)| {
                        // `a:b`, `a-b`, `a:`; the first place must be there.
                        (n.is_empty() && i > 0)
                            || (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    }) && item.bytes().filter(|b| matches!(b, b':' | b'-')).count() <= 2
                });
            if !ok {
                return bad("places and ranges of places of the playlist");
            }
        }
        "--merge-output-format" if !MERGE_FORMATS.contains(&value) => {
            return bad("not a known container");
        }
        "--audio-format" if !AUDIO_FORMATS.contains(&value) => {
            return bad("not a known audio format");
        }
        "--convert-subs" if !SUBTITLE_FORMATS.contains(&value) => {
            return bad("not a known subtitle format");
        }
        _ => {}
    }
    Ok(())
}

/// Checks an ffmpeg or ffprobe argument vector. Inputs (`-i`) and outputs
/// (other bare arguments) must be local paths: no protocol prefix, no UNC
/// path, nothing starting with `-`.
pub fn check_ffmpeg(program: Program, args: &[String]) -> Result<()> {
    let name = program.name();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        no_nul(name, arg)?;
        if !arg.starts_with('-') {
            local_path(name, arg)?;
            continue;
        }
        option(
            name,
            arg,
            &mut it,
            FFMPEG_FLAGS,
            FFMPEG_VALUED,
            |opt, value| match opt {
                "-i" => local_path(name, value),
                "-f" if !FFMPEG_FORMATS.contains(&value) => {
                    Err(refused(name, format!("format {value:?} is not allowed")))
                }
                _ => Ok(()),
            },
        )?;
    }
    Ok(())
}

/// One option at `arg`, consuming its value from `rest` if it takes one.
fn option<'a>(
    program: &'static str,
    arg: &'a str,
    rest: &mut impl Iterator<Item = &'a String>,
    flags: &[&str],
    valued: &[&str],
    check_value: impl Fn(&str, &str) -> Result<()>,
) -> Result<()> {
    if !arg.starts_with('-') || arg == "-" {
        return Err(refused(
            program,
            format!("{arg:?}: bare arguments are not allowed here (URLs go after --)"),
        ));
    }
    let (name, inline) = match arg.split_once('=') {
        Some((n, v)) if arg.starts_with("--") => (n, Some(v)),
        _ => (arg, None),
    };
    if flags.contains(&name) && inline.is_none() {
        return Ok(());
    }
    if valued.contains(&name) {
        let value = match inline {
            Some(v) => v,
            None => rest
                .next()
                .ok_or_else(|| refused(program, format!("{name} needs a value")))?,
        };
        no_nul(program, value)?;
        return check_value(name, value);
    }
    Err(refused(program, format!("option {name} is not allowed")))
}

fn local_path(program: &'static str, p: &str) -> Result<()> {
    let bytes = p.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let colon_ok = if drive {
        !p[2..].contains(':')
    } else {
        !p.contains(':')
    };
    let unc = p.starts_with("\\\\") || p.starts_with("//");
    if p.is_empty() || p.starts_with('-') || unc || !colon_ok || p.contains('|') {
        return Err(refused(program, format!("{p:?} is not a local file path")));
    }
    Ok(())
}

fn no_nul(program: &'static str, s: &str) -> Result<()> {
    if s.contains('\0') {
        return Err(refused(
            program,
            "an argument contains a NUL character".into(),
        ));
    }
    Ok(())
}

fn refused(program: &'static str, detail: String) -> Error {
    Error::Argument { program, detail }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn every_forbidden_yt_dlp_option_is_refused() {
        for opt in YT_DLP_FORBIDDEN {
            assert!(
                !YT_DLP_FLAGS.contains(opt) && !YT_DLP_VALUED.contains(opt),
                "{opt} is on an allowlist"
            );
            for args in [
                v(&[opt]),
                v(&[opt, "value"]),
                v(&[&format!("{opt}=value")]),
                v(&["-J", opt, "x", "--", "https://example.com/v"]),
            ] {
                assert!(check_yt_dlp(&args).is_err(), "{args:?} passed");
            }
        }
    }

    #[test]
    fn the_prefix_is_allowed() {
        check_yt_dlp(&v(YT_DLP_PREFIX)).unwrap();
    }

    #[test]
    fn typical_yt_dlp_command_lines_pass() {
        for args in [
            v(&["--version"]),
            v(&[
                "-J",
                "--no-playlist",
                "--flat-playlist",
                "--ignore-no-formats-error",
                "--cookies",
                "C:\\t\\c.txt",
                "--",
                "https://example.com/watch?v=1",
            ]),
            v(&[
                "-f",
                "bv*[height<=1080]+ba/b[height<=1080]",
                "-o",
                "%(title)s [%(id)s].%(ext)s",
                "--newline",
                "--progress-template",
                "download:%(progress)j",
                "--concurrent-fragments=4",
                "--",
                "https://example.com/v",
                "http://example.com/w",
            ]),
            // A value that looks like an option is still the value.
            v(&["-o", "--exec", "--", "https://example.com/v"]),
        ] {
            check_yt_dlp(&args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        }
    }

    #[test]
    fn yt_dlp_refusals() {
        for args in [
            v(&["https://example.com/v"]),            // URL before --
            v(&["--", "file:///C:/Windows/win.ini"]), // not http(s)
            v(&["--", "--exec"]),                     // not a URL
            v(&["-fbest"]),                           // attached value
            v(&["-Jx"]),                              // clustered flags
            v(&["--exe", "calc"]),                    // abbreviation
            v(&["--version=1"]),                      // flag with a value
            v(&["-o"]),                               // missing value
            v(&["-"]),                                // stdin
            v(&["-o", "a\0b"]),
        ] {
            assert!(check_yt_dlp(&args).is_err(), "{args:?} passed");
        }
    }

    #[test]
    fn playlist_choices_are_flags() {
        // "This video" or "the whole playlist" of a watch page in a list.
        for flag in ["--no-playlist", "--yes-playlist", "--flat-playlist"] {
            check_yt_dlp(&v(&[flag])).unwrap();
        }
    }

    #[test]
    fn values_that_name_places_proxies_and_languages_are_checked() {
        let folder = if cfg!(windows) {
            r"C:\Users\you\Videos"
        } else {
            "/home/you/Videos"
        };
        for (opt, good) in [
            ("--proxy", "http://proxy.test:3128"),
            ("--proxy", "socks5h://127.0.0.1:1080"),
            ("--proxy", ""),
            ("-o", "%(title)s [%(id)s].%(ext)s"),
            ("--output", "%(title).80s.%(ext)s"),
            ("-P", folder),
            ("--sub-langs", "en,pt-BR,zh_Hans"),
            ("--merge-output-format", "mp4"),
            ("--audio-format", "m4a"),
            ("--convert-subs", "srt"),
            (
                "--add-headers",
                "User-Agent:Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
            ),
            ("--add-headers", "referer: https://v.test/watch"),
            ("--referer", "https://v.test/watch?v=1"),
            ("--playlist-items", "1:200"),
            ("-I", "201:400"),
            ("-I", "1-5,8,10:"),
        ] {
            check_yt_dlp(&v(&[opt, good])).unwrap_or_else(|e| panic!("{opt} {good}: {e}"));
        }
        for (opt, bad) in [
            ("--proxy", "file:///etc/passwd"),
            ("--proxy", "ftp://proxy.test"),
            ("--proxy", "proxy.test:3128"),
            ("--proxy", "http://proxy.test\r\nX: y"),
            ("-o", "../%(title)s.%(ext)s"),
            ("-o", "sub/%(title)s.%(ext)s"),
            ("-o", r"C:\x\%(title)s.%(ext)s"),
            ("-o", r"sub\%(title)s.%(ext)s"),
            ("-o", "subtitle:%(title)s.%(ext)s"),
            ("-o", "a..b"),
            ("-o", " "),
            ("-P", r"relative\dir"),
            ("-P", r"temp:C:\x"),
            ("--paths", "home:/tmp"),
            ("--sub-langs", "all"),
            ("--sub-langs", "en.*"),
            ("--sub-langs", "en,,de"),
            ("--sub-langs", ""),
            ("--merge-output-format", "exe"),
            ("--audio-format", "bat"),
            ("--convert-subs", "html"),
            ("--add-headers", "Cookie:sid=1"),
            ("--add-headers", "Authorization:Bearer x"),
            ("--add-headers", "User-Agent"),
            ("--add-headers", "User-Agent:"),
            ("--add-headers", "User-Agent:a\r\nCookie:b"),
            ("--add-headers", "X-User-Agent:a"),
            ("--referer", "file:///C:/a"),
            ("--referer", "v.test/watch"),
            ("--referer", "https://v.test/\nX"),
        ] {
            assert!(
                check_yt_dlp(&v(&[opt, bad])).is_err(),
                "{opt} {bad:?} passed"
            );
            assert!(
                check_yt_dlp(&v(&[&format!("{opt}={bad}")])).is_err() || !opt.starts_with("--"),
                "{opt}={bad:?} passed"
            );
        }
    }

    #[test]
    fn every_forbidden_ffmpeg_option_is_refused() {
        for opt in FFMPEG_FORBIDDEN {
            assert!(!FFMPEG_FLAGS.contains(opt) && !FFMPEG_VALUED.contains(opt));
            for p in [Program::Ffmpeg, Program::Ffprobe] {
                assert!(check_ffmpeg(p, &v(&[opt, "x"])).is_err(), "{opt} passed");
            }
        }
    }

    #[test]
    fn ffmpeg_inputs_and_outputs_must_be_local_paths() {
        check_ffmpeg(Program::Ffmpeg, &v(&["-version"])).unwrap();
        check_ffmpeg(
            Program::Ffmpeg,
            &v(&[
                "-nostdin",
                "-y",
                "-i",
                "C:\\v\\a.mp4",
                "-i",
                "b.m4a",
                "-c",
                "copy",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-f",
                "mp4",
                "out.mp4",
            ]),
        )
        .unwrap();
        check_ffmpeg(
            Program::Ffprobe,
            &v(&[
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                "/tmp/x.mkv",
            ]),
        )
        .unwrap();
        for bad in [
            "http://example.com/a.mp4",
            "concat:a.mp4|b.mp4",
            "subfile,,start,0,end,1,,:x",
            "file:C:/x",
            "\\\\server\\share\\a.mp4",
            "C:\\a:b",
            "-",
            "",
        ] {
            assert!(
                check_ffmpeg(Program::Ffmpeg, &v(&["-i", bad, "o.mp4"])).is_err(),
                "{bad}"
            );
            assert!(
                check_ffmpeg(Program::Ffmpeg, &v(&["-i", "a.mp4", bad])).is_err(),
                "{bad}"
            );
        }
        for fmt in ["concat", "lavfi", "hls", "image2"] {
            assert!(check_ffmpeg(Program::Ffmpeg, &v(&["-f", fmt, "-i", "a"])).is_err());
        }
    }
}
