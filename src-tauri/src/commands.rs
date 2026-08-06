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
    RetentionPolicy,
};
use crate::core::settings;
use crate::core::state::{CancelRegistry, ScanResult, ScanStore};
use crate::core::types::ExecReport;
use crate::modules::cleaner::{self, CleanerContext, CleanerSummary};
use crate::modules::duplicates::{self, DuplicateProgress, DuplicateScan};
use crate::modules::shutdown_timer::{ScheduleMode, ShutdownTimer, TimerStatus};

/// Everything the settings page reads and writes, in one round trip.
///
/// One command rather than one per field: the page saves as a unit, and a
/// half-applied settings change is a state nobody has a story for.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Which cleaner packs run, and what each pack is.
    pub cleaner: CleanerSettingsView,
    /// Duplicate-finder settings.
    pub duplicates: duplicates::DuplicatesSettings,
    /// Quarantine retention, already validated.
    pub quarantine: QuarantineSettings,
}

/// The cleaner's settings plus enough about the shipped packs for the page to
/// render a list without a second command.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanerSettingsView {
    /// Every pack that loaded, in load order.
    pub packs: Vec<PackView>,
}

/// One rule pack as the settings page sees it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackView {
    /// Pack id, the key the toggle writes.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Whether it runs right now.
    pub enabled: bool,
    /// What it would do if the user had never touched it.
    pub enabled_by_default: bool,
    /// How many of its rules compiled on this machine.
    pub rule_count: usize,
}

/// Quarantine retention as the settings page sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineSettings {
    /// Days an item is kept before the purge takes it.
    pub retention_days: i64,
    /// Size cap in bytes; the oldest items are evicted past it.
    pub max_bytes: u64,
}

impl From<RetentionPolicy> for QuarantineSettings {
    fn from(policy: RetentionPolicy) -> Self {
        Self {
            retention_days: policy.retention_days,
            max_bytes: policy.max_bytes,
        }
    }
}

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
    db: State<'_, Database>,
    cancels: State<'_, CancelRegistry>,
) -> Result<ScanResult, ApiError> {
    if module_id != "cleaner" {
        return Err(CoreError::UnknownModule(module_id).into());
    }

    // Which packs run is the user's decision, re-read per scan so a change on
    // the settings page takes effect on the next scan rather than the next
    // restart.
    let settings: cleaner::CleanerSettings = db
        .with(|conn| settings::get_module(conn, "cleaner"))?
        .unwrap_or_default();

    let scan_id = Uuid::new_v4();
    let cancel = cancels.start(scan_id);
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
                settings: &settings,
                protected: services.brain.protected_paths(),
                recycle_bin: services.recycle_bin.as_ref(),
                now: Utc::now(),
                cancel: &cancel,
            },
            Some(&mut emit),
        )
    };

    cancels.finish(scan_id);

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
/// `confirm_empty_recycle_bin` is a separate answer to a separate question.
/// Everything else in a batch goes to quarantine and can be undone; emptying
/// the bin is final and takes every drive with it, and it used to trigger off a
/// selected id like anything else (security review H6). The dialog only sets
/// this when a recycle-bin item is in the selection, and without it that item
/// is skipped with a reason.
///
/// # Errors
/// Returns an error if the scan is unknown or the ledger is unusable. Item
/// failures land in the report.
#[tauri::command]
pub async fn execute<R: Runtime>(
    app: AppHandle<R>,
    scan_id: Uuid,
    approved: Vec<Uuid>,
    confirm_empty_recycle_bin: bool,
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
        executor::execute(
            &units,
            &env,
            &executor::ExecOptions {
                confirm_empty_recycle_bin,
            },
            Some(&mut emit),
        )
    })?;

    notify_quarantine_changed(&app, &services, &db, report.batch_id);
    Ok(report)
}

/// Ask a running scan to stop (ARCHITECTURE section 3).
///
/// Returns `false` when the scan already finished, which is not an error — the
/// user pressed stop as the last file went through.
///
/// # Errors
/// Infallible today; keeps the shared `Result` shape.
#[tauri::command]
pub fn cancel_scan(scan_id: Uuid, cancels: State<'_, CancelRegistry>) -> Result<bool, ApiError> {
    let stopped = cancels.cancel(scan_id);
    tracing::info!(scan = %scan_id, stopped, "cancel requested");
    Ok(stopped)
}

/* -------------------------------------------------------------------------
 * Duplicates: scan -> review -> execute
 * ---------------------------------------------------------------------- */

