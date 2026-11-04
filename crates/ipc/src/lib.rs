//! Boltget's local IPC: how a client (the `boltget` CLI today, the browser's
//! native messaging host later) reaches the process that owns the engine and
//! its database (the desktop app).
//!
//! The protocol is specified in `docs/ARCHITECTURE.md`, "CLI to app (named
//! pipe)". In short:
//!
//! - **Transport.** A Windows named pipe, `\\.\pipe\boltget-<user SID>-<dir>`,
//!   whose ACL admits only the current user, that refuses remote clients, and
//!   whose two ends check that the other is in the same logon session. `<dir>`
//!   is a hash of the data directory, so a client reaches the owner of the
//!   database it would otherwise open itself. Elsewhere, a Unix domain socket
//!   with `0600` permissions ([`Endpoint`]).
//! - **Framing.** Each message is a 4-byte little-endian length, then that
//!   many bytes of UTF-8 JSON; at most 1 MiB ([`frame`]).
//! - **Messages.** The client opens with [`Request::Hello`] and its protocol
//!   version; the owner answers [`Response::Hello`] or a
//!   [`Response::Error`] with the key `error.protocol_version`. Then one
//!   request, one response, in turn; [`Request::Watch`] instead streams a
//!   download's events and progress until it stops ([`proto`]).
//!
//! The owner side is [`Server`]; the client side is [`Client`].

mod client;
mod endpoint;
pub mod frame;
#[cfg(windows)]
mod platform;
pub mod proto;
mod server;
#[cfg(test)]
mod tests;
mod transport;

use std::io;

pub use client::Client;
pub use endpoint::Endpoint;
pub use proto::{
    AddSpec, Finished, OLDEST_SUPPORTED, PROTOCOL_VERSION, ProgressReport, RefreshSpec, Request,
    RequestSpec, Response, ResumeSpec, WireError,
};
pub use server::{Owner, Server};

/// Why talking over the pipe failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The pipe or socket failed.
    #[error("{0}")]
    Io(#[from] io::Error),
    /// A frame announced more than [`frame::MAX_FRAME`] bytes.
    #[error("a message of {0} bytes exceeds the 1 MiB limit")]
    TooLarge(u32),
    /// A frame did not hold a message of this protocol.
    #[error("malformed message: {0}")]
    Json(#[from] serde_json::Error),
    /// The other end closed the connection.
    #[error("the connection was closed")]
    Closed,
    /// A message arrived that the protocol does not allow here.
    #[error("unexpected message: {0}")]
    Protocol(String),
    /// The two ends speak different protocol versions. The text says which.
    #[error("{0}")]
    Version(String),
    /// Another process already serves this endpoint.
    #[error("another process already serves {0}")]
    InUse(String),
}
