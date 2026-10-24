//! # boltget-engine
//!
//! The BoltGet download engine. The behaviour is specified in `docs/ENGINE.md`;
//! deviations are recorded there.
#![warn(missing_docs)]

mod backoff;
mod config;
mod error;
mod filename;
mod headers;
mod platform;
mod request;

pub use config::EngineConfig;
pub use error::{Error, ErrorKind, Result};
pub use headers::Validators;
pub use request::{RequestContext, redact_url};
pub mod segment;
mod writer;

pub use platform::{Preallocation, is_sparse};