/// What a duplicate scan hands the UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateScanResult {
    /// Id for `execute` and `cancel_scan`.
    pub scan_id: Uuid,
    /// The sets, their copies and the excluded second names.
    pub scan: DuplicateScan,
    /// One recommendation per copy. Never `safe`.
    pub recommendations: Vec<crate::core::types::Recommendation>,
    /// Bytes the redundant copies occupy.
    pub wasted_bytes: u64,
}

/// Run a duplicate scan over the configured roots.
///
/// Blocking work; the caller runs it off the UI thread. Read-only.
///
/// # Errors
/// Returns an error only if the settings could not be read.
#[tauri::command]
pub async fn start_duplicate_scan<R: Runtime>(
    app: AppHandle<R>,
    roots: Option<Vec<PathBuf>>,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
    db: State<'_, Database>,
    cancels: State<'_, CancelRegistry>,
) -> Result<DuplicateScanResult, ApiError> {
    let stored: duplicates::DuplicatesSettings = db
        .with(|conn| settings::get_module(conn, "duplicates"))?
        .unwrap_or_default();
    let stored = stored.validated();

    // An explicit selection from the folder picker wins; then the saved roots;
    // then the profile folders module spec 03 names.
    let roots = roots
        .filter(|roots| !roots.is_empty())
        .or_else(|| Some(stored.roots.clone()).filter(|roots| !roots.is_empty()))
        .unwrap_or_else(|| duplicates::default_roots(&crate::core::platform::SystemEnv));

    let scan_id = Uuid::new_v4();
    let cancel = cancels.start(scan_id);
    let throttle = std::sync::Mutex::new(ProgressThrottle::new());

    let emit = |progress: &DuplicateProgress| {
        // The sink is called from several rayon threads at once, so the throttle
        // is shared rather than owned by one of them.
        let allowed = throttle
            .lock()
            .map(|mut throttle| throttle.should_emit())
            .unwrap_or(false);
        if !allowed {
            return;
        }
        let payload = ScanProgress {
            scan_id,
            module: "duplicates".to_owned(),
            items_scanned: progress.done,
            current_path: Some(progress.current_path.display().to_string()),
            pct: percent(progress.done, progress.total),
        };
        if let Err(err) = events::emit_scan_progress(&app, &payload) {
            tracing::debug!(error = %err, "duplicate progress event was not delivered");
        }
    };

    let scan = {
        let protected = services.brain.protected_paths();
        duplicates::scan(
            &duplicates::DuplicateContext {
                roots: &roots,
                protected,
                settings: &stored,
                cancel: &cancel,
            },
            Some(&emit),
        )
    };
    cancels.finish(scan_id);

    let recommendations = duplicates::analyze(&scan, &services.brain);
    tracing::info!(
        scan = %scan_id,
        sets = scan.sets.len(),
        hashed = scan.files_hashed,
        cancelled = scan.cancelled,
        "duplicate scan finished"
    );

    // Kept in the same store the cleaner uses, so `execute` needs no second
    // code path: a duplicate recommendation is a recommendation.
    store.insert(ScanResult {
        scan_id,
        module: "duplicates".to_owned(),
        findings: scan.findings.clone(),
        recommendations: recommendations.clone(),
        skipped_rules: scan.skipped_roots.clone(),
    });

    let _ = events::emit_scan_done(
        &app,
        &ScanDone {
            scan_id,
            module: "duplicates".to_owned(),
            finding_count: scan.findings.len() as u64,
        },
    );

    Ok(DuplicateScanResult {
        wasted_bytes: scan.wasted_bytes(),
        scan_id,
        scan,
        recommendations,
    })
}

fn percent(done: u64, total: u64) -> Option<u8> {
    if total == 0 {
        return None;
    }
    u8::try_from(done.saturating_mul(100) / total).ok()
}

/* -------------------------------------------------------------------------
 * Settings
 * ---------------------------------------------------------------------- */

