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

/// The app's theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
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
}
