//! Keep-awake: "do not sleep for the next N minutes", nothing persisted.
//!
//! Why a worker thread: `SetThreadExecutionState` is per thread, so a session
//! owns one dedicated thread that sets the flag, waits for the deadline or a
//! stop message, clears the flag and exits. The call is never made from a
//! command thread or the async runtime. The OS call sits behind
//! [`ExecutionState`], so every test runs against a recording fake.
//!
//! The countdown follows the wall-clock deadline (re-checked at least every
//! [`WAKE_INTERVAL`]), not tick counting. Closing the app ends the session:
//! [`Drop`] stops it, and the OS drops the flag with the thread anyway.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::platform::{AwakeRequest, ExecutionState, RecordingExecutionState};

/// Longest session. There is deliberately no "forever" mode.
pub const MAX_MINUTES: u32 = 24 * 60;

/// How often the worker re-reads the wall clock while waiting.
const WAKE_INTERVAL: StdDuration = StdDuration::from_secs(30);

/// Called after every start, stop and expiry so the UI can refresh.
pub type ChangeNotifier = Arc<dyn Fn() + Send + Sync>;

/// What the UI needs to draw the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepAwakeStatus {
    /// Whether a session is running.
    pub active: bool,
    /// When it ends; the UI computes the countdown from this.
    pub deadline: Option<DateTime<Utc>>,
    /// Whether the screen is kept on too.
    pub display: bool,
}

impl KeepAwakeStatus {
    const IDLE: Self = Self {
        active: false,
        deadline: None,
        display: false,
    };
}

/// Check a requested length and turn it into a duration.
///
/// # Errors
/// Returns [`CoreError::InvalidTimer`] for zero minutes or more than
/// [`MAX_MINUTES`].
pub fn validate_minutes(minutes: u32) -> Result<Duration> {
    if minutes == 0 {
        return Err(CoreError::InvalidTimer(
            "pick at least one minute".to_owned(),
        ));
    }
    if minutes > MAX_MINUTES {
        return Err(CoreError::InvalidTimer(format!(
            "that is more than {} hours; pick something shorter — there is no keep-awake-forever mode",
            MAX_MINUTES / 60
        )));
    }
    Ok(Duration::minutes(i64::from(minutes)))
}

struct Session {
    id: Uuid,
    deadline: DateTime<Utc>,
    display: bool,
    stop: Sender<()>,
    worker: JoinHandle<()>,
}

type Shared = Arc<Mutex<Option<Session>>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Option<Session>> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Owns the one keep-awake session the app can have.
pub struct KeepAwake {
    state: Arc<dyn ExecutionState>,
    notify: ChangeNotifier,
    session: Shared,
}

impl std::fmt::Debug for KeepAwake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeepAwake")
            .field("active", &lock(&self.session).is_some())
            .finish()
    }
}

impl Default for KeepAwake {
    /// The safe default: a recorder, so an accidental instance touches nothing.
    fn default() -> Self {
        Self::new(Arc::new(RecordingExecutionState::new()), Arc::new(|| {}))
    }
}

