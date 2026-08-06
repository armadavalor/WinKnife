//! `#[tauri::command]` entry points.
//!
//! Deliberately thin: no filesystem, no registry, no SQL here — commands only
//! translate between IPC and `core`/`modules` and map errors to [`ApiError`]
//! (CONVENTIONS.md). The one thing they do own is the *composition*: which
//! quarantine, which brain, which recycle-bin wrapper a run uses. That is the
//! seam that keeps the real Win32 calls out of every test.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use tauri::{AppHandle, Runtime, State};
use uuid::Uuid;

use crate::core::brain::Brain;
use crate::core::db::Database;
use crate::core::error::{ApiError, CoreError};
use crate::core::events::{
    self, ExecProgress, ProgressThrottle, QuarantineChanged, ScanDone, ScanProgress,
};
use crate::core::executor::{self, ExecEnvironment};
use crate::core::module::{ModuleManifest, ModuleRegistry};
use crate::core::platform::{PowerAction, RecycleBin};
use crate::core::quarantine::{
    PurgeReport, Quarantine, QuarantineBatch, QuarantineItem, QuarantineStats, RestoreOutcome,
};
use crate::core::scanner::CancellationToken;
use crate::core::state::{ScanResult, ScanStore};
use crate::core::types::ExecReport;
use crate::modules::cleaner::{self, CleanerContext, CleanerSummary};
use crate::modules::shutdown_timer::{ScheduleMode, ShutdownTimer, TimerStatus};

/// Long-lived services the commands compose, registered as Tauri state.
///
/// Bundling them means a command signature stays readable and, more usefully,
/// that a test can build the whole set against a tempdir in three lines.
pub struct Services {
    /// Where removed items go.
    pub quarantine: Quarantine,
    /// Hard gates and rule packs.
    pub brain: Brain,
    /// The recycle bin, real on Windows and a stub everywhere else.
    pub recycle_bin: Arc<dyn RecycleBin>,
    /// Directory the rule packs were loaded from, for diagnostics.
    pub rules_dir: PathBuf,
}

/// Manifests of every registered module, in sidebar order.
///
/// # Errors
/// Currently infallible, but keeps the `Result` shape every command shares so
/// the UI's error handling does not change when it grows one.
#[tauri::command]
pub fn list_modules(registry: State<'_, ModuleRegistry>) -> Result<Vec<ModuleManifest>, ApiError> {
    Ok(registry.manifests())
}

/* -------------------------------------------------------------------------
 * Cleaner: scan -> review -> execute
 * ---------------------------------------------------------------------- */

/// Run a scan and return its findings and recommendations.
///
/// Blocking work; the caller runs it off the UI thread.
///
/// # Errors
/// Returns an error only if the scan could not start at all.
#[tauri::command]
pub async fn start_scan<R: Runtime>(
    app: AppHandle<R>,
    module_id: String,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
) -> Result<ScanResult, ApiError> {
    if module_id != "cleaner" {
        return Err(CoreError::UnknownModule(module_id).into());
    }

    let scan_id = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let mut throttle = ProgressThrottle::new();

    let output = {
        let mut emit = |items: u64, path: &std::path::Path| {
            if !throttle.should_emit() {
                return;
            }
            let payload = ScanProgress {
                scan_id,
                module: module_id.clone(),
                items_scanned: items,
                current_path: Some(path.display().to_string()),
                pct: None,
            };
            if let Err(err) = events::emit_scan_progress(&app, &payload) {
                tracing::debug!(error = %err, "scan progress event was not delivered");
            }
        };

        cleaner::scan(
            &CleanerContext {
                packs: services.brain.packs(),
                recycle_bin: services.recycle_bin.as_ref(),
                now: Utc::now(),
                cancel: &cancel,
            },
            Some(&mut emit),
        )
    };

    let recommendations = cleaner::analyze(&output.findings, &services.brain);
    let result = ScanResult {
        scan_id,
        module: module_id.clone(),
        findings: output.findings,
        recommendations,
        skipped_rules: output.skipped_rules,
    };

    tracing::info!(
        scan = %scan_id,
        findings = result.findings.len(),
        unreadable = output.unreadable.len(),
        "scan finished"
    );

    let done = ScanDone {
        scan_id,
        module: module_id,
        finding_count: result.findings.len() as u64,
    };
    let _ = events::emit_scan_done(&app, &done);

    store.insert(result.clone());
    Ok(result)
}

