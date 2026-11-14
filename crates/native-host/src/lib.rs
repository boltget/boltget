//! `boltget-native-host`: the small program a browser starts to let the
//! Boltget extension talk to the app (docs/ARCHITECTURE.md, "The native
//! messaging host"; docs/BROWSER-INTEGRATION.md, "Native messaging").
//!
//! The browser starts it once per port or per one-off message and talks
//! over stdin and stdout ([`stdio`]). Each message is checked
//! ([`boltget_ipc::browser::decode`]), relayed to the app over its pipe as
//! [`boltget_ipc::Request::Browser`], and the app's reply goes back
//! ([`session`]). If the app is not running, a message that brings a
//! download starts it ([`launch`]); whatever goes wrong, the extension gets
//! a reply that lets the browser keep the download. The host never touches
//! the database or the network itself, and ends when stdin closes.
//!
//! The same binary registers itself with the browsers for the installer
//! (`--register`, `--unregister`), and the app's Settings use
//! [`registration`] to show and repair it.

pub mod launch;
pub mod registration;
pub mod session;
pub mod stdio;

use std::path::PathBuf;

use boltget_ipc::Endpoint;

pub use session::{Budget, Session, Settings};

/// Where the app's database is: `BOLTGET_DATA_DIR`, as for the app and the
/// CLI, else the default. The folder is created, as the app creates it, so
/// both name the same pipe.
pub fn data_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("BOLTGET_DATA_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(boltget_engine::default_data_dir)?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

impl Settings {
    /// The settings of an installed host: the app's pipe for
    /// [`data_dir`], and the app next to this executable (or
    /// [`launch::APP_ENV`]).
    pub fn from_env() -> Self {
        let endpoint = data_dir().and_then(|d| Endpoint::for_data_dir(&d).ok());
        let exe = std::env::current_exe().ok();
        let app = launch::find_app(
            exe.as_deref().and_then(std::path::Path::parent),
            std::env::var_os(launch::APP_ENV),
        );
        Self {
            endpoint,
            app,
            budget: Budget::default(),
        }
    }
}
