//! A media job's partial and temporary files (docs/VIDEO.md, "Errors and
//! pause/resume: as built").
//!
//! yt-dlp names every file of one video after the same stem, the output
//! template's result without its extension (`Title [id]`): the format files
//! it merges (`Title [id].f137.mp4`), their partial files (`….part`), the
//! fragment state of HLS and DASH downloads (`….ytdl`, `….part-Frag12`),
//! FFmpeg's output while it works (`Title [id].temp.mp4`), subtitles
//! (`Title [id].en.vtt`) and the result (`Title [id].mp4`). Knowing the stem
//! and the folder is enough to find them, also after Boltget restarts.
//!
//! Only names of those shapes, directly in the job's folder, are ever
//! returned: a file of the user's that merely starts with the same title is
//! not one of them, and nothing in other folders or subfolders is looked at.

use std::path::{Path, PathBuf};

/// Subtitle file extensions yt-dlp writes (`--write-subs`, `--convert-subs`).
const SUBTITLE_EXTS: &[&str] = &[
    "vtt", "srt", "ass", "ssa", "ttml", "srv1", "srv2", "srv3", "json3", "lrc", "dfxp",
];

/// Which of a job's files [`files`] looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// What yt-dlp and FFmpeg leave only while they work: `.part`, `.ytdl`,
    /// `.part-Frag*` and `*.temp.*` files. Safe to delete once a job has
    /// completed.
    Temporary,
    /// Everything an unfinished job has written: the temporary files, the
    /// format files waiting to be merged (`*.f137.mp4`) and subtitles
    /// (`*.en.vtt`). Not the result itself.
    Unfinished,
}

/// The stem of a file yt-dlp reported (its name, as `[boltget] download`
/// lines carry it): the name without its extension, without a format suffix
/// (`.f137`) and, for a subtitle, without its language (`.en`). `None` when
/// nothing would be left.
pub fn stem_of(file: &Path) -> Option<String> {
    let name = file.file_name()?.to_str()?;
    let (mut stem, ext) = name.rsplit_once('.')?;
    if SUBTITLE_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
        stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
    } else if let Some((s, last)) = stem.rsplit_once('.')
        && format_id(last)
    {
        stem = s;
    }
    let stem = stem.trim_end();
    (!stem.is_empty() && stem != "." && stem != "..").then(|| stem.to_owned())
}

/// `f137`, `fhls-720p`, `fdash-video=800000`: a format suffix as yt-dlp
/// writes it (`f` and the format's id).
fn format_id(segment: &str) -> bool {
    segment.len() > 1
        && segment.starts_with('f')
        && segment[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '='))
}

/// The files of the job whose files are named after `stem` in `dir` (not
/// its subfolders), of the kinds `scope` names. A stem with a path
/// separator, or an empty one, finds nothing.
pub fn files(dir: &Path, stem: &str, scope: Scope) -> Vec<PathBuf> {
    if stem.is_empty()
        || stem == "."
        || stem == ".."
        || stem.contains(['/', '\\'])
        || stem.chars().any(char::is_control)
    {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.");
    let mut out = Vec::new();
    for entry in entries.flatten() {
        // Regular files only: no folders, no links.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let Some(rest) = name.to_str().and_then(|n| n.strip_prefix(&prefix)) else {
            continue;
        };
        if belongs(rest, scope) {
            out.push(entry.path());
        }
    }
    out.sort();
    out
}

/// Whether a name's part after `stem.` is one of a job's files of `scope`.
fn belongs(rest: &str, scope: Scope) -> bool {
    let lower = rest.to_ascii_lowercase();
    let last = rest.rsplit('.').next().unwrap_or_default();
    let temporary = last.eq_ignore_ascii_case("part")
        || last.eq_ignore_ascii_case("ytdl")
        || lower.contains(".part-frag")
        || (lower.starts_with("temp.") && lower.len() > "temp.".len());
    if temporary || scope == Scope::Temporary {
        return temporary;
    }
    let parts: Vec<&str> = rest.split('.').collect();
    match parts[..] {
        // A format file waiting to be merged.
        [id, ext] if format_id(id) && plain(ext) => true,
        // A subtitle.
        [lang, ext] => {
            !lang.is_empty()
                && lang.len() <= 32
                && lang
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                && SUBTITLE_EXTS.contains(&ext.to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

fn plain(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        let s = |n: &str| stem_of(Path::new(n));
        assert_eq!(s("Clip [abc].f137.mp4").as_deref(), Some("Clip [abc]"));
        assert_eq!(s("Clip [abc].mp4").as_deref(), Some("Clip [abc]"));
        assert_eq!(s("Clip [abc].en.vtt").as_deref(), Some("Clip [abc]"));
        assert_eq!(s("Clip [abc].fhls-720p.mp4").as_deref(), Some("Clip [abc]"));
        assert_eq!(s("Mr. Robot [abc].mp4").as_deref(), Some("Mr. Robot [abc]"));
        assert_eq!(s("07 - Song [x].webm").as_deref(), Some("07 - Song [x]"));
        let full = if cfg!(windows) {
            r"C:\Videos\Clip [abc].f251.webm"
        } else {
            "/videos/Clip [abc].f251.webm"
        };
        assert_eq!(s(full).as_deref(), Some("Clip [abc]"));
        assert_eq!(s("noext"), None);
        assert_eq!(s(".mp4"), None);
    }

    #[test]
    fn only_the_jobs_own_files_and_only_in_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let stem = "Clip [abc]";
        let mine = [
            "Clip [abc].f137.mp4",
            "Clip [abc].f137.mp4.part",
            "Clip [abc].f140.m4a.ytdl",
            "Clip [abc].f140.m4a.part-Frag12",
            "Clip [abc].f140.m4a.part-Frag12.part",
            "Clip [abc].temp.mp4",
            "Clip [abc].en.vtt",
            "Clip [abc].pt-BR.srt",
        ];
        let theirs = [
            // The result itself, and the user's own files.
            "Clip [abc].mp4",
            "Clip [abc] (my edit).mp4",
            "Clip [abcd].f137.mp4.part",
            "Clip.mp4.part",
            "Clip [abc].notes.txt",
            "Clip [abc].edited.mp4",
            "Other [xyz].f137.mp4.part",
            "Clip [abc].temp",
        ];
        for n in mine.iter().chain(&theirs) {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        std::fs::create_dir(d.join("Clip [abc].f1.mp4.part")).unwrap();
        std::fs::create_dir(d.join("sub")).unwrap();
        std::fs::write(d.join("sub").join("Clip [abc].f137.mp4.part"), b"x").unwrap();

        let names = |scope| -> Vec<String> {
            files(d, stem, scope)
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        let mut expected: Vec<String> = mine.iter().map(|s| (*s).to_owned()).collect();
        expected.sort();
        assert_eq!(names(Scope::Unfinished), expected);
        assert_eq!(
            names(Scope::Temporary),
            [
                "Clip [abc].f137.mp4.part",
                "Clip [abc].f140.m4a.part-Frag12",
                "Clip [abc].f140.m4a.part-Frag12.part",
                "Clip [abc].f140.m4a.ytdl",
                "Clip [abc].temp.mp4",
            ]
        );
        for bad in ["", ".", "..", "x/y", r"x\y"] {
            assert!(files(d, bad, Scope::Unfinished).is_empty(), "{bad:?}");
        }
        assert_eq!(
            files(&d.join("missing"), stem, Scope::Unfinished),
            Vec::<PathBuf>::new()
        );
    }
}