/// Fetch a scan the UI already ran.
///
/// # Errors
/// Returns [`CoreError::UnknownScan`] when the scan is no longer retained.
#[tauri::command]
pub fn get_recommendations(
    scan_id: Uuid,
    store: State<'_, ScanStore>,
) -> Result<ScanResult, ApiError> {
    Ok(store.get(scan_id)?)
}

/// Green/yellow/red totals for the header line after a scan.
///
/// # Errors
/// See [`get_recommendations`].
#[tauri::command]
pub fn scan_summary(
    scan_id: Uuid,
    store: State<'_, ScanStore>,
) -> Result<CleanerSummary, ApiError> {
    let scan = store.get(scan_id)?;
    Ok(cleaner::summarise(&scan.recommendations))
}

/// Apply the recommendations the user approved.
///
/// # Errors
/// Returns an error if the scan is unknown or the ledger is unusable. Item
/// failures land in the report.
#[tauri::command]
pub async fn execute<R: Runtime>(
    app: AppHandle<R>,
    scan_id: Uuid,
    approved: Vec<Uuid>,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
    db: State<'_, Database>,
) -> Result<ExecReport, ApiError> {
    let scan = store.get(scan_id)?;
    let units = executor::plan(&approved, &scan.recommendations, &scan.findings);

    let mut throttle = ProgressThrottle::new();
    let module = scan.module.clone();
    let report = db.with(|conn| {
        let env = ExecEnvironment {
            quarantine: &services.quarantine,
            conn,
            brain: &services.brain,
            recycle_bin: services.recycle_bin.as_ref(),
        };
        let mut emit = |done: u64, total: u64, finding: &crate::core::types::Finding| {
            // The final tick always goes out; the rest are throttled.
            if done != total && !throttle.should_emit() {
                return;
            }
            let payload = ExecProgress {
                scan_id,
                module: module.clone(),
                items_done: done,
                items_total: total,
                current_path: Some(finding.path.display().to_string()),
            };
            if let Err(err) = events::emit_exec_progress(&app, &payload) {
                tracing::debug!(error = %err, "exec progress event was not delivered");
            }
        };
        executor::execute(&units, &env, Some(&mut emit))
    })?;

    notify_quarantine_changed(&app, &services, &db, report.batch_id);
    Ok(report)
}

/* -------------------------------------------------------------------------
 * Quarantine drawer
 * ---------------------------------------------------------------------- */

