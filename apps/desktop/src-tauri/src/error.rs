//! The error every command returns (docs/ARCHITECTURE.md, "UI to backend").

use serde::Serialize;
use specta::Type;

/// An error the UI shows by looking up `key` in the string catalog
/// (`packages/ui/src/messages/en.json`), never by showing prose from Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub(crate) struct UiError {
    /// Message-catalog key, such as `error.network`.
    pub(crate) key: String,
    /// English diagnostic text for logs and developer tools; never shown as
    /// a UI string.
    pub(crate) message: Option<String>,
}

impl UiError {
    /// An error with a catalog key only.
    pub(crate) fn key(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            message: None,
        }
    }

    /// An error with a catalog key and a diagnostic.
    pub(crate) fn with_message(key: &str, message: impl Into<String>) -> Self {
        Self {
            key: key.to_owned(),
            message: Some(message.into()),
        }
    }

    /// The window could not be changed.
    pub(crate) fn window() -> Self {
        Self::key("error.window")
    }

    /// The engine has not started (another process owns the database, or it
    /// failed to open).
    pub(crate) fn engine_unavailable() -> Self {
        Self::key("error.engine_unavailable")
    }
}

impl From<boltget_engine::Error> for UiError {
    fn from(e: boltget_engine::Error) -> Self {
        Self::from(&e)
    }
}

impl From<&boltget_engine::Error> for UiError {
    fn from(e: &boltget_engine::Error) -> Self {
        Self::with_message(e.message_key(), e.to_string())
    }
}
