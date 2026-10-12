//! `SetThreadExecutionState`, behind a trait so keep-awake is testable.
//!
//! Why: the call is **per thread** and only blocks idle sleep while that
//! thread lives, so the keep-awake engine owns one worker thread per session
//! and calls [`ExecutionState::apply`] only from it. The trait is what the
//! engine holds; unit tests give it [`RecordingExecutionState`], which records
//! each call together with the thread it came from, and never reaches the OS.
//!
//! `SystemExecutionState` is crate-private like the other real
//! implementations: it is only reachable through
//! `platform::real_execution_state`, and refuses until `lib.rs::run()` has
//! armed real OS effects. The call itself is harmless (nothing is persisted,
//! the flag dies with the thread); the guard is there so a test run never
//! shows up in `powercfg /requests`.

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

#[cfg(windows)]
use crate::core::error::CoreError;
use crate::core::error::Result;

/// What a session asks the system to stay awake for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwakeRequest {
    /// `ES_CONTINUOUS | ES_SYSTEM_REQUIRED`, plus `ES_DISPLAY_REQUIRED` when
    /// `display` is set.
    Hold {
        /// Keep the screen on too.
        display: bool,
    },
    /// `ES_CONTINUOUS` alone: clears whatever this thread asked for.
    Clear,
}

/// Applies an [`AwakeRequest`] **to the calling thread**.
pub trait ExecutionState: Send + Sync {
    /// Apply `request` to the thread this is called from.
    ///
    /// # Errors
    /// Returns [`CoreError::PowerAction`](crate::core::error::CoreError::PowerAction)
    /// when the OS refuses, and `PowerActionBlocked` from a context that must
    /// never reach the OS.
    fn apply(&self, request: AwakeRequest) -> Result<()>;
}

/// One recorded call: what was asked and from which thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCall {
    /// The request.
    pub request: AwakeRequest,
    /// The thread that made it.
    pub thread: ThreadId,
}

/// Records calls instead of making them. The only implementation any test, and
/// the `--dry-run` build, ever sees.
#[derive(Debug, Clone, Default)]
pub struct RecordingExecutionState {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl RecordingExecutionState {
    /// A recorder with an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every call so far, oldest first.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The requests only, oldest first.
    pub fn requests(&self) -> Vec<AwakeRequest> {
        self.calls().into_iter().map(|c| c.request).collect()
    }
}

impl ExecutionState for RecordingExecutionState {
    fn apply(&self, request: AwakeRequest) -> Result<()> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(RecordedCall {
                request,
                thread: std::thread::current().id(),
            });
        Ok(())
    }
}

/// The real thing, Windows only.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemExecutionState;

#[cfg(windows)]
impl ExecutionState for SystemExecutionState {
    fn apply(&self, request: AwakeRequest) -> Result<()> {
        use windows::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
        };

        if !crate::core::platform::real_os_effects_armed() {
            return Err(CoreError::PowerActionBlocked("keep-awake"));
        }

        let flags = match request {
            AwakeRequest::Clear => ES_CONTINUOUS,
            AwakeRequest::Hold { display: false } => ES_CONTINUOUS | ES_SYSTEM_REQUIRED,
            AwakeRequest::Hold { display: true } => {
                ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
            }
        };
        // SAFETY: the function takes a flag word and no pointers. It affects
        // only the calling thread, which is the engine's worker by contract.
        let previous = unsafe { SetThreadExecutionState(flags) };
        if previous.0 == 0 {
            return Err(CoreError::PowerAction(
                "SetThreadExecutionState was refused".to_owned(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recorder_notes_request_and_thread() {
        let rec = RecordingExecutionState::new();
        rec.apply(AwakeRequest::Hold { display: true }).expect("ok");
        rec.apply(AwakeRequest::Clear).expect("ok");
        let calls = rec.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].thread, std::thread::current().id());
        assert_eq!(
            rec.requests(),
            vec![AwakeRequest::Hold { display: true }, AwakeRequest::Clear]
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_system_implementation_refuses_inside_tests() {
        let err = SystemExecutionState
            .apply(AwakeRequest::Hold { display: false })
            .expect_err("a test must never reach the OS");
        assert!(matches!(err, CoreError::PowerActionBlocked(_)));
    }
}
