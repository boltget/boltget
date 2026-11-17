//! The app's settings (docs/APP-DESIGN.md, "Settings as built"): what the
//! engine keeps (the global speed limit) and the app's own choices, one
//! value per key in the engine's `setting` table (`app.<key>`), so adding a
//! setting needs no migration and a value a newer version wrote is kept.
//!
//! Changes arrive as a [`SettingsPatch`], are validated here (the page
//! checks the same rules first, to say what is wrong next to the field),
//! stored, and applied at once: proxies and the connection cap to the
//! engine, start with Windows to the registry, clipboard monitoring and
//! notifications where they are used.

use std::path::Path;

use boltget_engine::{Engine, NoProxy, Proxy, ProxyRules};
use boltget_ipc::browser::{
    CaptureRules, MAX_MIN_SIZE, MAX_RULE_EXTENSIONS, MAX_RULE_SITES, normalise_extension,
    normalise_site,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::UiError;
use crate::platform;

/// Connections per download the engine allows (docs/ENGINE.md).
pub(crate) const MAX_CONNECTIONS: u32 = 32;
/// The engine's default cap.
pub(crate) const DEFAULT_CONNECTIONS: u32 = 16;
/// Extensions a category may hold, so a pasted list stays reasonable.
const MAX_EXTENSIONS: usize = 200;

/// Built-in categories by file type (APP-DESIGN #2), in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CategoryId {
    Video,
    Music,
    Archives,
    Programs,
    Documents,
    /// Everything else; it has no extensions of its own.
    Other,
}

impl CategoryId {
    pub(crate) const ALL: [Self; 6] = [
        Self::Video,
        Self::Music,
        Self::Archives,
        Self::Programs,
        Self::Documents,
        Self::Other,
    ];

    /// The built-in extensions, space-separated. The page has the same list
    /// (`apps/desktop/src/lib/list/category.ts`, checked by a test).
    pub(crate) fn default_extensions(self) -> &'static str {
        match self {
            Self::Video => "mp4 mkv avi mov webm wmv flv m4v mpg mpeg m2ts 3gp",
            Self::Music => "mp3 flac wav aac ogg oga m4a opus wma aiff alac mka",
            Self::Archives => "zip rar 7z tar gz tgz bz2 xz zst lz lzma cab iso img",
            Self::Programs => {
                "exe msi msix msixbundle appx appxbundle dmg pkg deb rpm apk appimage flatpak snap"
            }
            Self::Documents => {
                "pdf doc docx xls xlsx ppt pptx odt ods odp rtf txt md csv epub mobi"
            }
            Self::Other => "",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Music => "music",
            Self::Archives => "archives",
            Self::Programs => "programs",
            Self::Documents => "documents",
            Self::Other => "other",
        }
    }
}

/// One category: the extensions that put a file in it, and where its
/// downloads go (`null`: the default download folder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Category {
    pub(crate) id: CategoryId,
    /// Lowercase, without dots.
    pub(crate) extensions: Vec<String>,
    pub(crate) folder: Option<String>,
}

impl Category {
    fn builtin(id: CategoryId) -> Self {
        Self {
            id,
            extensions: id
                .default_extensions()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            folder: None,
        }
    }
}

/// How downloads reach the internet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProxyMode {
    /// Direct.
    #[default]
    None,
    /// The proxy Windows is set to use (docs/ENGINE.md, "Proxies").
    System,
    /// The proxy in `url`.
    Manual,
}

/// The proxy setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxySettings {
    pub(crate) mode: ProxyMode,
    /// `host:port`, or `http://`, `https://` or `socks5://` with it. Kept
    /// while another mode is chosen. Never holds a password.
    pub(crate) url: String,
    /// Hosts that go direct with a manual proxy, as Windows writes them:
    /// `*.corp.example; 10.*; <local>`.
    pub(crate) bypass: String,
}

/// The picker's container toggle (docs/VIDEO.md): "Prefer MP4" (the
/// default, for compatibility) or "Prefer original".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MediaContainer {
    #[default]
    Mp4,
    Original,
}

impl From<MediaContainer> for boltget_engine::ContainerPreference {
    fn from(c: MediaContainer) -> Self {
        match c {
            MediaContainer::Mp4 => Self::Mp4,
            MediaContainer::Original => Self::Original,
        }
    }
}

impl From<boltget_engine::ContainerPreference> for MediaContainer {
    fn from(c: boltget_engine::ContainerPreference) -> Self {
        match c {
            boltget_engine::ContainerPreference::Original => Self::Original,
            boltget_engine::ContainerPreference::Mp4 => Self::Mp4,
        }
    }
}

/// Settings, Video (docs/APP-DESIGN.md, "Video as built").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoSettings {
    /// What the picker prefers until the user flips the toggle.
    pub(crate) container: MediaContainer,
    /// A yt-dlp the user chose instead of the one Boltget installs: used as
    /// it is, neither verified nor updated by Boltget.
    pub(crate) yt_dlp_path: Option<String>,
    /// An `ffmpeg` the user chose (its folder must also hold `ffprobe`),
    /// likewise unverified.
    pub(crate) ffmpeg_path: Option<String>,
}

/// Settings, Video as stored: a field missing (an older version wrote it)
/// is its default.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredVideo {
    container: MediaContainer,
    yt_dlp_path: Option<String>,
    ffmpeg_path: Option<String>,
}

/// A change to Settings, Video; absent fields stay, an empty path goes back
/// to the installed tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct VideoPatch {
    pub(crate) container: Option<MediaContainer>,
    pub(crate) yt_dlp_path: Option<String>,
    pub(crate) ffmpeg_path: Option<String>,
}

/// The app's theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
}

