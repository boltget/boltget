//! Telling the browsers where the host is (docs/BROWSER-INTEGRATION.md,
//! "Native messaging host registration on Windows").
//!
//! Each browser looks the host's name up under its own registry key; the
//! key's default value is the path of a manifest file, which names the
//! host's executable and the extensions allowed to start it. Boltget writes
//! two manifests next to the host (one in Chromium's format, for Chrome and
//! Edge, one in Firefox's) and one key per browser, all in the current
//! user's part of the registry, so no administrator is needed.
//!
//! The extension ids come from one file,
//! `packages/native-messaging/extensions.json`, which the extension's build
//! reads too. Release builds register [`browser::HOST_NAME`] for the store
//! ids; development builds [`browser::DEV_HOST_NAME`] for the development
//! extension (its id is pinned by the public key in the same file), so the
//! two never meet.
//!
//! The installer runs `boltget-native-host --register` and, on uninstall,
//! `--unregister`; the app's "Repair browser integration" does the same.

use std::io;
use std::path::{Path, PathBuf};

use boltget_ipc::browser;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The extension ids, in the one place they are kept.
const EXTENSIONS: &str = include_str!("../../../packages/native-messaging/extensions.json");

/// The host's file name.
#[cfg(windows)]
pub const HOST_EXE: &str = "boltget-native-host.exe";
/// The host's file name.
#[cfg(not(windows))]
pub const HOST_EXE: &str = "boltget-native-host";

/// Which extensions a build serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flavor {
    /// The store extensions, under [`browser::HOST_NAME`].
    Release,
    /// The development extension, under [`browser::DEV_HOST_NAME`].
    Development,
}

impl Flavor {
    /// This build's: development in debug builds, release otherwise.
    pub fn current() -> Self {
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Release
        }
    }

    /// The host name registered for it.
    pub fn host_name(self) -> &'static str {
        match self {
            Self::Release => browser::HOST_NAME,
            Self::Development => browser::DEV_HOST_NAME,
        }
    }
}

/// The extensions one flavor lets start the host.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extensions {
    /// The host name (checked against [`Flavor::host_name`]).
    pub host: String,
    /// Chromium extension ids (Chrome Web Store, Edge Add-ons, or the
    /// development id).
    pub chromium: Vec<String>,
    /// The public key that pins the development extension's id.
    #[serde(default)]
    pub chromium_key: Option<String>,
    /// Firefox extension ids (`browser_specific_settings.gecko.id`).
    pub firefox: Vec<String>,
}

#[derive(Deserialize)]
struct AllExtensions {
    release: Extensions,
    development: Extensions,
}

/// The extensions of `flavor`, from `extensions.json`.
pub fn extensions(flavor: Flavor) -> Result<Extensions, RegistrationError> {
    let all: AllExtensions =
        serde_json::from_str(EXTENSIONS).map_err(|e| RegistrationError::Config(e.to_string()))?;
    let ext = match flavor {
        Flavor::Release => all.release,
        Flavor::Development => all.development,
    };
    if ext.host != flavor.host_name() {
        return Err(RegistrationError::Config(format!(
            "extensions.json names {} for {flavor:?}",
            ext.host
        )));
    }
    let chromium_id =
        |id: &String| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b));
    if !ext.chromium.iter().all(chromium_id) {
        return Err(RegistrationError::Config(
            "a Chromium extension id is not 32 letters from a to p".into(),
        ));
    }
    Ok(ext)
}

/// A browser Boltget registers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Browser {
    /// Google Chrome (other Chromium browsers may read its key too).
    Chrome,
    /// Microsoft Edge.
    Edge,
    /// Mozilla Firefox.
    Firefox,
}

impl Browser {
    /// Every browser, in the order Settings lists them.
    pub const ALL: [Self; 3] = [Self::Chrome, Self::Edge, Self::Firefox];

    /// The registry key, under `HKEY_CURRENT_USER`, whose default value
    /// names the manifest.
    pub fn registry_key(self, host: &str) -> String {
        let vendor = match self {
            Self::Chrome => r"Software\Google\Chrome",
            Self::Edge => r"Software\Microsoft\Edge",
            Self::Firefox => r"Software\Mozilla",
        };
        format!(r"{vendor}\NativeMessagingHosts\{host}")
    }

    fn manifest(self) -> Manifest {
        match self {
            Self::Chrome | Self::Edge => Manifest::Chromium,
            Self::Firefox => Manifest::Firefox,
        }
    }
}

/// The two manifest formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manifest {
    Chromium,
    Firefox,
}

impl Manifest {
    fn file_name(self, host: &str) -> String {
        match self {
            Self::Chromium => format!("{host}.chrome.json"),
            Self::Firefox => format!("{host}.firefox.json"),
        }
    }

