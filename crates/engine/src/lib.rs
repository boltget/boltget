//! # boltget-engine
//!
//! The BoltGet download engine: probing without `HEAD`, dynamic segmentation
//! ("split the largest remaining segment"), one explicit HTTP connection per
//! worker, sparse preallocation, positional writes from a dedicated writer
//! thread, retries with backoff and stall detection.
//!
//! [`Engine::open`] keeps downloads in a SQLite database owned by one process at
//! a time: progress is checkpointed only after the bytes are durable on disk, and
//! [`Engine::resume`] continues a paused or interrupted download from its last
//! checkpoint. [`Engine::new`] is the in-memory variant.
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
mod integrity;
mod platform;
mod probe;
mod ratelimit;
mod refresh;
mod request;
pub mod segment;
mod store;
mod writer;

pub use config::{EngineConfig, Settings};
pub use engine::{
    AddRequest, ConflictPolicy, Destination, DownloadHandle, Engine, LimitScope, Removed,
    ResumeOptions,
};
pub use error::{Error, ErrorKind, Result};
pub use events::{Completed, DownloadId, DownloadInfo, EngineEvent, ErrorInfo, Progress, State};
pub use headers::Validators;
pub use integrity::{Checksum, ChecksumSource, HashAlgo};
pub use platform::{Preallocation, is_sparse};
pub use probe::{ProbeResult, RangeSupport};
pub use ratelimit::parse_rate;
pub use refresh::{RefreshEvidence, RefreshOutcome};
pub use request::{RequestContext, redact_url};
pub use store::{
    ChecksumStatus, DB_FILE, DownloadSummary, LOCK_FILE, SavedError, default_data_dir,
};

/// Re-exported so callers can build headers for [`RequestContext::header`].
pub use ::http::{HeaderName, HeaderValue, header};

/// The suffix of in-progress files (`<name>.boltget`).
pub const PARTIAL_SUFFIX: &str = download::PARTIAL_SUFFIX;