/// What the browser extension hands over (docs/BROWSER-INTEGRATION.md,
/// "Capture rules"): the protocol's [`CaptureRules`] as the page sees them.
/// The `hello` answer carries them to the extension. Stored as one value
/// ([`stored_capture`] reads it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSettings {
    /// Capture at all.
    pub(crate) enabled: bool,
    /// The smallest download captured when its size is known, in bytes.
    pub(crate) min_size: u64,
    /// File types captured, lowercase, without dots.
    pub(crate) extensions: Vec<String>,
    /// Sites never captured (a site includes its subdomains).
    pub(crate) never_sites: Vec<String>,
    /// Sites always captured, whatever the type and size.
    pub(crate) always_sites: Vec<String>,
    /// Capture in private windows too.
    pub(crate) private_windows: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self::from(&CaptureRules::default())
    }
}

impl From<&CaptureRules> for CaptureSettings {
    fn from(r: &CaptureRules) -> Self {
        Self {
            enabled: r.enabled,
            min_size: r.min_size,
            extensions: r.extensions.clone(),
            never_sites: r.never_sites.clone(),
            always_sites: r.always_sites.clone(),
            private_windows: r.private_windows,
        }
    }
}

impl CaptureSettings {
    /// The rules as the protocol sends them.
    pub(crate) fn rules(&self) -> CaptureRules {
        let mut r = CaptureRules::default();
        r.enabled = self.enabled;
        r.min_size = self.min_size;
        r.extensions.clone_from(&self.extensions);
        r.never_sites.clone_from(&self.never_sites);
        r.always_sites.clone_from(&self.always_sites);
        r.private_windows = self.private_windows;
        r
    }
}

/// Every setting, as the page sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, each one row in Settings"
)]
pub(crate) struct AppSettings {
    /// Global speed limit in bytes per second; `null` is unlimited. Changed
    /// with `set_speed_limit`.
    pub(crate) speed_limit: Option<u64>,
    /// Most connections one download may use (1 to 32).
    pub(crate) max_connections: u32,
    /// Where downloads go when their category has no folder; `null` is the
    /// Windows Downloads folder.
    pub(crate) download_folder: Option<String>,
    /// The categories, in sidebar order, `other` last.
    pub(crate) categories: Vec<Category>,
    pub(crate) proxy: ProxySettings,
    pub(crate) theme: ThemeSetting,
    /// Windows notifications for finished downloads and problems.
    pub(crate) notifications: bool,
    /// Closing the window keeps Boltget running in the tray.
    pub(crate) close_to_tray: bool,
    /// Boltget starts in the tray when the user signs in to Windows.
    pub(crate) start_with_windows: bool,
    /// Offer links copied to the clipboard (off by default).
    pub(crate) clipboard: bool,
    /// Downloads the browser hands over open the add sheet first instead of
    /// starting (off by default: docs/BROWSER-INTEGRATION.md, "Capture
    /// flow").
    pub(crate) browser_ask: bool,
    /// What the browser extension hands over.
    pub(crate) capture: CaptureSettings,
    /// A download from a private browser window leaves the list once it
    /// completes; the file stays (off by default:
    /// docs/BROWSER-INTEGRATION.md, "What is never captured").
    pub(crate) forget_private: bool,
    /// Video and audio from web pages.
    pub(crate) video: VideoSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            speed_limit: None,
            max_connections: DEFAULT_CONNECTIONS,
            download_folder: None,
            categories: CategoryId::ALL.map(Category::builtin).to_vec(),
            proxy: ProxySettings::default(),
            theme: ThemeSetting::System,
            notifications: true,
            close_to_tray: true,
            start_with_windows: false,
            clipboard: false,
            browser_ask: false,
            capture: CaptureSettings::default(),
            forget_private: false,
            video: VideoSettings::default(),
        }
    }
}

/// A change to some settings; absent fields stay as they are. Empty
/// strings clear a folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct SettingsPatch {
    pub(crate) max_connections: Option<u32>,
    pub(crate) download_folder: Option<String>,
    /// One category's extensions and folder (others are unchanged).
    pub(crate) category: Option<CategoryPatch>,
    pub(crate) proxy: Option<ProxySettings>,
    pub(crate) theme: Option<ThemeSetting>,
    pub(crate) notifications: Option<bool>,
    pub(crate) close_to_tray: Option<bool>,
    pub(crate) start_with_windows: Option<bool>,
    pub(crate) clipboard: Option<bool>,
    pub(crate) browser_ask: Option<bool>,
    /// Some of the capture rules.
    pub(crate) capture: Option<CapturePatch>,
    pub(crate) forget_private: Option<bool>,
    pub(crate) video: Option<VideoPatch>,
}

/// A change to some capture rules; absent fields stay as they are. The
/// lists replace the stored ones whole, each entry as typed (normalised
/// here, refused when it is not a file type or a site).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct CapturePatch {
    pub(crate) enabled: Option<bool>,
    pub(crate) min_size: Option<u64>,
    pub(crate) extensions: Option<Vec<String>>,
    pub(crate) never_sites: Option<Vec<String>>,
    pub(crate) always_sites: Option<Vec<String>>,
    pub(crate) private_windows: Option<bool>,
}

/// A change to one category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CategoryPatch {
    pub(crate) id: CategoryId,
    /// As typed: separated by spaces, commas or semicolons, with or without
    /// a leading `.` or `*.`. Absent: unchanged.
    pub(crate) extensions: Option<String>,
    /// Absent: unchanged; empty: the default download folder.
    pub(crate) folder: Option<String>,
}

// Keys in the `setting` table (each stored as `app.<key>`).
const KEY_MAX_CONNECTIONS: &str = "max_connections";
const KEY_DOWNLOAD_FOLDER: &str = "download_folder";
const KEY_CATEGORIES: &str = "categories";
const KEY_PROXY: &str = "proxy";
const KEY_THEME: &str = "theme";
const KEY_NOTIFICATIONS: &str = "notifications";
const KEY_CLOSE_TO_TRAY: &str = "close_to_tray";
const KEY_CLIPBOARD: &str = "clipboard";
const KEY_BROWSER_ASK: &str = "browser_ask";
const KEY_CAPTURE: &str = "capture";
const KEY_FORGET_PRIVATE: &str = "forget_private";
const KEY_VIDEO: &str = "video";

