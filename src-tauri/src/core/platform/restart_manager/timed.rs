//! Bounded waits around the Restart Manager (issue #41, A12/A13).
//!
//! `RmGetList` can stall on a busy system and `RmShutdown` waits for the
//! program to answer, which a hung program never does. Each call runs on its
//! own thread; past the timeout the caller gets [`RmError::TimedOut`] and the
//! thread is left to finish on its own (its session still ends itself).
//! Nothing is forced: the close stays the same polite request.

use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use super::{CloseOutcome, LockInfo, RestartManager, RmError, UniqueProcess};

/// Longest wait for "which programs use these files".
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest wait for a program to answer a polite close request.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(60);

/// A [`RestartManager`] whose queries and close requests give up after a
/// bounded wait.
pub struct TimedRestartManager {
    inner: Arc<dyn RestartManager>,
    query_timeout: Duration,
    close_timeout: Duration,
}

impl TimedRestartManager {
    /// Wrap `inner` with the given limits.
    pub fn new(
        inner: Arc<dyn RestartManager>,
        query_timeout: Duration,
        close_timeout: Duration,
    ) -> Self {
        Self {
            inner,
            query_timeout,
            close_timeout,
        }
    }

    // shortcut: a timed-out call keeps its thread until Windows answers; a
    // user retrying a stuck query adds one thread per retry. Fine at human pace.
    fn bounded<T: Send + 'static>(
        &self,
        call: &'static str,
        timeout: Duration,
        work: impl FnOnce(&dyn RestartManager) -> Result<T, RmError> + Send + 'static,
    ) -> Result<T, RmError> {
        let inner = Arc::clone(&self.inner);
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            // The receiver is gone after a timeout; nobody is left to tell.
            let _ = tx.send(work(inner.as_ref()));
        });
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                tracing::warn!(
                    call,
                    seconds = timeout.as_secs(),
                    "restart manager call timed out"
                );
                Err(RmError::TimedOut {
                    call,
                    seconds: timeout.as_secs(),
                })
            }
            // The sender dropped without sending: the worker panicked. Hand the
            // panic on, as a call on this thread would have.
            Err(mpsc::RecvTimeoutError::Disconnected) => match worker.join() {
                Err(panic) => std::panic::resume_unwind(panic),
                Ok(()) => unreachable!("the worker always sends before it returns"),
            },
        }
    }
}

impl RestartManager for TimedRestartManager {
    fn locking_processes(&self, paths: &[PathBuf]) -> Result<Vec<LockInfo>, RmError> {
        let paths = paths.to_vec();
        self.bounded("RmGetList", self.query_timeout, move |rm| {
            rm.locking_processes(&paths)
        })
    }

    fn request_close(&self, process: UniqueProcess) -> Result<CloseOutcome, RmError> {
        self.bounded("RmShutdown", self.close_timeout, move |rm| {
            rm.request_close(process)
        })
    }

    fn exe_path(&self, pid: u32) -> Option<PathBuf> {
        self.inner.exe_path(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(50);
    const STALL: Duration = Duration::from_secs(5);

    /// Answers instantly, or stalls / panics on demand.
    struct Scripted {
        stall: bool,
        panic: bool,
    }

    impl Scripted {
        fn wait(&self) {
            if self.panic {
                panic!("restart manager blew up");
            }
            if self.stall {
                std::thread::sleep(STALL);
            }
        }
    }

    impl RestartManager for Scripted {
        fn locking_processes(&self, paths: &[PathBuf]) -> Result<Vec<LockInfo>, RmError> {
            self.wait();
            assert_eq!(paths, [PathBuf::from("a.txt")]);
            Ok(Vec::new())
        }
        fn request_close(&self, _process: UniqueProcess) -> Result<CloseOutcome, RmError> {
            self.wait();
            Ok(CloseOutcome::Closed)
        }
        fn exe_path(&self, pid: u32) -> Option<PathBuf> {
            Some(PathBuf::from(format!("{pid}.exe")))
        }
    }

    fn timed(stall: bool, panic: bool) -> TimedRestartManager {
        TimedRestartManager::new(Arc::new(Scripted { stall, panic }), SHORT, SHORT)
    }

    const PROCESS: UniqueProcess = UniqueProcess {
        pid: 42,
        start_time: 7,
    };

    #[test]
    fn a_prompt_answer_passes_straight_through() {
        let rm = timed(false, false);
        assert!(rm
            .locking_processes(&[PathBuf::from("a.txt")])
            .expect("query")
            .is_empty());
        assert_eq!(
            rm.request_close(PROCESS).expect("close"),
            CloseOutcome::Closed
        );
        assert_eq!(rm.exe_path(9), Some(PathBuf::from("9.exe")));
    }

    #[test]
    fn a_stalled_query_or_close_gives_up_after_the_timeout() {
        let rm = timed(true, false);
        let started = std::time::Instant::now();
        assert!(matches!(
            rm.locking_processes(&[PathBuf::from("a.txt")]),
            Err(RmError::TimedOut {
                call: "RmGetList",
                ..
            })
        ));
        assert!(matches!(
            rm.request_close(PROCESS),
            Err(RmError::TimedOut {
                call: "RmShutdown",
                ..
            })
        ));
        assert!(
            started.elapsed() < STALL,
            "the caller must not wait for the stall"
        );
    }

    #[test]
    fn a_panic_in_the_worker_reaches_the_caller() {
        // A timeout CI cannot hit: a slow panic (backtrace capture) must
        // still surface as the panic, not as TimedOut.
        let rm = TimedRestartManager::new(
            Arc::new(Scripted {
                stall: false,
                panic: true,
            }),
            STALL,
            STALL,
        );
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rm.locking_processes(&[PathBuf::from("a.txt")])
        }));
        assert!(caught.is_err());
    }

    #[test]
    fn the_limits_are_long_enough_for_a_human_answer() {
        assert!(QUERY_TIMEOUT >= Duration::from_secs(10));
        assert!(CLOSE_TIMEOUT >= QUERY_TIMEOUT);
    }
}
