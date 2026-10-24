//! Filename resolution: `Content-Disposition` (RFC 6266 / RFC 8187), URL fallback,
//! MIME-based extensions, Windows-safe sanitization and collision handling.
//!
//! See `docs/ENGINE.md`, "Filename resolution".

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode;
use url::Url;

/// Maximum length of one path component, in UTF-16 code units (NTFS limit).
const MAX_COMPONENT_UTF16: usize = 255;

/// Inputs for [`resolve`], in precedence order.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NameSources<'a> {
    /// A name the user typed (highest precedence).
    pub(crate) user: Option<&'a str>,
    /// Raw `Content-Disposition` header bytes.
    pub(crate) content_disposition: Option<&'a [u8]>,
    /// A name suggested by the browser.
    pub(crate) suggested: Option<&'a str>,
    /// The final URL after redirects.
    pub(crate) final_url: Option<&'a Url>,
    /// `Content-Type` of the response.
    pub(crate) content_type: Option<&'a str>,
}

/// Resolves and sanitizes the file name for a download.
pub(crate) fn resolve(src: NameSources<'_>) -> String {
    // A user-typed name is taken as-is (after sanitization): no extension is added.
    if let Some(user) = src.user.map(sanitize).filter(|n| !n.is_empty()) {
        return user;
    }
    let candidate = src
        .content_disposition
        .and_then(parse_content_disposition)
        .or_else(|| src.suggested.map(str::to_owned))
        .or_else(|| src.final_url.and_then(name_from_url))
        .map(|n| sanitize(&n))
        .filter(|n| !n.is_empty());

    let mut name = candidate.unwrap_or_else(|| "download".to_owned());
    if Path::new(&name).extension().is_none()
        && let Some(ext) = src.content_type.and_then(extension_for_mime)
    {
        name.push('.');
        name.push_str(ext);
    }
    sanitize(&name)
}

/// Extracts the file name from a `Content-Disposition` header value.
///
/// Prefers `filename*` (RFC 8187, `UTF-8` and `ISO-8859-1`), then `filename`, with the
/// common real-world deviations handled: raw UTF-8 bytes and percent-encoded UTF-8 in
/// the plain `filename` parameter.
pub(crate) fn parse_content_disposition(raw: &[u8]) -> Option<String> {
    let params = split_params(raw);
    let mut plain = None;
    let mut extended = None;
    for (name, value) in params.iter().skip(1) {
        // The first item is the disposition type.
        let name = name.trim();
        if name.eq_ignore_ascii_case("filename*") {
            if extended.is_none() {
                extended = decode_ext_value(value);
            }
        } else if name.eq_ignore_ascii_case("filename") && plain.is_none() {
            plain = Some(decode_plain_value(value));
        }
    }
    extended
        .or(plain)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Splits a header into `;`-separated `name=value` items, honouring quoted strings.
/// Values are returned with quotes removed and backslash escapes resolved.
fn split_params(raw: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = raw.len();
    while i < n {
        // name
        let name_start = i;
        while i < n && raw[i] != b'=' && raw[i] != b';' {
            i += 1;
        }
        let name = String::from_utf8_lossy(&raw[name_start..i])
            .trim()
            .to_owned();
        let mut value = Vec::new();
        if i < n && raw[i] == b'=' {
            i += 1;
            while i < n && (raw[i] == b' ' || raw[i] == b'\t') {
                i += 1;
            }
            if i < n && raw[i] == b'"' {
                i += 1;
                while i < n && raw[i] != b'"' {
                    if raw[i] == b'\\' && i + 1 < n {
                        i += 1;
                    }
                    value.push(raw[i]);
                    i += 1;
                }
                // closing quote (if any), then skip to the next ';'
                while i < n && raw[i] != b';' {
                    i += 1;
                }
            } else {
                let start = i;
                while i < n && raw[i] != b';' {
                    i += 1;
                }
                value.extend_from_slice(trim_bytes(&raw[start..i]));
            }
        }
        if i < n && raw[i] == b';' {
            i += 1;
        }
        if !name.is_empty() || !value.is_empty() {
            out.push((name, value));
        }
    }
    out
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(start, |p| p + 1);
    &b[start..end]
}

/// Decodes an RFC 8187 `ext-value`: `charset'language'pct-encoded`.
fn decode_ext_value(value: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(value).ok()?;
    let mut parts = s.splitn(3, '\'');
    let charset = parts.next()?.trim();
    let _language = parts.next()?;
    let encoded = parts.next()?;
    let bytes: Vec<u8> = percent_decode(encoded.as_bytes()).collect();
    if charset.eq_ignore_ascii_case("utf-8") {
        String::from_utf8(bytes).ok()
    } else if charset.eq_ignore_ascii_case("iso-8859-1") || charset.eq_ignore_ascii_case("latin1") {
        Some(latin1(&bytes))
    } else {
        None
    }
}

/// Decodes a plain `filename` value with the usual heuristics.
fn decode_plain_value(value: &[u8]) -> String {
    let text = match std::str::from_utf8(value) {
        Ok(s) => s.to_owned(),
        // Not UTF-8: the spec says ISO-8859-1.
        Err(_) => latin1(value),
    };
    if text.contains('%') {
        let decoded: Vec<u8> = percent_decode(text.as_bytes()).collect();
        if decoded != text.as_bytes()
            && let Ok(d) = String::from_utf8(decoded)
        {
            return d;
        }
    }
    text
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// The last non-empty path segment of a URL, percent-decoded.
pub(crate) fn name_from_url(url: &Url) -> Option<String> {
    let seg = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded: Vec<u8> = percent_decode(seg.as_bytes()).collect();
    let s = String::from_utf8(decoded).unwrap_or_else(|e| latin1(e.as_bytes()));
    (!s.trim().is_empty()).then_some(s)
}

/// A small built-in MIME type → extension table.
pub(crate) fn extension_for_mime(content_type: &str) -> Option<&'static str> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Some(match mime.as_str() {
        "application/pdf" => "pdf",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/x-tar" => "tar",
        "application/x-xz" => "xz",
        "application/x-bzip2" => "bz2",
        "application/zstd" => "zst",
        "application/x-msdownload" | "application/vnd.microsoft.portable-executable" => "exe",
        "application/x-msi" | "application/x-ms-installer" => "msi",
        "application/x-iso9660-image" => "iso",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/epub+zip" => "epub",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        _ => return None,
    })
}