/// Reads one stored value; a missing or unreadable one is the default.
async fn read<T: serde::de::DeserializeOwned>(engine: &Engine, key: &str) -> Option<T> {
    match engine.app_setting(key).await {
        Ok(Some(v)) => match serde_json::from_value(v) {
            Ok(value) => Some(value),
            Err(e) => {
                tracing::warn!(key, error = %e, "ignoring an unreadable setting");
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(key, error = %e, "could not read a setting");
            None
        }
    }
}

async fn write<T: Serialize>(engine: &Engine, key: &str, value: &T) -> Result<(), UiError> {
    let value = serde_json::to_value(value)
        .map_err(|e| UiError::with_message("error.internal", e.to_string()))?;
    engine.set_app_setting(key, Some(value)).await?;
    Ok(())
}

/// The stored settings, with defaults for what is not stored, and whether
/// Boltget starts with Windows now (the registry is the truth for that).
pub(crate) async fn load(engine: &Engine) -> Result<AppSettings, UiError> {
    let mut s = AppSettings {
        speed_limit: engine.settings().await?.speed_limit,
        ..AppSettings::default()
    };
    if let Some(n) = read::<u32>(engine, KEY_MAX_CONNECTIONS).await {
        s.max_connections = n.clamp(1, MAX_CONNECTIONS);
    }
    s.download_folder = read::<String>(engine, KEY_DOWNLOAD_FOLDER)
        .await
        .filter(|f| Path::new(f).is_absolute());
    if let Some(stored) = read::<Vec<Category>>(engine, KEY_CATEGORIES).await {
        s.categories = merge_categories(&stored);
    }
    if let Some(p) = read::<ProxySettings>(engine, KEY_PROXY).await {
        s.proxy = p;
    }
    if let Some(t) = read(engine, KEY_THEME).await {
        s.theme = t;
    }
    if let Some(b) = read(engine, KEY_NOTIFICATIONS).await {
        s.notifications = b;
    }
    if let Some(b) = read(engine, KEY_CLOSE_TO_TRAY).await {
        s.close_to_tray = b;
    }
    if let Some(b) = read(engine, KEY_CLIPBOARD).await {
        s.clipboard = b;
    }
    if let Some(b) = read(engine, KEY_BROWSER_ASK).await {
        s.browser_ask = b;
    }
    if let Some(c) = read::<serde_json::Value>(engine, KEY_CAPTURE)
        .await
        .and_then(|v| stored_capture(&v))
    {
        s.capture = c;
    }
    if let Some(b) = read(engine, KEY_FORGET_PRIVATE).await {
        s.forget_private = b;
    }
    if let Some(v) = read::<StoredVideo>(engine, KEY_VIDEO).await {
        s.video = VideoSettings {
            container: v.container,
            yt_dlp_path: v.yt_dlp_path.filter(|p| Path::new(p).is_absolute()),
            ffmpeg_path: v.ffmpeg_path.filter(|p| Path::new(p).is_absolute()),
        };
    }
    s.start_with_windows = platform::autostart_enabled();
    Ok(s)
}

/// The built-in categories with what was stored for them: one entry per
/// built-in id, in order, whatever the stored list holds.
fn merge_categories(stored: &[Category]) -> Vec<Category> {
    CategoryId::ALL
        .map(|id| {
            let mut c = stored
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .unwrap_or_else(|| Category::builtin(id));
            if id == CategoryId::Other {
                c.extensions.clear();
            }
            c.folder = c.folder.filter(|f| Path::new(f).is_absolute());
            c
        })
        .to_vec()
}

/// The stored capture rules, made valid: a field missing (written by an
/// older version) is its default, one unknown (a newer version's) is
/// ignored, and entries are normalised and cut to the limits rather than
/// dropped. `None` when the value is not rules at all.
fn stored_capture(stored: &serde_json::Value) -> Option<CaptureSettings> {
    let mut value = serde_json::to_value(CaptureSettings::default()).ok()?;
    let (Some(base), Some(fields)) = (value.as_object_mut(), stored.as_object()) else {
        tracing::warn!("ignoring stored capture rules that are not an object");
        return None;
    };
    for (key, v) in fields {
        if let Some(slot) = base.get_mut(key) {
            *slot = v.clone();
        }
    }
    match serde_json::from_value::<CaptureSettings>(value) {
        Ok(c) => Some(CaptureSettings::from(&c.rules().sanitised())),
        Err(e) => {
            tracing::warn!(error = %e, "ignoring unreadable capture rules");
            None
        }
    }
}

/// Typed entries of a capture list, normalised by `normalise`, duplicates
/// dropped; the first that is not one refuses them all, with `problem`
/// naming it.
fn parse_entries(
    entries: &[String],
    normalise: fn(&str) -> Option<String>,
    max: usize,
    problem: &'static str,
    too_many: &'static str,
) -> Result<Vec<String>, UiError> {
    let mut out: Vec<String> = Vec::new();
    for raw in entries.iter().filter(|e| !e.trim().is_empty()) {
        let Some(entry) = normalise(raw) else {
            let shown: String = raw.trim().chars().take(64).collect();
            return Err(UiError::with_message(problem, shown));
        };
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    if out.len() > max {
        return Err(UiError::key(too_many));
    }
    Ok(out)
}

/// Applies a change to the capture rules, checking each field.
fn patched_capture(
    current: &CaptureSettings,
    patch: &CapturePatch,
) -> Result<CaptureSettings, UiError> {
    let mut next = current.clone();
    if let Some(b) = patch.enabled {
        next.enabled = b;
    }
    if let Some(n) = patch.min_size {
        if n > MAX_MIN_SIZE {
            return Err(UiError::key("settings.problem.capture_size"));
        }
        next.min_size = n;
    }
    if let Some(list) = &patch.extensions {
        next.extensions = parse_entries(
            list,
            normalise_extension,
            MAX_RULE_EXTENSIONS,
            "settings.problem.extension",
            "settings.problem.capture_extensions_many",
        )?;
    }
    if let Some(list) = &patch.never_sites {
        next.never_sites = parse_entries(
            list,
            normalise_site,
            MAX_RULE_SITES,
            "settings.problem.site",
            "settings.problem.sites_many",
        )?;
    }
    if let Some(list) = &patch.always_sites {
        next.always_sites = parse_entries(
            list,
            normalise_site,
            MAX_RULE_SITES,
            "settings.problem.site",
            "settings.problem.sites_many",
        )?;
    }
    if let Some(b) = patch.private_windows {
        next.private_windows = b;
    }
    // What was checked above is what the protocol allows; this keeps the two
    // from drifting apart.
    next.rules()
        .validate()
        .map_err(|e| UiError::with_message("error.internal", e.to_string()))?;
    Ok(next)
}

/// Splits typed extensions: spaces, commas or semicolons between them, with
/// or without `.` or `*.` in front, any case. Duplicates are dropped.
pub(crate) fn parse_extensions(text: &str) -> Result<Vec<String>, UiError> {
    let mut out: Vec<String> = Vec::new();
    for raw in text
        .split([' ', ',', ';', '\t', '\n'])
        .filter(|e| !e.is_empty())
    {
        let ext = raw
            .trim_start_matches('*')
            .trim_start_matches('.')
            .to_ascii_lowercase();
        let valid = !ext.is_empty()
            && ext.len() <= 16
            && ext
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+'));
        if !valid {
            return Err(UiError::key("settings.problem.extension"));
        }
        if !out.contains(&ext) {
            out.push(ext);
        }
    }
    if out.len() > MAX_EXTENSIONS {
        return Err(UiError::key("settings.problem.extensions_many"));
    }
    Ok(out)
}

/// A folder the user typed or picked: absolute, and not a file.
fn parse_folder(text: &str) -> Result<Option<String>, UiError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let path = Path::new(text);
    if !path.is_absolute() {
        return Err(UiError::key("settings.problem.folder_relative"));
    }
    if path.is_file() {
        return Err(UiError::key("settings.problem.folder_file"));
    }
    Ok(Some(text.to_owned()))
}

/// A program the user points the video tools at: an absolute path to a
/// file (an `.exe` on Windows); empty goes back to the installed tool. The
/// program itself is checked by running it (`set_media_tool_path`).
pub(crate) fn parse_program_path(text: &str) -> Result<Option<String>, UiError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let path = Path::new(text);
    let exe = !cfg!(windows)
        || path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    if !path.is_absolute() || !exe {
        return Err(UiError::key("settings.problem.program_path"));
    }
    if !path.is_file() {
        return Err(UiError::key("settings.problem.program_missing"));
    }
    Ok(Some(text.to_owned()))
}