    fn contents(self, ext: &Extensions, host_exe: &Path) -> serde_json::Value {
        let mut m = json!({
            "name": ext.host,
            "description": "Boltget native messaging host",
            "path": host_exe.to_string_lossy(),
            "type": "stdio",
        });
        match self {
            Self::Chromium => {
                m["allowed_origins"] = ext
                    .chromium
                    .iter()
                    .map(|id| format!("chrome-extension://{id}/"))
                    .collect();
            }
            Self::Firefox => m["allowed_extensions"] = ext.firefox.clone().into(),
        }
        m
    }
}

/// Where a browser's registration stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The browser finds this host.
    Registered,
    /// The browser has no key for the host.
    Missing,
    /// The key or its manifest leads somewhere else, or nowhere (Boltget
    /// moved, or a file is gone).
    WrongPath,
    /// The manifest is this host's but names other extensions (an older
    /// release's).
    Outdated,
}

/// One browser's registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserStatus {
    /// The browser.
    pub browser: Browser,
    /// How it stands.
    pub state: State,
}

/// Why registering failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistrationError {
    /// `extensions.json` does not hold what it should.
    #[error("extensions.json: {0}")]
    Config(String),
    /// The host is not where it should be.
    #[error("the native host is missing: {0}")]
    NoHost(PathBuf),
    /// Writing or removing a file or key failed.
    #[error("{what}: {source}")]
    Io {
        /// What was being done.
        what: String,
        /// Why it failed.
        source: io::Error,
    },
    /// The browsers' registration is not implemented on this system yet.
    #[error("browser integration is supported on Windows only for now")]
    Unsupported,
}

fn io_error(what: impl Into<String>) -> impl FnOnce(io::Error) -> RegistrationError {
    let what = what.into();
    move |source| RegistrationError::Io { what, source }
}

/// The registry, as far as registration needs it: one string per key (its
/// default value). [`system`] is the real one; tests use a map.
pub trait Registry {
    /// The key's default value; `None` when the key does not exist.
    fn get(&self, key: &str) -> io::Result<Option<String>>;
    /// Creates the key if needed and sets its default value.
    fn set(&self, key: &str, value: &str) -> io::Result<()>;
    /// Deletes the key and everything under it; a missing key is fine.
    fn delete(&self, key: &str) -> io::Result<()>;
}

/// One host's registration: its executable, its manifests' folder, and the
/// extensions it serves.
#[derive(Debug, Clone)]
pub struct Registration {
    host_exe: PathBuf,
    dir: PathBuf,
    flavor: Flavor,
}

impl Registration {
    /// The host `host_exe` of `flavor`, with its manifests beside it.
    pub fn new(host_exe: impl Into<PathBuf>, flavor: Flavor) -> Self {
        let host_exe = host_exe.into();
        let dir = host_exe
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        Self {
            host_exe,
            dir,
            flavor,
        }
    }

    /// The host in `dir` (the app's own folder, where the installer puts
    /// both).
    pub fn in_dir(dir: &Path, flavor: Flavor) -> Self {
        Self::new(dir.join(HOST_EXE), flavor)
    }

    /// The host's executable.
    pub fn host_exe(&self) -> &Path {
        &self.host_exe
    }

    fn manifest_path(&self, m: Manifest) -> PathBuf {
        self.dir.join(m.file_name(self.flavor.host_name()))
    }

    /// Writes both manifests and every browser's key. Run again, it
    /// repairs: everything is rewritten for this host.
    pub fn register(&self, registry: &dyn Registry) -> Result<(), RegistrationError> {
        let ext = extensions(self.flavor)?;
        if !self.host_exe.is_file() {
            return Err(RegistrationError::NoHost(self.host_exe.clone()));
        }
        for m in [Manifest::Chromium, Manifest::Firefox] {
            let path = self.manifest_path(m);
            let text = serde_json::to_string_pretty(&m.contents(&ext, &self.host_exe))
                .map_err(|e| RegistrationError::Config(e.to_string()))?;
            std::fs::write(&path, text + "\n")
                .map_err(io_error(format!("writing {}", path.display())))?;
        }
        for b in Browser::ALL {
            let key = b.registry_key(&ext.host);
            let manifest = self.manifest_path(b.manifest());
            registry
                .set(&key, &manifest.to_string_lossy())
                .map_err(io_error(format!("writing HKCU\\{key}")))?;
        }
        Ok(())
    }