/// Everything the settings page shows.
///
/// # Errors
/// Returns an error if the settings table cannot be read.
#[tauri::command]
pub fn get_settings(
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<AppSettings, ApiError> {
    Ok(db.with(|conn| read_settings(&services, conn))?)
}

/// Save the settings page.
///
/// Everything is re-validated on the way in, whatever the page sent. The
/// retention window goes through [`RetentionPolicy::validated`] and the
/// duplicate size floor through `DuplicatesSettings::validated`, because a
/// stored `0` used to mean "purge everything on the next start" (security review
/// H7) and a settings UI must not be able to write one.
///
/// Returns the settings as they were actually stored, so the page shows the
/// clamped values rather than the ones it asked for.
///
/// # Errors
/// Returns an error if the settings could not be written.
#[tauri::command]
pub fn set_settings(
    settings_in: AppSettings,
    services: State<'_, Services>,
    db: State<'_, Database>,
) -> Result<AppSettings, ApiError> {
    let policy = RetentionPolicy::validated(
        settings_in.quarantine.retention_days,
        settings_in.quarantine.max_bytes,
    );
    let duplicates = settings_in.duplicates.validated();

    let mut cleaner_settings = cleaner::CleanerSettings::default();
    for pack in &settings_in.cleaner.packs {
        cleaner_settings.set_enabled(pack.id.clone(), pack.enabled);
    }

    let stored = db.with(|conn| {
        settings::set_module(conn, "cleaner", &cleaner_settings)?;
        settings::set_module(conn, "duplicates", &duplicates)?;
        settings::set_raw(
            conn,
            settings::KEY_RETENTION_DAYS,
            &policy.retention_days.to_string(),
        )?;
        settings::set_raw(conn, settings::KEY_MAX_BYTES, &policy.max_bytes.to_string())?;
        read_settings(&services, conn)
    })?;

    tracing::info!(
        retention_days = policy.retention_days,
        max_bytes = policy.max_bytes,
        min_size_bytes = duplicates.min_size_bytes,
        "settings saved"
    );
    Ok(stored)
}

/// Read the current settings, joining the stored overrides onto the packs that
/// actually loaded.
fn read_settings(
    services: &Services,
    conn: &rusqlite::Connection,
) -> crate::core::error::Result<AppSettings> {
    let cleaner_settings: cleaner::CleanerSettings =
        settings::get_module(conn, "cleaner")?.unwrap_or_default();
    let duplicates = settings::get_module::<duplicates::DuplicatesSettings>(conn, "duplicates")?
        .unwrap_or_default()
        .validated();

    let packs = services
        .brain
        .packs()
        .iter()
        .map(|pack| PackView {
            id: pack.id().to_owned(),
            name: pack.spec.name.clone(),
            description: pack.spec.description.clone(),
            enabled: cleaner_settings.is_enabled(pack),
            enabled_by_default: pack.spec.enabled_by_default,
            rule_count: pack.rules.len(),
        })
        .collect();

    // Read back through `validated` as well, so a row somebody edited by hand in
    // the database shows up already clamped rather than as the value that would
    // have been dangerous.
    let policy = RetentionPolicy::validated(
        settings::get_i64(
            conn,
            settings::KEY_RETENTION_DAYS,
            crate::core::quarantine::DEFAULT_RETENTION_DAYS,
        )?,
        settings::get_u64(
            conn,
            settings::KEY_MAX_BYTES,
            crate::core::quarantine::DEFAULT_MAX_BYTES,
        )?,
    );

    Ok(AppSettings {
        cleaner: CleanerSettingsView { packs },
        duplicates,
        quarantine: policy.into(),
    })
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
/// Persist before arming. The other order left a live countdown running
/// whenever the write failed, while the UI reported the failure and showed no
/// timer — the worst of both (security review H4).
#[tauri::command]
pub fn shutdown_schedule(
    mode: ScheduleMode,
    action: PowerAction,
    timer: State<'_, Arc<ShutdownTimer>>,
    db: State<'_, Database>,
) -> Result<TimerStatus, ApiError> {
    let now = Utc::now();
    let persisted = ShutdownTimer::prepare(mode, action, now)?;
    db.with(|conn| {
        crate::core::settings::set_json(conn, crate::core::settings::KEY_ACTIVE_TIMER, &persisted)
    })?;
    timer.arm(persisted);
    Ok(timer.status(now))
}

/// Cancel the timer. Cancelling when nothing is armed is not an error.
///
/// Persistence is cleared *before* memory. The other order disarmed the live
/// timer and then, if the write failed, left the row behind — so the shutdown
/// the user had just cancelled came back on the next app start
/// (security review H3). Failing here leaves the timer armed and visibly so,
/// which is a state the user can act on.
///
/// # Errors
/// Returns an error only if the setting could not be cleared.
#[tauri::command]
pub fn shutdown_cancel(
    timer: State<'_, Arc<ShutdownTimer>>,
    db: State<'_, Database>,
) -> Result<TimerStatus, ApiError> {
    db.with(|conn| crate::core::settings::clear(conn, crate::core::settings::KEY_ACTIVE_TIMER))?;
    timer.cancel();
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