/// A manual proxy: a valid proxy URL without a password (Boltget keeps no
/// secrets in its database; docs/ARCHITECTURE.md, Security).
pub(crate) fn parse_proxy_url(text: &str) -> Result<Proxy, UiError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(UiError::key("settings.problem.proxy_empty"));
    }
    let proxy =
        Proxy::parse(text).map_err(|e| UiError::with_message("settings.problem.proxy", e))?;
    if proxy.has_credentials() {
        return Err(UiError::key("settings.problem.proxy_credentials"));
    }
    Ok(proxy)
}

/// Applies `patch` to `current`: the new settings, or the first problem.
/// Nothing outside the value changes here.
pub(crate) fn patched(
    current: &AppSettings,
    patch: &SettingsPatch,
) -> Result<AppSettings, UiError> {
    let mut next = current.clone();
    if let Some(n) = patch.max_connections {
        if !(1..=MAX_CONNECTIONS).contains(&n) {
            return Err(UiError::key("settings.problem.connections"));
        }
        next.max_connections = n;
    }
    if let Some(folder) = &patch.download_folder {
        next.download_folder = parse_folder(folder)?;
    }
    if let Some(c) = &patch.category {
        let extensions = match &c.extensions {
            Some(text) if c.id != CategoryId::Other => Some(parse_extensions(text)?),
            _ => None,
        };
        if let Some(extensions) = &extensions
            && let Some(ext) = extensions.iter().find(|ext| {
                next.categories
                    .iter()
                    .any(|other| other.id != c.id && other.extensions.contains(ext))
            })
        {
            return Err(UiError::with_message(
                "settings.problem.extension_taken",
                format!("{ext} is in another category"),
            ));
        }
        let folder = c.folder.as_deref().map(parse_folder).transpose()?;
        if let Some(category) = next.categories.iter_mut().find(|x| x.id == c.id) {
            if let Some(extensions) = extensions {
                category.extensions = extensions;
            }
            if let Some(folder) = folder {
                category.folder = folder;
            }
        }
    }
    if let Some(p) = &patch.proxy {
        let proxy = ProxySettings {
            mode: p.mode,
            url: p.url.trim().to_owned(),
            bypass: p.bypass.trim().to_owned(),
        };
        if proxy.mode == ProxyMode::Manual {
            parse_proxy_url(&proxy.url)?;
        } else if Proxy::parse(&proxy.url).is_ok_and(|p| p.has_credentials()) {
            // A URL kept for later must not hold a password either.
            return Err(UiError::key("settings.problem.proxy_credentials"));
        }
        next.proxy = proxy;
    }
    if let Some(t) = patch.theme {
        next.theme = t;
    }
    if let Some(b) = patch.notifications {
        next.notifications = b;
    }
    if let Some(b) = patch.close_to_tray {
        next.close_to_tray = b;
    }
    if let Some(b) = patch.start_with_windows {
        next.start_with_windows = b;
    }
    if let Some(b) = patch.clipboard {
        next.clipboard = b;
    }
    if let Some(b) = patch.browser_ask {
        next.browser_ask = b;
    }
    if let Some(c) = &patch.capture {
        next.capture = patched_capture(&next.capture, c)?;
    }
    if let Some(b) = patch.forget_private {
        next.forget_private = b;
    }
    if let Some(v) = &patch.video {
        if let Some(c) = v.container {
            next.video.container = c;
        }
        if let Some(p) = &v.yt_dlp_path {
            next.video.yt_dlp_path = parse_program_path(p)?;
        }
        if let Some(p) = &v.ffmpeg_path {
            next.video.ffmpeg_path = parse_program_path(p)?;
        }
    }
    Ok(next)
}

