//! Where the sidecars come from, and what Boltget trusts about them.
//!
//! - **yt-dlp:** the latest release of `yt-dlp/yt-dlp` (or of
//!   `yt-dlp/yt-dlp-nightly-builds` on the nightly channel), read from the
//!   GitHub releases API. Its `SHA2-256SUMS` must carry a valid signature by
//!   yt-dlp's release key, pinned below, and the binary must match its line in
//!   that file.
//! - **FFmpeg:** one build per platform, pinned below with its URL, size and
//!   SHA-256 (the built-in sidecar manifest). It changes with Boltget releases.
//!
//! Downloads go over HTTPS to [`OFFICIAL_HOSTS`] only, redirects included.
//! See `docs/VIDEO.md`, "Verification".

use serde::Deserialize;

use crate::pgp::PinnedKey;
use crate::{Program, Tool};

/// The hosts sidecar downloads may reach: the GitHub API, github.com and the
/// hosts GitHub redirects release downloads to.
pub const OFFICIAL_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// The fingerprint of yt-dlp's release-signing key, as GnuPG prints it
/// (`public.key` in the yt-dlp repository). [`yt_dlp_key`] must hash to it.
pub const YT_DLP_KEY_FINGERPRINT: &str = "AC0CBBE6848D6A873464AF4E57CF65933B5A7581";

/// The modulus of yt-dlp's release-signing key (RSA 4096), in hex.
const YT_DLP_KEY_N: &str = concat!(
    "f4ac5f738c63c0b74b6196de42d5e6f371c0155fb35bd6ff9ea908e4f9611975",
    "31135428375803399e002b947766fdf66acd480efa9018ac1eb418a7a0edf7d4",
    "0dff92d1714a11975e44b61e01aaaa2a14a26d56a50e3bce04ecedbb75bad3b1",
    "cb113400a80ef04cfb88b765aa9a92fad0d21ccabdbb2f1aa681d37798dbd9da",
    "22cef9b3f49ca47f0be3e013d1f8cadcb00dad76cccc7a3e66429d538e3aee97",
    "644166754c71750f5f143a9df4b701298d8b0ce712628d6c698d185a26561edc",
    "d5a19bb590ece39d2e458eaff204724b2260c7ee93711e6eecfcf9dababa6640",
    "5b4316bba4b7eda58439e9294986aff23fdc81ea1ff5fef673e84b8ba2a90e47",
    "a5b941f012bee167e96a2120ef92e7bee38f7d920f298c976bfd00ed0a233308",
    "0eac3f3ca334fe54290a92427e8a35d9ee01cd8e670169474a2bb7c320439823",
    "adcbc9ce905a41904e28057df4945d61c308ef5a5cd84fdde8ea15680632d094",
    "b2b6e4709916b9663bab9011cbfd391db48cdcae0353d7aab67c66895d5643c9",
    "d7b88f49809411be1e4f4e49f2e4bb71ab3726aaf58ef538c705baecd7804f10",
    "ef5f717c4a2ff08505bcd70d2bef02002b76aaa53289dc43fca271395f2faab3",
    "27c10a162a96d7840c4dd027e92bae653a1d5c24e2227c284fb5a38ee14f28a7",
    "6f7c7d463f86e7a9dcd39f32f10920c812d7f27cfb085fef4623b26493fcd98d",
);

/// yt-dlp's release-signing key ("Simon Sawicki (yt-dlp signing key)", created
/// 2023-02-26, no expiry).
pub fn yt_dlp_key() -> PinnedKey {
    PinnedKey {
        created: 1_677_455_406,
        n: hex_decode(YT_DLP_KEY_N).unwrap_or_default(),
        e: vec![0x01, 0x00, 0x01],
    }
}

/// A yt-dlp release channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Stable releases (the default).
    #[default]
    Stable,
    /// Nightly builds: fixes sooner, less tested. An advanced opt-in.
    Nightly,
}

impl Channel {
    /// `stable` or `nightly`.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Nightly => "nightly",
        }
    }
}

/// How a downloaded file becomes an installed version directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// The download is the program itself, saved as `file`.
    Executable {
        /// The file name inside the version directory.
        file: String,
    },
    /// A zip archive: `strip` leading path components are dropped from every
    /// entry, and only entries under `keep` (after stripping; `None`: all) are
    /// unpacked.
    Zip {
        /// Leading components to drop.
        strip: usize,
        /// The only directory to unpack, such as `bin/`.
        keep: Option<String>,
    },
}

/// The release asset of yt-dlp for this platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSpec {
    /// The asset's file name in the release (and in `SHA2-256SUMS`).
    pub name: String,
    /// How to unpack it.
    pub layout: Layout,
    /// The programs it provides, relative to the version directory.
    pub programs: Vec<(Program, String)>,
}