    /// Removes every browser's key and both manifests.
    pub fn unregister(&self, registry: &dyn Registry) -> Result<(), RegistrationError> {
        let host = self.flavor.host_name();
        for b in Browser::ALL {
            let key = b.registry_key(host);
            registry
                .delete(&key)
                .map_err(io_error(format!("removing HKCU\\{key}")))?;
        }
        for m in [Manifest::Chromium, Manifest::Firefox] {
            let path = self.manifest_path(m);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(format!("removing {}", path.display()))(e)),
            }
        }
        Ok(())
    }

    /// How each browser's registration stands.
    pub fn status(&self, registry: &dyn Registry) -> Vec<BrowserStatus> {
        let ext = extensions(self.flavor).ok();
        Browser::ALL
            .iter()
            .map(|&browser| BrowserStatus {
                browser,
                state: self.state(registry, browser, ext.as_ref()),
            })
            .collect()
    }

    fn state(&self, registry: &dyn Registry, b: Browser, ext: Option<&Extensions>) -> State {
        let Ok(Some(path)) = registry.get(&b.registry_key(self.flavor.host_name())) else {
            return State::Missing;
        };
        let expected = self.manifest_path(b.manifest());
        if !same_path(Path::new(&path), &expected) {
            return State::WrongPath;
        }
        let Some(manifest) = std::fs::read(&expected)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        else {
            return State::WrongPath;
        };
        let points_here = manifest["path"]
            .as_str()
            .is_some_and(|p| same_path(Path::new(p), &self.host_exe));
        if !points_here || !self.host_exe.is_file() {
            return State::WrongPath;
        }
        match ext {
            Some(ext) if manifest == b.manifest().contents(ext, &self.host_exe) => {
                State::Registered
            }
            _ => State::Outdated,
        }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// The current user's registry.
#[cfg(windows)]
pub fn system() -> Result<impl Registry, RegistrationError> {
    Ok(WindowsRegistry)
}

/// The current user's registry: none here yet (manifest folders replace it
/// on macOS and Linux, after 1.0).
#[cfg(not(windows))]
pub fn system() -> Result<NoRegistry, RegistrationError> {
    Err(RegistrationError::Unsupported)
}

/// The registry of a system that has none.
#[cfg(not(windows))]
#[derive(Debug)]
pub enum NoRegistry {}

#[cfg(not(windows))]
impl Registry for NoRegistry {
    fn get(&self, _key: &str) -> io::Result<Option<String>> {
        match *self {}
    }
    fn set(&self, _key: &str, _value: &str) -> io::Result<()> {
        match *self {}
    }
    fn delete(&self, _key: &str) -> io::Result<()> {
        match *self {}
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct WindowsRegistry;

#[cfg(windows)]
impl Registry for WindowsRegistry {
    fn get(&self, key: &str) -> io::Result<Option<String>> {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        match hkcu.open_subkey(key) {
            Ok(k) => match k.get_value::<String, _>("") {
                Ok(v) => Ok(Some(v)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn set(&self, key: &str, value: &str) -> io::Result<()> {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let (k, _) = hkcu.create_subkey(key)?;
        k.set_value("", &value)
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        match hkcu.delete_subkey_all(key) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct MapRegistry(RefCell<HashMap<String, String>>);

    impl Registry for MapRegistry {
        fn get(&self, key: &str) -> io::Result<Option<String>> {
            Ok(self.0.borrow().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> io::Result<()> {
            self.0.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> io::Result<()> {
            self.0.borrow_mut().remove(key);
            Ok(())
        }
    }

    fn states(r: &Registration, reg: &MapRegistry) -> Vec<State> {
        r.status(reg).into_iter().map(|s| s.state).collect()
    }

    fn install() -> (tempfile::TempDir, Registration) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(HOST_EXE), b"").unwrap();
        let r = Registration::in_dir(dir.path(), Flavor::Release);
        (dir, r)
    }

    #[test]
    fn the_extension_ids_are_well_formed() {
        let release = extensions(Flavor::Release).unwrap();
        assert_eq!(release.host, "app.boltget.native_host");
        assert_eq!(release.firefox, ["extension@boltget.app"]);
        assert_eq!(
            release.chromium.len(),
            2,
            "Chrome Web Store and Edge Add-ons"
        );
        let dev = extensions(Flavor::Development).unwrap();
        assert_eq!(dev.host, "app.boltget.native_host_dev");
        assert!(release.chromium.iter().all(|id| !dev.chromium.contains(id)));
        assert!(release.firefox.iter().all(|id| !dev.firefox.contains(id)));
    }

    #[test]
    fn the_development_id_is_the_pinned_keys() {
        // Chromium derives an unpacked extension's id from its key: the
        // first 128 bits of the SHA-256 of the public key, as letters a-p.
        let dev = extensions(Flavor::Development).unwrap();
        let key = dev.chromium_key.expect("a pinned key");
        let der = base64_decode(&key);
        let digest = <sha2::Sha256 as sha2::Digest>::digest(&der);
        let id: String = digest[..16]
            .iter()
            .flat_map(|b| [b >> 4, b & 0xf])
            .map(|n| char::from(b'a' + n))
            .collect();
        assert_eq!(dev.chromium, [id]);
    }

    #[test]
    fn register_check_and_unregister() {
        let (dir, r) = install();
        let reg = MapRegistry::default();
        assert_eq!(states(&r, &reg), [State::Missing; 3]);
        r.register(&reg).unwrap();
        assert_eq!(states(&r, &reg), [State::Registered; 3]);

        let chrome = reg
            .get(r"Software\Google\Chrome\NativeMessagingHosts\app.boltget.native_host")
            .unwrap()
            .unwrap();
        let edge = reg
            .get(r"Software\Microsoft\Edge\NativeMessagingHosts\app.boltget.native_host")
            .unwrap()
            .unwrap();
        let firefox = reg
            .get(r"Software\Mozilla\NativeMessagingHosts\app.boltget.native_host")
            .unwrap()
            .unwrap();
        assert_eq!(chrome, edge, "one Chromium manifest");
        assert_ne!(chrome, firefox);
        let m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&chrome).unwrap()).unwrap();
        assert_eq!(m["name"], "app.boltget.native_host");
        assert_eq!(m["type"], "stdio");
        assert_eq!(
            m["path"],
            dir.path().join(HOST_EXE).to_string_lossy().as_ref()
        );
        assert_eq!(
            m["allowed_origins"][0],
            "chrome-extension://aeobielfcjcidnadamggpnomipkijcbj/"
        );
        assert!(m.get("allowed_extensions").is_none());
        let f: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&firefox).unwrap()).unwrap();
        assert_eq!(f["allowed_extensions"][0], "extension@boltget.app");

        r.unregister(&reg).unwrap();
        assert_eq!(states(&r, &reg), [State::Missing; 3]);
        assert!(!Path::new(&chrome).exists() && !Path::new(&firefox).exists());
        r.unregister(&reg).unwrap();
    }

    #[test]
    fn what_needs_repair() {
        let (dir, r) = install();
        let reg = MapRegistry::default();
        r.register(&reg).unwrap();
        let key = Browser::Firefox.registry_key(browser::HOST_NAME);

        // The key leads elsewhere (another install).
        reg.set(&key, r"C:\Elsewhere\app.boltget.native_host.firefox.json")
            .unwrap();
        assert_eq!(states(&r, &reg)[2], State::WrongPath);
        // The manifest is gone.
        r.register(&reg).unwrap();
        let manifest = reg.get(&key).unwrap().unwrap();
        std::fs::remove_file(&manifest).unwrap();
        assert_eq!(states(&r, &reg)[2], State::WrongPath);
        // The manifest names other extensions.
        r.register(&reg).unwrap();
        let mut m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        m["allowed_extensions"] = json!(["old@boltget.app"]);
        std::fs::write(&manifest, m.to_string()).unwrap();
        assert_eq!(states(&r, &reg)[2], State::Outdated);
        // The host moved: the manifests point at nothing.
        r.register(&reg).unwrap();
        std::fs::remove_file(dir.path().join(HOST_EXE)).unwrap();
        assert_eq!(states(&r, &reg), [State::WrongPath; 3]);
        assert!(matches!(
            r.register(&reg),
            Err(RegistrationError::NoHost(_))
        ));
    }

    #[test]
    fn development_builds_register_their_own_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(HOST_EXE), b"").unwrap();
        let dev = Registration::in_dir(dir.path(), Flavor::Development);
        let release = Registration::in_dir(dir.path(), Flavor::Release);
        let reg = MapRegistry::default();
        dev.register(&reg).unwrap();
        assert_eq!(states(&dev, &reg), [State::Registered; 3]);
        assert_eq!(states(&release, &reg), [State::Missing; 3]);
        assert!(
            reg.get(&Browser::Chrome.registry_key(browser::DEV_HOST_NAME))
                .unwrap()
                .unwrap()
                .ends_with("app.boltget.native_host_dev.chrome.json")
        );
    }

    fn base64_decode(text: &str) -> Vec<u8> {
        let value = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64"),
        };
        let digits: Vec<u8> = text.bytes().filter(|&c| c != b'=').map(value).collect();
        digits
            .chunks(4)
            .flat_map(|c| {
                let n = c.iter().fold(0u32, |n, &d| (n << 6) | u32::from(d)) << (6 * (4 - c.len()));
                let bytes = n.to_be_bytes();
                bytes[1..c.len()].to_vec()
            })
            .collect()
    }
}