/// Stores what differs between `old` and `new`, and changes Windows' Run
/// key when start with Windows changed. The speed limit is not stored here
/// (`Engine::set_limit` does that).
pub(crate) async fn store(
    engine: &Engine,
    old: &AppSettings,
    new: &AppSettings,
) -> Result<(), UiError> {
    if old.start_with_windows != new.start_with_windows {
        platform::set_autostart(new.start_with_windows)
            .map_err(|e| UiError::with_message("settings.problem.autostart", e))?;
    }
    if old.max_connections != new.max_connections {
        write(engine, KEY_MAX_CONNECTIONS, &new.max_connections).await?;
    }
    if old.download_folder != new.download_folder {
        match &new.download_folder {
            Some(f) => write(engine, KEY_DOWNLOAD_FOLDER, f).await?,
            None => engine.set_app_setting(KEY_DOWNLOAD_FOLDER, None).await?,
        }
    }
    if old.categories != new.categories {
        write(engine, KEY_CATEGORIES, &new.categories).await?;
    }
    if old.proxy != new.proxy {
        write(engine, KEY_PROXY, &new.proxy).await?;
    }
    if old.theme != new.theme {
        write(engine, KEY_THEME, &new.theme).await?;
    }
    if old.notifications != new.notifications {
        write(engine, KEY_NOTIFICATIONS, &new.notifications).await?;
    }
    if old.close_to_tray != new.close_to_tray {
        write(engine, KEY_CLOSE_TO_TRAY, &new.close_to_tray).await?;
    }
    if old.clipboard != new.clipboard {
        write(engine, KEY_CLIPBOARD, &new.clipboard).await?;
    }
    if old.browser_ask != new.browser_ask {
        write(engine, KEY_BROWSER_ASK, &new.browser_ask).await?;
    }
    if old.capture != new.capture {
        write(engine, KEY_CAPTURE, &new.capture).await?;
    }
    if old.forget_private != new.forget_private {
        write(engine, KEY_FORGET_PRIVATE, &new.forget_private).await?;
    }
    if old.video != new.video {
        write(engine, KEY_VIDEO, &new.video).await?;
    }
    Ok(())
}

/// The Windows proxy settings as read (`WinINet`, current user).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SystemProxy {
    /// `ProxyServer` when "Use a proxy server" is on.
    pub(crate) server: Option<String>,
    /// `ProxyOverride`.
    pub(crate) bypass: Option<String>,
    /// A setup script (PAC), which Boltget does not run.
    pub(crate) script: Option<String>,
}

/// The rules downloads use for a proxy setting; `system` reads Windows'
/// settings, only when they are asked for.
pub(crate) fn rules(proxy: &ProxySettings, system: impl FnOnce() -> SystemProxy) -> ProxyRules {
    match proxy.mode {
        ProxyMode::None => ProxyRules::direct(),
        ProxyMode::Manual => match parse_proxy_url(&proxy.url) {
            Ok(p) => ProxyRules {
                no_proxy: NoProxy::parse_windows(&proxy.bypass),
                ..ProxyRules::all(p)
            },
            // Validated when it was set; a stored value that no longer
            // parses goes direct rather than nowhere.
            Err(_) => ProxyRules::direct(),
        },
        ProxyMode::System => {
            let s = system();
            ProxyRules::from_windows(
                s.server.as_deref().unwrap_or(""),
                s.bypass.as_deref().unwrap_or(""),
            )
        }
    }
}

/// What the Network section shows about Windows' proxy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemProxyInfo {
    /// The proxy `https` links would use (`host:port`, no credentials), or
    /// the `http` one; `null` when Windows uses none.
    pub(crate) proxy: Option<String>,
    /// Windows uses a setup script, which Boltget ignores.
    pub(crate) script: bool,
}

impl From<&SystemProxy> for SystemProxyInfo {
    fn from(s: &SystemProxy) -> Self {
        let rules = ProxyRules::from_windows(
            s.server.as_deref().unwrap_or(""),
            s.bypass.as_deref().unwrap_or(""),
        );
        let proxy = rules
            .https
            .as_ref()
            .or(rules.http.as_ref())
            .map(Proxy::without_credentials);
        Self {
            proxy,
            script: s.script.as_deref().is_some_and(|u| !u.trim().is_empty()),
        }
    }
}

/// The category of a file, from its name's extension.
pub(crate) fn category_of<'a>(categories: &'a [Category], name: &str) -> Option<&'a Category> {
    let ext = extension(name)?;
    categories.iter().find(|c| c.extensions.contains(&ext))
}