/// A pinned build (the built-in sidecar manifest's entry for one platform).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedBuild {
    /// The version, used as the directory name and checked in the smoke test.
    pub version: String,
    /// The download URL.
    pub url: String,
    /// The download's size in bytes.
    pub size: u64,
    /// The download's SHA-256 (lowercase hex).
    pub sha256: String,
    /// How to unpack it.
    pub layout: Layout,
    /// The programs it provides, relative to the version directory.
    pub programs: Vec<(Program, String)>,
}

/// Everything the manager needs to know about where tools come from.
#[derive(Debug, Clone)]
pub struct Sources {
    /// The GitHub API URL of the latest stable yt-dlp release.
    pub yt_dlp_stable: String,
    /// The GitHub API URL of the latest nightly yt-dlp build.
    pub yt_dlp_nightly: String,
    /// yt-dlp's asset for this platform (`None`: no build).
    pub yt_dlp_asset: Option<AssetSpec>,
    /// The key yt-dlp's checksum files must be signed with.
    pub yt_dlp_key: PinnedKey,
    /// The FFmpeg build for this platform (`None`: no build).
    pub ffmpeg: Option<PinnedBuild>,
    /// The only hosts downloads may reach (redirects included).
    pub allowed_hosts: Vec<String>,
    /// Refuse plain-HTTP URLs (always on outside tests).
    pub require_https: bool,
}

impl Sources {
    /// The official sources for the platform Boltget was built for.
    pub fn official() -> Self {
        Sources {
            yt_dlp_stable: "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest".into(),
            yt_dlp_nightly:
                "https://api.github.com/repos/yt-dlp/yt-dlp-nightly-builds/releases/latest".into(),
            yt_dlp_asset: yt_dlp_asset(),
            yt_dlp_key: yt_dlp_key(),
            ffmpeg: ffmpeg_build(),
            allowed_hosts: OFFICIAL_HOSTS.iter().map(|h| (*h).to_owned()).collect(),
            require_https: true,
        }
    }

    pub(crate) fn release_url(&self, channel: Channel) -> &str {
        match channel {
            Channel::Stable => &self.yt_dlp_stable,
            Channel::Nightly => &self.yt_dlp_nightly,
        }
    }

    /// Whether `url` may be downloaded from: the scheme and host are allowed.
    pub(crate) fn check_url(&self, tool: Tool, url: &str) -> crate::Result<()> {
        let parsed = url::Url::parse(url)
            .map_err(|e| crate::Error::release(tool, format!("bad URL: {e}")))?;
        let https = parsed.scheme() == "https";
        if !(https || (!self.require_https && parsed.scheme() == "http")) {
            return Err(crate::Error::release(
                tool,
                "only HTTPS downloads are allowed",
            ));
        }
        let host = parsed.host_str().unwrap_or_default();
        if !self
            .allowed_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(host))
        {
            return Err(crate::Error::release(
                tool,
                format!("{host} is not an allowed download host"),
            ));
        }
        Ok(())
    }
}

/// The yt-dlp asset for the platform Boltget was built for. Windows uses the
/// unpacked ("onedir") build: it starts faster than the self-extracting
/// single file and is flagged less by antivirus products.
pub fn yt_dlp_asset() -> Option<AssetSpec> {
    let (name, layout, exe) = if cfg!(all(windows, target_arch = "x86_64")) {
        ("yt-dlp_win.zip", zip_all(), "yt-dlp.exe")
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        ("yt-dlp_win_arm64.zip", zip_all(), "yt-dlp.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        ("yt-dlp_linux", executable("yt-dlp"), "yt-dlp")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        ("yt-dlp_linux_aarch64", executable("yt-dlp"), "yt-dlp")
    } else if cfg!(target_os = "macos") {
        ("yt-dlp_macos", executable("yt-dlp"), "yt-dlp")
    } else {
        return None;
    };
    Some(AssetSpec {
        name: name.into(),
        layout,
        programs: vec![(Program::YtDlp, exe.into())],
    })
}

fn zip_all() -> Layout {
    Layout::Zip {
        strip: 0,
        keep: None,
    }
}

fn executable(file: &str) -> Layout {
    Layout::Executable { file: file.into() }
}

/// The pinned FFmpeg build for the platform Boltget was built for: BtbN's
/// FFmpeg-Builds, release branch 9.0, GPL, shared libraries (the smaller
/// download), from a month-end autobuild (those are kept long-term). Only
/// `bin/` is unpacked. Other platforms have none yet: their users point
/// Boltget at the system's FFmpeg.
pub fn ffmpeg_build() -> Option<PinnedBuild> {
    const TAG: &str = "https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-10-31-12-59/";
    let (file, size, sha256) = if cfg!(all(windows, target_arch = "x86_64")) {
        (
            "ffmpeg-n9.0.2-39-g7adfbe40dc-win64-gpl-shared-9.0.zip",
            85_920_857,
            "1d9823c35c7a7e2f54d1438cf61030d6efabe3278de01c191309d1e828bb74df",
        )
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        (
            "ffmpeg-n9.0.2-39-g7adfbe40dc-winarm64-gpl-shared-9.0.zip",
            60_550_580,
            "f6b7927191e11fe20b88785e5549703f58326d2ab1b35291d94e21f7cead7a17",
        )
    } else {
        return None;
    };
    Some(PinnedBuild {
        version: "n9.0.2-39-g7adfbe40dc".into(),
        url: format!("{TAG}{file}"),
        size,
        sha256: sha256.into(),
        layout: Layout::Zip {
            strip: 1,
            keep: Some("bin/".into()),
        },
        programs: vec![
            (Program::Ffmpeg, "bin/ffmpeg.exe".into()),
            (Program::Ffprobe, "bin/ffprobe.exe".into()),
        ],
    })
}

/// A release as the GitHub API describes it (the fields Boltget reads).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    #[serde(default)]
    pub(crate) assets: Vec<GithubAsset>,
}

