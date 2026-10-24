//! # boltget-engine
//!
//! The BoltGet download engine: probing without `HEAD`, dynamic segmentation
//! ("split the largest remaining segment"), one explicit HTTP connection per
//! worker, sparse preallocation, positional writes from a dedicated writer
//! thread, retries with backoff and stall detection.
//!
//! The behaviour is specified in `docs/ENGINE.md`; deviations are recorded there.
//!
//! ```no_run
//! use boltget_engine::{AddRequest, Destination, Engine, EngineConfig, RequestContext};
//!
//! # async fn demo() -> Result<(), boltget_engine::Error> {
//! let engine = Engine::new(EngineConfig::default())?;
//! let ctx = RequestContext::new("https://example.com/file.iso")?;
//! let handle = engine.add(AddRequest::new(ctx, Destination::Directory(".".into())));
//! let done = handle.wait().await?;
//! println!("saved {} bytes to {}", done.bytes, done.path.display());
//! # Ok(())
//! # }
//! ```
#![warn(missing_docs)]

mod backoff;
mod config;
mod download;
mod engine;
mod error;
mod events;
mod filename;
mod headers;
mod http;
mod platform;
mod probe;
mod request;
pub mod segment;
mod writer;

pub use config::EngineConfig;
pub use engine::{AddRequest, ConflictPolicy, Destination, DownloadHandle, Engine};
pub use error::{Error, ErrorKind, Result};
pub use events::{Completed, DownloadId, DownloadInfo, EngineEvent, ErrorInfo, Progress, State};
pub use headers::Validators;
pub use platform::{Preallocation, is_sparse};
pub use probe::{ProbeResult, RangeSupport};
pub use request::{RequestContext, redact_url};

/// Re-exported so callers can build headers for [`RequestContext::header`].
pub use ::http::{HeaderName, HeaderValue, header};

/// The suffix of in-progress files (`<name>.boltget`).
pub const PARTIAL_SUFFIX: &str = download::PARTIAL_SUFFIX;