/// The lowercase extension of a file name (`ubuntu.ISO` is `iso`).
pub(crate) fn extension(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    let ext = &name[dot + 1..];
    (dot > 0 && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// Where a new download goes when the user does not choose: its category's
/// folder, else the default download folder, else `downloads` (Windows'
/// Downloads folder).
pub(crate) fn folder_for(
    settings: &AppSettings,
    name: Option<&str>,
    downloads: Option<&Path>,
) -> Option<std::path::PathBuf> {
    let category = name
        .and_then(|n| category_of(&settings.categories, n))
        .or_else(|| {
            settings
                .categories
                .iter()
                .find(|c| c.id == CategoryId::Other)
        });
    category
        .and_then(|c| c.folder.as_deref())
        .or(settings.download_folder.as_deref())
        .map(std::path::PathBuf::from)
        .or_else(|| downloads.map(Path::to_path_buf))
}

/// For logs: the category's name.
impl std::fmt::Display for CategoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(p: &str) -> String {
        if cfg!(windows) {
            format!("C:\\{p}")
        } else {
            format!("/{p}")
        }
    }

    fn patch() -> SettingsPatch {
        SettingsPatch::default()
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let s = AppSettings::default();
        assert_eq!(s.max_connections, 16);
        assert!(s.notifications && s.close_to_tray);
        assert!(!s.clipboard, "clipboard monitoring is opt-in");
        assert!(!s.start_with_windows);
        assert_eq!(s.proxy.mode, ProxyMode::None);
        assert_eq!(s.theme, ThemeSetting::System);
        let ids: Vec<CategoryId> = s.categories.iter().map(|c| c.id).collect();
        assert_eq!(ids, CategoryId::ALL);
        assert_eq!(s.categories[5].extensions.len(), 0);
        assert!(s.categories.iter().all(|c| c.folder.is_none()));
    }

    #[test]
    fn the_page_has_the_same_builtin_extensions() {
        let page = include_str!("../../src/lib/list/category.ts");
        for id in CategoryId::ALL {
            if id == CategoryId::Other {
                continue;
            }
            let line = format!("{}: \"{}\"", id.name(), id.default_extensions());
            assert!(page.contains(&line), "category.ts lacks {line}");
        }
    }

    #[test]
    fn extensions_are_read_as_people_type_them() {
        assert_eq!(
            parse_extensions(".MKV, *.mp4;webm  mkv").unwrap(),
            ["mkv", "mp4", "webm"]
        );
        assert_eq!(parse_extensions("").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_extensions("tar.gz").unwrap_err().key,
            "settings.problem.extension"
        );
        assert!(parse_extensions("a/b").is_err());
        assert!(parse_extensions(&"x".repeat(17)).is_err());
        let many: Vec<String> = (0..201).map(|i| format!("e{i}")).collect();
        assert_eq!(
            parse_extensions(&many.join(" ")).unwrap_err().key,
            "settings.problem.extensions_many"
        );
    }

    #[test]
    fn patches_validate_each_field() {
        let s = AppSettings::default();
        // Connections.
        let p = SettingsPatch {
            max_connections: Some(8),
            ..patch()
        };
        assert_eq!(patched(&s, &p).unwrap().max_connections, 8);
        for bad in [0, 33] {
            let p = SettingsPatch {
                max_connections: Some(bad),
                ..patch()
            };
            assert_eq!(
                patched(&s, &p).unwrap_err().key,
                "settings.problem.connections"
            );
        }
        // Folders: absolute, or empty for the default.
        let p = SettingsPatch {
            download_folder: Some(abs("Data\\Downloads")),
            ..patch()
        };
        let next = patched(&s, &p).unwrap();
        assert_eq!(
            next.download_folder.as_deref(),
            Some(abs("Data\\Downloads").as_str())
        );
        let p = SettingsPatch {
            download_folder: Some("  ".into()),
            ..patch()
        };
        assert_eq!(patched(&next, &p).unwrap().download_folder, None);
        let p = SettingsPatch {
            download_folder: Some("relative\\dir".into()),
            ..patch()
        };
        assert_eq!(
            patched(&s, &p).unwrap_err().key,
            "settings.problem.folder_relative"
        );
        // A file is not a folder.
        let file = tempfile::NamedTempFile::new().unwrap();
        let p = SettingsPatch {
            download_folder: Some(file.path().to_string_lossy().into_owned()),
            ..patch()
        };
        assert_eq!(
            patched(&s, &p).unwrap_err().key,
            "settings.problem.folder_file"
        );
        // Plain switches.
        let p = SettingsPatch {
            theme: Some(ThemeSetting::Dark),
            notifications: Some(false),
            close_to_tray: Some(false),
            clipboard: Some(true),
            start_with_windows: Some(true),
            ..patch()
        };
        let next = patched(&s, &p).unwrap();
        assert_eq!(next.theme, ThemeSetting::Dark);
        assert!(!next.notifications && !next.close_to_tray);
        assert!(next.clipboard && next.start_with_windows);
        // An empty patch changes nothing.
        assert_eq!(patched(&s, &patch()).unwrap(), s);
    }

    #[test]
    fn categories_change_one_at_a_time_without_sharing_extensions() {
        let s = AppSettings::default();
        let p = SettingsPatch {
            category: Some(CategoryPatch {
                id: CategoryId::Video,
                extensions: Some("mp4 mkv ts".into()),
                folder: Some(abs("Videos")),
            }),
            ..patch()
        };
        let next = patched(&s, &p).unwrap();
        let video = &next.categories[0];
        assert_eq!(video.extensions, ["mp4", "mkv", "ts"]);
        assert_eq!(video.folder.as_deref(), Some(abs("Videos").as_str()));
        assert_eq!(next.categories[1], s.categories[1], "others are unchanged");

        // Only the folder: the extensions stay.
        let p = SettingsPatch {
            category: Some(CategoryPatch {
                id: CategoryId::Video,
                extensions: None,
                folder: Some(String::new()),
            }),
            ..patch()
        };
        let cleared = patched(&next, &p).unwrap();
        assert_eq!(cleared.categories[0].extensions, ["mp4", "mkv", "ts"]);
        assert_eq!(cleared.categories[0].folder, None);

        // An extension of another category is refused.
        let p = SettingsPatch {
            category: Some(CategoryPatch {
                id: CategoryId::Video,
                extensions: Some("mp4 mp3".into()),
                folder: None,
            }),
            ..patch()
        };
        assert_eq!(
            patched(&s, &p).unwrap_err().key,
            "settings.problem.extension_taken"
        );
        // Other never gets extensions, but may get a folder.
        let p = SettingsPatch {
            category: Some(CategoryPatch {
                id: CategoryId::Other,
                extensions: Some("xyz".into()),
                folder: Some(abs("Misc")),
            }),
            ..patch()
        };
        let next = patched(&s, &p).unwrap();
        assert_eq!(next.categories[5].extensions.len(), 0);
        assert_eq!(
            next.categories[5].folder.as_deref(),
            Some(abs("Misc").as_str())
        );
    }

    #[test]
    fn stored_categories_merge_with_the_builtins() {
        let stored = vec![
            Category {
                id: CategoryId::Music,
                extensions: vec!["mp3".into()],
                folder: Some(abs("Music")),
            },
            Category {
                id: CategoryId::Other,
                extensions: vec!["junk".into()],
                folder: Some("relative".into()),
            },
        ];
        let merged = merge_categories(&stored);
        assert_eq!(merged.len(), 6);
        assert_eq!(merged[0], Category::builtin(CategoryId::Video));
        assert_eq!(merged[1].extensions, ["mp3"]);
        assert_eq!(merged[1].folder.as_deref(), Some(abs("Music").as_str()));
        assert_eq!(merged[5].extensions.len(), 0);
        assert_eq!(merged[5].folder, None, "a relative folder is dropped");
    }

    #[test]
    fn manual_proxies_are_checked_and_keep_no_password() {
        let s = AppSettings::default();
        let manual = |url: &str| SettingsPatch {
            proxy: Some(ProxySettings {
                mode: ProxyMode::Manual,
                url: url.into(),
                bypass: "*.corp.example".into(),
            }),
            ..patch()
        };
        let next = patched(&s, &manual(" proxy.corp:8080 ")).unwrap();
        assert_eq!(next.proxy.url, "proxy.corp:8080");
        assert_eq!(
            patched(&s, &manual("")).unwrap_err().key,
            "settings.problem.proxy_empty"
        );
        assert_eq!(
            patched(&s, &manual("ftp://p.corp")).unwrap_err().key,
            "settings.problem.proxy"
        );
        let err = patched(&s, &manual("http://me:secret@p.corp:8080")).unwrap_err();
        assert_eq!(err.key, "settings.problem.proxy_credentials");
        assert!(!format!("{err:?}").contains("secret"));
        // Another mode may keep an unfinished URL, never a password.
        let other = |mode, url: &str| SettingsPatch {
            proxy: Some(ProxySettings {
                mode,
                url: url.into(),
                bypass: String::new(),
            }),
            ..patch()
        };
        assert!(patched(&s, &other(ProxyMode::System, "half:")).is_ok());
        assert_eq!(
            patched(&s, &other(ProxyMode::None, "http://me:pw@p.corp"))
                .unwrap_err()
                .key,
            "settings.problem.proxy_credentials"
        );
    }

    #[test]
    fn proxy_settings_resolve_to_rules() {
        let none = ProxySettings::default();
        let unread = || -> SystemProxy { panic!("Windows' settings are read only for System") };
        assert!(rules(&none, unread).is_direct());

        let manual = ProxySettings {
            mode: ProxyMode::Manual,
            url: "socks5://127.0.0.1:1080".into(),
            bypass: "*.corp.example".into(),
        };
        let r = rules(&manual, unread);
        assert!(r.https.is_some() && !r.no_proxy.matches("example.com"));
        assert!(r.no_proxy.matches("files.corp.example"));
        assert!(r.no_proxy.matches("localhost"));

        let system = ProxySettings {
            mode: ProxyMode::System,
            ..ProxySettings::default()
        };
        let r = rules(&system, || SystemProxy {
            server: Some("http=h.corp:3128;https=s.corp:3129".into()),
            bypass: Some("<local>".into()),
            script: None,
        });
        assert!(r.https.is_some() && !r.no_proxy.matches("example.com"));
        assert!(r.no_proxy.matches("intranet"));
        // Windows without a proxy (or with a script only): direct.
        assert!(rules(&system, SystemProxy::default).is_direct());
        let script_only = SystemProxy {
            script: Some("http://wpad/proxy.pac".into()),
            ..SystemProxy::default()
        };
        assert!(rules(&system, || script_only.clone()).is_direct());
        let info = SystemProxyInfo::from(&script_only);
        assert_eq!(info.proxy, None);
        assert!(info.script);
        let info = SystemProxyInfo::from(&SystemProxy {
            server: Some("http=h.corp:3128;https=s.corp:3129".into()),
            ..SystemProxy::default()
        });
        assert_eq!(info.proxy.as_deref(), Some("http://s.corp:3129"));
        assert!(!info.script);
    }

    #[test]
    fn files_find_their_category_and_folder() {
        let mut s = AppSettings::default();
        let cats = &s.categories;
        assert_eq!(
            category_of(cats, "ubuntu.ISO").unwrap().id,
            CategoryId::Archives
        );
        assert_eq!(
            category_of(cats, "movie.mkv").unwrap().id,
            CategoryId::Video
        );
        assert!(category_of(cats, "notes").is_none());
        assert!(category_of(cats, ".bashrc").is_none());
        assert!(category_of(cats, "trailing.").is_none());

        let downloads = std::path::PathBuf::from(abs("Users\\me\\Downloads"));
        // Nothing set: Windows' Downloads folder.
        assert_eq!(
            folder_for(&s, Some("a.mkv"), Some(&downloads)),
            Some(downloads.clone())
        );
        assert_eq!(folder_for(&s, None, None), None);
        // A default folder, then a category folder.
        s.download_folder = Some(abs("Data"));
        s.categories[0].folder = Some(abs("Videos"));
        s.categories[5].folder = Some(abs("Misc"));
        let path = |p: &str| Some(std::path::PathBuf::from(abs(p)));
        assert_eq!(
            folder_for(&s, Some("a.mkv"), Some(&downloads)),
            path("Videos")
        );
        assert_eq!(
            folder_for(&s, Some("a.zip"), Some(&downloads)),
            path("Data")
        );
        // Unknown types are Other.
        assert_eq!(
            folder_for(&s, Some("a.xyz"), Some(&downloads)),
            path("Misc")
        );
        assert_eq!(folder_for(&s, None, Some(&downloads)), path("Misc"));
    }

    fn capture(patch: CapturePatch) -> SettingsPatch {
        SettingsPatch {
            capture: Some(patch),
            ..SettingsPatch::default()
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn capture_rules_default_to_the_protocols() {
        let s = AppSettings::default();
        assert_eq!(s.capture.rules(), CaptureRules::default());
        assert!(s.capture.enabled && !s.capture.private_windows);
        assert_eq!(s.capture.min_size, 1 << 20);
        // The page has the same defaults (DEFAULT_CAPTURE in capture.ts, which
        // its own test holds to DEFAULT_EXTENSIONS).
        let json = serde_json::to_value(&s.capture).unwrap();
        assert_eq!(
            json.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "alwaysSites",
                "enabled",
                "extensions",
                "minSize",
                "neverSites",
                "privateWindows"
            ]
        );
    }

    #[test]
    fn capture_patches_normalise_and_check_each_field() {
        let s = AppSettings::default();
        let next = patched(
            &s,
            &capture(CapturePatch {
                enabled: Some(false),
                min_size: Some(0),
                extensions: Some(strings(&[".ZIP", "*.7z", "zip", " ", "pdf"])),
                never_sites: Some(strings(&["https://Bank.example/login", "*.corp.example"])),
                always_sites: Some(strings(&["cdn.example:8443"])),
                private_windows: Some(true),
            }),
        )
        .unwrap();
        let c = &next.capture;
        assert!(!c.enabled && c.private_windows);
        assert_eq!(c.min_size, 0);
        assert_eq!(c.extensions, ["zip", "7z", "pdf"]);
        assert_eq!(c.never_sites, ["bank.example", "corp.example"]);
        assert_eq!(c.always_sites, ["cdn.example"]);
        assert_eq!(
            next.capture.rules().validate(),
            Ok(()),
            "the hello answer is valid"
        );

        // Absent fields stay.
        let only = patched(
            &next,
            &capture(CapturePatch {
                min_size: Some(5 << 20),
                ..CapturePatch::default()
            }),
        )
        .unwrap();
        assert_eq!(only.capture.min_size, 5 << 20);
        assert_eq!(only.capture.never_sites, c.never_sites);

        // Refusals, each with the words for its field.
        let refused = |p: CapturePatch| patched(&s, &capture(p)).unwrap_err().key;
        assert_eq!(
            refused(CapturePatch {
                min_size: Some(MAX_MIN_SIZE + 1),
                ..CapturePatch::default()
            }),
            "settings.problem.capture_size"
        );
        assert_eq!(
            refused(CapturePatch {
                extensions: Some(strings(&["tar.gz"])),
                ..CapturePatch::default()
            }),
            "settings.problem.extension"
        );
        assert_eq!(
            refused(CapturePatch {
                extensions: Some((0..=MAX_RULE_EXTENSIONS).map(|i| format!("e{i}")).collect()),
                ..CapturePatch::default()
            }),
            "settings.problem.capture_extensions_many"
        );
        let err = patched(
            &s,
            &capture(CapturePatch {
                always_sites: Some(strings(&["ok.example", "not a site"])),
                ..CapturePatch::default()
            }),
        )
        .unwrap_err();
        assert_eq!(err.key, "settings.problem.site");
        assert_eq!(err.message.as_deref(), Some("not a site"));
        assert_eq!(
            refused(CapturePatch {
                never_sites: Some(
                    (0..=MAX_RULE_SITES)
                        .map(|i| format!("s{i}.example"))
                        .collect()
                ),
                ..CapturePatch::default()
            }),
            "settings.problem.sites_many"
        );
        // A refused patch changes nothing, the other fields included.
        assert!(
            patched(
                &s,
                &SettingsPatch {
                    browser_ask: Some(true),
                    capture: Some(CapturePatch {
                        never_sites: Some(strings(&["?"])),
                        ..CapturePatch::default()
                    }),
                    ..SettingsPatch::default()
                }
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn forgetting_private_downloads_is_off_until_turned_on() {
        let data = tempfile::tempdir().unwrap();
        let engine = Engine::open(boltget_engine::EngineConfig::default(), data.path()).unwrap();
        let before = load(&engine).await.unwrap();
        assert!(!before.forget_private);
        let after = patched(
            &before,
            &SettingsPatch {
                forget_private: Some(true),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert!(after.forget_private);
        assert_eq!(after.capture, before.capture, "not a capture rule");
        store(&engine, &before, &after).await.unwrap();
        assert!(load(&engine).await.unwrap().forget_private);
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn capture_rules_are_stored_and_read_back() {
        let data = tempfile::tempdir().unwrap();
        let engine = Engine::open(boltget_engine::EngineConfig::default(), data.path()).unwrap();

        // Nothing stored (a settings file from before): the defaults.
        let before = load(&engine).await.unwrap();
        assert_eq!(before.capture, CaptureSettings::default());

        let after = patched(
            &before,
            &capture(CapturePatch {
                min_size: Some(8 << 20),
                never_sites: Some(strings(&["bank.example"])),
                ..CapturePatch::default()
            }),
        )
        .unwrap();
        store(&engine, &before, &after).await.unwrap();
        let read = load(&engine).await.unwrap();
        assert_eq!(read.capture, after.capture);

        // A value another version wrote: a field missing, one unknown, an
        // entry not normalised, too many sites. Kept, made valid.
        let sites: Vec<String> = (0..600).map(|i| format!("S{i}.example")).collect();
        engine
            .set_app_setting(
                KEY_CAPTURE,
                Some(serde_json::json!({
                    "enabled": false,
                    "extensions": ["ISO", ".zip", "bad ext"],
                    "neverSites": sites,
                    "futureField": 1,
                })),
            )
            .await
            .unwrap();
        let c = load(&engine).await.unwrap().capture;
        assert!(!c.enabled);
        assert_eq!(c.min_size, 1 << 20, "missing: the default");
        assert_eq!(c.extensions, ["iso", "zip"]);
        assert_eq!(c.never_sites.len(), MAX_RULE_SITES);
        assert_eq!(c.never_sites[0], "s0.example");
        c.rules().validate().unwrap();

        // An unreadable value: the defaults.
        engine
            .set_app_setting(KEY_CAPTURE, Some(serde_json::json!("garbage")))
            .await
            .unwrap();
        assert_eq!(
            load(&engine).await.unwrap().capture,
            CaptureSettings::default()
        );
        engine.shutdown().await;
    }
}