/// Batches that still hold something, newest first.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
#[tauri::command]
pub fn quarantine_list(
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<Vec<QuarantineBatch>, ApiError> {
    Ok(db.with(|conn| services.quarantine.list_batches(conn))?)
}

/// Items of one batch, so the drawer can expand it.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
#[tauri::command]
pub fn quarantine_batch_items(
    batch_id: Uuid,
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<Vec<QuarantineItem>, ApiError> {
    Ok(db.with(|conn| services.quarantine.items_in_batch(conn, batch_id))?)
}

/// Totals across everything still held.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
#[tauri::command]
pub fn quarantine_stats(
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<QuarantineStats, ApiError> {
    Ok(db.with(|conn| services.quarantine.stats(conn))?)
}

/// What a restore did, per item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    /// Items that went back.
    pub restored: Vec<RestoreOutcome>,
    /// Items that did not, with the reason.
    pub failed: Vec<RestoreFailure>,
}

/// One item that could not be restored.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreFailure {
    /// Item id.
    pub item_id: Uuid,
    /// Why.
    pub error: String,
}

/// Restore a whole batch.
///
/// # Errors
/// Returns an error only if the ledger cannot be read; per-item failures are
/// in the report.
#[tauri::command]
pub fn quarantine_restore<R: Runtime>(
    app: AppHandle<R>,
    batch_id: Uuid,
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<RestoreReport, ApiError> {
    let (restored, failed) = db.with(|conn| services.quarantine.restore_batch(conn, batch_id))?;
    notify_quarantine_changed(&app, &services, &db, Some(batch_id));
    Ok(RestoreReport {
        restored,
        failed: failed
            .into_iter()
            .map(|(item_id, error)| RestoreFailure { item_id, error })
            .collect(),
    })
}

/// Restore one item.
///
/// # Errors
/// Returns [`CoreError::UnknownQuarantineItem`] for an unknown id.
#[tauri::command]
pub fn quarantine_restore_item<R: Runtime>(
    app: AppHandle<R>,
    item_id: Uuid,
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<RestoreOutcome, ApiError> {
    let outcome = db.with(|conn| services.quarantine.restore_item(conn, item_id))?;
    notify_quarantine_changed(&app, &services, &db, None);
    Ok(outcome)
}

/// Run the retention purge and the size cap now.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
#[tauri::command]
pub fn quarantine_purge<R: Runtime>(
    app: AppHandle<R>,
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<PurgeReport, ApiError> {
    let report = db.with(|conn| services.quarantine.purge(conn, Utc::now()))?;
    notify_quarantine_changed(&app, &services, &db, None);
    Ok(report)
}

fn notify_quarantine_changed<R: Runtime>(
    app: &AppHandle<R>,
    services: &Services,
    db: &Database,
    batch_id: Option<Uuid>,
) {
    let stats = db
        .with(|conn| services.quarantine.stats(conn))
        .unwrap_or_default();
    let payload = QuarantineChanged {
        batch_id,
        item_count: stats.item_count,
        total_bytes: stats.total_bytes,
    };
    if let Err(err) = events::emit_quarantine_changed(app, &payload) {
        tracing::debug!(error = %err, "quarantine event was not delivered");
    }
}

/* -------------------------------------------------------------------------
 * Shutdown timer
 * ---------------------------------------------------------------------- */

/// Arm a timer, replacing any existing one, and persist it.
///
/// # Errors
/// Returns [`CoreError::InvalidTimer`] for a target that is zero-length, over
/// 24 hours away, or not a real clock time.
#[tauri::command]
pub fn shutdown_schedule(
    mode: ScheduleMode,
    action: PowerAction,
    timer: State<'_, Arc<ShutdownTimer>>,
    db: State<'_, Database>,
) -> Result<TimerStatus, ApiError> {
    let now = Utc::now();
    let persisted = timer.schedule(mode, action, now)?;
    db.with(|conn| {
        crate::core::settings::set_json(conn, crate::core::settings::KEY_ACTIVE_TIMER, &persisted)
    })?;
    Ok(timer.status(now))
}

/// Cancel the timer. Cancelling when nothing is armed is not an error.
///
/// # Errors
/// Returns an error only if the setting could not be cleared.
#[tauri::command]
pub fn shutdown_cancel(
    timer: State<'_, Arc<ShutdownTimer>>,
    db: State<'_, Database>,
) -> Result<TimerStatus, ApiError> {
    timer.cancel();
    db.with(|conn| crate::core::settings::clear(conn, crate::core::settings::KEY_ACTIVE_TIMER))?;
    Ok(timer.status(Utc::now()))
}

/// Current timer state.
///
/// # Errors
/// Infallible today; keeps the shared `Result` shape.
#[tauri::command]
pub fn shutdown_status(timer: State<'_, Arc<ShutdownTimer>>) -> Result<TimerStatus, ApiError> {
    Ok(timer.status(Utc::now()))
}

#[cfg(test)]
mod tests {
    use crate::modules;

    #[test]
    fn list_modules_returns_the_registry_contents() {
        // The command body is `registry.manifests()`; `State` cannot be built
        // outside a running app, so the delegation target is tested directly.
        let registry = modules::registry().expect("registry");
        let manifests = registry.manifests();

        assert_eq!(manifests.len(), registry.len());
        assert!(manifests.iter().any(|m| m.id == "cleaner"));
        assert!(manifests.iter().any(|m| m.id == "shutdown-timer"));
    }

    #[test]
    fn manifests_serialise_for_the_webview() {
        let manifests = modules::registry().expect("registry").manifests();
        let json = serde_json::to_value(&manifests).expect("serialize");
        let first = json.get(0).expect("at least one module");

        assert!(first.get("id").is_some(), "got {json}");
        assert!(
            first
                .get("capabilities")
                .and_then(|c| c.get("hasScan"))
                .is_some(),
            "capabilities must be camelCase, got {json}"
        );
    }

    #[test]
    fn the_restore_report_serialises_in_camel_case() {
        let json = serde_json::to_value(super::RestoreReport {
            restored: Vec::new(),
            failed: vec![super::RestoreFailure {
                item_id: uuid::Uuid::nil(),
                error: "locked".to_owned(),
            }],
        })
        .expect("serialize");
        assert!(
            json.get("failed")
                .and_then(|f| f.get(0))
                .and_then(|f| f.get("itemId"))
                .is_some(),
            "got {json}"
        );
    }
}
