//! Typed Rust -> UI events.
//!
//! Why: event names and payload shapes are a contract with `src/lib/api.ts`.
//! Keeping them here means no route ever guesses a string, and the throttle
//! that keeps progress at 10 events/sec lives in one place
//! (ARCHITECTURE section 3).

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};

/// A scan reported progress.
pub const SCAN_PROGRESS: &str = "scan:progress";
/// A scan finished.
pub const SCAN_DONE: &str = "scan:done";
/// An execute batch reported progress.
pub const EXEC_PROGRESS: &str = "exec:progress";
/// The quarantine contents changed (item added, restored or purged).
pub const QUARANTINE_CHANGED: &str = "quarantine:changed";
/// A keep-awake session started, stopped or expired. No payload: the UI
/// re-reads `keep_awake_status`.
pub const KEEP_AWAKE_CHANGED: &str = "keep-awake:changed";

/// Upper bound on progress events per second, per ARCHITECTURE section 3.
pub const MAX_PROGRESS_EVENTS_PER_SEC: u32 = 10;

/// Minimum wall-clock gap between two progress events.
pub const MIN_PROGRESS_INTERVAL: Duration =
    Duration::from_nanos(1_000_000_000 / MAX_PROGRESS_EVENTS_PER_SEC as u64);

/// Payload of [`SCAN_PROGRESS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    /// Scan this belongs to.
    pub scan_id: Uuid,
    /// Module doing the scanning.
    pub module: String,
    /// Items walked so far.
    pub items_scanned: u64,
    /// Path currently being looked at, for the progress toast.
    pub current_path: Option<String>,
    /// Completion percentage when it can be estimated at all.
    pub pct: Option<u8>,
}

/// Payload of [`SCAN_DONE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanDone {
    /// Scan that finished.
    pub scan_id: Uuid,
    /// Module that ran it.
    pub module: String,
    /// How many findings it produced.
    pub finding_count: u64,
}

/// Payload of [`EXEC_PROGRESS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecProgress {
    /// Scan whose recommendations are being applied.
    pub scan_id: Uuid,
    /// Module doing the work.
    pub module: String,
    /// Items processed so far.
    pub items_done: u64,
    /// Items in the batch.
    pub items_total: u64,
    /// Path currently being acted on.
    pub current_path: Option<String>,
}

/// Payload of [`QUARANTINE_CHANGED`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineChanged {
    /// Batch that changed, when the change was batch-scoped.
    pub batch_id: Option<Uuid>,
    /// Items currently held in quarantine.
    pub item_count: u64,
    /// Bytes currently held in quarantine.
    pub total_bytes: u64,
}

/// Emit a typed payload under a known event name.
///
/// # Errors
/// Returns [`CoreError::Emit`] when the WebView could not be reached.
fn emit<R: Runtime, T: Serialize + Clone>(
    app: &AppHandle<R>,
    event: &'static str,
    payload: &T,
) -> Result<()> {
    app.emit(event, payload.clone())
        .map_err(|source| CoreError::Emit { event, source })
}

/// Emit [`SCAN_PROGRESS`]. Callers should gate this with a [`ProgressThrottle`].
///
/// # Errors
/// See [`CoreError::Emit`].
pub fn emit_scan_progress<R: Runtime>(app: &AppHandle<R>, payload: &ScanProgress) -> Result<()> {
    emit(app, SCAN_PROGRESS, payload)
}

/// Emit [`SCAN_DONE`].
///
/// # Errors
/// See [`CoreError::Emit`].
pub fn emit_scan_done<R: Runtime>(app: &AppHandle<R>, payload: &ScanDone) -> Result<()> {
    emit(app, SCAN_DONE, payload)
}

/// Emit [`EXEC_PROGRESS`]. Callers should gate this with a [`ProgressThrottle`].
///
/// # Errors
/// See [`CoreError::Emit`].
pub fn emit_exec_progress<R: Runtime>(app: &AppHandle<R>, payload: &ExecProgress) -> Result<()> {
    emit(app, EXEC_PROGRESS, payload)
}