impl KeepAwake {
    /// An engine that applies requests through `state` and calls `notify` on
    /// every change.
    pub fn new(state: Arc<dyn ExecutionState>, notify: ChangeNotifier) -> Self {
        Self {
            state,
            notify,
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Start (or replace) a session of `minutes`.
    ///
    /// # Errors
    /// See [`validate_minutes`]; also when the OS refuses the request.
    pub fn start(&self, minutes: u32, display: bool, now: DateTime<Utc>) -> Result<KeepAwakeStatus> {
        let span = validate_minutes(minutes)?;
        self.start_until(now + span, display)
    }

    /// Start (or replace) a session ending at `deadline`. Callers validate the
    /// length; this exists so tests can use a deadline a few milliseconds out.
    fn start_until(&self, deadline: DateTime<Utc>, display: bool) -> Result<KeepAwakeStatus> {
        // The old session is fully stopped (flag cleared, thread joined) before
        // the new one begins, so a restart replaces the deadline cleanly.
        self.stop_quietly();

        let id = Uuid::new_v4();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let state = Arc::clone(&self.state);
        let shared = Arc::clone(&self.session);
        let notify = Arc::clone(&self.notify);

        let worker = std::thread::Builder::new()
            .name("keep-awake".to_owned())
            .spawn(move || {
                run_worker(
                    &*state, id, deadline, display, &stop_rx, &ready_tx, &shared, &notify,
                );
            })
            .map_err(|err| CoreError::PowerAction(format!("could not start the worker: {err}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = worker.join();
                return Err(err);
            }
            Err(_) => {
                let _ = worker.join();
                return Err(CoreError::PowerAction(
                    "the keep-awake worker stopped before it started".to_owned(),
                ));
            }
        }

        *lock(&self.session) = Some(Session {
            id,
            deadline,
            display,
            stop: stop_tx,
            worker,
        });
        (self.notify)();
        Ok(self.status())
    }

    /// Stop the session. Stopping when nothing runs is not an error.
    pub fn stop(&self) -> KeepAwakeStatus {
        if self.stop_quietly() {
            (self.notify)();
        }
        self.status()
    }

    /// Stop without notifying; returns whether a session was running.
    fn stop_quietly(&self) -> bool {
        // Take the session out and release the lock before joining: the worker
        // takes the same lock on expiry and must not wait on us.
        let taken = lock(&self.session).take();
        match taken {
            Some(session) => {
                let _ = session.stop.send(());
                let _ = session.worker.join();
                true
            }
            None => false,
        }
    }

    /// The current state.
    pub fn status(&self) -> KeepAwakeStatus {
        match &*lock(&self.session) {
            Some(s) => KeepAwakeStatus {
                active: true,
                deadline: Some(s.deadline),
                display: s.display,
            },
            None => KeepAwakeStatus::IDLE,
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        self.stop_quietly();
    }
}

/// The body of the dedicated thread. Every `SetThreadExecutionState` call of a
/// session happens here.
#[allow(clippy::too_many_arguments)]
fn run_worker(
    state: &dyn ExecutionState,
    id: Uuid,
    deadline: DateTime<Utc>,
    display: bool,
    stop_rx: &mpsc::Receiver<()>,
    ready_tx: &Sender<Result<()>>,
    shared: &Shared,
    notify: &ChangeNotifier,
) {
    let held = state.apply(AwakeRequest::Hold { display });
    let failed = held.is_err();
    let _ = ready_tx.send(held);
    if failed {
        return;
    }

    let expired = loop {
        let remaining = deadline - Utc::now();
        if remaining <= Duration::zero() {
            break true;
        }
        let wait = remaining.to_std().unwrap_or(WAKE_INTERVAL).min(WAKE_INTERVAL);
        match stop_rx.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break false,
            Err(RecvTimeoutError::Timeout) => {}
        }
    };

    if let Err(err) = state.apply(AwakeRequest::Clear) {
        tracing::warn!(error = %err, "keep-awake: clearing the execution state failed");
    }

    if expired {
        // Only report expiry when we are still the current session; a stop that
        // got there first has already taken it out and will notify itself.
        let was_current = {
            let mut guard = lock(shared);
            if guard.as_ref().is_some_and(|s| s.id == id) {
                *guard = None;
                true
            } else {
                false
            }
        };
        if was_current {
            notify();
        }
    }
}

/// Build the engine the app manages: the real OS call, or a recorder under
/// `--dry-run`, and an event emitted on every change.
pub fn build<R: tauri::Runtime>(app: &tauri::AppHandle<R>, dry_run: bool) -> KeepAwake {
    let state: Arc<dyn ExecutionState> = if dry_run {
        Arc::new(RecordingExecutionState::new())
    } else {
        crate::core::platform::real_execution_state()
    };
    let handle = app.clone();
    KeepAwake::new(
        state,
        Arc::new(move || {
            if let Err(err) = crate::core::events::emit_keep_awake_changed(&handle) {
                tracing::debug!(error = %err, "keep-awake event was not delivered");
            }
        }),
    )
}

/// Registry entry. No scan, no execute: the page drives its own commands.
#[derive(Debug, Default)]
pub struct KeepAwakeModule;

impl ToolModule for KeepAwakeModule {
    fn id(&self) -> &'static str {
        "keep-awake"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Keep Awake".to_owned(),
            description:
                "Stop the PC from sleeping for a set time, with a countdown and a stop button."
                    .to_owned(),
            icon: "coffee".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn engine() -> (KeepAwake, RecordingExecutionState, Arc<AtomicUsize>) {
        let rec = RecordingExecutionState::new();
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let engine = KeepAwake::new(
            Arc::new(rec.clone()),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        (engine, rec, count)
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            std::thread::sleep(StdDuration::from_millis(10));
        }
        panic!("condition not reached in 2s");
    }

    #[test]
    fn minutes_are_validated() {
        assert!(validate_minutes(0).is_err());
        assert!(validate_minutes(1).is_ok());
        assert!(validate_minutes(MAX_MINUTES).is_ok());
        let err = validate_minutes(MAX_MINUTES + 1).expect_err("over 24h");
        assert!(err.to_string().contains("24 hours"));
        assert!(validate_minutes(u32::MAX).is_err());
    }

    #[test]
    fn invalid_requests_never_reach_the_os() {
        let (engine, rec, _) = engine();
        assert!(engine.start(0, false, Utc::now()).is_err());
        assert!(engine.start(MAX_MINUTES + 1, false, Utc::now()).is_err());
        assert!(rec.calls().is_empty());
        assert!(!engine.status().active);
    }

    #[test]
    fn start_then_stop_sets_and_clears_the_flag_on_one_worker_thread() {
        let (engine, rec, notified) = engine();
        let status = engine.start(30, false, Utc::now()).expect("start");
        assert!(status.active);
        assert!(!status.display);
        assert!(status.deadline.is_some());

        let after = engine.stop();
        assert!(!after.active);

        let calls = rec.calls();
        assert_eq!(
            rec.requests(),
            vec![AwakeRequest::Hold { display: false }, AwakeRequest::Clear]
        );
        assert_eq!(calls[0].thread, calls[1].thread, "one thread per session");
        assert_ne!(
            calls[0].thread,
            std::thread::current().id(),
            "never the caller's thread"
        );
        assert_eq!(notified.load(Ordering::SeqCst), 2, "start + stop");
    }

    #[test]
    fn the_display_option_adds_the_display_flag() {
        let (engine, rec, _) = engine();
        engine.start(5, true, Utc::now()).expect("start");
        assert!(engine.status().display);
        engine.stop();
        assert_eq!(rec.requests()[0], AwakeRequest::Hold { display: true });
    }

    #[test]
    fn expiry_clears_the_flag_and_notifies() {
        let (engine, rec, notified) = engine();
        engine
            .start_until(Utc::now() + Duration::milliseconds(80), false)
            .expect("start");
        wait_until(|| !engine.status().active);
        wait_until(|| rec.requests().len() == 2);
        assert_eq!(rec.requests()[1], AwakeRequest::Clear);
        assert_eq!(notified.load(Ordering::SeqCst), 2, "start + expiry");
        // A later stop is a harmless no-op and does not double-clear.
        engine.stop();
        assert_eq!(rec.requests().len(), 2);
    }

    #[test]
    fn restarting_replaces_the_deadline_and_uses_a_fresh_thread() {
        let (engine, rec, _) = engine();
        let now = Utc::now();
        let first = engine.start(30, false, now).expect("first");
        let second = engine.start(120, true, now).expect("second");
        assert!(second.deadline > first.deadline);
        assert!(second.display);

        assert_eq!(
            rec.requests(),
            vec![
                AwakeRequest::Hold { display: false },
                AwakeRequest::Clear,
                AwakeRequest::Hold { display: true },
            ]
        );
        let calls = rec.calls();
        assert_ne!(calls[0].thread, calls[2].thread);

        engine.stop();
        assert_eq!(rec.requests().len(), 4);
    }

    #[test]
    fn dropping_the_engine_clears_the_flag() {
        let (engine, rec, _) = engine();
        engine.start(30, false, Utc::now()).expect("start");
        drop(engine);
        assert_eq!(rec.requests().last(), Some(&AwakeRequest::Clear));
    }

    #[test]
    fn stopping_an_idle_engine_is_quiet() {
        let (engine, rec, notified) = engine();
        assert!(!engine.stop().active);
        assert!(rec.calls().is_empty());
        assert_eq!(notified.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn status_wire_format_is_camel_case() {
        let json = serde_json::to_value(KeepAwakeStatus::IDLE).expect("json");
        assert_eq!(
            json,
            serde_json::json!({ "active": false, "deadline": null, "display": false })
        );
    }

    #[test]
    fn the_manifest_is_complete() {
        let m = KeepAwakeModule.manifest();
        assert_eq!(m.id, "keep-awake");
        assert!(!m.capabilities.has_scan);
    }
}