/// One release asset.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GithubAsset {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) size: u64,
    pub(crate) browser_download_url: String,
}

impl GithubRelease {
    pub(crate) fn asset(&self, name: &str) -> Option<&GithubAsset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// The SHA-256 for `name` in a `SHA2-256SUMS` file (`<hex>  <name>` lines,
/// `*` before the name for binary mode). `None` if it is not listed, or
/// listed more than once.
pub(crate) fn sum_for(sums: &str, name: &str) -> Option<String> {
    let mut found = None;
    for line in sums.lines() {
        let Some((hex, file)) = line.trim_end().split_once(char::is_whitespace) else {
            continue;
        };
        let file = file.trim_start();
        let file = file.strip_prefix('*').unwrap_or(file);
        if file == name {
            if found.is_some() || hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            found = Some(hex.to_ascii_lowercase());
        }
    }
    found
}

/// Whether `v` is safe as a version directory name: 1 to 64 characters from
/// `A-Z a-z 0-9 . _ -`, not starting with a dot.
pub(crate) fn valid_version(v: &str) -> bool {
    (1..=64).contains(&v.len())
        && !v.starts_with('.')
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_key_has_the_published_fingerprint() {
        assert_eq!(yt_dlp_key().n.len(), 512);
        assert_eq!(yt_dlp_key().fingerprint_hex(), YT_DLP_KEY_FINGERPRINT);
    }

    #[test]
    fn sums_lookup() {
        let sums = "aa  other\n\
            e51e69e89489f199530f13d32a4342da3ddb2cf34842f364603a066db05afbf3  yt-dlp\n\
            F5389D3698ECDE4EF06F4E6B1CE0294FE9AC1396228C8990B8D3416D6D177A7D *yt-dlp.exe\n";
        assert_eq!(
            sum_for(sums, "yt-dlp").as_deref(),
            Some("e51e69e89489f199530f13d32a4342da3ddb2cf34842f364603a066db05afbf3")
        );
        assert_eq!(
            sum_for(sums, "yt-dlp.exe").as_deref(),
            Some("f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d")
        );
        assert_eq!(sum_for(sums, "yt-dlp_win.zip"), None);
        // Listed twice: ambiguous, refused.
        let twice = format!("{sums}{}  yt-dlp\n", "0".repeat(64));
        assert_eq!(sum_for(&twice, "yt-dlp"), None);
    }

    #[test]
    fn version_names() {
        for ok in ["2026.10.26", "n9.0.2-39-g7adfbe40dc", "2026.10.27.123456"] {
            assert!(valid_version(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "C:",
            "1 2",
            &"9".repeat(65),
        ] {
            assert!(!valid_version(bad), "{bad}");
        }
    }

    #[test]
    fn official_urls_are_pinned_hosts() {
        let s = Sources::official();
        s.check_url(Tool::YtDlp, &s.yt_dlp_stable).unwrap();
        s.check_url(Tool::YtDlp, &s.yt_dlp_nightly).unwrap();
        if let Some(b) = &s.ffmpeg {
            s.check_url(Tool::Ffmpeg, &b.url).unwrap();
            assert!(valid_version(&b.version));
        }
        assert!(s.check_url(Tool::YtDlp, "http://github.com/x").is_err());
        assert!(s.check_url(Tool::YtDlp, "https://example.com/x").is_err());
    }
}