/// Makes a name safe as a single path component on Windows (applied on all platforms).
pub(crate) fn sanitize(name: &str) -> String {
    // Strip directory components: keep only what follows the last separator.
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 || c == '\u{7f}' => '_',
            c => c,
        })
        .collect();
    // Trailing dots and spaces are silently dropped by Windows; remove them ourselves.
    let trimmed_len = s.trim_end_matches(['.', ' ']).len();
    s.truncate(trimmed_len);
    let s = s.trim_start().to_owned();
    if s.is_empty() || s.chars().all(|c| c == '.') {
        return String::new();
    }
    let s = if is_reserved(&s) { format!("_{s}") } else { s };
    truncate_utf16(&s, MAX_COMPONENT_UTF16)
}

fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end();
    let upper = stem.to_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    for prefix in ["COM", "LPT"] {
        if let Some(rest) = upper.strip_prefix(prefix) {
            let mut chars = rest.chars();
            if let (Some(c), None) = (chars.next(), chars.next())
                && matches!(c, '1'..='9' | '¹' | '²' | '³')
            {
                return true;
            }
        }
    }
    false
}

/// Truncates to at most `max` UTF-16 code units, preserving the extension.
fn truncate_utf16(name: &str, max: usize) -> String {
    let units = |s: &str| s.encode_utf16().count();
    if units(name) <= max {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.') {
        // Keep "reasonable" extensions only; a 200-character "extension" is not one.
        Some(i) if i > 0 && units(&name[i..]) <= 32 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let budget = max - units(ext);
    let mut out = String::new();
    let mut used = 0;
    for c in stem.chars() {
        let w = c.len_utf16();
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    let trimmed = out.trim_end_matches(['.', ' ']).len();
    out.truncate(trimmed);
    out.push_str(ext);
    out
}

/// Returns `dir/name`, or `dir/stem (k).ext` for the smallest `k` such that
/// `is_taken` returns false.
pub(crate) fn non_colliding(dir: &Path, name: &str, is_taken: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !is_taken(&first) {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut k: u32 = 1;
    loop {
        let candidate = dir.join(format!("{stem} ({k}){ext}"));
        if !is_taken(&candidate) {
            return candidate;
        }
        k += 1;
    }
}

#[cfg(test)]
#[allow(clippy::case_sensitive_file_extension_comparisons)]
mod tests {
    use super::*;

    fn cd(s: &str) -> Option<String> {
        parse_content_disposition(s.as_bytes())
    }

    #[test]
    fn content_disposition_corpus() {
        assert_eq!(cd("attachment; filename=\"a.txt\""), Some("a.txt".into()));
        assert_eq!(cd("attachment; filename=a.txt"), Some("a.txt".into()));
        assert_eq!(cd("inline; filename=\"a b.pdf\""), Some("a b.pdf".into()));
        assert_eq!(cd("attachment;filename=\"x.zip\";"), Some("x.zip".into()));
        assert_eq!(cd("ATTACHMENT; FILENAME=\"up.bin\""), Some("up.bin".into()));
        // filename* wins regardless of order.
        assert_eq!(
            cd("attachment; filename=\"fallback.txt\"; filename*=UTF-8''na%C3%AFve%20file.txt"),
            Some("naïve file.txt".into())
        );
        assert_eq!(
            cd("attachment; filename*=UTF-8''%E2%82%AC%20rates.pdf; filename=\"EUR rates.pdf\""),
            Some("€ rates.pdf".into())
        );
        assert_eq!(
            cd("attachment; filename*=iso-8859-1'en'%A3%20rates.txt"),
            Some("£ rates.txt".into())
        );
        assert_eq!(
            cd("attachment; filename*=utf-8'en'caf%C3%A9.txt"),
            Some("café.txt".into())
        );
        // Escaped quote inside quoted-string.
        assert_eq!(
            cd(r#"attachment; filename="a\"b.txt""#),
            Some("a\"b.txt".into())
        );
        // Semicolon inside quotes.
        assert_eq!(
            cd("attachment; filename=\"a;b.txt\""),
            Some("a;b.txt".into())
        );
        // Percent-encoded UTF-8 in the plain parameter (against the spec, common).
        assert_eq!(
            cd("attachment; filename=\"%E6%96%87%E4%BB%B6.txt\""),
            Some("文件.txt".into())
        );
        // A literal percent sign that does not form valid UTF-8 stays as-is.
        assert_eq!(
            cd("attachment; filename=\"100%.txt\""),
            Some("100%.txt".into())
        );
        // Unsupported charset in filename* falls back to filename.
        assert_eq!(
            cd("attachment; filename*=koi8-r''%C1.txt; filename=\"b.txt\""),
            Some("b.txt".into())
        );
        assert_eq!(cd("attachment"), None);
        assert_eq!(cd("attachment; filename=\"\""), None);
        assert_eq!(cd(""), None);
    }

    #[test]
    fn content_disposition_raw_bytes() {
        // Raw UTF-8 bytes in the header.
        let raw = "attachment; filename=\"résumé.pdf\"".as_bytes();
        assert_eq!(parse_content_disposition(raw), Some("résumé.pdf".into()));
        // Raw Latin-1 bytes.
        let raw = b"attachment; filename=\"r\xe9sum\xe9.pdf\"";
        assert_eq!(parse_content_disposition(raw), Some("résumé.pdf".into()));
    }

    #[test]
    fn sanitization() {
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("..\\..\\boot.ini"), "boot.ini");
        assert_eq!(sanitize("a<b>c:d\"e|f?g*h.txt"), "a_b_c_d_e_f_g_h.txt");
        assert_eq!(sanitize("tab\there.txt"), "tab_here.txt");
        assert_eq!(sanitize("trailing. . "), "trailing");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("con.txt"), "_con.txt");
        assert_eq!(sanitize("Com1.tar.gz"), "_Com1.tar.gz");
        assert_eq!(sanitize("LPT²"), "_LPT²");
        assert_eq!(sanitize("COM0"), "COM0");
        assert_eq!(sanitize("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(sanitize(".."), "");
        assert_eq!(sanitize("..."), "");
        assert_eq!(sanitize("/"), "");
        assert_eq!(sanitize(".bashrc"), ".bashrc");
    }

    #[test]
    fn truncation_preserves_extension() {
        let long = format!("{}.iso", "a".repeat(300));
        let s = sanitize(&long);
        assert_eq!(s.encode_utf16().count(), 255);
        assert!(s.ends_with(".iso"));
        // Astral characters count as two UTF-16 units and must not be split.
        let emoji = format!("{}.txt", "😀".repeat(200));
        let s = sanitize(&emoji);
        assert!(s.encode_utf16().count() <= 255);
        assert!(s.ends_with(".txt"));
    }

    #[test]
    fn resolve_precedence() {
        let url = Url::parse("https://example.com/dir/file%20name.tar.gz?token=x").unwrap();
        let base = NameSources {
            final_url: Some(&url),
            ..Default::default()
        };
        assert_eq!(resolve(base), "file name.tar.gz");
        let with_cd = NameSources {
            content_disposition: Some(b"attachment; filename=cd.bin"),
            ..base
        };
        assert_eq!(resolve(with_cd), "cd.bin");
        let with_user = NameSources {
            user: Some("mine.dat"),
            ..with_cd
        };
        assert_eq!(resolve(with_user), "mine.dat");
        let suggested = NameSources {
            suggested: Some("browser.bin"),
            ..base
        };
        assert_eq!(resolve(suggested), "browser.bin");
    }

    #[test]
    fn resolve_extension_and_fallback() {
        let url = Url::parse("https://example.com/download").unwrap();
        let src = NameSources {
            final_url: Some(&url),
            content_type: Some("application/pdf; charset=binary"),
            ..Default::default()
        };
        assert_eq!(resolve(src), "download.pdf");
        let root = Url::parse("https://example.com/").unwrap();
        let src = NameSources {
            final_url: Some(&root),
            content_type: Some("application/zip"),
            ..Default::default()
        };
        assert_eq!(resolve(src), "download.zip");
        let src = NameSources {
            final_url: Some(&root),
            ..Default::default()
        };
        assert_eq!(resolve(src), "download");
    }

    #[test]
    fn collisions() {
        let dir = Path::new("d");
        let taken = [dir.join("a.txt"), dir.join("a (1).txt")];
        let p = non_colliding(dir, "a.txt", |p| taken.iter().any(|t| t == p));
        assert_eq!(p, dir.join("a (2).txt"));
        let p = non_colliding(dir, "b", |p| p == dir.join("b"));
        assert_eq!(p, dir.join("b (1)"));
        let p = non_colliding(dir, "c.txt", |_| false);
        assert_eq!(p, dir.join("c.txt"));
    }
}