/// Emit [`QUARANTINE_CHANGED`].
///
/// # Errors
/// See [`CoreError::Emit`].
pub fn emit_quarantine_changed<R: Runtime>(
    app: &AppHandle<R>,
    payload: &QuarantineChanged,
) -> Result<()> {
    emit(app, QUARANTINE_CHANGED, payload)
}

/// Emit [`KEEP_AWAKE_CHANGED`].
///
/// # Errors
/// See [`CoreError::Emit`].
pub fn emit_keep_awake_changed<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    emit(app, KEEP_AWAKE_CHANGED, &())
}

/// Rate limiter for progress events.
///
/// A scan walking a hundred thousand files would otherwise flood the WebView
/// and make the UI slower than the scan. The first call always passes so the
/// user sees immediate feedback.
#[derive(Debug, Clone)]
pub struct ProgressThrottle {
    interval: Duration,
    last_emitted: Option<Instant>,
}

impl ProgressThrottle {
    /// Throttle at [`MAX_PROGRESS_EVENTS_PER_SEC`].
    pub fn new() -> Self {
        Self::with_interval(MIN_PROGRESS_INTERVAL)
    }

    /// Throttle at a custom minimum interval.
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            interval,
            last_emitted: None,
        }
    }

    /// Whether an event may be emitted now. Records the decision.
    pub fn should_emit(&mut self) -> bool {
        self.should_emit_at(Instant::now())
    }

    /// Clock-injectable variant of [`ProgressThrottle::should_emit`].
    pub fn should_emit_at(&mut self, now: Instant) -> bool {
        let allowed = match self.last_emitted {
            None => true,
            Some(last) => now.duration_since(last) >= self.interval,
        };
        if allowed {
            self.last_emitted = Some(now);
        }
        allowed
    }

    /// Forget the last emission, so the next call passes.
    pub fn reset(&mut self) {
        self.last_emitted = None;
    }
}

impl Default for ProgressThrottle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_follow_the_scope_verb_convention() {
        for name in [SCAN_PROGRESS, SCAN_DONE, EXEC_PROGRESS, QUARANTINE_CHANGED] {
            let (scope, verb) = name.split_once(':').expect("event name needs a colon");
            assert!(!scope.is_empty() && !verb.is_empty(), "bad name {name}");
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == ':'),
                "bad name {name}"
            );
        }
    }

    #[test]
    fn scan_progress_uses_camel_case_field_names() {
        let json = serde_json::to_value(ScanProgress {
            scan_id: Uuid::nil(),
            module: "dummy".to_owned(),
            items_scanned: 7,
            current_path: None,
            pct: Some(50),
        })
        .expect("serialize");
        assert!(json.get("itemsScanned").is_some(), "got {json}");
        assert!(json.get("scanId").is_some(), "got {json}");
    }

    #[test]
    fn throttle_interval_matches_ten_per_second() {
        assert_eq!(MIN_PROGRESS_INTERVAL, Duration::from_millis(100));
    }

    #[test]
    fn throttle_lets_the_first_event_through() {
        let mut throttle = ProgressThrottle::new();
        assert!(throttle.should_emit_at(Instant::now()));
    }

    #[test]
    fn throttle_blocks_events_inside_the_interval() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::with_interval(Duration::from_millis(100));

        assert!(throttle.should_emit_at(start));
        assert!(!throttle.should_emit_at(start + Duration::from_millis(50)));
        assert!(!throttle.should_emit_at(start + Duration::from_millis(99)));
        assert!(throttle.should_emit_at(start + Duration::from_millis(100)));
        assert!(!throttle.should_emit_at(start + Duration::from_millis(150)));
        assert!(throttle.should_emit_at(start + Duration::from_millis(200)));
    }

    #[test]
    fn throttle_caps_a_burst_at_ten_per_second() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::new();
        // One tick every millisecond for a second: 1000 candidates.
        let emitted = (0..1_000)
            .filter(|ms| throttle.should_emit_at(start + Duration::from_millis(*ms)))
            .count();
        assert_eq!(emitted, MAX_PROGRESS_EVENTS_PER_SEC as usize);
    }

    #[test]
    fn throttle_reset_lets_the_next_event_through() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::new();
        assert!(throttle.should_emit_at(start));
        throttle.reset();
        assert!(throttle.should_emit_at(start + Duration::from_millis(1)));
    }
}
