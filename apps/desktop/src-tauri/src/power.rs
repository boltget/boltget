//! What a finished queue may ask for: putting the computer to sleep or
//! shutting it down (docs/APP-DESIGN.md, "Queues as built"). The bridge
//! runs the 60-second countdown; this module only acts at its end.
//!
//! The system call sits behind [`Power`] so tests (and development builds)
//! can never sleep or shut down the machine: debug builds use [`LogOnly`],
//! which logs what it would do. Only release builds use [`System`].

use std::sync::Arc;

use crate::bridge::dto::FinishAction;

/// Carries out a finish action.
pub(crate) trait Power: Send + Sync + std::fmt::Debug + 'static {
    /// Sleeps or shuts down now (never called with
    /// [`FinishAction::Nothing`]).
    fn act(&self, action: FinishAction);
}

/// Logs the action and does nothing else: development builds and tests.
#[derive(Debug, Default)]
pub(crate) struct LogOnly;

impl Power for LogOnly {
    fn act(&self, action: FinishAction) {
        tracing::warn!(
            ?action,
            "a queue finished; a release build would do this now (development builds never do)"
        );
    }
}

/// Windows' own sleep and shut down.
#[derive(Debug, Default)]
pub(crate) struct System;

impl Power for System {
    fn act(&self, action: FinishAction) {
        let result = match action {
            FinishAction::Nothing => Ok(()),
            FinishAction::Sleep => crate::platform::sleep(),
            FinishAction::ShutDown => crate::platform::shut_down(),
        };
        if let Err(e) = result {
            tracing::error!(?action, error = %e, "could not carry out the finish action");
        }
    }
}

/// The power control for this build: the real one only in release builds.
pub(crate) fn for_this_build() -> Arc<dyn Power> {
    if cfg!(debug_assertions) {
        Arc::new(LogOnly)
    } else {
        Arc::new(System)
    }
}
